//! The Playback Coordinator: applies [`PlaybackUpdate`]s to session state and
//! owns the record-then-move sequence — committing play history for the track
//! that just ended before asking [`Continuation`] what follows, and stopping
//! when nothing does. What follows is **Continuation**'s answer, not a rule
//! spelled here; the Audio Engine asks the same answer for a listener's skip.
//!
//! Threading follows the Audio Engine pattern: nothing here spawns threads in
//! its decision logic. [`PlaybackCoordinator::spawn`] is the composition
//! root's one-stop wiring: it constructs the coordinator and runs the recv
//! loop on a dedicated thread. The loop is a thin `recv` → core-call shell;
//! every decision lives in [`PlaybackCoordinator::apply_update`], which is
//! synchronous and callable without threads so tests can drive it directly.

use crate::app::state::PlaybackSession;
use crate::domain::continuation::{Continuation, Trigger};
use crate::domain::{PlaybackCommand, PlaybackState, PlaybackUpdate};
use crossbeam_channel::{Receiver, Sender};
use riff_persistence::store::LibraryMutationStore;
use riff_persistence::sync::MutexExt;
use std::sync::{Arc, Mutex};
/// Applies [`PlaybackUpdate`]s to the shared session state, continuing
/// playback through [`Continuation`] when a track ends.
pub struct PlaybackCoordinator {
    state: Arc<Mutex<PlaybackSession>>,
    update_rx: Receiver<PlaybackUpdate>,
    cmd_tx: Sender<PlaybackCommand>,
    mutations: Box<dyn LibraryMutationStore + Send>,
    /// Playback errors surface as typed notices through the event inbox's notice
    /// channel instead of a cross-slice state write: the coordinator sends
    /// the pre-formatted user-facing message here and never touches the
    /// library session's status slot.
    notice_tx: Sender<String>,
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
        use PlaybackUpdate::{Error, PositionChanged, StateChanged, TrackChanged, TrackEnded};
        let mut state = self.state.lock_or_recover();

        match update {
            StateChanged(s) => state.playback_state = s,
            PositionChanged(p) => state.current_position = p,
            TrackChanged(id) => {
                // The engine has chosen what is current; the arbiter moves the
                // queue's index onto it (and an id the queue does not hold
                // changes nothing).
                let _ = Continuation::settled_on(&mut state.queue, &id);
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
            let state = self.state.lock_or_recover();
            state.queue.current_track().cloned()
        };

        let Some(current_id) = current_id else {
            // No current track — nothing to record, just advance
            {
                let mut state = self.state.lock_or_recover();
                Self::advance_queue(&self.cmd_tx, &mut state);
            }
            return;
        };

        let played_at = std::time::SystemTime::now();

        if let Err(e) = self.mutations.record_track_played(&current_id, played_at) {
            let _ = self
                .notice_tx
                .send(format!("Failed to record play history: {e}"));
        }

        // Then advance the queue - drop the lock first
        {
            let mut state = self.state.lock_or_recover();
            Self::advance_queue(&self.cmd_tx, &mut state);
        }
    }
    fn advance_queue(cmd_tx: &Sender<PlaybackCommand>, state: &mut PlaybackSession) {
        // Play history for the track that just ended is committed by the
        // caller before asking here: that ordering is Continuation's stated
        // contract, and the queue has already moved by the time it answers.
        match Continuation::after(&mut state.queue, Trigger::TrackEnded) {
            Continuation::Play(id) => {
                let _ = cmd_tx.send(PlaybackCommand::Play(id));
            }
            Continuation::Stop => {
                // Nothing follows — stop, and drop the current index with it.
                state.playback_state = PlaybackState::Stopped;
                state.queue.current_index = None;
            }
        }
    }
}
