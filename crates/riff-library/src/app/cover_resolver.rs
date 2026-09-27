//! Cover art resolution: embedded first, then filesystem fallback.

use crate::app::errors::LibraryError;
use crate::infra::ports::{CoverLoader, DecodedCover, MetadataReader, RequestedSize};
use riff_persistence::errors::StoreError;
use riff_persistence::thumbnail::{ThumbnailBox, ThumbnailCache};
use riff_persistence::track::CoverSource;
use std::path::Path;
use std::sync::Arc;

/// The file names a track's directory is probed for, in priority order: an
/// album carrying both `cover.jpg` and `folder.jpg` shows its `cover.jpg`.
const TRACK_COVER_NAMES: [&str; 12] = [
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "album.jpg",
    "album.jpeg",
    "album.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
];

/// The file names a folder row probes its own directory for.
///
/// Deliberately narrower than [`TRACK_COVER_NAMES`]: the tile is the directory's
/// *own* cover, so a `folder.jpg` / `album.jpg` / `front.jpg` sidecar — which is
/// that album's art for every track inside it — stays invisible here, and
/// embedded tag artwork is never a candidate.
///
/// `cover.gif` is excluded on purpose: the `image` dependency is built with only
/// the JPEG and PNG decoders (so the loader reports GIF as unsupported), and an
/// animated GIF would render an arbitrary first frame. Reopening it is the
/// feature list, the loader's format gate, and this array — see the Decisions
/// section of `.scratch/folder-covers/execution-plan-2026-09-20.md`.
const FOLDER_COVER_NAMES: [&str; 3] = ["cover.jpg", "cover.jpeg", "cover.png"];

/// Resolves cover art for a track using the priority: embedded > filesystem fallback.
pub struct CoverResolver {
    metadata_reader: Box<dyn MetadataReader>,
    cover_loader: Box<dyn CoverLoader>,
    /// The persistent Thumbnail cache. Consulted here rather than in a component
    /// of its own because this already decides "may I skip this read" — a second
    /// place answering that question is how the two come apart. Shared because the
    /// Composition Root keeps a handle for the user's clear action (`06`), and two
    /// instances pointing at one directory would not be one cache.
    cache: Arc<dyn ThumbnailCache>,
}

impl CoverResolver {
    pub fn new(
        metadata_reader: Box<dyn MetadataReader>,
        cover_loader: Box<dyn CoverLoader>,
        cache: Arc<dyn ThumbnailCache>,
    ) -> Self {
        Self {
            metadata_reader,
            cover_loader,
            cache,
        }
    }

    /// Resolve cover art for `track_path`, scaled to fit `size`.
    /// `read_embedded_artwork == false` (the Settings Library pane's "Read
    /// embedded artwork" toggle, design-handoff issue 12) skips the tag read
    /// entirely and goes straight to the filesystem fallback — the tags are
    /// never opened for art. The decoded pixels come back from the loader
    /// adapter; this resolver only ever names the source.
    pub fn resolve(
        &self,
        track_path: &Path,
        read_embedded_artwork: bool,
        size: RequestedSize,
    ) -> Result<Option<DecodedCover>, LibraryError> {
        let source = if read_embedded_artwork {
            self.metadata_reader.read_cover_source(track_path)?
        } else {
            CoverSource::None
        };
        // Only a subject that is still artless after the folder probe reaches
        // the loader as `CoverSource::None` — and it still reaches it, because
        // "there is no cover" is the loader's answer to give, not this
        // resolver's. Nothing is persisted for that answer, which is what makes
        // "a cover.jpg appeared in this folder later" correct by construction:
        // no directory-mtime key component, and no rescan signal.
        let source = match Self::source_file(&source, track_path) {
            Some(_) => source,
            None => Self::find_filesystem_cover(track_path)?,
        };
        self.sized(&source, track_path, size)
    }

    /// Resolve the cover art *of a directory itself*, for the Folders-tree row
    /// that shows it. `dir` is probed, never entered: only its own files are
    /// candidates and no audio tags are opened.
    ///
    /// A directory with no matching file answers `Ok(None)` rather than an
    /// error, because the cover worker negative-caches an artless answer but
    /// logs a warning for a failed one — an error here would be logged on every
    /// frame the row paints.
    pub fn resolve_folder(
        &self,
        dir: &Path,
        size: RequestedSize,
    ) -> Result<Option<DecodedCover>, LibraryError> {
        let source = Self::first_candidate(dir, &FOLDER_COVER_NAMES);
        self.sized(&source, dir, size)
    }

