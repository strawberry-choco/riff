use crate::app::errors::AppError;
use crate::app::traits::{MetadataWriter, TagEdit};
use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::read_from_path;
use lofty::tag::{ItemKey, Tag};
use std::path::Path;

/// [`MetadataWriter`] implementation backed by `lofty`.
pub struct LoftyMetadataWriter;

impl LoftyMetadataWriter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LoftyMetadataWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataWriter for LoftyMetadataWriter {
    fn write_metadata(&self, path: &Path, edit: &TagEdit) -> Result<(), AppError> {
        let mut tagged_file = read_from_path(path)
            .map_err(|e| AppError::MetadataWrite(format!("failed to read file: {e}")))?;

        // Prefer the primary tag for the format; fall back to any existing
        // tag; otherwise create a new tag of the format's primary type.
        let tag: &mut Tag = if let Some(tag) = tagged_file.primary_tag_mut() {
            tag
        } else if let Some(tag) = tagged_file.first_tag_mut() {
            tag
        } else {
            let tag_type = tagged_file.primary_tag_type();
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file
                .primary_tag_mut()
                .expect("tag just inserted must be present")
        };

        // Accessor setters handle format-specific key mapping, replace any
        // same-key item, and preserve every other item in the tag.
        if let Some(ref title) = edit.title {
            tag.set_title(title.clone());
        }
        if let Some(ref artist) = edit.artist {
            tag.set_artist(artist.clone());
        }
        if let Some(ref album) = edit.album {
            tag.set_album(album.clone());
        }
        if let Some(ref album_artist) = edit.album_artist {
            // `Accessor` has no album-artist setter; write the keyed item
            // directly (replaces any same-key item, preserves the rest).
            tag.insert_text(ItemKey::AlbumArtist, album_artist.clone());
        }
        if let Some(ref genre) = edit.genre {
            tag.set_genre(genre.clone());
        }
        if let Some(year) = edit.year {
            tag.set_year(year);
        }
        if let Some(track_number) = edit.track_number {
            tag.set_track(track_number);
        }

        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| AppError::MetadataWrite(e.to_string()))?;

        Ok(())
    }
}
