use std::time::Duration;

/// Playback state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Repeat mode for the queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    None,
    All,
    One,
}

/// Playback position information.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaybackPosition {
    pub current: std::time::Duration,
    pub total: Option<std::time::Duration>,
}

/// Commands sent to the playback engine.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackCommand {
    Play(crate::domain::TrackId),
    Pause,
    Resume,
    Stop,
    Seek(std::time::Duration),
    SetVolume(f32),
    Next,
    Previous,
    PlayNext(crate::domain::TrackId),
    AddToQueue(crate::domain::TrackId),
    /// Append a batch of tracks in one command, so the queue mutates once
    /// under one lock (folder "play all" enqueues N tracks without N lock
    /// round-trips and N shuffle regenerations).
    AddMany(Vec<crate::domain::TrackId>),
    PlayPause,
}

/// Updates sent from the playback engine to the UI.
#[derive(Debug, Clone)]
pub enum PlaybackUpdate {
    StateChanged(PlaybackState),
    PositionChanged(PlaybackPosition),
    TrackChanged(crate::domain::TrackId),
    TrackEnded,
    Error(String),
}

/// Nanoseconds per second, as `u128` so intermediate products of
/// duration-to-sample conversions cannot overflow.
pub const NANOS_PER_SEC: u128 = 1_000_000_000;
/// Same constant as `u64` for use in contexts that do not need the wide type.
pub const NANOS_PER_SEC_U64: u64 = 1_000_000_000;

/// Convert a frame count to a [`Duration`] at the given sample rate using
/// exact integer arithmetic. Degenerate (zero) rates clamp to 1 Hz rather
/// than dividing by zero.
///
/// Playback-owned conversion math: the decoder adapter imports it from here
/// (not from application-layer policy), and it moves with the playback slice
/// when the backend splits into crates.
#[must_use]
pub fn duration_from_frames(frames: u64, rate: u32) -> Duration {
    let rate = u64::from(rate.max(1));
    let secs = frames / rate;
    let nanos = (frames % rate).saturating_mul(NANOS_PER_SEC_U64) / rate;
    // nanos < 1e9 always holds (nanos < NANOS_PER_SEC_U64), so this cannot fail.
    Duration::new(secs, u32::try_from(nanos).unwrap_or(0))
}

/// Convert a [`Duration`] to a frame count at the given sample rate using
/// exact integer arithmetic. Saturates instead of overflowing for durations
/// beyond ~584 billion years; degenerate (zero) rates clamp to 1 Hz.
///
/// Playback-owned conversion math: see [`duration_from_frames`].
#[must_use]
pub fn frames_from_duration(position: Duration, rate: u32) -> u64 {
    let rate = u128::from(u64::from(rate.max(1)));
    let total_nanos =
        u128::from(position.as_secs()) * NANOS_PER_SEC + u128::from(position.subsec_nanos());
    let frames = total_nanos * rate / NANOS_PER_SEC;
    u64::try_from(frames).unwrap_or(u64::MAX)
}
