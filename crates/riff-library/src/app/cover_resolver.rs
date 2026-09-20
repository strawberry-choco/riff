//! Cover art resolution: embedded first, then filesystem fallback.

use crate::app::errors::LibraryError;
use crate::infra::ports::{CoverLoader, DecodedCover, MetadataReader, RequestedSize};
use riff_persistence::track::CoverSource;
use std::path::Path;

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
}

impl CoverResolver {
    pub fn new(
        metadata_reader: Box<dyn MetadataReader>,
        cover_loader: Box<dyn CoverLoader>,
    ) -> Self {
        Self {
            metadata_reader,
            cover_loader,
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

        match source {
            CoverSource::Embedded(_) | CoverSource::Filesystem(_) => {
                self.cover_loader.load_cover(&source, size)
            }
            CoverSource::None => {
                let fallback = Self::find_filesystem_cover(track_path)?;
                self.cover_loader.load_cover(&fallback, size)
            }
        }
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
        self.cover_loader.load_cover(&source, size)
    }

    fn find_filesystem_cover(track_path: &Path) -> Result<CoverSource, LibraryError> {
        let parent = track_path
            .parent()
            .ok_or_else(|| LibraryError::Io("Track has no parent directory".to_string()))?;

        Ok(Self::first_candidate(parent, &TRACK_COVER_NAMES))
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
