//! The Playback Coordinator: applies [`PlaybackUpdate`]s to session state and
//! owns playback continuation (the CONTEXT.md `PlaybackCoordinator` term) —
//! committing
//! play history before advancing, repeat-one re-play, auto-advance, and
//! stopping when nothing follows. It is the decider of queue continuation;
//! the Audio Engine only reports what happened.
//!
//! Threading follows the Audio Engine pattern: nothing here spawns threads in
//! its decision logic. [`PlaybackCoordinator::spawn`] is the composition
//! root's one-stop wiring: it constructs the coordinator and runs the recv
//! loop on a dedicated thread. The loop is a thin `recv` → core-call shell;
//! every decision lives in [`PlaybackCoordinator::apply_update`], which is
//! synchronous and callable without threads so tests can drive it directly.

use crate::app::state::AppState;
use crate::app::store::LibraryMutationStore;
use crate::domain::{PlaybackCommand, PlaybackState, PlaybackUpdate, RepeatMode};
use crossbeam_channel::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::app::MutexExt;

/// Applies [`PlaybackUpdate`]s to the shared session state and drives
/// auto-advance when a track ends.
pub struct PlaybackCoordinator {
    state: Arc<Mutex<AppState>>,
    update_rx: Receiver<PlaybackUpdate>,
    cmd_tx: Sender<PlaybackCommand>,
    mutations: Box<dyn LibraryMutationStore + Send>,
}

impl PlaybackCoordinator {
    /// Wire a coordinator over its shared state, the engine's update stream,
    /// the command channel back to the engine, and the Application Store's
    /// Library mutation port (for play history). Exposed separately from
    /// [`Self::spawn`] so tests can drive the synchronous core directly.
    #[must_use]
    pub fn new(
        state: Arc<Mutex<AppState>>,
        update_rx: Receiver<PlaybackUpdate>,
        cmd_tx: Sender<PlaybackCommand>,
        mutations: Box<dyn LibraryMutationStore + Send>,
    ) -> Self {
        Self {
            state,
            update_rx,
            cmd_tx,
            mutations,
        }
    }

    /// Spawn the coordinator's recv loop on a dedicated thread — exactly how
    /// the composition root runs the Audio Engine and the background service
    /// workers. Returns the thread handle; dropping it detaches the loop.
    pub fn spawn(
        state: Arc<Mutex<AppState>>,
        update_rx: Receiver<PlaybackUpdate>,
        cmd_tx: Sender<PlaybackCommand>,
        mutations: Box<dyn LibraryMutationStore + Send>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || Self::new(state, update_rx, cmd_tx, mutations).run())
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
        match update {
            PlaybackUpdate::StateChanged(new_state) => {
                self.state.lock_or_recover().playback_state = new_state;
            }
            PlaybackUpdate::PositionChanged(pos) => {
                self.state.lock_or_recover().current_position = pos;
            }
            PlaybackUpdate::TrackChanged(track_id) => {
                let mut locked = self.state.lock_or_recover();
                locked.queue.current_index =
                    locked.queue.tracks.iter().position(|id| id == &track_id);
            }
            PlaybackUpdate::TrackEnded => {
                self.handle_track_ended();
            }
            PlaybackUpdate::Error(msg) => {
                tracing::error!("Playback error: {}", msg);
                let mut locked = self.state.lock_or_recover();
                locked.playback_state = PlaybackState::Stopped;
                locked.scan_status = Some(format!("Playback error: {msg}"));
            }
        }
    }

    /// Record play history for the track that just finished — the queue's current
    /// track at this moment, before the auto-advance below moves the index — and
    /// advance the queue (or stop when nothing follows).
    ///
    /// The play commits to the Application Store FIRST as its own single durable
    /// transaction (ticket 06), so a crash right after the track ends cannot lose
    /// it; the mutation adapter bumps the session generation so Session
    /// Projections refetch.
    fn handle_track_ended(&mut self) {
        let finished_id = {
            let locked = self.state.lock_or_recover();
            locked.queue.current_track().cloned()
        };
        if let Some(finished_id) = finished_id {
            // The mutation adapter bumps the session generation when the play
            // commits; the mirror no longer tracks play history.
            match self
                .mutations
                .record_track_played(&finished_id, std::time::SystemTime::now())
            {
                Ok(true) => {}
                Ok(false) => tracing::debug!(?finished_id, "finished track is not in the store"),
                Err(e) => {
                    tracing::error!("Failed to persist play history for {finished_id:?}: {e}");
                }
            }
        }
        let next_track = {
            let mut locked = self.state.lock_or_recover();
            if locked.queue.repeat == RepeatMode::One {
                // Repeat-one loops the SAME track (Task 4.1): the queue
                // deliberately doesn't model it (`advance()` would move on), so
                // re-play the current track. If the engine already handed off
                // gaplessly, its Play(current) dedup guard swallows this no-op.
                locked.queue.current_track().cloned()
            } else {
                locked.queue.advance().cloned()
            }
        };
        if let Some(track_id) = next_track {
            let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
        } else {
            let mut locked = self.state.lock_or_recover();
            locked.playback_state = PlaybackState::Stopped;
        }
    }
}
