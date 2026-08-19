//! Pure helpers for gapless playback (Task 4.1) and sample-exact position
//! tracking.
//!
//! The audio engine lives in the binary (`src/main.rs`) and cannot be
//! unit-tested headlessly (no audio device in CI), so every decision the
//! engine makes about gapless handoffs is factored into pure functions here
//! and the engine calls THESE — the tested logic is the real logic.
//!
//! All time/sample conversions use pure integer arithmetic (no lossy
//! float-to-int casts), so they are exact and overflow-safe.
//!
//! App layer: std-only, no external crates.

use std::time::Duration;

/// Nanoseconds per second, as `u128` so intermediate products of
/// duration-to-sample conversions cannot overflow.
const NANOS_PER_SEC: u128 = 1_000_000_000;
/// Same constant as `u64` for use in contexts that do not need the wide type.
const NANOS_PER_SEC_U64: u64 = 1_000_000_000;

/// Gapless handoff keeps the same cpal stream running across the track
/// boundary, so it requires the current and next track to share the exact
/// same effective sample rate and channel count. Any mismatch means the
/// existing gapped path (stop → reinitialize → restart) must run instead.
#[must_use]
pub fn formats_gapless_compatible(
    cur_rate: u32,
    cur_ch: u16,
    next_rate: u32,
    next_ch: u16,
) -> bool {
    cur_rate == next_rate && cur_ch == next_ch
}

/// Queue-derived conditions affecting gapless eligibility, grouped so the
/// decision inputs stay cohesive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueConditions {
    /// Shuffle mode: the successor is not predictable far enough ahead.
    pub shuffle: bool,
    /// Repeat-one looping is handled by its own engine EOF branch.
    pub repeat_one: bool,
}

/// Conditions that decide whether a gapless handoff may be attempted at EOF.
/// Grouped into one value so the decision function stays readable; see
/// [`is_gapless_eligible`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GaplessConditions {
    pub queue: QueueConditions,
    /// Current and successor formats share rate/channels.
    pub formats_compatible: bool,
    /// A pre-buffered successor exists and is the queue's natural successor.
    pub has_successor: bool,
}

/// Whether a gapless handoff to the NATURAL SUCCESSOR may be attempted at
/// EOF. Repeat-one looping is a separate handoff case (the same track loops,
/// decided by the caller), so `repeat_one` disqualifies here. Shuffle is
/// excluded because the next track is not predictable far enough ahead.
#[must_use]
pub fn is_gapless_eligible(conditions: GaplessConditions) -> bool {
    !conditions.queue.shuffle
        && !conditions.queue.repeat_one
        && conditions.formats_compatible
        && conditions.has_successor
}

/// Convert a frame count to a [`Duration`] at the given sample rate using
/// exact integer arithmetic. Degenerate (zero) rates clamp to 1 Hz rather
/// than dividing by zero.
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
#[must_use]
pub fn frames_from_duration(position: Duration, rate: u32) -> u64 {
    let rate = u128::from(u64::from(rate.max(1)));
    let total_nanos =
        u128::from(position.as_secs()) * NANOS_PER_SEC + u128::from(position.subsec_nanos());
    let frames = total_nanos * rate / NANOS_PER_SEC;
    u64::try_from(frames).unwrap_or(u64::MAX)
}

/// Interleaved-sample count equivalent of a [`Duration`] at the given format:
/// `frames_from_duration` times the channel count. Saturates rather than
/// overflowing.
#[must_use]
pub fn samples_from_duration(position: Duration, rate: u32, channels: u16) -> usize {
    let frames = frames_from_duration(position, rate);
    let samples = frames.saturating_mul(u64::from(channels));
    usize::try_from(samples).unwrap_or(usize::MAX)
}

/// Sample-exact elapsed time from a count of decoded interleaved samples.
///
/// `samples` counts individual interleaved values, so it is divided by the
/// channel count to get frames, then by the sample rate to get seconds.
/// Degenerate inputs (zero rate/channels) yield a defined result rather
/// than dividing by zero.
#[must_use]
pub fn elapsed_from_samples(samples: usize, rate: u32, channels: u16) -> Duration {
    let frame_count = samples / usize::from(channels).max(1);
    duration_from_frames(frame_count as u64, rate)
}

/// Max seconds of pre-buffered successor audio (see the engine's
/// `PRE_BUFFER_SECONDS`). Inputs beyond this cap are nonsensical for a
/// pre-buffer and are clamped.
const MAX_PRE_BUFFER_SECONDS: f32 = 3600.0;

/// Number of interleaved samples that fit in `seconds` of audio at the given
/// format — the pre-buffer cap. Negative or non-finite input yields 0.
#[must_use]
pub fn pre_buffer_cap(rate: u32, channels: u16, seconds: f32) -> usize {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    // Clamp so the integer conversion below stays in range; a pre-buffer of
    // more than an hour of audio is never requested.
    let capped = f64::from(seconds.min(MAX_PRE_BUFFER_SECONDS));
    let duration = Duration::from_secs_f64(capped);
    // frames-per-second * seconds, computed as integer nanosecond math.
    let frames_per_sec = u128::from(rate) * u128::from(channels);
    let total_nanos =
        u128::from(duration.as_secs()) * NANOS_PER_SEC + u128::from(duration.subsec_nanos());
    let samples = total_nanos * frames_per_sec / NANOS_PER_SEC;
    usize::try_from(samples).unwrap_or(usize::MAX)
}
