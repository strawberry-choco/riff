use crate::app::errors::AppError;
use crate::app::traits::{AudioFormatInfo, MetadataReader};
use crate::domain::{CoverSource, TrackMetadata};
use lofty::file::TaggedFile;
use lofty::prelude::*;
use lofty::read_from_path;
use lofty::tag::{ItemKey, Tag};
use std::path::Path;
use std::time::Duration;

/// Parse a `ReplayGain` gain string such as `"-6.54 dB"`, `"+3.21 dB"`, or a
/// bare `"3.21"` into dB. Strips a trailing `dB` (case-insensitive), trims,
/// and parses; malformed values yield `None`.
pub fn parse_replaygain_gain(s: &str) -> Option<f32> {
    let lower = s.trim().to_ascii_lowercase();
    let without_unit = lower.strip_suffix("db").unwrap_or(&lower);
    without_unit.trim().parse::<f32>().ok()
}

/// Extract a year from a tag text value: a bare `"1959"` parses directly,
/// and date-style values (`"1959-08-17"`, full `ID3v2` timestamps) yield
/// their leading four digits. Values without a plausible leading year give
/// `None`.
fn year_from_text(s: &str) -> Option<u32> {
    let digits: String = s
        .trim()
        .chars()
        .take(4)
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u32>().ok()
}

pub struct LoftyMetadataReader;

impl Default for LoftyMetadataReader {
    fn default() -> Self {
        Self::new()
    }
}

impl LoftyMetadataReader {
    pub fn new() -> Self {
        Self
    }

    fn read_tagged_file(path: &Path) -> Result<TaggedFile, AppError> {
        read_from_path(path)
            .map_err(|e| AppError::MetadataRead(format!("Failed to read file: {e}")))
    }

    /// The primary tag of a tagged file, falling back to the first attached
    /// tag of any type, or `None` when the file carries no tags at all.
    fn best_tag(tagged_file: &TaggedFile) -> Option<&Tag> {
        tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
    }

    /// Extract [`TrackMetadata`] from a tag.
    fn metadata_from_tag(tag: &Tag) -> TrackMetadata {
        let text_val =
            |item: &lofty::tag::TagItem| -> Option<String> { item.value().clone().into_string() };

        let mut metadata = TrackMetadata::default();
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
            metadata.track_number = text_val(item).and_then(|s| s.parse::<u32>().ok());
        }
        if let Some(item) = tag.get(&ItemKey::DiscNumber) {
            metadata.disc_number = text_val(item).and_then(|s| s.parse::<u32>().ok());
        }
        if let Some(item) = tag.get(&ItemKey::Genre) {
            metadata.genre = text_val(item);
        }
        // Year: a dedicated `Year` item wins (APE); otherwise the year is
        // the leading digits of a `RecordingDate` (ID3v2 `TDRC`, Vorbis
        // `DATE`, RIFF `ICRD`, MP4 `©day`). This mirrors what
        // `Accessor::set_year` writes, so tags written through
        // `LoftyMetadataWriter` round-trip when the file is re-read.
        metadata.year = tag
            .get(&ItemKey::Year)
            .or_else(|| tag.get(&ItemKey::RecordingDate))
            .and_then(text_val)
            .and_then(|s| year_from_text(&s));
        if let Some(item) = tag.get(&ItemKey::Comment) {
            metadata.comment = text_val(item);
        }
        if let Some(item) = tag.get(&ItemKey::Composer) {
            metadata.composer = text_val(item);
        }

        // ReplayGain (Task 4.3): `ItemKey` is not `Copy` in lofty 0.19, so the
        // dedicated variants are passed by reference.
        metadata.replaygain_track_gain = tag
            .get_string(&ItemKey::ReplayGainTrackGain)
            .and_then(parse_replaygain_gain);
        metadata.replaygain_track_peak = tag
            .get_string(&ItemKey::ReplayGainTrackPeak)
            .and_then(|s| s.trim().parse::<f32>().ok());

        metadata
    }

    /// Locate embedded JPEG/PNG cover art in a tag, if any.
    fn cover_from_tag(tag: &Tag) -> CoverSource {
        for picture in tag.pictures() {
            let mime_type = picture.mime_type();
            if mime_type == Some(&lofty::picture::MimeType::Jpeg)
                || mime_type == Some(&lofty::picture::MimeType::Png)
            {
                return CoverSource::Embedded(picture.data().to_vec());
            }
        }
        CoverSource::None
    }

    fn audio_format_from(tagged_file: &TaggedFile) -> AudioFormatInfo {
        let properties = tagged_file.properties();
        AudioFormatInfo {
            sample_rate: properties.sample_rate().unwrap_or(44_100),
            channels: u16::from(properties.channels().unwrap_or(2)),
            duration: Some(properties.duration()),
        }
    }
}

impl MetadataReader for LoftyMetadataReader {
    fn read_metadata(&self, path: &Path) -> Result<TrackMetadata, AppError> {
        let tagged_file = Self::read_tagged_file(path)?;
        let Some(tag) = Self::best_tag(&tagged_file) else {
            return Ok(TrackMetadata::default());
        };
        Ok(Self::metadata_from_tag(tag))
    }

    fn read_duration(&self, path: &Path) -> Result<Option<Duration>, AppError> {
        let tagged_file = Self::read_tagged_file(path)?;
        Ok(Some(tagged_file.properties().duration()))
    }

    fn read_cover_source(&self, path: &Path) -> Result<CoverSource, AppError> {
        let tagged_file = Self::read_tagged_file(path)?;
        Ok(match Self::best_tag(&tagged_file) {
            Some(tag) => Self::cover_from_tag(tag),
            None => CoverSource::None,
        })
    }

    fn read_audio_format(&self, path: &Path) -> Result<AudioFormatInfo, AppError> {
        let tagged_file = Self::read_tagged_file(path)?;
        Ok(Self::audio_format_from(&tagged_file))
    }

    fn read_all(
        &self,
        path: &Path,
    ) -> Result<
        (
            TrackMetadata,
            Option<Duration>,
            CoverSource,
            AudioFormatInfo,
        ),
        AppError,
    > {
        let tagged_file = Self::read_tagged_file(path)?;

        let metadata = Self::best_tag(&tagged_file)
            .map_or_else(TrackMetadata::default, Self::metadata_from_tag);
        let cover_source =
            Self::best_tag(&tagged_file).map_or(CoverSource::None, Self::cover_from_tag);
        let audio_format = Self::audio_format_from(&tagged_file);

        Ok((metadata, audio_format.duration, cover_source, audio_format))
    }
}
