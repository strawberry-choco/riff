---
feature: Metadata Extraction
epic: Music Library
status: implemented
priority: P0
depends_on: [Library Scanning]
personas: []
source_docs: []
implementation_notes: |
  Implemented via LoftyMetadataReader in infra/metadata_reader.rs using lofty.
  Extracts: Title, Artist, Album, Album Artist, Track Number, Disc Number,
  Genre, Year, Composer, Comment. Also reads cover source and duration.
---

# Metadata Extraction

## Problem Statement

Users organize their music by artist, album, and other metadata. Without accurate metadata extraction, the library view is useless — users see filenames instead of meaningful titles. The player must read standard audio metadata tags (ID3v2 for MP3, Vorbis Comments for OGG/FLAC, MP4 atoms for M4A) to build a searchable, browsable library.

## User / Personas

**Organized Collector**: A user who has meticulously tagged their music library with proper artist, album, and track names. They expect their tags to be read accurately.

**Casual Listener**: A user with a messy library. They expect basic metadata (artist, title) to be extracted, and accept filename-based fallback for missing tags.

## Scope

**In scope:**
- Extract from all supported formats: MP3 (ID3v1, ID3v2.3, ID3v2.4), FLAC/OGG (Vorbis Comments), M4A (MP4 ilst atoms), Opus (Vorbis Comments in OGG container), WAV (INFO chunk, ID3)
- Fields: Title, Artist, Album, Album Artist, Track Number, Disc Number, Genre, Year, Composer, Comment
- Graceful handling of missing tags (use empty string, not error)
- Graceful handling of malformed tags (skip malformed frame, read what is valid)
- Unicode support for all tag fields (UTF-8, UTF-16, Latin-1 as appropriate)

**Out of scope:**
- Writing or editing tags (read-only in MVP)
- Lyrics extraction
- ReplayGain metadata reading (volume normalization data)
- Cover art extraction (covered by separate Cover Art Resolution feature)
- Custom user-defined tag fields beyond the standard set

## Boundary Conditions

- Files with no metadata tags at all: all fields return empty strings
- Files with partial metadata: available fields are populated, missing fields are empty
- Extremely large tag blocks (>1MB) are truncated to prevent memory issues
- Malformed UTF-16 sequences are replaced with replacement character (U+FFFD)
- Numeric fields (track number, year, disc number) are parsed as integers; invalid values are treated as missing

## Assumptions

- The `lofty` pure Rust library reads all target tag formats reliably
- Users have tagged their files with standard fields (artist, album, title)
- Album Artist is the primary sorting field for album grouping; Artist is the per-track field
- Filenames can be used as a crude fallback for Title when tags are missing

## Scenarios

### Scenario 1: Extract metadata from a well-tagged file
A user has a properly tagged MP3 or FLAC file.

**Acceptance Criteria:**
- Given a FLAC file with Vorbis Comments containing ARTIST=Radiohead, ALBUM=OK Computer, TITLE=Paranoid Android, when the metadata extractor processes it, then it returns the exact values for each field
- Given an MP3 file with ID3v2.4 tags containing TPE1, TALB, TIT2 frames with UTF-8 text, when processed, then the Unicode text is correctly decoded and stored

### Scenario 2: Handle missing or partial metadata
A user's file has some tags but not all.

**Acceptance Criteria:**
- Given an MP3 file with only a Title tag and no Artist or Album tags, when processed, then Title is populated and Artist/Album are empty strings
- Given a WAV file with no metadata tags at all, when processed, then all fields are empty strings
- Given any file with missing metadata, when the library displays it, then the UI shows empty fields gracefully (no "unknown" placeholders unless the user sets them)

### Scenario 3: Handle malformed metadata
A user has a file with corrupted or non-standard tag data.

**Acceptance Criteria:**
- Given an MP3 file with a corrupted ID3v2 frame, when processed, then the corrupted frame is skipped and any valid frames are still read
- Given a file with an unsupported tag version (e.g., ID3v2.2), when processed, then the extractor attempts to read what it can and does not crash
- Given a tag with invalid UTF-16 byte sequences, when processed, then invalid sequences are replaced with the Unicode replacement character

## Implementation Notes

1. **Metadata abstraction**: Create a `TrackMetadata` struct with all standard fields as `Option<String>` or `Option<u32>` for numeric fields. This normalizes across different tag formats.
2. **lofty integration**: Use `lofty::probe::read_from_path()` to read tags. Lofty automatically detects the tag format based on the file type.
3. **Field mapping**: Map lofty's format-specific tag items to the unified `TrackMetadata` fields. Document the mapping explicitly (e.g., `TagItem::Artist -> TrackMetadata::artist`).
4. **Error handling**: Distinguish between "file not found", "unsupported format", "no tags", and "malformed tags". Only malformed tag errors should be logged, not shown to the user.
5. **Filename fallback**: For the Title field specifically, if no title tag exists, parse the filename (remove extension, replace underscores with spaces) as a crude fallback. This is a convenience for untagged files.

## Open Questions

- [ ] Should we use `Option<String>` for all fields or empty string for missing? (Empty string is simpler for UI binding)
- [ ] Do we need to handle multi-value tags (e.g., multiple artists per track)? (Non-blocking: concatenate with comma for MVP)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
