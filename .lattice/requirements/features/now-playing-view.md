---
feature: Now Playing View
epic: User Interface
status: partial
priority: P2
depends_on: [Playback Queue, Cover Art Display]
personas: []
source_docs: []
implementation_gaps: |
  Done: Basic view stub exists with track info (title, artist, album) and
  upcoming queue list. Toggle via ViewMode::NowPlaying.
  Missing: Large cover art display, full metadata fields (Album Artist, Year,
  Genre, Track Number), clickable up-next items, proper progress bar.
---

# Now Playing View

## Problem Statement

Users want a dedicated view that focuses on the currently playing track, showing large cover art, full track metadata, and a peek at upcoming tracks in the queue. This view provides an immersive, focused listening experience compared to the compact control bar.

## User / Personas

**Immersive Listener**: A user who wants to lean back and enjoy the music with a visually pleasing, focused view showing the current track in detail.

## Scope

**In scope:**
- Large cover art display (prominent, central)
- Full track metadata: Title, Artist, Album, Album Artist, Year, Genre, Track Number
- Progress bar with large, readable time display
- Playback controls (play/pause, next, previous)
- Up next queue: Show the next 5 tracks in the queue
- Clicking a track in the "up next" list jumps to that track
- Accessible via a button in the control bar or a dedicated panel toggle

**Out of scope:**
- Lyrics display
- Artist biography or album information
- Social features (share, scrobble)
- Visualization / spectrum analyzer
- Full-screen mode (just a larger panel within the main window)

## Boundary Conditions

- View is accessible even when the library panel is collapsed
- Large cover art is limited to the panel size (not full window)
- Up next list shows track names only (no cover thumbnails for performance)
- When the queue is empty, the "up next" section shows an empty state
- When no track is playing, the view shows an empty state or the last played track

## Assumptions

- Users will toggle to this view when they want a focused experience, and back to the library view when browsing
- The now playing view shares the same cover art resolution logic as the regular cover display
- Queue data is available and up-to-date (from the Playback Queue feature)

## Scenarios

### Scenario 1: View current track details
A user wants to see detailed information about the playing track.

**Acceptance Criteria:**
- Given a track is playing, when the user opens the Now Playing view, then large cover art and all metadata fields are displayed
- Given a track with complete metadata, when displayed, then all fields (Title, Artist, Album, Album Artist, Year, Genre, Track Number) are shown
- Given a track with missing metadata fields, when displayed, then only the available fields are shown (no "Unknown" placeholders)

### Scenario 2: See what's coming up next
A user wants to see the upcoming tracks in the queue.

**Acceptance Criteria:**
- Given the queue has tracks [A, B, C, D, E, F] and track A is playing, when the user opens Now Playing, then the "Up Next" section shows tracks B through F (or the next 5)
- Given the user clicks on track D in the "Up Next" list, when clicked, then playback jumps to track D and it begins playing
- Given the queue is empty, when the user opens Now Playing, then the "Up Next" section shows "Queue is empty"

## Implementation Notes

1. **Panel layout**: Use an `egui::CentralPanel` or a dedicated tab/panel for the Now Playing view. When active, it replaces the library explorer + track list with a large, centered layout.
2. **Toggle mechanism**: Add a button (e.g., a "Now Playing" icon or text) in the control bar or top bar that toggles between Library view and Now Playing view.
3. **Cover art**: Use the same cover art resolution and texture caching as the main cover display, but render at a larger size (e.g., 400x400px).
4. **Queue peek**: Render a simple `egui::ScrollArea` with rows for upcoming tracks. Each row shows track number, title, artist, and duration. Clicking a row sends a "jump to track" command to the playback queue.
5. **Empty state**: When no track is playing, show a message: "Select a track to start playing" with a button to open the library.

## Open Questions

- [ ] Should Now Playing be a panel within the main window or a separate window/popup? (Non-blocking: panel within main window for MVP)
- [ ] Should the Now Playing view auto-open when a track starts playing? (Non-blocking: no, user manually toggles)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
