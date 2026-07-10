---
feature: Cover Art Resolution
epic: Music Library
status: implemented
priority: P1
depends_on: [Metadata Extraction]
personas: []
source_docs: []
implementation_notes: |
  Implemented in app/cover_resolver.rs (CoverResolver) with priority:
  embedded metadata > filesystem fallback (cover.jpg, folder.jpg, etc.)
  Case-insensitive lookup. Image decoding via image crate in infra/cover_loader.rs.
---

# Cover Art Resolution

## Problem Statement

Displaying album cover art is a core visual feature of any music player. Covers can be embedded in audio file metadata or stored as separate image files in the same directory. The player must implement a deterministic resolution strategy: prefer embedded metadata images, then fall back to common filesystem image names (case-insensitive).

## User / Personas

**Visual Listener**: A user who appreciates browsing their library by album covers and expects artwork to display consistently across all their files.

**Tagging Purist**: A user who embeds cover art in every file's metadata and expects that embedded art to take precedence over any filesystem images.

## Scope

**In scope:**
- Extract embedded cover art from audio metadata (ID3v2 APIC frames, FLAC/OGG Vorbis Comment METADATA_BLOCK_PICTURE, M4A covr atoms)
- Fall back to filesystem images in the same directory as the audio file
- Supported filesystem image names (case-insensitive): cover.jpg, cover.jpeg, cover.png, folder.jpg, folder.jpeg, folder.png, album.jpg, album.jpeg, album.png, front.jpg, front.jpeg, front.png
- Supported image formats: JPEG, PNG
- Image caching in memory (don't re-read from disk on every display frame)
- Image size limits (skip images >10MB to prevent memory issues)

**Out of scope:**
- GIF or WebP image support
- Online cover art lookup (discogs, musicbrainz, etc.) — explicitly offline-only player
- Cover art editing or embedding
- Multiple cover art per track (front, back, booklet) — only primary front cover
- Animated covers

## Boundary Conditions

- No embedded cover and no filesystem image: display a placeholder/blank cover
- Multiple valid filesystem images in one directory: use the first one found in the priority order (cover > folder > album > front)
- Corrupted image files: display a broken image placeholder, don't crash
- Very large embedded images (>10MB): skip them and fall back to filesystem or placeholder
- Case sensitivity: filesystem image lookup must be case-insensitive (works on case-sensitive filesystems like ext4)

## Assumptions

- Most users have either embedded cover art OR a cover.jpg in the album directory, not both
- The `image` crate can decode JPEG and PNG reliably in pure Rust
- Embedded images are typically front covers, not back covers or booklet pages
- Memory is sufficient to hold decoded cover art for the currently visible tracks (dozens of images, not thousands)

## Scenarios

### Scenario 1: Display embedded cover art
A user's audio file has cover art embedded in its metadata.

**Acceptance Criteria:**
- Given an MP3 file with an APIC frame containing a JPEG image, when the cover art resolver processes it, then the embedded JPEG is extracted and decoded for display
- Given a FLAC file with a METADATA_BLOCK_PICTURE block containing a PNG, when processed, then the embedded PNG is extracted and decoded
- Given any file with embedded cover art, when the player displays the track, then the embedded cover takes precedence over any filesystem image in the same directory

### Scenario 2: Display filesystem cover art
A user's album directory contains a cover image but files have no embedded art.

**Acceptance Criteria:**
- Given an audio file with no embedded cover art and a `cover.jpg` in the same directory, when the track is displayed, then `cover.jpg` is loaded and displayed
- Given a directory with `Cover.PNG` (mixed case) and no embedded art, when the track is displayed, then the case-insensitive lookup finds and displays `Cover.PNG`
- Given a directory with multiple candidate images (`cover.jpg`, `folder.png`), when the track is displayed, then `cover.jpg` is chosen because it has higher priority

### Scenario 3: Handle missing cover art
A track has neither embedded art nor a filesystem image.

**Acceptance Criteria:**
- Given a track with no embedded cover and no filesystem image in its directory, when displayed, then a default placeholder/blank cover is shown
- Given a track with corrupted embedded image data, when displayed, then the resolver falls back to filesystem images if available, or shows the placeholder

## Implementation Notes

1. **Resolution strategy**: Implement a `CoverArtResolver` with a `resolve(track_path)` method that returns an enum: `Embedded(Vec<u8>)`, `Filesystem(PathBuf)`, or `None`. The strategy is:
   a. Check embedded metadata via lofty (read APIC, METADATA_BLOCK_PICTURE, covr)
   b. If no embedded, scan the directory for case-insensitive matches against the priority list
   c. Return the first match found
2. **Image decoding**: Use the `image` crate to decode JPEG/PNG to an RGBA buffer. Handle decode errors gracefully (return None, log error).
3. **Caching**: Maintain an LRU cache of decoded cover images keyed by track path (or cover source path). Limit cache to 50 entries to control memory usage.
4. **Async loading**: Load and decode cover art on a background thread so the UI remains responsive when scrolling through a large library.
5. **Case-insensitive filesystem scanning**: Read the directory entries and compare lowercase filenames against the lowercase candidate list.

## Open Questions

- [ ] What should the placeholder cover look like? (Non-blocking: a simple colored square with the track/album name text, or just a gray box)
- [ ] Should we cache decoded images as raw RGBA or as the `image` crate's `DynamicImage`? (Non-blocking: `DynamicImage` is easier to pass to egui texture system)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
