use crate::app::traits::{MetadataReader, CoverLoader, CoverImage};
use crate::app::errors::AppError;
use crate::domain::CoverSource;
use std::path::PathBuf;

/// Resolves cover art for a track using the priority: embedded > filesystem fallback.
pub struct CoverResolver {
    metadata_reader: Box<dyn MetadataReader>,
    cover_loader: Box<dyn CoverLoader>,
}

impl CoverResolver {
    pub fn new(metadata_reader: Box<dyn MetadataReader>, cover_loader: Box<dyn CoverLoader>) -> Self {
        Self { metadata_reader, cover_loader }
    }

    pub fn resolve(&self, track_path: &PathBuf) -> Result<Option<CoverImage>, AppError> {
        let source = self.metadata_reader.read_cover_source(track_path)?;

        match source {
            CoverSource::Embedded(_) => {
                self.cover_loader.load_cover(&source)
            }
            CoverSource::None => {
                let fallback = self.find_filesystem_cover(track_path)?;
                self.cover_loader.load_cover(&fallback)
            }
            CoverSource::Filesystem(_) => self.cover_loader.load_cover(&source),
        }
    }

    fn find_filesystem_cover(&self, track_path: &PathBuf) -> Result<CoverSource, AppError> {
        let parent = track_path.parent()
            .ok_or_else(|| AppError::Io("Track has no parent directory".to_string()))?;

        let candidates = [
            "cover.jpg", "cover.jpeg", "cover.png",
            "folder.jpg", "folder.jpeg", "folder.png",
            "album.jpg", "album.jpeg", "album.png",
            "front.jpg", "front.jpeg", "front.png",
        ];

        if let Ok(entries) = std::fs::read_dir(parent) {
            let mut found_files: Vec<(String, std::fs::DirEntry)> = Vec::new();
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            found_files.push((name.to_lowercase(), entry));
                        }
                    }
                }
            }

            for candidate in &candidates {
                let candidate_lower = candidate.to_lowercase();
                if let Some((_, entry)) = found_files.iter().find(|(name, _)| name == &candidate_lower) {
                    return Ok(CoverSource::Filesystem(entry.path()));
                }
            }
        }

        Ok(CoverSource::None)
    }
}
