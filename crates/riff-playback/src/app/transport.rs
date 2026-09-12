//! The Transport port: the UI's intent-level playback interface.
//!
//! The UI never names [`PlaybackCommand`] variants or touches a raw channel;
//! it issues intents ([`Transport`]) and the [`ChannelTransport`] adapter
//! owns the mapping onto engine commands, seek clamping, and volume math.
//! Playback continuation stays with the `PlaybackCoordinator`, which keeps its
//! own raw channel (out of scope here).
//!
//! # Why the mutator methods take `&mut PlaybackSession`
//!
//! The port promises only what the adapter delivers. `seek` and `play_pause`
//! merely read the session (`&`); the mutators complete the user intent in
//! place — `set_volume` clamps and stores the slider value, `toggle_mute`
//! flips the flag, `toggle_shuffle`/`toggle_repeat` flip the queue state —
//! and send whatever the Audio Engine actually needs (a `SetVolume` carrying
//! the effective volume; nothing at all for shuffle/repeat, which the engine
//! reads off the shared session). Every UI call site hands the playback
//! session it already holds — never the library session — so no code path
//! ever holds both session locks at once.

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

    /// Set the volume to `vol` (0.0–1.0): clamp it, store it on the
    /// session, and send the engine a `SetVolume` carrying the effective
    /// (mute-aware) level.
    fn set_volume(&self, session: &mut PlaybackSession, vol: f32);

    /// Toggle mute: flip the session flag and send the engine a `SetVolume`
    /// carrying the effective level, so muting zeroes and unmuting restores
    /// what the engine hears while the slider keeps its value.
    fn toggle_mute(&self, session: &mut PlaybackSession);

    /// Skip to the next track.
    fn next(&self);

    /// Skip to the previous track.
    fn previous(&self);

    /// Queue `track` to play next (after current).
    fn play_next(&self, track: TrackId);
    /// Add `track` to the end of the queue.
    fn add_to_queue(&self, track: TrackId);

    /// Play `first`, append `rest` behind it as one batch (folder/album
    /// enqueue pattern): one `Play` plus a single `AddMany`, so the queue
    /// mutates once under one lock with one shuffle regeneration.
    fn play_many(&self, first: TrackId, rest: Vec<TrackId>);

    /// Toggle shuffle mode on the session queue. No command is sent — the
    /// engine reads the shared session.
    fn toggle_shuffle(&self, session: &mut PlaybackSession);

    /// Toggle repeat mode on the session queue (None → All → One → None).
    /// No command is sent — the engine reads the shared session.
    fn toggle_repeat(&self, session: &mut PlaybackSession);

    /// Toggle play/pause.
    fn play_pause(&self, session: &PlaybackSession);
}

/// Production adapter: maps [`Transport`] intents onto `PlaybackCommand`
/// sends over the UI's command channel. All methods are infallible — a send
/// only fails when the engine channel is closed, which is logged and then
/// dropped (the former `let _ = send(..)` semantics).
///
/// The optional recorder (see [`ChannelTransport::new_recording`]) is the
/// adapter's observability hook: when wired at the composition root, every
/// dispatched command is reported to the backend's event inbox before it is
/// forwarded, so mouse, keyboard, and tray paths all land on one observable
/// surface.
/// The adapter's observability hook: reports every dispatched command
/// synchronously before it is forwarded (see
/// [`ChannelTransport::new_recording`]).
pub type DispatchRecorder = dyn Fn(&PlaybackCommand) + Send + Sync;

pub struct ChannelTransport {
    cmd_tx: Sender<PlaybackCommand>,
    recorder: Option<Box<DispatchRecorder>>,
}

impl ChannelTransport {
    /// Create a new transport wrapping the given command channel. Nothing
    /// is recorded; observability is opt-in via [`Self::new_recording`].
    pub fn new(cmd_tx: Sender<PlaybackCommand>) -> Self {
        Self {
            cmd_tx,
            recorder: None,
        }
    }

    /// Create a recording transport: every dispatched command is reported
    /// through `recorder` synchronously before the command is forwarded.
    /// The composition root passes a closure that pushes the command onto
    /// the shared backend event inbox, keeping this crate decoupled from
    /// the backend's concrete event type.
    pub fn new_recording(cmd_tx: Sender<PlaybackCommand>, recorder: Box<DispatchRecorder>) -> Self {
        Self {
            cmd_tx,
            recorder: Some(recorder),
        }
    }

