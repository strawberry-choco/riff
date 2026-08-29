use crate::app::errors::{LibraryError, PlaybackError};
use crate::domain::{CoverSource, TrackMetadata};
use std::path::Path;
use std::time::Duration;

/// Factory for audio decoders: mints a fresh [`AudioDecoder`] on every call.
/// The audio engine uses it for both its primary decoder and the gapless
/// pre-decode decoder, so each owns independent codec state.
pub type DecoderFactory = Box<dyn Fn() -> Box<dyn AudioDecoder> + Send>;

/// Trait for audio decoders (implemented by infrastructure).
pub trait AudioDecoder: Send {
    fn open(&mut self, path: &Path) -> Result<AudioFormatInfo, PlaybackError>;
    /// Decode the next packet of interleaved f32 samples into `out`,
    /// returning the number of samples written. Callers reuse one buffer
    /// across calls so steady-state decoding performs no per-chunk heap
    /// allocations. Returns `Ok(0)` at end of stream.
    ///
    /// A short fill (fewer samples than `out.len()`) is normal: one call
    /// never spans more than one decoded packet.
    fn next_frames(&mut self, out: &mut [f32]) -> Result<usize, PlaybackError>;
    fn seek(&mut self, position: Duration) -> Result<(), PlaybackError>;
    fn duration(&self) -> Option<Duration>;
    /// Release the currently open file's resources (format reader, decoder,
    /// sample buffer) without opening a new file. Safe to call when nothing is
    /// open. Subsequent `next_frames`/`seek` calls will error until `open`.
    fn close(&mut self) {}
}

/// Audio format information returned by decoder.
#[derive(Debug, Clone)]
pub struct AudioFormatInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Option<Duration>,
}

/// Trait for audio output (implemented by infrastructure).
pub trait AudioOutput: Send {
    fn initialize(&mut self, sample_rate: u32, channels: u16) -> Result<(), PlaybackError>;
    fn start(&mut self) -> Result<(), PlaybackError>;
    fn stop(&mut self) -> Result<(), PlaybackError>;
    fn write_samples(&mut self, samples: &[f32]) -> Result<usize, PlaybackError>;
    fn set_volume(&mut self, volume: f32);
    /// Set the `ReplayGain` linear factor (Task 4.3) applied alongside volume
    /// in the sample-scaling step. Default no-op so mocks/simple outputs need
    /// not implement it; `1.0` means no adjustment.
    fn set_replaygain(&mut self, _factor: f32) {}
    /// The sample rate the output stream was ACTUALLY built with (Task 4.1).
    /// On Windows WASAPI shared mode the device often locks to its default
    /// rate (commonly 48 kHz) regardless of the requested rate, so this can
    /// differ from the rate passed to [`Self::initialize`]. The gapless
    /// format-compatibility gate compares against it. Defaults to the 44.1
    /// kHz startup value.
    fn effective_sample_rate(&self) -> u32 {
        44_100
    }
    /// Number of samples currently queued in the output buffer.
    fn buffer_len(&self) -> usize;
    /// Discard all queued samples without playing them.
    fn clear_buffer(&mut self);
}

/// Trait for metadata readers (implemented by infrastructure).
pub trait MetadataReader: Send + Sync {
    fn read_metadata(&self, path: &Path) -> Result<TrackMetadata, LibraryError>;
    fn read_duration(&self, path: &Path) -> Result<Option<Duration>, LibraryError>;
    fn read_cover_source(&self, path: &Path) -> Result<CoverSource, LibraryError>;
    fn read_audio_format(&self, path: &Path) -> Result<AudioFormatInfo, LibraryError>;
    fn read_all(
        &self,
        path: &Path,
    ) -> Result<
        (
            TrackMetadata,
            Option<Duration>,
            CoverSource,
            AudioFormatInfo,
        ),
        LibraryError,
    >;
}

/// Trait for cover art loaders (implemented by infrastructure).
pub trait CoverLoader: Send + Sync {
    fn load_cover(&self, source: &CoverSource) -> Result<Option<CoverImage>, LibraryError>;
}

// Ports whose single definitions live in the library slice, re-exported so
// existing `riff_backend::app::traits::` import paths keep resolving. The
// real adapter implementations (lofty metadata writer, notify watcher) live
// in `riff-infra` and implement these through riff-library's definitions.
pub use riff_library::app::traits::{CoverImageFormat, FilesystemWatch, MetadataWriter, TagEdit};

/// Decoded cover image ready for UI display.
#[derive(Debug, Clone)]
pub struct CoverImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
