use std::path::PathBuf;
use std::time::Duration;
use lofty::prelude::*;
use lofty::tag::ItemKey;
use lofty::read_from_path;
use lofty::file::TaggedFile;
use crate::app::traits::MetadataReader;
use crate::app::errors::AppError;
use crate::domain::{TrackMetadata, CoverSource};

pub struct LoftyMetadataReader;

impl LoftyMetadataReader {
    pub fn new() -> Self {
        Self
    }

    fn read_tagged_file(path: &PathBuf) -> Result<TaggedFile, AppError> {
        read_from_path(path)
            .map_err(|e| AppError::MetadataRead(format!("Failed to read file: {}", e)))
    }
}

impl MetadataReader for LoftyMetadataReader {
    fn read_metadata(&self, path: &PathBuf) -> Result<TrackMetadata, AppError> {
        let tagged_file = Self::read_tagged_file(path)?;

        let tag = match tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
            Some(tag) => tag,
            None => return Ok(TrackMetadata::default()),
        };

        let mut metadata = TrackMetadata::default();

        let text_val = |item: &lofty::tag::TagItem| -> Option<String> {
            item.value().clone().into_string()
        };

        if let Some(item) = tag.get(&ItemKey::TrackTitle) {
            metadata.title = text_val(item);
        }
        if let Some(item) = tag.get(&ItemKey::TrackArtist) {
            metadata.artist = text_val(item);
        }
        if let Some(item) = tag.get(&ItemKey::AlbumTitle) {
            metadata.album = text_val(item);
        }
        if let Some(item) = tag.get(&ItemKey::AlbumArtist) {
            metadata.album_artist = text_val(item);
        }
        if let Some(item) = tag.get(&ItemKey::TrackNumber) {
            metadata.track_number = text_val(item)
                .and_then(|s| s.parse::<u32>().ok());
        }
        if let Some(item) = tag.get(&ItemKey::DiscNumber) {
            metadata.disc_number = text_val(item)
                .and_then(|s| s.parse::<u32>().ok());
        }
        if let Some(item) = tag.get(&ItemKey::Genre) {
            metadata.genre = text_val(item);
        }
        if let Some(item) = tag.get(&ItemKey::Year) {
            metadata.year = text_val(item)
                .and_then(|s| s.parse::<u32>().ok());
        }
        if let Some(item) = tag.get(&ItemKey::Comment) {
            metadata.comment = text_val(item);
        }
        if let Some(item) = tag.get(&ItemKey::Composer) {
            metadata.composer = text_val(item);
        }

        Ok(metadata)
    }

    fn read_duration(&self, path: &PathBuf) -> Result<Option<Duration>, AppError> {
        let tagged_file = Self::read_tagged_file(path)?;

        let properties = tagged_file.properties();
        Ok(Some(properties.duration()))
    }

    fn read_cover_source(&self, path: &PathBuf) -> Result<CoverSource, AppError> {
        let tagged_file = Self::read_tagged_file(path)?;

        if let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
            for picture in tag.pictures() {
                let mime_type = picture.mime_type();
                if mime_type == Some(&lofty::picture::MimeType::Jpeg) || 
                   mime_type == Some(&lofty::picture::MimeType::Png) {
                    return Ok(CoverSource::Embedded(picture.data().to_vec()));
                }
            }
        }

        Ok(CoverSource::None)
    }

    fn read_all(&self, path: &PathBuf) -> Result<(TrackMetadata, Option<Duration>, CoverSource), AppError> {
        let tagged_file = Self::read_tagged_file(path)?;

        let metadata = {
            let tag = match tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
                Some(tag) => tag,
                None => {
                    let properties = tagged_file.properties();
                    return Ok((TrackMetadata::default(), Some(properties.duration()), CoverSource::None));
                }
            };

            let mut metadata = TrackMetadata::default();

            let text_val = |item: &lofty::tag::TagItem| -> Option<String> {
                item.value().clone().into_string()
            };

            if let Some(item) = tag.get(&ItemKey::TrackTitle) {
                metadata.title = text_val(item);
            }
            if let Some(item) = tag.get(&ItemKey::TrackArtist) {
                metadata.artist = text_val(item);
            }
            if let Some(item) = tag.get(&ItemKey::AlbumTitle) {
                metadata.album = text_val(item);
            }
            if let Some(item) = tag.get(&ItemKey::AlbumArtist) {
                metadata.album_artist = text_val(item);
            }
            if let Some(item) = tag.get(&ItemKey::TrackNumber) {
                metadata.track_number = text_val(item)
                    .and_then(|s| s.parse::<u32>().ok());
            }
            if let Some(item) = tag.get(&ItemKey::DiscNumber) {
                metadata.disc_number = text_val(item)
                    .and_then(|s| s.parse::<u32>().ok());
            }
            if let Some(item) = tag.get(&ItemKey::Genre) {
                metadata.genre = text_val(item);
            }
            if let Some(item) = tag.get(&ItemKey::Year) {
                metadata.year = text_val(item)
                    .and_then(|s| s.parse::<u32>().ok());
            }
            if let Some(item) = tag.get(&ItemKey::Comment) {
                metadata.comment = text_val(item);
            }
            if let Some(item) = tag.get(&ItemKey::Composer) {
                metadata.composer = text_val(item);
            }

            metadata
        };

        let properties = tagged_file.properties();
        let duration = Some(properties.duration());

        let cover_source = if let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
            let mut source = CoverSource::None;
            for picture in tag.pictures() {
                let mime_type = picture.mime_type();
                if mime_type == Some(&lofty::picture::MimeType::Jpeg) || 
                   mime_type == Some(&lofty::picture::MimeType::Png) {
                    source = CoverSource::Embedded(picture.data().to_vec());
                    break;
                }
            }
            source
        } else {
            CoverSource::None
        };

        Ok((metadata, duration, cover_source))
    }
}
