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
    /// Register `path` for recursive watching. On failure the `Err` carries
    /// the raw reason, which the caller prefixes into the user-facing
    /// `WatchState::Warning` diagnostic.
    fn watch(&mut self, path: &std::path::Path) -> Result<(), LibraryError>;

    /// Stop watching `path`. Failures are surfaced for symmetry but callers
    /// typically ignore them — unwatching an already-gone root is not a
    /// user-facing error.
    fn unwatch(&mut self, path: &std::path::Path) -> Result<(), LibraryError>;
}

/// Container format of still-encoded cover bytes, detected by the loader.
///
/// Own enum rather than an infrastructure crate's format type: the port DTO
/// must not drag an image-decoding dependency into this crate. The image
/// features enabled for cover loading are exactly JPEG and PNG, so these two
/// variants cover every container the loader can hand out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverImageFormat {
    Jpeg,
    Png,
}

/// Decoded cover image ready for UI display.
#[derive(Debug, Clone)]
pub struct CoverImage {
    pub data: Vec<u8>,
    pub format: CoverImageFormat,
}

use crate::domain::TrackMetadata;

impl TagEdit {
    /// Apply only the `Some` fields of this edit to `metadata`, leaving every
    /// other field untouched. Used to refresh the Store facts after a
    /// successful write to the (source-of-truth) file tags.
    pub fn apply_to(&self, metadata: &mut TrackMetadata) {
        if let Some(ref title) = self.title {
            metadata.title = Some(title.clone());
        }
        if let Some(ref artist) = self.artist {
            metadata.artist = Some(artist.clone());
        }
        if let Some(ref album) = self.album {
            metadata.album = Some(album.clone());
        }
        if let Some(ref album_artist) = self.album_artist {
            metadata.album_artist = Some(album_artist.clone());
        }
        if let Some(track_number) = self.track_number {
            metadata.track_number = Some(track_number);
        }
        if let Some(disc_number) = self.disc_number {
            metadata.disc_number = Some(disc_number);
        }
        if let Some(ref genre) = self.genre {
            metadata.genre = Some(genre.clone());
        }
        if let Some(year) = self.year {
            metadata.year = Some(year);
        }
        if let Some(ref composer) = self.composer {
            metadata.composer = Some(composer.clone());
        }
        if let Some(ref comment) = self.comment {
            metadata.comment = Some(comment.clone());
        }
        if let Some(gain) = self.replaygain_track_gain {
            metadata.replaygain_track_gain = Some(gain);
        }
        if let Some(peak) = self.replaygain_track_peak {
            metadata.replaygain_track_peak = Some(peak);
        }
    }
}
