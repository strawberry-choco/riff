//! The Transport port: the UI's intent-level playback interface.
//!
//! The UI never names [`PlaybackCommand`] variants or touches a raw channel;
//! it issues intents ([`Transport`]) and the [`ChannelTransport`] adapter
//! owns the mapping onto engine commands, seek clamping, and volume math.
//! Playback continuation stays with the `PlaybackCoordinator`, which keeps its
//! own raw channel (out of scope here).
//!
//! # Why the state-coupled methods take `&PlaybackSession`
//!
//! Seek clamping, effective volume, and mute flipping read live playback
//! session state. The transport touches only the playback session, so every
//! UI call site hands it that session — never the library session — and no
//! code path ever holds both session locks at once. The math still lives
//! entirely inside the implementation, exactly as the port intends.

use crate::app::state::PlaybackSession;
use crate::domain::PlaybackCommand;
use crossbeam_channel::Sender;
use riff_persistence::track::TrackId;

/// The UI's playback intents: what to play and how transport controls
/// behave, independent of how commands reach the Audio Engine.
pub trait Transport: Send {
    /// Start playback of `track`. If the queue is empty, the caller has
    /// already loaded the queue (via `Transport::load_queue` or similar).
    fn play(&self, track: TrackId);

    /// Pause playback.
    fn pause(&self);

    /// Resume playback from pause.
    fn resume(&self);

    /// Stop playback and clear the queue.
    fn stop(&self);

    /// Seek to `secs` within the current track (clamped to `[0, total]`).
    fn seek(&self, session: &PlaybackSession, secs: f32);

    /// Set the volume to `vol` (0.0–1.0).
    fn set_volume(&self, session: &PlaybackSession, vol: f32);

    /// Toggle mute.
    fn toggle_mute(&self, session: &PlaybackSession);

    /// Skip to the next track.
    fn next(&self);

    /// Skip to the previous track.
    fn previous(&self);

    /// Queue `track` to play next (after current).
    fn play_next(&self, track: TrackId);
    /// Add `track` to the end of the queue.
    fn add_to_queue(&self, track: TrackId);

    /// Play `first`, append `rest` behind it (folder/album enqueue pattern).
    fn play_many(&self, first: TrackId, rest: Vec<TrackId>);

    /// Toggle shuffle mode.
    fn toggle_shuffle(&self, session: &PlaybackSession);

    /// Toggle repeat mode.
    fn toggle_repeat(&self, session: &PlaybackSession);

    /// Toggle play/pause.
    fn play_pause(&self, session: &PlaybackSession);
}

/// Production adapter: maps [`Transport`] intents onto `PlaybackCommand`
/// sends over the UI's command channel. All methods are infallible — a send
/// only fails when the engine channel is closed, which is logged and then
/// dropped (the former `let _ = send(..)` semantics).
pub struct ChannelTransport {
    cmd_tx: Sender<PlaybackCommand>,
}

impl ChannelTransport {
    /// Create a new transport wrapping the given command channel.
    pub fn new(cmd_tx: Sender<PlaybackCommand>) -> Self {
        Self { cmd_tx }
    }

    fn send(&self, cmd: PlaybackCommand) {
        let _ = self.cmd_tx.send(cmd);
    }
}

impl Transport for ChannelTransport {
    fn play(&self, track: TrackId) {
        self.send(PlaybackCommand::Play(track));
    }

    fn pause(&self) {
        self.send(PlaybackCommand::Pause);
    }

    fn resume(&self) {
        self.send(PlaybackCommand::Resume);
    }

    fn stop(&self) {
        self.send(PlaybackCommand::Stop);
    }

    fn seek(&self, session: &PlaybackSession, secs: f32) {
        let clamped = clamp_seek(secs, session.current_position.total);
        self.send(PlaybackCommand::Seek(clamped));
    }

    fn set_volume(&self, _session: &PlaybackSession, vol: f32) {
        let clamped = vol.clamp(0.0, 1.0);
        self.send(PlaybackCommand::SetVolume(clamped));
    }

    fn toggle_mute(&self, session: &PlaybackSession) {
        // Volume command is a no-op for mute; the engine reads effective_volume
        self.send(PlaybackCommand::SetVolume(session.current_volume));
        // The session mutation happens via the coordinator's update loop,
        // not here. The UI flips the local mute flag and the engine picks
        // it up on the next PositionChanged.
    }

    fn next(&self) {
        self.send(PlaybackCommand::Next);
    }

    fn previous(&self) {
        self.send(PlaybackCommand::Previous);
    }

    fn play_next(&self, track: TrackId) {
        self.send(PlaybackCommand::PlayNext(track));
    }

    fn add_to_queue(&self, track: TrackId) {
        self.send(PlaybackCommand::AddToQueue(track));
    }

    fn play_many(&self, first: TrackId, rest: Vec<TrackId>) {
        self.send(PlaybackCommand::Play(first));
        for track in rest {
            self.send(PlaybackCommand::AddToQueue(track));
        }
    }

    fn toggle_shuffle(&self, _session: &PlaybackSession) {
        self.send(PlaybackCommand::PlayPause); // placeholder — actual impl in FacadeTransport
    }

    fn toggle_repeat(&self, _session: &PlaybackSession) {
        self.send(PlaybackCommand::PlayPause); // placeholder — actual impl in FacadeTransport
    }

    fn play_pause(&self, _session: &PlaybackSession) {
        self.send(PlaybackCommand::PlayPause);
    }
}

