//! The lofty tag reader for riff. `lofty::read_from_path` is the primary
//! path — its tag coverage is broader than the fallback's, so a file lofty
//! can read stays on the lofty path. Every lofty error funnels through a
//! fallback that re-reads the file via symphonia's format probe (the same
//! crate the audio engine already uses for playback), so a lofty parse bug
//! does not drop the file from the Library entirely.
//!
//! The fallback is deliberately keyed on lofty's result, not on the file
//! extension: if lofty fails for any reason on any format, symphonia gets
//! a shot, and every fallback is logged via `tracing::warn!` so a new lofty
//! bug surfaces as an observable event rather than silent data loss.
//!
//! Limitation: symphonia cannot read ReplayGain. A file lofty fails to read
//! will not surface ReplayGain values, even if the file carries them. The
//! four ReplayGain fields of the fallback's `TrackMetadata` are `None`.
use lofty::file::TaggedFile;
use lofty::prelude::*;
use lofty::read_from_path;
use lofty::tag::{ItemKey, Tag};
use riff_library::app::errors::LibraryError;
use riff_playback::domain::duration_from_frames;
use riff_persistence::track::{CoverSource, TrackMetadata};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use symphonia::core::codecs::audio::CODEC_ID_NULL_AUDIO;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::meta::StandardTag;
use symphonia::default::get_probe;

/// Parse a `ReplayGain` gain string such as `"-6.54 dB"`, `"+3.21 dB"`, or a
/// bare `"3.21"` into dB. Strips a trailing `dB` (case-insensitive, without
/// lowercasing the whole value), trims, and parses; malformed values yield
/// `None`.
///
/// The suffixed form is the contract the measuring tooling writes — riff's
/// own writer included (`LoftyMetadataWriter::write_replaygain`) — so a file
/// riff has measured parses back through this parser unchanged.
pub fn parse_replaygain_gain(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    let bytes = trimmed.as_bytes();
    let without_unit = if bytes.len() >= 2
        && bytes[bytes.len() - 2].eq_ignore_ascii_case(&b'd')
        && bytes[bytes.len() - 1].eq_ignore_ascii_case(&b'b')
    {
        // Both suffix bytes are ASCII, so the slice is char-boundary safe.
        trimmed[..trimmed.len() - 2].trim_end()
    } else {
        trimmed
    };
    without_unit.parse::<f32>().ok()
}

/// Parse an RFC 7845 R128 gain value: an integer in Q7.8 — units of 1/256 dB
/// — so `-1674` is `−6.5390625 dB`. Malformed values yield `None`.
fn parse_r128_gain(s: &str) -> Option<f32> {
    #[allow(clippy::cast_precision_loss)]
    s.trim().parse::<i32>().ok().map(|raw| raw as f32 / 256.0)
}

/// Format facts for the richer inherent read API: the stream's shape plus
/// its total duration. (The library slice's port carries the slim two-field
/// variant without the duration, which travels in the tuple instead.)
#[derive(Debug, Clone)]
pub struct AudioFormatInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Option<Duration>,
}

