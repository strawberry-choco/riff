use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::read_from_path;
use lofty::tag::{ItemKey, Tag};
use riff_library::app::errors::LibraryError;
use riff_library::app::traits::{MetadataWriter, TagEdit};
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
        if let Some(gain) = edit.replaygain_track_gain {
            // Mirror the reader's parsing contract (`parse_replaygain_gain`
            // strips a case-insensitive `dB` suffix), so a written tag
            // round-trips when the file is re-read.
            tag.insert_text(ItemKey::ReplayGainTrackGain, format!("{gain} dB"));
        }
        if let Some(peak) = edit.replaygain_track_peak {
            tag.insert_text(ItemKey::ReplayGainTrackPeak, format!("{peak}"));
        }

        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| LibraryError::MetadataWrite(e.to_string()))?;

        Ok(())
    }
}
