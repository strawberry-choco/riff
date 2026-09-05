//! Cover art resolution: embedded first, then filesystem fallback.

use crate::app::errors::LibraryError;
use crate::infra::ports::{CoverImage, CoverLoader, MetadataReader};
use riff_persistence::track::CoverSource;
use std::path::Path;

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

    /// Resolve cover art for `track_path`. `read_embedded_artwork == false`
    /// (the Settings Library pane's "Read embedded artwork" toggle,
    /// design-handoff issue 12) skips the tag read entirely and goes
    /// straight to the filesystem fallback — the tags are never opened for
    /// art.
    pub fn resolve(
        &self,
        track_path: &Path,
        read_embedded_artwork: bool,
    ) -> Result<Option<CoverImage>, LibraryError> {
        let source = if read_embedded_artwork {
            self.metadata_reader.read_cover_source(track_path)?
        } else {
            CoverSource::None
        };

        match source {
            CoverSource::Embedded(_) | CoverSource::Filesystem(_) => {
                self.cover_loader.load_cover(&source)
            }
            CoverSource::None => {
                let fallback = Self::find_filesystem_cover(track_path)?;
                self.cover_loader.load_cover(&fallback)
            }
        }
    }

    fn find_filesystem_cover(track_path: &Path) -> Result<CoverSource, LibraryError> {
        let parent = track_path
            .parent()
            .ok_or_else(|| LibraryError::Io("Track has no parent directory".to_string()))?;

        let candidates = [
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

        if let Ok(entries) = std::fs::read_dir(parent) {
            let mut found_files: Vec<(String, std::fs::DirEntry)> = Vec::new();
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata()
                    && metadata.is_file()
                    && let Some(name) = entry.file_name().to_str()
                {
                    found_files.push((name.to_lowercase(), entry));
                }
            }

            for candidate in &candidates {
                let candidate_lower = candidate.to_lowercase();
                if let Some((_, entry)) = found_files
                    .iter()
                    .find(|(name, _)| name == &candidate_lower)
                {
                    return Ok(CoverSource::Filesystem(entry.path()));
                }
            }
        }

        Ok(CoverSource::None)
    }
}