/// Extract a year from a tag text value: a bare `"1959"` parses directly,
/// and date-style values (`"1959-08-17"`, full `ID3v2` timestamps) yield
/// their leading four digits. Values without a plausible leading year give
/// `None`. Byte-sliced over the ASCII digit prefix — no per-char iterator
/// collect.
fn year_from_text(s: &str) -> Option<u32> {
    let bytes = s.trim().as_bytes();
    let mut end = 0;
    while end < 4 && end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == 0 {
        return None;
    }
    // The prefix is all ASCII digits, so slicing is char-boundary safe.
    std::str::from_utf8(&bytes[..end]).ok()?.parse::<u32>().ok()
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

    fn read_tagged_file(path: &Path) -> Result<TaggedFile, LibraryError> {
        read_from_path(path)
            .map_err(|e| LibraryError::MetadataRead(format!("Failed to read file: {e}")))
    }

    /// The primary tag of a tagged file, falling back to the first attached
    /// tag of any type, or `None` when the file carries no tags at all.
    fn best_tag(tagged_file: &TaggedFile) -> Option<&Tag> {
        tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
    }

    /// Extract [`TrackMetadata`] from a tag, knowing which format the file
    /// is: Opus carries its `ReplayGain` in the R128 convention per RFC 7845
    /// (Q7.8 unit encoding), every other format in the RG convention.
    fn metadata_from_tag(tag: &Tag, opus: bool) -> TrackMetadata {
        let text_val =
            |item: &lofty::tag::TagItem| -> Option<String> { item.value().clone().into_string() };

        let mut metadata = TrackMetadata::default();
        if let Some(item) = tag.get(ItemKey::TrackTitle) {
            metadata.title = text_val(item);
        }
        if let Some(item) = tag.get(ItemKey::TrackArtist) {
            metadata.artist = text_val(item);
        }
        if let Some(item) = tag.get(ItemKey::AlbumTitle) {
            metadata.album = text_val(item);
        }
        if let Some(item) = tag.get(ItemKey::AlbumArtist) {
            metadata.album_artist = text_val(item);
        }
        if let Some(item) = tag.get(ItemKey::TrackNumber) {
            metadata.track_number = text_val(item).and_then(|s| s.parse::<u32>().ok());
        }
        if let Some(item) = tag.get(ItemKey::DiscNumber) {
            metadata.disc_number = text_val(item).and_then(|s| s.parse::<u32>().ok());
        }
        if let Some(item) = tag.get(ItemKey::Genre) {
            metadata.genre = text_val(item);
        }
        // Year: a dedicated `Year` item wins (APE); otherwise the year is
        // the leading digits of a `RecordingDate` (ID3v2 `TDRC`, Vorbis
        // `DATE`, RIFF `ICRD`, MP4 `©day`). This mirrors what
        // `LoftyMetadataWriter` writes for years, so tags written through
        // it round-trip when the file is re-read.
        metadata.year = tag
            .get(ItemKey::Year)
            .or_else(|| tag.get(ItemKey::RecordingDate))
            .and_then(text_val)
            .and_then(|s| year_from_text(&s));
        if let Some(item) = tag.get(ItemKey::Comment) {
            metadata.comment = text_val(item);
        }
        if let Some(item) = tag.get(ItemKey::Composer) {
            metadata.composer = text_val(item);
        }

        // ReplayGain (Task 4.3): the dedicated `ItemKey` variants are passed
        // by value (`ItemKey` is `Copy` as of lofty 0.25).
        //
        // Gains: on Opus the RFC 7845 R128 items win — an integer in Q7.8
        // (units of 1/256 dB) — falling back to the RG-convention items other
        // tools may have written. Peaks have no R128 standard, so they ride
        // the RG-convention items on every format, bare linear ratios.
        metadata.replaygain_track_gain = if opus {
            tag.get_string(ItemKey::R128TrackGain)
                .and_then(parse_r128_gain)
                .or_else(|| {
                    tag.get_string(ItemKey::ReplayGainTrackGain)
                        .and_then(parse_replaygain_gain)
                })
        } else {
            tag.get_string(ItemKey::ReplayGainTrackGain)
                .and_then(parse_replaygain_gain)
        };
        metadata.replaygain_track_peak = tag
            .get_string(ItemKey::ReplayGainTrackPeak)
            .and_then(|s| s.trim().parse::<f32>().ok());
        metadata.replaygain_album_gain = if opus {
            tag.get_string(ItemKey::R128AlbumGain)
                .and_then(parse_r128_gain)
                .or_else(|| {
                    tag.get_string(ItemKey::ReplayGainAlbumGain)
                        .and_then(parse_replaygain_gain)
                })
        } else {
            tag.get_string(ItemKey::ReplayGainAlbumGain)
                .and_then(parse_replaygain_gain)
        };
        metadata.replaygain_album_peak = tag
            .get_string(ItemKey::ReplayGainAlbumPeak)
            .and_then(|s| s.trim().parse::<f32>().ok());

        metadata
    }

    /// Whether the tagged file is an Opus stream — the one format whose
    /// `ReplayGain` tags follow the RFC 7845 R128 convention.
    fn is_opus(tagged_file: &TaggedFile) -> bool {
        tagged_file.file_type() == lofty::file::FileType::Opus
    }

    /// Locate embedded JPEG/PNG cover art in a tag, if any.
    fn cover_from_tag(tag: &Tag) -> CoverSource {
        for picture in tag.pictures() {
            let mime_type = picture.mime_type();
            if mime_type == Some(&lofty::picture::MimeType::Jpeg)
                || mime_type == Some(&lofty::picture::MimeType::Png)
            {
                return CoverSource::Embedded(Arc::from(picture.data()));
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

// Fallback: symphonia. Called only when lofty's `read_from_path` fails, so
// the tags come from symphonia's `MetadataLog` rather than lofty's `Tag`.
// The statelessness convention (`#[allow(clippy::unused_self)]`) carries
// over so the port implementations can delegate one-for-one.
#[allow(clippy::unused_self)]
impl LoftyMetadataReader {
    /// Read the file through symphonia's format probe when lofty errors.
    /// Tags are pulled from the latest metadata revision's media container;
    /// stream facts come from the first audio track. ReplayGain is not
    /// exposed by symphonia's metadata layer, so those four fields stay
    /// `None`. Cover art is not surfaced either — the fallback returns
    /// `CoverSource::None`.
    fn read_via_symphonia(
        &self,
        path: &Path,
    ) -> Result<
        (
            TrackMetadata,
            Option<Duration>,
            CoverSource,
            AudioFormatInfo,
        ),
        LibraryError,
    > {
        let source = std::fs::File::open(path).map_err(|e| {
            LibraryError::MetadataRead(format!("Failed to open file: {e}"))
        })?;
        let mss = symphonia::core::io::MediaSourceStream::new(
            Box::new(source),
            symphonia::core::io::MediaSourceStreamOptions::default(),
        );
        let mut format: Box<dyn FormatReader> = get_probe()
            .probe(
                &Hint::new(),
                mss,
                FormatOptions::default(),
                symphonia::core::meta::MetadataOptions::default(),
            )
            .map_err(|e| LibraryError::MetadataRead(format!("Symphonia probe failed: {e}")))?;

        let track = format
            .tracks()
            .iter()
            .find(|t| {
                matches!(
                    t.codec_params,
                    Some(CodecParameters::Audio(ref p)) if p.codec != CODEC_ID_NULL_AUDIO
                )
            })
            .ok_or_else(|| LibraryError::MetadataRead("No audio track found".to_string()))?;

        let audio = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(p)) => p,
            _ => unreachable!("track filter guarantees Audio codec params"),
        };
        let sample_rate = audio
            .sample_rate
            .ok_or_else(|| LibraryError::MetadataRead("Unknown sample rate".to_string()))?;
        let channels = audio
            .channels
            .as_ref()
            .map_or(2u16, |c| u16::try_from(c.count()).unwrap_or(u16::MAX));
        let duration = track
            .num_frames
            .map(|frames| duration_from_frames(frames, sample_rate));

        // Pull the media-level tags out of the metadata log. The revision
        // reference returned by `skip_to_latest` is scoped to this block so
        // the borrow on `format.metadata()` cannot outlive the local.
        let tags_copy: Vec<symphonia::core::meta::Tag> = {
            let mut metadata = format.metadata();
            match metadata.skip_to_latest() {
                Some(revision) => revision.media.tags.clone(),
                None => Vec::new(),
            }
        };

        // The first tag whose standard form matches one of the given variant
        // extractors, mapped to a String (numbers are stringified for the
        // shared `Option<String>` return). Returning `None` when the file's
        // tags don't set that field matches lofty's own behavior.
        let first_std =
            |variants: &[fn(&StandardTag) -> Option<String>]| -> Option<String> {
                variants.iter().find_map(|variant| {
                    tags_copy
                        .iter()
                        .find_map(|t| t.std.as_ref().and_then(variant))
                })
            };

        let mut metadata = TrackMetadata::default();
        metadata.title = first_std(&[
            |s| match s {
                StandardTag::TrackTitle(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ]);
        metadata.artist = first_std(&[
            |s| match s {
                StandardTag::Artist(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ]);
        metadata.album = first_std(&[
            |s| match s {
                StandardTag::Album(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ]);
        metadata.album_artist = first_std(&[
            |s| match s {
                StandardTag::AlbumArtist(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ]);
        metadata.composer = first_std(&[
            |s| match s {
                StandardTag::Composer(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ]);
        metadata.comment = first_std(&[
            |s| match s {
                StandardTag::Comment(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ]);
        metadata.genre = first_std(&[
            |s| match s {
                StandardTag::Genre(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ]);
        metadata.track_number = first_std(&[
            |s| match s {
                StandardTag::TrackNumber(v) => Some(v.to_string()),
                _ => None,
            },
        ])
        .and_then(|s| s.parse::<u32>().ok());
        metadata.disc_number = first_std(&[
            |s| match s {
                StandardTag::DiscNumber(v) => Some(v.to_string()),
                _ => None,
            },
        ])
        .and_then(|s| s.parse::<u32>().ok());
        metadata.year = first_std(&[
            |s| match s {
                StandardTag::RecordingYear(v) => Some((*v as u32).to_string()),
                _ => None,
            },
            |s| match s {
                StandardTag::RecordingTime(v) => Some(v.as_str().to_owned()),
                _ => None,
            },
        ])
        .and_then(|s| year_from_text(&s));

        let audio_format = AudioFormatInfo {
            sample_rate,
            channels,
            duration: Some(duration.unwrap_or_default()),
        };

        Ok((metadata, duration, CoverSource::None, audio_format))
    }
}

// The reader is stateless, but the read methods take `&self` so the
// port implementations can delegate to them one-for-one.
#[allow(clippy::unused_self)]
impl LoftyMetadataReader {
    /// Read all metadata from a file: tags only, defaulting every absent
    /// field.
    pub fn read_metadata(&self, path: &Path) -> Result<TrackMetadata, LibraryError> {
        let tagged_file = Self::read_tagged_file(path)?;
        let Some(tag) = Self::best_tag(&tagged_file) else {
            return Ok(TrackMetadata::default());
        };
        Ok(Self::metadata_from_tag(tag, Self::is_opus(&tagged_file)))
    }

    pub fn read_cover_source(&self, path: &Path) -> Result<CoverSource, LibraryError> {
        let tagged_file = Self::read_tagged_file(path)?;
        Ok(match Self::best_tag(&tagged_file) {
            Some(tag) => Self::cover_from_tag(tag),
            None => CoverSource::None,
        })
    }

    /// One-pass read: metadata, duration, cover source, and audio format in
    /// a single file open. When lofty's `read_from_path` fails, fall back
    /// to symphonia's format probe so the file still enters the Library
    /// with its stream facts and any tags symphonia can surface. The
    /// fallback loses ReplayGain and cover art, which are not readable via
    /// symphonia's metadata layer.
    pub fn read_all(
        &self,
        path: &Path,
    ) -> Result<
        (
            TrackMetadata,
            Option<Duration>,
            CoverSource,
            AudioFormatInfo,
        ),
        LibraryError,
    > {
        let tagged_file = match Self::read_tagged_file(path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    "lofty failed for {:?}: {}; falling back to symphonia",
                    path,
                    e
                );
                return self.read_via_symphonia(path);
            }
        };

        let metadata = Self::best_tag(&tagged_file).map_or_else(TrackMetadata::default, |tag| {
            Self::metadata_from_tag(tag, Self::is_opus(&tagged_file))
        });
        let cover_source =
            Self::best_tag(&tagged_file).map_or(CoverSource::None, Self::cover_from_tag);
        let audio_format = Self::audio_format_from(&tagged_file);

        Ok((metadata, audio_format.duration, cover_source, audio_format))
    }
}

/// The library capability's reader port, served over the same lofty internals
/// as the richer inherent API above. The library port collapses the optional
/// duration into a default and carries the slim two-field format info.
impl riff_library::app::traits::MetadataReader for LoftyMetadataReader {
    fn read_all(
        &self,
        path: &Path,
    ) -> Result<
        (
            TrackMetadata,
            Duration,
            CoverSource,
            riff_library::app::traits::AudioFormatInfo,
        ),
        LibraryError,
    > {
        let (metadata, duration, cover_source, audio_format) = self.read_all(path)?;
        Ok((
            metadata,
            duration.unwrap_or_default(),
            cover_source,
            riff_library::app::traits::AudioFormatInfo {
                sample_rate: audio_format.sample_rate,
                channels: audio_format.channels,
            },
        ))
    }

    fn read_cover_source(&self, path: &Path) -> Result<CoverSource, LibraryError> {
        self.read_cover_source(path)
    }
}
