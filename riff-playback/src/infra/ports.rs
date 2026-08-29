//! Port traits for playback infrastructure.
//!
//! Infrastructure implementations (symphonia decoder, cpal output) live in
//! `riff-infra` and implement these traits.

use std::time::Duration;

/// Factory for audio decoders: mints a fresh [`AudioDecoder`] on every call.
/// The audio engine uses it for both its primary decoder and the gapless
/// pre-decode decoder, so each owns independent codec state.
pub type DecoderFactory = Box<dyn Fn() -> Box<dyn AudioDecoder> + Send>;

/// Trait for audio decoders (implemented by infrastructure).
pub trait AudioDecoder: Send {
    /// The source path this decoder is reading from.
    fn source_path(&self) -> &std::path::Path;

    /// Initialize the decoder for the given track. Returns the audio format
    /// info (sample rate, channels) or an error if the file cannot be read.
    fn init(
        &mut self,
        path: &std::path::Path,
    ) -> Result<AudioFormatInfo, crate::app::errors::PlaybackError>;

    /// Decode the next chunk of audio into `buf` (interleaved f32 samples).
    /// Returns the number of samples written, or `None` at EOF.
    fn next_frames(&mut self, buf: &mut [f32]) -> Option<usize>;

    /// Seek to the given position. Returns the actual position seeked to.
    fn seek(&mut self, position: Duration) -> Duration;

    /// Get the total duration of the source, if known.
    fn duration(&self) -> Option<Duration>;
}

/// Audio format information returned by decoder.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFormatInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioFormatInfo {
    /// Check if this format is compatible with another for gapless handoff.
    #[must_use]
    pub fn compatible_with(&self, other: &AudioFormatInfo) -> bool {
        self.sample_rate == other.sample_rate && self.channels == other.channels
    }
}

/// Trait for audio output (implemented by infrastructure).
pub trait AudioOutput: Send {
    /// Start the output stream with the given format.
    fn start(&mut self, format: AudioFormatInfo) -> Result<(), crate::app::errors::PlaybackError>;

    /// Write decoded audio samples to the output.
    /// Returns the number of samples accepted (may be less than input if buffer is full).
    fn write(&mut self, samples: &[f32]) -> usize;

    /// Stop the output stream.
    fn stop(&mut self);

    /// Set the output volume (0.0–1.0).
    fn set_volume(&mut self, volume: f32);

    /// Get the current output latency (frames).
    fn latency(&self) -> u32;
}
