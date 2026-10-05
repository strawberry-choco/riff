//! Port traits for library infrastructure.
//!
//! Infrastructure implementations (lofty metadata, image cover, walkdir scanner,
//! notify watcher) live in `riff-infra` and implement these traits.

use crate::app::errors::LibraryError;
use riff_persistence::thumbnail::{EncodedThumbnail, ThumbnailBox};
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
///
/// `ReplayGain` is deliberately not an editable field here: a gain is
/// measured from the audio, not typed by a listener, so a Tag Edit can
/// neither set nor clear it — the values survive a Metadata edit untouched.
/// `ReplayGain` travels its own write path ([`ReplayGainWriter`]), driven by
/// the `ReplayGain` Pass and the Inline Tag Editor's `ReplayGain` rows.
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
    }
}

/// Trait for metadata (tag) writers (implemented by infrastructure).
pub trait MetadataWriter: Send {
    /// Write the given edit to the file at `path`.
    fn write_tags(&self, path: &std::path::Path, edit: &TagEdit) -> Result<(), LibraryError>;
}

/// The `ReplayGain` facts one write carries, mirroring what the file tags
/// hold: gains in dB, peaks as linear ratios. Only `Some` fields are
/// written; `None` fields leave the existing tag value untouched — a write
/// never clears a value it was not asked to set.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ReplayGainTags {
    pub track_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain: Option<f32>,
    pub album_peak: Option<f32>,
}

impl ReplayGainTags {
    /// A write of just a Track's own measured pair.
    #[must_use]
    pub fn track_pair(gain: f32, peak: f32) -> Self {
        Self {
            track_gain: Some(gain),
            track_peak: Some(peak),
            ..Self::default()
        }
    }

    /// A write of just an Album's shared pair.
    #[must_use]
    pub fn album_pair(gain: f32, peak: f32) -> Self {
        Self {
            album_gain: Some(gain),
            album_peak: Some(peak),
            ..Self::default()
        }
    }

    /// Whether this write would change anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.track_gain.is_none()
            && self.track_peak.is_none()
            && self.album_gain.is_none()
            && self.album_peak.is_none()
    }
}

/// Trait for `ReplayGain` tag writers (implemented by infrastructure).
///
/// `ReplayGain` travels its own write path, never the Metadata one: the
/// [`TagEdit`] DTO stays closed. The string contract is the reader's
/// gain-string parser's, so anything riff writes, riff and every other
/// player read back — ` dB`-suffixed gains, bare-ratio peaks — with Opus
/// per RFC 7845 (R128-convention gains in their Q7.8 unit encoding).
pub trait ReplayGainWriter: Send {
    /// Write the given `ReplayGain` facts to the file at `path`.
    fn write_replaygain(
        &self,
        path: &std::path::Path,
        tags: &ReplayGainTags,
    ) -> Result<(), LibraryError>;
}

/// One Track's `ReplayGain` 2.0 measurement: the track gain against the
/// −18 LUFS reference, and the track's true peak as a linear ratio. The
/// album aggregate is computed from these plus each member's duration — see
/// [`crate::app::replaygain::album_aggregate`].
///
/// Both fields are finite. A Track with no loudness verdict — digital silence,
/// which the BS.1770 gate reports as no energy at all — lands the domain's
/// no-verdict pair ([`crate::app::replaygain::REPLAYGAIN_FALLBACK_GAIN_DB`]
/// and [`crate::app::replaygain::REPLAYGAIN_FALLBACK_PEAK`]) rather than an
/// infinite gain, which no tag form, player, or Album aggregate can honor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackLoudness {
    /// Track gain in dB: the value that brings this Track to the reference
    /// loudness. Negative attenuates, positive amplifies (peak-capped).
    pub track_gain_db: f32,
    /// True peak as a linear ratio (0..=1+); caps the applied gain so
    /// amplified samples cannot clip.
    pub track_peak: f32,
}

/// Trait for loudness analyzers (implemented by infrastructure over the
/// decoder stack and the `ebur128` crate). The measurement standard is
/// `ReplayGain` 2.0 — BS.1770 integrated loudness against a −18 LUFS
/// reference, true-peak measurement; `ReplayGain` 1.0 is never produced.
pub trait LoudnessAnalyzer: Send + Sync {
    /// Decode and measure one audio file. One decode yields the Track's own
    /// values; its contribution to the Album aggregate is the same
    /// measurement plus the member's duration at aggregation time.
    ///
    /// A file that yields no loudness verdict — digital silence, whose gated
    /// BS.1770 measurement has no energy to report — returns the no-verdict
    /// pair, not an error and not an infinite gain: the values a measurement
    /// returns are always finite and writable.
    fn measure_track(&self, path: &std::path::Path) -> Result<TrackLoudness, LibraryError>;
}

/// Trait for cover art loaders (implemented by infrastructure).
pub trait CoverLoader: Send + Sync {
    /// Decode `source` into RGBA8 pixels scaled to fit within `size`, or
    /// `Ok(None)` when there is no cover. The decode lives here rather than in
    /// the caller so no consumer — least of all the UI's render loop — pays
    /// for it.
    fn load_cover(
        &self,
        source: &CoverSource,
        size: RequestedSize,
    ) -> Result<Option<DecodedCover>, LibraryError>;

    /// Decode bytes that arrived from somewhere *other* than a `CoverSource` —
    /// the Thumbnail cache — and fit them into `size`.
    ///
    /// Reached through this port rather than called directly because the one
    /// decode implementation is `riff-infra`'s, and the resolver that needs it
    /// sits below it in the dependency chain. Split out of `load_cover` so the
    /// cached route and the source route cannot drift apart.
    fn decode_thumbnail(
        &self,
        bytes: &[u8],
        size: RequestedSize,
    ) -> Result<DecodedCover, LibraryError>;

    /// Encode a decoded Cover into the bytes the Thumbnail cache stores.
    fn encode_thumbnail(&self, cover: &DecodedCover) -> Result<EncodedThumbnail, LibraryError>;
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

/// The display box the caller wants the cover to fit within.
///
/// Carried on the request so the worker can hand back pixels that are already
/// the right size: the consumer never holds a full-resolution image it will
/// only ever draw at thumbnail size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestedSize {
    pub width: u32,
    pub height: u32,
}

impl From<RequestedSize> for ThumbnailBox {
    /// The display box a request wants *is* the cache entry's box — one
    /// requested size, one stored rung. Mapping rather than reusing the type
    /// keeps the persistence contract free of this crate's vocabulary.
    fn from(size: RequestedSize) -> Self {
        Self {
            width: size.width,
            height: size.height,
        }
    }
}

/// Cover art decoded to RGBA8 pixels by the [`CoverLoader`] adapter.
///
/// A plain struct with no image-decoding dependency in this crate: the
/// decoding itself is `riff-infra`'s job, and this is only the shape its
/// result crosses the port boundary in. Produced on the cover worker thread,
/// so a consumer's remaining work is the cheap wrap-and-upload step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCover {
    /// Unpremultiplied RGBA8, row-major, length `width * height * 4`.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
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
    }
}