    /// The one resolution path every Cover shares: look for a cached Thumbnail of
    /// this Source at this box, and only read and decode the Source if there is
    /// none.
    fn sized(
        &self,
        source: &CoverSource,
        track_path: &Path,
        size: RequestedSize,
    ) -> Result<Option<DecodedCover>, LibraryError> {
        // The cache is keyed on the Source's file rather than the Track's, which
        // is what collapses the twelve Tracks of one album onto one stored
        // ladder. For embedded art the Source *is* the Track's own file, so a hit
        // has still cost the tag read that produced `source`: lofty
        // materialises the picture bytes (`metadata_reader.rs:156`), so the parse
        // necessarily reads the whole embedded image. What a hit removes is the
        // decode and the resize — not the read, and claiming otherwise here would
        // invite a later "optimisation" premised on a falsehood.
        let Some(source_path) = Self::source_file(source, track_path) else {
            return self.cover_loader.load_cover(source, size);
        };
        let box_ = size.into();
        if let Some(cached) = self.cache.load(source_path, box_) {
            return self
                .cover_loader
                .decode_thumbnail(&cached.bytes, size)
                .map(Some);
        }

        let Some(cover) = self.cover_loader.load_cover(source, size)? else {
            return Ok(None);
        };
        self.persist(source_path, box_, &cover);
        Ok(Some(cover))
    }

    /// Encode and store the rung just decoded. A failure here is logged and
    /// dropped: a cache that cannot be written costs one more decode later, and
    /// must never fail the resolution the caller is waiting on.
    fn persist(&self, source_path: &Path, box_: ThumbnailBox, cover: &DecodedCover) {
        let thumbnail = match self.cover_loader.encode_thumbnail(cover) {
            Ok(thumbnail) => thumbnail,
            Err(e) => {
                tracing::warn!("Could not encode the Thumbnail for {source_path:?}: {e}");
                return;
            }
        };
        if let Err(e) = self.cache.store(source_path, box_, &thumbnail) {
            tracing::warn!("Could not cache the Thumbnail for {source_path:?}: {e}");
        }
    }

    /// Which file the Cover physically lives in — the cache key. Embedded art is
    /// inside the Track's own file; a folder Cover is the image itself.
    ///
    /// Exhaustive over the three variants, so a future `CoverSource` is a compile
    /// error here rather than a rung that silently never caches.
    fn source_file<'a>(source: &'a CoverSource, track_path: &'a Path) -> Option<&'a Path> {
        match source {
            CoverSource::Embedded(_) => Some(track_path),
            CoverSource::Filesystem(path) => Some(path),
            CoverSource::None => None,
        }
    }

    fn find_filesystem_cover(track_path: &Path) -> Result<CoverSource, LibraryError> {
        let parent = track_path
            .parent()
            .ok_or_else(|| LibraryError::Io("Track has no parent directory".to_string()))?;

        Ok(Self::first_candidate(parent, &TRACK_COVER_NAMES))
    }

    /// Delete every cached Thumbnail. The worker runs this between resolutions, so
    /// no write can be in flight against it; the caller reports the outcome and
    /// nothing else depends on it.
    ///
    /// This clears the rungs, not the resolutions in flight: a Cover already
    /// decoded is still delivered, and a Cover already reported artless stays in the
    /// worker's negative cache for now — that cache is RAM-only and is deliberately
    /// out of this feature's scope.
    pub fn clear_cache(&self) -> Result<(), StoreError> {
        self.cache.clear()
    }

    /// The one directory probe behind every cover-art lookup: read `dir`,
    /// collect its plain file names case-insensitively, and answer with the
    /// first name in `names` that has a file there.
    ///
    /// `names` is a *priority* order, not a membership set — a directory
    /// carrying both `cover.jpg` and `folder.jpg` resolves to whichever comes
    /// first, so callers must pass it in the order they want honoured. A hit
    /// reports the directory entry's own path, preserving the on-disk spelling.
    /// A directory that cannot be read and a directory with no match both
    /// answer [`CoverSource::None`]: an artless lookup is never an error, which
    /// is what lets the cover worker negative-cache the miss.
    fn first_candidate(dir: &Path, names: &[&str]) -> CoverSource {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut found_files: Vec<(String, std::fs::DirEntry)> = Vec::new();
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata()
                    && metadata.is_file()
                    && let Some(name) = entry.file_name().to_str()
                {
                    found_files.push((name.to_lowercase(), entry));
                }
            }

            for candidate in names {
                let candidate_lower = candidate.to_lowercase();
                if let Some((_, entry)) = found_files
                    .iter()
                    .find(|(name, _)| name == &candidate_lower)
                {
                    return CoverSource::Filesystem(entry.path());
                }
            }
        }

        CoverSource::None
    }
}
