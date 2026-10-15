//! Pure helpers for gapless playback (Task 4.1) and sample-exact position
//! tracking.
//!
//! The audio engine (`infra/audio_engine.rs`) is too hardware-bound to
//! exercise headlessly in CI (no audio device), so every decision the
//! engine makes about gapless handoffs is factored into pure functions here
//! and the engine calls THESE — the tested logic is the real logic.
//!
//! All time/sample conversions use pure integer arithmetic (no lossy
//! float-to-int casts), so they are exact and overflow-safe.
//!
//! App layer: std-only, no external crates.

use std::time::Duration;

pub use crate::domain::playback::{duration_from_frames, frames_from_duration};

/// Conditions that decide whether a gapless handoff may be attempted at EOF.
/// Grouped into one value so the decision function stays readable; see
/// [`is_gapless_eligible`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GaplessConditions {
    pub shuffle: bool,
    pub format_compatible: bool,
    pub has_successor: bool,
}

/// Whether a gapless handoff may be attempted at EOF. Shuffle disqualifies
/// because the successor is not predictable far enough ahead; an incompatible
/// format disqualifies because the running stream cannot be reused; no
/// successor disqualifies because there is nothing pre-buffered to flush.
/// Repeat-one handoffs reach this same predicate.
#[must_use]
pub fn is_gapless_eligible(conditions: GaplessConditions) -> bool {
    !conditions.shuffle && conditions.format_compatible && conditions.has_successor
}

/// Interleaved-sample count equivalent of a [`Duration`] at the given format:
/// `frames_from_duration` times the channel count. Saturates rather than
/// overflowing.
#[must_use]
pub fn samples_from_duration(position: Duration, rate: u32, channels: u16) -> usize {
    let frames = frames_from_duration(position, rate);
    let total_samples = u128::from(frames) * u128::from(channels);
    usize::try_from(total_samples).unwrap_or(usize::MAX)
}

/// Sample-exact elapsed time from a count of decoded interleaved samples.
///
/// `samples` counts individual interleaved values, so it is divided by the
/// channel count to get frames, then by the sample rate to get seconds.
/// Degenerate inputs (zero rate/channels) yield a defined result rather
/// than dividing by zero.
#[must_use]
pub fn elapsed_from_samples(samples: usize, rate: u32, channels: u16) -> Duration {
    // Degenerate inputs clamp to 1 so the math stays defined (no divide by
    // zero, no panic): the result is a defined estimate either way.
    let rate = rate.max(1);
    let channels = channels.max(1);
    let frames = samples / usize::from(channels);
    duration_from_frames(frames.try_into().unwrap_or(u64::MAX), rate)
}

/// Max seconds of pre-buffered successor audio (see the engine's
/// `PRE_BUFFER_SECONDS`). Inputs beyond this cap are nonsensical for a
/// pre-buffer and are clamped.
const MAX_PRE_BUFFER_SECONDS: f32 = 3600.0;

/// Number of interleaved samples that fit in `seconds` of audio at the given
/// format — the pre-buffer cap. Negative or non-finite input yields 0.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "the pre-buffer cap is a bounded estimate; saturation is harmless"
)]
pub fn pre_buffer_cap(rate: u32, channels: u16, seconds: f32) -> usize {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    let capped = seconds.min(MAX_PRE_BUFFER_SECONDS);
    let samples_per_sec = u64::from(rate) * u64::from(channels);
    (samples_per_sec as f64 * f64::from(capped)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::playback::{duration_from_frames, frames_from_duration};

    #[test]
    fn frames_duration_roundtrip() {
        let rate = 48000;
        let frames = 123_456_789;
        let d = duration_from_frames(frames, rate);
        let back = frames_from_duration(d, rate);
        assert_eq!(back, frames);
    }

    #[test]
    fn zero_rate_clamped() {
        let d = duration_from_frames(100, 0);
        assert_eq!(d, Duration::from_secs(100));
    }

    #[test]
    fn is_gapless_eligible_basic() {
        // The positive case: not shuffled, formats compatible, a successor exists.
        assert!(is_gapless_eligible(GaplessConditions {
            shuffle: false,
            format_compatible: true,
            has_successor: true,
        }));
        // Shuffle disqualifies: the successor is not predictable far enough ahead.
        assert!(!is_gapless_eligible(GaplessConditions {
            shuffle: true,
            format_compatible: true,
            has_successor: true,
        }));
        // Format incompatibility disqualifies: the running stream cannot be reused.
        assert!(!is_gapless_eligible(GaplessConditions {
            shuffle: false,
            format_compatible: false,
            has_successor: true,
        }));
        // No successor disqualifies: nothing is pre-buffered to flush.
        assert!(!is_gapless_eligible(GaplessConditions {
            shuffle: false,
            format_compatible: true,
            has_successor: false,
        }));
    }

    #[test]
    fn samples_from_duration_matches_manual() {
        let d = Duration::from_secs(2);
        let samples = samples_from_duration(d, 48000, 2);
        assert_eq!(samples, 48000 * 2 * 2);
    }

    #[test]
    fn elapsed_from_samples_inverse() {
        let d = Duration::from_millis(1234);
        let samples = samples_from_duration(d, 48000, 2);
        let back = elapsed_from_samples(samples, 48000, 2);
        assert_eq!(back, d);
    }

    #[test]
    fn pre_buffer_cap_clamped() {
        assert_eq!(pre_buffer_cap(48000, 2, 4.0), 48000 * 2 * 4);
        assert_eq!(pre_buffer_cap(48000, 2, -1.0), 0);
        assert_eq!(pre_buffer_cap(48000, 2, f32::NAN), 0);
        assert_eq!(pre_buffer_cap(48000, 2, 10000.0), 48000 * 2 * 3600);
    }
}
