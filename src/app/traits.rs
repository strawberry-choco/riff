use std::path::PathBuf;
use std::time::Duration;
use crate::app::errors::AppError;
use crate::domain::{TrackMetadata, CoverSource};

/// Trait for audio decoders (implemented by infrastructure).
pub trait AudioDecoder: Send {
    fn open(&mut self, path: &PathBuf) -> Result<AudioFormatInfo, AppError>;
    fn next_frames(&mut self, samples: usize) -> Result<Option<Vec<f32>>, AppError>;
    fn seek(&mut self, position: Duration) -> Result<(), AppError>;
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
    fn initialize(&mut self, sample_rate: u32, channels: u16) -> Result<(), AppError>;
    fn start(&mut self) -> Result<(), AppError>;
    fn stop(&mut self) -> Result<(), AppError>;
    fn write_samples(&mut self, samples: &[f32]) -> Result<usize, AppError>;
    fn set_volume(&mut self, volume: f32);
    /// Number of samples currently queued in the output buffer.
    fn buffer_len(&self) -> usize;
    /// Discard all queued samples without playing them.
    fn clear_buffer(&mut self);
}

/// Trait for metadata readers (implemented by infrastructure).
pub trait MetadataReader: Send + Sync {
    fn read_metadata(&self, path: &PathBuf) -> Result<TrackMetadata, AppError>;
    fn read_duration(&self, path: &PathBuf) -> Result<Option<Duration>, AppError>;
    fn read_cover_source(&self, path: &PathBuf) -> Result<CoverSource, AppError>;
    fn read_audio_format(&self, path: &PathBuf) -> Result<AudioFormatInfo, AppError>;
    fn read_all(&self, path: &PathBuf) -> Result<(TrackMetadata, Option<Duration>, CoverSource, AudioFormatInfo), AppError>;
}

/// Trait for cover art loaders (implemented by infrastructure).
pub trait CoverLoader: Send + Sync {
    fn load_cover(&self, source: &CoverSource) -> Result<Option<CoverImage>, AppError>;
}

/// Decoded cover image ready for UI display.
#[derive(Debug, Clone)]
pub struct CoverImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
