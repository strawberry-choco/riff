//! The Playback Coordinator: applies [`PlaybackUpdate`]s to session state and
//! owns playback continuation — committing play history before advancing,
//! repeat-one re-play, auto-advance, and stopping when nothing follows. It is
//! the decider of queue continuation; the Audio Engine only reports what
//! happened.
//!
//! Threading follows the Audio Engine pattern: nothing here spawns threads in
//! its decision logic. [`PlaybackCoordinator::spawn`] is the composition
//! root's one-stop wiring: it constructs the coordinator and runs the recv
//! loop on a dedicated thread. The loop is a thin `recv` → core-call shell;
//! every decision lives in [`PlaybackCoordinator::apply_update`], which is
//! synchronous and callable without threads so tests can drive it directly.

use crate::app::errors::StoreError;
use crate::domain::{PlaybackCommand, PlaybackPosition, PlaybackQueue, PlaybackState, PlaybackUpdate, RepeatMode};
use crossbeam_channel::{Receiver, Sender};
use riff_persistence::store::LibraryMutationStore;
use std::sync::{Arc, Mutex};
/// Applies [`PlaybackUpdate`]s to the shared session state and drives
/// auto-advance when a track ends.
pub struct PlaybackCoordinator {
    state: Arc<Mutex<PlaybackSession>>,
    update_rx: Receiver<PlaybackUpdate>,
    cmd_tx: Sender<PlaybackCommand>,
    mutations: Box<dyn LibraryMutationStore + Send>,
    /// Playback errors surface as typed notices through the facade's notice
    /// channel instead of a cross-slice state write: the coordinator sends
    /// the pre-formatted user-facing message here and never touches the
    /// library session's status slot.
    notice_tx: Sender<String>,
}

/// Playback session state — the fields the audio engine, coordinator, and
/// transport touch. Lives behind its own `Arc<Mutex<>>`, separate from the
/// Library Session, so no code path ever holds both session locks at once.
#[derive(Debug, Clone)]
pub struct PlaybackSession {
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    /// Mute flag: independent of `current_volume` — the slider keeps its
    /// value while muted. The engine always receives
    /// [`Self::effective_volume`], so a muted app stays silent until unmuted.
    pub muted: bool,
    /// `ReplayGain` flag: opt-in loudness normalization. When `true`, the
    /// engine applies each track's `REPLAYGAIN_TRACK_GAIN` (peak-capped) in
    /// the audio output's volume-scaling step.
    pub replaygain_enabled: bool,
}


impl PlaybackCoordinator {
    /// Wire a coordinator over its shared state, the engine's update stream,
    /// the command channel back to the engine, and the Application Store's
    /// Library mutation port (for play history). Exposed separately from
    /// [`Self::spawn`] so tests can drive the synchronous core directly.
    #[must_use]
    pub fn new(
        state: Arc<Mutex<PlaybackSession>>,
        update_rx: Receiver<PlaybackUpdate>,
        cmd_tx: Sender<PlaybackCommand>,
        mutations: Box<dyn LibraryMutationStore + Send>,
        notice_tx: Sender<String>,
    ) -> Self {
        Self {
            state,
            update_rx,
            cmd_tx,
            mutations,
            notice_tx,
        }
    }

    /// Spawn the coordinator's recv loop on a dedicated thread — exactly how
    /// the composition root runs the Audio Engine and the background service
    /// workers. Returns the thread handle; dropping it detaches the loop.
    pub fn spawn(
        state: Arc<Mutex<PlaybackSession>>,
        update_rx: Receiver<PlaybackUpdate>,
        cmd_tx: Sender<PlaybackCommand>,
        mutations: Box<dyn LibraryMutationStore + Send>,
        notice_tx: Sender<String>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || Self::new(state, update_rx, cmd_tx, mutations, notice_tx).run())
    }

    /// Block applying updates until every update sender is dropped. Spawns
    /// nothing; run this on the dedicated coordinator thread (or call
    /// [`Self::apply_update`] directly in tests).
    pub fn run(mut self) {
        while let Ok(update) = self.update_rx.recv() {
            self.apply_update(update);
        }
    }

    /// The synchronous core: apply one [`PlaybackUpdate`] to the session
    /// state, driving continuation when a track ends.
    pub fn apply_update(&mut self, update: PlaybackUpdate) {
        use PlaybackUpdate::*;
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        match update {
            StateChanged(s) => state.playback_state = s,
            PositionChanged(p) => state.current_position = p,
            TrackChanged(id) => {
                // Find the index of the new track in the queue
                if let Some(idx) = state.queue.tracks.iter().position(|t| t == &id) {
                    state.queue.current_index = Some(idx);
                }
            }
            TrackEnded => {
                // Drop the lock before calling handle_track_ended
                drop(state);
                self.handle_track_ended();
            }
            Error(msg) => {
                let _ = self.notice_tx.send(format!("Playback error: {msg}"));
            }
        }
    }

    /// Record play history for the track that just finished — the queue's
    /// current track at this moment, before the auto-advance below moves the
    /// index — and advance the queue (or stop when nothing follows).
    ///
    /// The play commits to the Application Store FIRST as its own single
    /// durable transaction, so a crash right after the track ends cannot
    /// lose it; the mutation adapter bumps the session generation so Session
    /// Projections refetch.
    fn handle_track_ended(&mut self) {
        let current_id = {
            let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.queue.current_track().cloned()
        };

        let Some(current_id) = current_id else {
            // No current track — nothing to record, just advance
            {
                let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                Self::advance_queue(&self.cmd_tx, &mut state);
            }
            return;
        };

        let played_at = std::time::SystemTime::now();

        if let Err(e) = self.mutations.record_track_played(&current_id, played_at) {
            let _ = self.notice_tx.send(format!("Failed to record play history: {e}"));
        }

        // Then advance the queue - drop the lock first
        {
            let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            Self::advance_queue(&self.cmd_tx, &mut state);
        }
    }
    fn advance_queue(cmd_tx: &Sender<PlaybackCommand>, state: &mut PlaybackSession) {
        if state.queue.tracks.is_empty() {
            state.playback_state = PlaybackState::Stopped;
            state.queue.current_index = None;
            return;
        }

        // Check for repeat-one loop
        let repeat_one = state.queue.repeat == RepeatMode::One
            && !state.queue.shuffle
            && state.queue.current_index.is_some();

        let next_track = if repeat_one {
            // Stay on the same track
            state.queue.current_track().cloned()
        } else {
            state.queue.advance().cloned()
        };

        match next_track {
            Some(id) => {
                // Send Play command for the next track
                let _ = cmd_tx.send(PlaybackCommand::Play(id.clone()));
            }
            None => {
                // Nothing follows — stop
                state.playback_state = PlaybackState::Stopped;
                state.queue.current_index = None;
            }
        }
    }
}

impl Default for PlaybackSession {
    fn default() -> Self {
        Self {
            queue: PlaybackQueue::default(),
            playback_state: PlaybackState::Stopped,
            current_position: PlaybackPosition::default(),
            current_volume: 1.0,
            muted: false,
            replaygain_enabled: false,
        }
    }
}

impl PlaybackSession {
    /// Effective volume the engine should use (respects mute).
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.current_volume
        }
    }
}