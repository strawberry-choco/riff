use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::read_from_path;
use lofty::tag::{ItemKey, Tag, TagItem};
use riff_library::app::errors::LibraryError;
use riff_library::app::traits::{MetadataWriter, ReplayGainTags, ReplayGainWriter, TagEdit};
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

/// Lofty re-serializes every existing frame on save and validates each one
/// strictly — including the ID3v2 `COMM`/`USLT` language field, which
/// real-world taggers have written as malformed bytes (`"\x00en"`). Any
/// write to such a file would fail forever, so a malformed language field is
/// normalized to `XXX` (unknown language) before saving: the frame's
/// description and text are untouched, and the file becomes writable by riff
/// and by every other tool that re-serializes it.
fn sanitize_language_frames(tag: &mut Tag) {
    fn language_is_valid(lang: &[u8; 3]) -> bool {
        lang.iter().all(u8::is_ascii_alphabetic)
    }
    for key in [ItemKey::Comment, ItemKey::UnsyncLyrics] {
        let mut items: Vec<TagItem> = tag.take(key).collect();
        for item in &mut items {
            if !language_is_valid(item.lang()) {
                item.set_lang(*b"XXX");
            }
        }
        for item in items {
            tag.insert(item);
        }
    }
}

/// The one save-error formatter: lofty's top-level `Display` is only
/// "failed to write <format> file", so the source chain is walked to the
/// root cause and spelled out — the reason a write failed is exactly what
/// the user needs to see.
fn save_error(e: lofty::error::FileEncodingError) -> LibraryError {
    use std::error::Error as _;
    let mut message = e.to_string();
    let mut source: Option<&(dyn std::error::Error + 'static)> = e.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    LibraryError::MetadataWrite(message)
}

/// The primary tag for the format, falling back to any existing tag;
/// otherwise a new tag of the format's primary type.
fn writable_tag(tagged_file: &mut lofty::file::TaggedFile) -> &mut Tag {
    // Boolean probes so no borrow is carried across the branches.
    let has_primary = tagged_file.primary_tag().is_some();
    let has_any = has_primary || tagged_file.first_tag().is_some();

    if has_primary {
        return tagged_file.primary_tag_mut().unwrap();
    }
    if has_any {
        return tagged_file.first_tag_mut().unwrap();
    }
    let tag_type = tagged_file.primary_tag_type();
    tagged_file.insert_tag(Tag::new(tag_type));
    tagged_file
        .primary_tag_mut()
        .expect("tag just inserted must be present")
}

impl MetadataWriter for LoftyMetadataWriter {
    /// Write the `Some` fields of `edit` to the tags of the file at `path`;
    /// `None` fields leave the existing tag values untouched.
    fn write_tags(&self, path: &Path, edit: &TagEdit) -> Result<(), LibraryError> {
        let mut tagged_file = read_from_path(path)
            .map_err(|e| LibraryError::MetadataWrite(format!("failed to read file: {e}")))?;

        let tag = writable_tag(&mut tagged_file);
        sanitize_language_frames(tag);

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
        if let Some(track_number) = edit.track_number {
            tag.set_track(track_number);
        }
        if let Some(disc_number) = edit.disc_number {
            // No `Accessor` disc setter; write the keyed item directly —
            // the reader parses `DiscNumber` back through its text form.
            tag.insert_text(ItemKey::DiscNumber, disc_number.to_string());
        }
        if let Some(ref genre) = edit.genre {
            tag.set_genre(genre.clone());
        }
        if let Some(year) = edit.year {
            // `Accessor::set_year` was removed in lofty 0.25; write the keyed
            // item directly (replaces any same-key item, preserves the rest).
            // `RecordingDate` is what the old setter mapped to per format
            // (RIFF `ICRD`, ID3v2 `TDRC`, Vorbis `DATE`) — a bare `Year`
            // item has no RIFF INFO key and would be dropped on save.
            tag.insert_text(ItemKey::RecordingDate, year.to_string());
        }
        if let Some(ref composer) = edit.composer {
            tag.insert_text(ItemKey::Composer, composer.clone());
        }
        if let Some(ref comment) = edit.comment {
            tag.insert_text(ItemKey::Comment, comment.clone());
        }
        // `ReplayGain` is deliberately absent here: `TagEdit` has no gain
        // field, so a Metadata save can neither set nor clear the file's own
        // values. ReplayGain travels its own write path
        // ([`LoftyMetadataWriter::write_replaygain`]).

        tag.save_to_path(path, WriteOptions::default())
            .map_err(save_error)?;

        Ok(())
    }
}

/// One `ReplayGain` item as the tags hold it: the item key to write and the
/// exact string encoding, already format-resolved.
struct ReplayGainItem {
    key: ItemKey,
    value: String,
}

/// The string contract the reader's gain-string parser accepts, per format:
/// gains carry a ` dB` suffix (RG convention) or a Q7.8 integer (Opus, RFC
/// 7845); peaks are bare linear ratios on every format — the R128 convention
/// standardizes no peak item.
fn replaygain_items(opus: bool, tags: &ReplayGainTags) -> Vec<ReplayGainItem> {
    let mut items = Vec::new();
    if let Some(gain) = tags.track_gain {
        let (key, value) = if opus {
            (ItemKey::R128TrackGain, r128_gain_string(gain))
        } else {
            (ItemKey::ReplayGainTrackGain, format!("{gain:.2} dB"))
        };
        items.push(ReplayGainItem { key, value });
    }
    if let Some(peak) = tags.track_peak {
        items.push(ReplayGainItem {
            key: ItemKey::ReplayGainTrackPeak,
            value: format!("{peak:.6}"),
        });
    }
    if let Some(gain) = tags.album_gain {
        let (key, value) = if opus {
            (ItemKey::R128AlbumGain, r128_gain_string(gain))
        } else {
            (ItemKey::ReplayGainAlbumGain, format!("{gain:.2} dB"))
        };
        items.push(ReplayGainItem { key, value });
    }
    if let Some(peak) = tags.album_peak {
        items.push(ReplayGainItem {
            key: ItemKey::ReplayGainAlbumPeak,
            value: format!("{peak:.6}"),
        });
    }
    items
}

/// dB → the RFC 7845 R128 unit encoding: a Q7.8 integer, units of 1/256 dB
/// (`-6.54 dB` → `-1674`). The encoding is the format's own lossiness —
/// half a hundredth of a dB.
#[allow(clippy::cast_possible_truncation)] // rounding lands before the cast
fn r128_gain_string(gain_db: f32) -> String {
    let q78 = (f64::from(gain_db) * 256.0).round();
    format!("{}", q78 as i64)
}

impl ReplayGainWriter for LoftyMetadataWriter {
    /// Write the `Some` fields of `tags` to the tags of the file at `path`;
    /// `None` fields leave the existing tag values untouched, and no other
    /// item in the tag is disturbed. Only `Some` fields are ever written, so
    /// a write cannot clear a value it was not asked to set.
    fn write_replaygain(&self, path: &Path, tags: &ReplayGainTags) -> Result<(), LibraryError> {
        if tags.is_empty() {
            return Ok(());
        }

        let mut tagged_file = read_from_path(path)
            .map_err(|e| LibraryError::MetadataWrite(format!("failed to read file: {e}")))?;
        let opus = tagged_file.file_type() == lofty::file::FileType::Opus;

        let tag = writable_tag(&mut tagged_file);
        sanitize_language_frames(tag);
        for item in replaygain_items(opus, tags) {
            tag.insert_text(item.key, item.value);
        }

        tag.save_to_path(path, WriteOptions::default())
            .map_err(save_error)?;

        Ok(())
    }
}
