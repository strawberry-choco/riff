//! The Transport port: the UI's intent-level playback interface.
//!
//! The UI never names [`PlaybackCommand`] variants or touches a raw channel;
//! it issues intents ([`Transport`]) and the [`ChannelTransport`] adapter
//! owns the mapping onto engine commands, seek clamping, and volume math.
//! Playback continuation stays with the `PlaybackCoordinator`, which keeps its
//! own raw channel (out of scope here).
//!
//! # Why the state-coupled methods take `&AppState`
//!
//! Seek clamping, effective volume, and mute flipping read live session
//! state. Every UI call site already holds the frame's `AppState` borrow
//! (the whole frame renders under one mutex guard), so the adapter receives
//! that borrow instead of locking a second handle to the same mutex — a
//! non-reentrant `std::sync::Mutex` would self-deadlock. The math still
//! lives entirely inside the implementation, exactly as the port intends.

use std::time::Duration;

use crossbeam_channel::Sender;

use crate::app::state::AppState;
use crate::domain::{PlaybackCommand, TrackId};

/// The UI's playback intents: what to play and how transport controls
/// behave, independent of how commands reach the Audio Engine.
pub trait Transport: Send {
    /// Start playing one track (replacing whatever plays).
    fn play(&self, track: TrackId);

    /// Play `first`, append `rest` behind it (folder/album enqueue pattern).
    fn play_many(&self, first: TrackId, rest: Vec<TrackId>);

    /// Queue a track to play right after the current one.
    fn play_next(&self, track: TrackId);

    /// Append a track to the end of the queue.
    fn add_to_queue(&self, track: TrackId);

    /// Pause playback.
    fn pause(&self);

    /// Resume playback.
    fn resume(&self);

    /// Advance to the next queue entry.
    fn next(&self);

    /// Step back to the previous queue entry.
    fn previous(&self);

    /// Stop playback.
    fn stop(&self);

    /// Seek to `position`, clamped against `state.current_position.total`
    /// inside the implementation so a drag past the end lands on the end.
    fn seek(&self, state: &AppState, position: Duration);

    /// Applies `state.effective_volume()` — the volume math lives here, so a
    /// muted app keeps sending zero while the slider value stays untouched.
    fn apply_volume_from_state(&self, state: &AppState);

    /// Flips mute in `state` and applies the resulting effective volume.
    fn toggle_mute(&self, state: &mut AppState);
}

/// Production adapter: maps [`Transport`] intents onto `PlaybackCommand`
/// sends over the UI's command channel. All methods are infallible — a send
/// only fails when the engine channel is closed, which is logged and then
/// dropped (the former `let _ = send(..)` semantics).
pub struct ChannelTransport {
    cmd_tx: Sender<PlaybackCommand>,
}

impl ChannelTransport {
    /// Wire the adapter to the UI's command-channel sender.
    pub fn new(cmd_tx: Sender<PlaybackCommand>) -> Self {
        Self { cmd_tx }
    }

    /// Infallible send: one warn per lost command, never a panic.
    fn send(&self, command: PlaybackCommand) {
        if let Err(e) = self.cmd_tx.send(command) {
            tracing::warn!("Playback command dropped, audio engine channel closed: {e}");
        }
    }
}

impl Transport for ChannelTransport {
    fn play(&self, track: TrackId) {
        self.send(PlaybackCommand::Play(track));
    }

    fn play_many(&self, first: TrackId, rest: Vec<TrackId>) {
        self.send(PlaybackCommand::Play(first));
        // The former folder-play site only batch-appended when something was
        // left after the first track; preserve that exact command sequence.
        if !rest.is_empty() {
            self.send(PlaybackCommand::AddMany(rest));
        }
    }

    fn play_next(&self, track: TrackId) {
        self.send(PlaybackCommand::PlayNext(track));
    }

    fn add_to_queue(&self, track: TrackId) {
        self.send(PlaybackCommand::AddToQueue(track));
    }

    fn pause(&self) {
        self.send(PlaybackCommand::Pause);
    }

    fn resume(&self) {
        self.send(PlaybackCommand::Resume);
    }

    fn next(&self) {
        self.send(PlaybackCommand::Next);
    }

    fn previous(&self) {
        self.send(PlaybackCommand::Previous);
    }

    fn stop(&self) {
        self.send(PlaybackCommand::Stop);
    }

    fn seek(&self, state: &AppState, position: Duration) {
        self.send(PlaybackCommand::Seek(clamp_seek(
            position.as_secs_f32(),
            state.current_position.total,
        )));
    }

    fn apply_volume_from_state(&self, state: &AppState) {
        self.send(PlaybackCommand::SetVolume(state.effective_volume()));
    }

    fn toggle_mute(&self, state: &mut AppState) {
        // Muting never moves the volume slider — it only zeroes the
        // effective volume sent to the engine; unmuting restores it.
        state.muted = !state.muted;
        self.send(PlaybackCommand::SetVolume(state.effective_volume()));
    }
}

/// Clamp a seek request (in seconds) into `[0, total]` so a drag past the end
/// of a track seeks to the end rather than beyond it (REQ-UI-005). When the
/// total duration is unknown there is nothing to clamp against, so the seek
/// falls back to the start; non-finite inputs (NaN/infinity) do the same.
pub fn clamp_seek(secs: f32, total: Option<std::time::Duration>) -> std::time::Duration {
    let finite = if secs.is_finite() { secs } else { 0.0 };
    let upper = total.map_or(0.0, |t| t.as_secs_f32());
    std::time::Duration::from_secs_f32(finite.clamp(0.0, upper.max(0.0)))
}