    fn send(&self, cmd: PlaybackCommand) {
        if let Some(recorder) = &self.recorder {
            recorder(&cmd);
        }
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

    fn set_volume(&self, session: &mut PlaybackSession, vol: f32) {
        session.current_volume = vol.clamp(0.0, 1.0);
        self.send(PlaybackCommand::SetVolume(session.effective_volume()));
    }

    fn toggle_mute(&self, session: &mut PlaybackSession) {
        session.muted = !session.muted;
        self.send(PlaybackCommand::SetVolume(session.effective_volume()));
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
        self.send(PlaybackCommand::AddMany(rest));
    }

    fn toggle_shuffle(&self, session: &mut PlaybackSession) {
        let was = session.queue.shuffle;
        session.queue.set_shuffle(!was);
    }

    fn toggle_repeat(&self, session: &mut PlaybackSession) {
        session.queue.toggle_repeat();
    }

    fn play_pause(&self, _session: &PlaybackSession) {
        self.send(PlaybackCommand::PlayPause);
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

    // --- Adapter behavior over a real command channel -----------------------

    use riff_persistence::track::TrackId;

    fn id(s: &str) -> TrackId {
        TrackId(s.to_string())
    }

    /// A transport wired to an inspectable command receiver.
    fn transport() -> (
        ChannelTransport,
        crossbeam_channel::Receiver<PlaybackCommand>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (ChannelTransport::new(tx), rx)
    }

    #[test]
    fn set_volume_sets_the_field_clamps_and_sends_the_effective_volume() {
        let (t, rx) = transport();
        let mut session = crate::app::state::PlaybackSession {
            muted: true,
            ..crate::app::state::PlaybackSession::default()
        };

        t.set_volume(&mut session, 2.0);
        assert!(
            (session.current_volume - 1.0).abs() < f32::EPSILON,
            "the volume clamps to 1.0"
        );
        assert_eq!(
            rx.try_recv().ok(),
            Some(PlaybackCommand::SetVolume(0.0)),
            "a muted app sends the muted (zero) effective volume"
        );

        session.muted = false;
        t.set_volume(&mut session, 0.7);
        assert!((session.current_volume - 0.7).abs() < f32::EPSILON);
        assert_eq!(
            rx.try_recv().ok(),
            Some(PlaybackCommand::SetVolume(0.7)),
            "the engine hears the new effective volume"
        );
    }

    #[test]
    fn toggle_mute_flips_the_flag_and_sends_the_effective_volume() {
        let (t, rx) = transport();
        let mut session = crate::app::state::PlaybackSession {
            current_volume: 0.7,
            ..crate::app::state::PlaybackSession::default()
        };

        t.toggle_mute(&mut session);
        assert!(session.muted);
        assert!(
            (session.current_volume - 0.7).abs() < f32::EPSILON,
            "muting keeps the slider value"
        );
        assert_eq!(
            rx.try_recv().ok(),
            Some(PlaybackCommand::SetVolume(0.0)),
            "muting zeroes what the engine hears"
        );
        assert!(
            rx.try_recv().is_err(),
            "one toggle sends exactly one command"
        );

        t.toggle_mute(&mut session);
        assert!(!session.muted);
        assert_eq!(
            rx.try_recv().ok(),
            Some(PlaybackCommand::SetVolume(0.7)),
            "unmuting restores the slider's volume"
        );
    }

    #[test]
    fn toggle_shuffle_flips_the_session_and_sends_nothing() {
        let (t, rx) = transport();
        let mut session = crate::app::state::PlaybackSession::default();

        t.toggle_shuffle(&mut session);
        assert!(session.queue.shuffle, "shuffle flips on");
        assert!(
            rx.try_recv().is_err(),
            "the engine reads the shared session — no command exists to send"
        );

        t.toggle_shuffle(&mut session);
        assert!(!session.queue.shuffle, "shuffle flips back off");
    }

    #[test]
    fn toggle_repeat_flips_the_session_and_sends_nothing() {
        let (t, rx) = transport();
        let mut session = crate::app::state::PlaybackSession::default();

        t.toggle_repeat(&mut session);
        assert_eq!(session.queue.repeat, crate::domain::RepeatMode::All);
        assert!(
            rx.try_recv().is_err(),
            "the engine reads the shared session — no command exists to send"
        );

        t.toggle_repeat(&mut session);
        assert_eq!(session.queue.repeat, crate::domain::RepeatMode::One);
    }

    #[test]
    fn play_many_sends_exactly_play_then_add_many() {
        let (t, rx) = transport();

        t.play_many(id("a"), vec![id("b"), id("c")]);

        assert_eq!(rx.try_recv().ok(), Some(PlaybackCommand::Play(id("a"))));
        assert_eq!(
            rx.try_recv().ok(),
            Some(PlaybackCommand::AddMany(vec![id("b"), id("c")])),
            "the batch mutates the queue once under one lock"
        );
        assert!(rx.try_recv().is_err(), "no per-track AddToQueue fan-out");
    }

    #[test]
    fn new_recording_reports_every_dispatched_command() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_for_recorder = std::sync::Arc::clone(&sink);
        let recorder = Box::new(move |cmd: &PlaybackCommand| {
            sink_for_recorder.lock().unwrap().push(format!("{cmd:?}"));
        });
        let t = ChannelTransport::new_recording(tx, recorder);
        let mut session = crate::app::state::PlaybackSession::default();

        t.play(id("a"));
        t.set_volume(&mut session, 0.5);

        let seen = sink.lock().unwrap();
        assert_eq!(seen.len(), 2, "every dispatch is recorded");
        assert!(
            seen[0].starts_with("Play("),
            "the play command is recorded first"
        );
        assert!(seen[1].starts_with("SetVolume("), "then the volume command");
        assert_eq!(rx.try_recv().ok(), Some(PlaybackCommand::Play(id("a"))));
    }

    #[test]
    fn plain_new_records_nothing() {
        let (t, _rx) = transport();
        let mut session = crate::app::state::PlaybackSession::default();
        // Compiles and runs without a recorder; observability is opt-in.
        t.play_pause(&session);
        t.set_volume(&mut session, 0.5);
    }
}