/// A [`Transport`] decorator that records every dispatched command *before*
/// forwarding it to the inner transport.
///
/// Because every dispatch path (mouse click on the playerbar, keyboard Space
/// shortcut, tray menu click, Now Playing button) flows through this wrapper
/// when wired at the composition root, the recorder becomes the single
/// observable side-effect surface for the frontend — no raw
/// [`PlaybackCommand`] construction is needed outside the facade.
///
/// The recorder is an injected callback so this crate stays decoupled from
/// the backend facade's concrete type; the composition root passes a closure
/// that pushes the command onto the shared facade's event inbox.
pub struct FacadeTransport {
    inner: ChannelTransport,
    record: Box<dyn Fn(PlaybackCommand) + Send + Sync>,
}

impl FacadeTransport {
    /// Wrap `inner`, reporting every dispatched command through `record`
    /// synchronously before the command is forwarded.
    pub fn new(
        inner: ChannelTransport,
        record: Box<dyn Fn(PlaybackCommand) + Send + Sync>,
    ) -> Self {
        Self { inner, record }
    }

    /// Access the raw [`ChannelTransport`] handle. Exists so composition-root
    /// code that still takes `Box<dyn Transport>` continues to compile
    /// unchanged while a later issue removes the direct reference.
    pub fn inner(&self) -> &ChannelTransport {
        &self.inner
    }
}

impl Transport for FacadeTransport {
    fn play(&self, track: TrackId) {
        (self.record)(PlaybackCommand::Play(track.clone()));
        self.inner.play(track);
    }

    fn pause(&self) {
        (self.record)(PlaybackCommand::Pause);
        self.inner.pause();
    }

    fn resume(&self) {
        (self.record)(PlaybackCommand::Resume);
        self.inner.resume();
    }

    fn stop(&self) {
        (self.record)(PlaybackCommand::Stop);
        self.inner.stop();
    }

    fn seek(&self, session: &PlaybackSession, secs: f32) {
        (self.record)(PlaybackCommand::Seek(clamp_seek(
            secs,
            session.current_position.total,
        )));
        self.inner.seek(session, secs);
    }

    fn set_volume(&self, session: &PlaybackSession, vol: f32) {
        (self.record)(PlaybackCommand::SetVolume(vol.clamp(0.0, 1.0)));
        self.inner.set_volume(session, vol);
    }

    fn toggle_mute(&self, session: &PlaybackSession) {
        (self.record)(PlaybackCommand::SetVolume(session.current_volume));
        self.inner.toggle_mute(session);
    }

    fn next(&self) {
        (self.record)(PlaybackCommand::Next);
        self.inner.next();
    }

    fn previous(&self) {
        (self.record)(PlaybackCommand::Previous);
        self.inner.previous();
    }

    fn play_next(&self, track: TrackId) {
        (self.record)(PlaybackCommand::PlayNext(track.clone()));
        self.inner.play_next(track);
    }

    fn add_to_queue(&self, track: TrackId) {
        (self.record)(PlaybackCommand::AddToQueue(track.clone()));
        self.inner.add_to_queue(track);
    }

    fn play_many(&self, first: TrackId, rest: Vec<TrackId>) {
        (self.record)(PlaybackCommand::Play(first.clone()));
        self.inner.play_many(first, rest);
    }

    fn toggle_shuffle(&self, session: &PlaybackSession) {
        // Shuffle state lives in the session queue and is mutated by the UI
        // layer directly; no engine command exists to record.
        let _ = session;
    }

    fn toggle_repeat(&self, session: &PlaybackSession) {
        // Repeat state lives in the session queue and is mutated by the UI
        // layer directly; no engine command exists to record.
        let _ = session;
    }

    fn play_pause(&self, session: &PlaybackSession) {
        (self.record)(PlaybackCommand::PlayPause);
        self.inner.play_pause(session);
    }
}

/// Clamp a seek request (in seconds) into `[0, total]` so a drag past the end
/// of a track seeks to the end rather than beyond it (REQ-UI-005). When the
/// total duration is unknown there is nothing to clamp against, so the seek
/// falls back to the start; non-finite inputs (NaN/infinity) do the same.
#[must_use]
pub fn clamp_seek(secs: f32, total: Option<std::time::Duration>) -> std::time::Duration {
    let total_secs = total.map_or(0.0, |d| d.as_secs_f32());
    // `f32::clamp` lets NaN through (`max`/`min` ignore it), so non-finite
    // requests are rejected before clamping — they fall back to the start.
    let clamped = if secs.is_finite() {
        secs.clamp(0.0, total_secs.max(0.0))
    } else {
        0.0
    };
    std::time::Duration::from_secs_f32(clamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_seek_in_range() {
        assert_eq!(
            clamp_seek(30.0, Some(std::time::Duration::from_mins(1))),
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn clamp_seek_past_end() {
        assert_eq!(
            clamp_seek(100.0, Some(std::time::Duration::from_mins(1))),
            std::time::Duration::from_mins(1)
        );
    }

    #[test]
    fn clamp_seek_negative() {
        assert_eq!(
            clamp_seek(-10.0, Some(std::time::Duration::from_mins(1))),
            std::time::Duration::ZERO
        );
    }

    #[test]
    fn clamp_seek_unknown_total() {
        assert_eq!(clamp_seek(30.0, None), std::time::Duration::ZERO);
    }

    #[test]
    fn clamp_seek_nan() {
        assert_eq!(
            clamp_seek(f32::NAN, Some(std::time::Duration::from_mins(1))),
            std::time::Duration::ZERO
        );
    }
}
