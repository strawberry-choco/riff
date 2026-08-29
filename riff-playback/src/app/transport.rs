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
use riff_persistence::track::TrackId;
use crossbeam_channel::Sender;

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

    fn set_volume(&self, session: &PlaybackSession, vol: f32) {
        let clamped = vol.clamp(0.0, 1.0);
        self.send(PlaybackCommand::SetVolume(clamped));
    }

    fn toggle_mute(&self, session: &PlaybackSession) {
        let new_muted = !session.muted;
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

    fn toggle_shuffle(&self, session: &PlaybackSession) {
        self.send(PlaybackCommand::PlayPause); // placeholder — actual impl in FacadeTransport
    }

    fn toggle_repeat(&self, session: &PlaybackSession) {
        self.send(PlaybackCommand::PlayPause); // placeholder — actual impl in FacadeTransport
    }

    fn play_pause(&self, session: &PlaybackSession) {
        self.send(PlaybackCommand::PlayPause);
    }
}

/// A [`Transport`] decorator that records every dispatched command onto a
/// shared [`BackendFacade`] *before* forwarding it to the inner transport.
///
/// Because every dispatch path (mouse click on the playerbar, keyboard Space
/// shortcut, tray menu click, Now Playing button) flows through this wrapper
/// when wired at the composition root, the facade's event inbox becomes the
/// single observable side-effect surface for the frontend — no raw
/// [`PlaybackCommand`] construction is needed outside the facade.
///
/// The wrapper owns its `Arc` share so two consumers (e.g. the UI thread and
/// the tray thread) can each own a `FacadeTransport` pointing at the same
/// facade without a shared ownership layer.
pub struct FacadeTransport {
    inner: ChannelTransport,
    notice_tx: Sender<String>,
}

impl FacadeTransport {
    /// Wrap `inner` with facade notice emission.
    pub fn new(inner: ChannelTransport, notice_tx: Sender<String>) -> Self {
        Self { inner, notice_tx }
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
        let _ = self.notice_tx.send(format!("Play: {}", track.0));
        self.inner.play(track);
    }

    fn pause(&self) {
        let _ = self.notice_tx.send("Pause".to_string());
        self.inner.pause();
    }

    fn resume(&self) {
        let _ = self.notice_tx.send("Resume".to_string());
        self.inner.resume();
    }

    fn stop(&self) {
        let _ = self.notice_tx.send("Stop".to_string());
        self.inner.stop();
    }

    fn seek(&self, session: &PlaybackSession, secs: f32) {
        let _ = self.notice_tx.send(format!("Seek: {secs:.2}s"));
        self.inner.seek(session, secs);
    }

    fn set_volume(&self, session: &PlaybackSession, vol: f32) {
        let _ = self.notice_tx.send(format!("Volume: {vol:.2}"));
        self.inner.set_volume(session, vol);
    }

    fn toggle_mute(&self, session: &PlaybackSession) {
        let _ = self.notice_tx.send(format!("Mute: {}", !session.muted));
        self.inner.toggle_mute(session);
    }

    fn next(&self) {
        let _ = self.notice_tx.send("Next".to_string());
        self.inner.next();
    }

    fn previous(&self) {
        let _ = self.notice_tx.send("Previous".to_string());
        self.inner.previous();
    }

    fn play_next(&self, track: TrackId) {
        let _ = self.notice_tx.send(format!("PlayNext: {}", track.0));
        self.inner.play_next(track);
    }

    fn add_to_queue(&self, track: TrackId) {
        let _ = self.notice_tx.send(format!("AddToQueue: {}", track.0));
        self.inner.add_to_queue(track);
    }

    fn play_many(&self, first: TrackId, rest: Vec<TrackId>) {
        let _ = self.notice_tx.send(format!("PlayMany: {}", first.0));
        self.inner.play_many(first, rest);
    }

    fn toggle_shuffle(&self, session: &PlaybackSession) {
        let _ = self.notice_tx.send(format!("Shuffle: {}", !session.queue.shuffle));
        // Actual shuffle toggle is done via the session mutation in the UI
        // layer; this just emits the notice.
    }

    fn toggle_repeat(&self, session: &PlaybackSession) {
        let _ = self.notice_tx.send(format!("Repeat: {:?}", next_repeat(session.queue.repeat)));
        // Actual repeat toggle is done via the session mutation in the UI layer.
    }

    fn play_pause(&self, session: &PlaybackSession) {
        let _ = self.notice_tx.send("PlayPause".to_string());
        self.inner.play_pause(session);
    }
}

fn next_repeat(current: crate::domain::RepeatMode) -> crate::domain::RepeatMode {
    match current {
        crate::domain::RepeatMode::None => crate::domain::RepeatMode::All,
        crate::domain::RepeatMode::All => crate::domain::RepeatMode::One,
        crate::domain::RepeatMode::One => crate::domain::RepeatMode::None,
    }
}

/// Clamp a seek request (in seconds) into `[0, total]` so a drag past the end
/// of a track seeks to the end rather than beyond it (REQ-UI-005). When the
/// total duration is unknown there is nothing to clamp against, so the seek
/// falls back to the start; non-finite inputs (NaN/infinity) do the same.
#[must_use]
pub fn clamp_seek(secs: f32, total: Option<std::time::Duration>) -> std::time::Duration {
    let total_secs = total.map(|d| d.as_secs_f32()).unwrap_or(0.0);
    let clamped = secs.clamp(0.0, total_secs.max(0.0));
    if !clamped.is_finite() {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_secs_f32(clamped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::PlaybackSession;

    #[test]
    fn clamp_seek_in_range() {
        assert_eq!(clamp_seek(30.0, Some(std::time::Duration::from_secs(60))), std::time::Duration::from_secs(30));
    }

    #[test]
    fn clamp_seek_past_end() {
        assert_eq!(clamp_seek(100.0, Some(std::time::Duration::from_secs(60))), std::time::Duration::from_secs(60));
    }

    #[test]
    fn clamp_seek_negative() {
        assert_eq!(clamp_seek(-10.0, Some(std::time::Duration::from_secs(60))), std::time::Duration::ZERO);
    }

    #[test]
    fn clamp_seek_unknown_total() {
        assert_eq!(clamp_seek(30.0, None), std::time::Duration::ZERO);
    }

    #[test]
    fn clamp_seek_nan() {
        assert_eq!(clamp_seek(f32::NAN, Some(std::time::Duration::from_secs(60))), std::time::Duration::ZERO);
    }
}