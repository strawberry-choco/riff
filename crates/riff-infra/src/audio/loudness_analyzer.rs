//! The `ReplayGain` 2.0 measurement adapter: the library capability's
//! [`LoudnessAnalyzer`] port served over the same decode stack playback uses
//! plus the pure-Rust `ebur128` crate (ADR 0013). One decode of the file
//! feeds every sample through the BS.1770 K-weighting filter; the integrated
//! loudness against the −18 LUFS reference becomes the track gain, and the
//! true peak (4× oversampled, so inter-sample overshoots are caught) becomes
//! the peak that caps it.
//!
//! A Track with no loudness verdict — digital silence, which the BS.1770 gate
//! reports as no energy at all — lands the domain's no-verdict pair instead of
//! an infinite gain, so what this port hands back is always finite and writable.

use crate::audio::decoder::{SymphoniaDecoder, default_codec_registry};
use riff_library::app::errors::LibraryError;
use riff_library::app::replaygain::{
    REPLAYGAIN_FALLBACK_GAIN_DB, REPLAYGAIN_FALLBACK_PEAK, REPLAYGAIN_REFERENCE_LUFS,
};
use riff_library::app::traits::{LoudnessAnalyzer, TrackLoudness};
use std::path::Path;

/// Interleaved sample buffer per decode step: large enough that the packet
/// loop is I/O-bound, small enough to stay allocation-flat.
const DECODE_CHUNK_SAMPLES: usize = 8192 * 2;

/// The analyzer is stateless: every measurement opens its own decoder over a
/// fresh codec registry, exactly the way the engine's decoder factory does.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ebur128LoudnessAnalyzer;

impl Ebur128LoudnessAnalyzer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl LoudnessAnalyzer for Ebur128LoudnessAnalyzer {
    fn measure_track(&self, path: &Path) -> Result<TrackLoudness, LibraryError> {
        let failure =
            |context: String, e: String| move || LibraryError::Loudness(format!("{context}: {e}"));

        let mut decoder = SymphoniaDecoder::new(default_codec_registry());
        let format = decoder
            .open(path)
            .map_err(|e| failure("decode failed".to_string(), e.to_string())())?;

        let channels = u32::from(format.channels).max(1);
        let mut meter = ebur128::EbuR128::new(
            channels,
            format.sample_rate,
            ebur128::Mode::I | ebur128::Mode::TRUE_PEAK,
        )
        .map_err(|e| failure("meter init failed".to_string(), e.to_string())())?;

        let mut chunk = vec![0.0_f32; DECODE_CHUNK_SAMPLES];
        loop {
            let written = decoder
                .next_frames(&mut chunk)
                .map_err(|e| failure("decode failed".to_string(), e.to_string())())?;
            if written == 0 {
                break;
            }
            meter
                .add_frames_f32(&chunk[..written])
                .map_err(|e| failure("metering failed".to_string(), e.to_string())())?;
        }

        let loudness = meter
            .loudness_global()
            .map_err(|e| failure("no integrated loudness".to_string(), e.to_string())())?;
        let mut peak = 0.0_f64;
        for channel in 0..channels {
            let channel_peak = meter
                .true_peak(channel)
                .map_err(|e| failure("no true peak".to_string(), e.to_string())())?;
            peak = peak.max(channel_peak);
        }

        // Narrowing f64 meter outputs into the port's f32 DTO: the
        // measurement's precision is well inside either width.
        #[allow(clippy::cast_possible_truncation)]
        let measured_gain = (REPLAYGAIN_REFERENCE_LUFS - loudness) as f32;
        #[allow(clippy::cast_possible_truncation)]
        let measured_peak = peak as f32;

        // A file with no signal has no integrated loudness — the gated
        // measurement is −inf — so its gain comes back infinite, and a peak
        // derived from no samples at all can come back non-finite too. Neither
        // is a value a tag, a player, or an Album aggregate can honor, so the
        // measurement falls back to the domain's no-verdict pair rather than
        // failing: the Track lands finite facts, plays unadjusted, and counts
        // as measured. A silent Track's peak is a real measurement of 0.0, so
        // only a non-finite value is ever replaced here.
        let track_gain_db = if measured_gain.is_finite() {
            measured_gain
        } else {
            REPLAYGAIN_FALLBACK_GAIN_DB
        };
        let track_peak = if measured_peak.is_finite() {
            measured_peak
        } else {
            REPLAYGAIN_FALLBACK_PEAK
        };

        Ok(TrackLoudness {
            track_gain_db,
            track_peak,
        })
    }
}
