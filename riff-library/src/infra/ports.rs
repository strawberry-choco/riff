//! Port traits for library infrastructure.
//!
//! Infrastructure implementations (lofty metadata, image cover, walkdir scanner,
//! notify watcher) live in `riff-infra` and implement these traits.

use crate::app::errors::LibraryError;
use riff_persistence::track::CoverSource;
use std::time::Duration;

/// Audio format information returned by decoder.
#[derive(Debug, Clone)]
pub struct AudioFormatInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Trait for metadata readers (implemented by infrastructure).
pub trait MetadataReader: Send + Sync {
    /// Read all metadata from a file.
    fn read_all(
        &self,
        path: &std::path::Path,
    ) -> Result<(TrackMetadata, Duration, CoverSource, AudioFormatInfo), LibraryError>;

    /// Read only the cover source from a file (lighter weight than full metadata read).
    fn read_cover_source(&self, path: &std::path::Path) -> Result<CoverSource, LibraryError>;
}

/// A requested edit to a track's metadata tags.
///
/// Pure application-layer DTO: only `Some` fields are written, `None` fields
/// leave the existing tag value untouched. Contains no infrastructure types.
#[derive(Debug, Clone, Default)]
pub struct TagEdit {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub composer: Option<String>,
    pub comment: Option<String>,
    pub replaygain_track_gain: Option<f32>,
    pub replaygain_track_peak: Option<f32>,
}

impl TagEdit {
    /// Whether this edit would change anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.album_artist.is_none()
            && self.track_number.is_none()
            && self.disc_number.is_none()
            && self.genre.is_none()
            && self.year.is_none()
            && self.composer.is_none()
            && self.comment.is_none()
            && self.replaygain_track_gain.is_none()
            && self.replaygain_track_peak.is_none()
    }
}

/// Trait for metadata (tag) writers (implemented by infrastructure).
pub trait MetadataWriter: Send {
    /// Write the given edit to the file at `path`.
    fn write_tags(&self, path: &std::path::Path, edit: &TagEdit) -> Result<(), LibraryError>;
}

/// Trait for cover art loaders (implemented by infrastructure).
pub trait CoverLoader: Send + Sync {
    fn load_cover(&self, source: &CoverSource) -> Result<Option<CoverImage>, LibraryError>;
}

/// Trait for filesystem watchers (implemented by infrastructure).
///
/// Watches directory roots for audio-file changes; debounced batches of
/// changed paths are delivered over a channel wired at construction time, not
/// through these methods. The watcher manager codes against this port and
/// never names the concrete adapter — the composition root injects the real
/// watcher. Errors carry a human-readable reason string so no infrastructure
/// error type leaks into the application layer.
pub trait FilesystemWatch: Send {
    /// Start watching the given paths.
    fn start(&mut self, paths: &[std::path::PathBuf]) -> Result<(), LibraryError>;

    /// Stop watching.
    fn stop(&mut self) -> Result<(), LibraryError>;
}

/// Decoded cover image ready for UI display.
#[derive(Debug, Clone)]
pub struct CoverImage {
    pub data: Vec<u8>,
    pub format: image::ImageFormat,
}

use crate::domain::TrackMetadata;
