---
feature: Cover Art Display
epic: User Interface
status: partial
priority: P1
depends_on: [Cover Art Resolution]
personas: []
source_docs: []
implementation_gaps: |
  Done: Full resolution pipeline exists (CoverResolver + ImageCoverLoader).
  Missing: Actual texture display in UI (currently shows emoji placeholder).
  Cover textures are not wired from resolver to egui rendering. Also missing
  LRU caching for decoded cover images. UI refactor in progress.
---

# Cover Art Display

## Problem Statement

Album artwork is a core visual element of the music listening experience. The UI must display the cover art for the currently playing track, resolving it from embedded metadata or filesystem images as defined in the Cover Art Resolution feature. The display must be responsive, handle missing art gracefully, and update immediately when the track changes.

## User / Personas

**Visual Listener**: A user who appreciates album artwork and wants a prominent, beautiful display of the current track's cover in the player window.

## Scope

**In scope:**
- Display cover art in the main player window (e.g., in the center panel or a dedicated cover art panel)
- Cover art updates immediately when the playing track changes
- Cover art updates when navigating the library (preview mode: show cover for selected track even if not playing)
- Placeholder display when no cover art is available (gray box with album/artist text, or a default icon)
- Support for JPEG and PNG images
- Image scaling to fit the available panel size (maintain aspect ratio, no distortion)
- Optional: Click to view larger cover art in a popup

**Out of scope:**
- Animated covers or GIFs
- Multiple cover images per album (front, back, booklet)
- Cover art editing or embedding
- Online cover art download or replacement
- Blurred background based on cover art colors (can be added later)

## Boundary Conditions

- Image aspect ratio is preserved (letterboxing or padding as needed)
- Maximum displayed dimension: 600x600px (larger images are scaled down)
- Minimum displayed dimension: 100x100px (smaller images are scaled up)
- Very large images (>10MB) are not loaded into memory (skip them)
- Corrupted images show the placeholder
- Image loading is non-blocking (UI remains responsive while cover loads)

## Assumptions

- The `image` crate can decode JPEG and PNG to RGBA buffers that egui can display
- `egui_extras::image` or `egui::Image` widget can display the decoded image data
- Most tracks have cover art available (either embedded or filesystem)
- Cover art is typically square (1:1 aspect ratio), but non-square images must be handled

## Scenarios

### Scenario 1: Display cover for playing track
A track starts playing and its cover art is displayed.

**Acceptance Criteria:**
- Given a track with embedded cover art begins playing, when the track starts, then the cover art is extracted and displayed within 500ms
- Given a track with a filesystem cover.jpg begins playing, when the track starts, then the image is loaded from disk and displayed
- Given the cover art is loaded, when it is displayed, then it is scaled to fit the panel while preserving aspect ratio

### Scenario 2: Handle missing cover art
A track has no cover art available.

**Acceptance Criteria:**
- Given a track with no embedded or filesystem cover art, when it is selected or played, then a placeholder is displayed
- Given a placeholder is displayed, when shown, then it displays the album name and artist name as text on a neutral background
- Given a track with corrupted cover data, when processed, then the placeholder is displayed and an error is logged

### Scenario 3: Navigate library and preview covers
A user browses the library and wants to see covers for tracks they hover over or select.

**Acceptance Criteria:**
- Given the user selects a track in the library explorer, when the selection changes, then the cover art display updates to show the selected track's cover
- Given the user is not playing anything, when they select tracks, then the cover art display acts as a preview of the selected track
- Given a track is both selected and playing, when displayed, then the playing track's cover takes precedence

## Implementation Notes

1. **Image loading**: Use the `image` crate to decode JPEG/PNG to `image::DynamicImage`. Convert to RGBA8 and upload to an `egui::TextureHandle` using `ctx.load_texture()`.
2. **Texture management**: Maintain a cache of `TextureHandle`s keyed by cover source path (or track path). This avoids re-decoding and re-uploading the same image repeatedly.
3. **Async loading**: Load cover art on a background thread. The UI shows a placeholder while loading. When the image is ready, send the decoded bytes back to the UI thread via a channel to create the texture.
4. **Placeholder**: Use an `egui::Frame` with a gray fill and centered text (Album - Artist) as the placeholder.
5. **Aspect ratio**: Use `ui.image().max_width().max_height()` with `egui::Image::new()` and `fit_to_exact_size` or `maintain_aspect_ratio` options.

## Open Questions

- [ ] Should we display cover art in the library list as small thumbnails? (Non-blocking: too expensive for large libraries in MVP, defer)
- [ ] Should the cover art panel be resizable/collapsible? (Non-blocking: fixed size is fine for MVP)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
