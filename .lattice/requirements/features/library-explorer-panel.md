---
feature: Library Explorer Panel
epic: User Interface
status: partial
priority: P0
depends_on: [Library Search]
personas: []
source_docs: []
implementation_gaps: |
  Done: Flat searchable track list with scroll area, artist/album metadata display
  in central panel, track selection and double-click-to-play.
  Missing: Directory tree view, artist/album hierarchy browser, right-click context
  menu (Play, Add to Queue, Play Next), visual playing indicator inline.
---

# Library Explorer Panel

## Problem Statement

Users need to browse their music library in two complementary ways: (1) a file-system-like tree view that mirrors their directory structure, and (2) a metadata-based view that groups by artist and album. Both views must be available in the same panel, switchable via tabs or a toggle. Users must be able to navigate, select tracks, and initiate playback from either view.

## User / Personas

**Folder Navigator**: A user who organizes music by folder structure (e.g., /Music/Artist/Album/Track.mp3). They want to see their folders and files exactly as they are on disk.

**Metadata Browser**: A user who wants to browse by artist name and see all albums by that artist, regardless of where the files are stored.

## Scope

**In scope:**
- Tree view: Directory tree mirroring the filesystem, with expandable/collapsible folders
- Tree view shows audio files with metadata-derived display names (Artist - Title) rather than raw filenames
- Artist/Album view: Flat list of artists; clicking an artist shows their albums; clicking an album shows its tracks
- Search-integrated view: When the user types in the search box, both views filter to show only matching items
- Double-click a track to play it (replace queue)
- Right-click context menu: Play, Add to Queue, Play Next
- Visual indication of the currently playing track in both views

**Out of scope:**
- Drag-and-drop reordering of tracks
- In-place renaming of files or folders
- File deletion or management (read-only library)
- Album art thumbnails in the tree view (too expensive for large libraries in MVP)
- Smart/auto playlists in the explorer

## Boundary Conditions

- Library with 50,000 tracks must not freeze the UI when expanding an artist or folder
- Deep directory structures (>10 levels) must render correctly in the tree view
- Empty folders are shown in tree view but hidden in artist/album view
- The currently playing track is highlighted in both views with a distinct color/icon
- Selected track remains selected when switching between tree and artist/album views

## Assumptions

- Users have their music organized in a recognizable structure (either by folder or by tags)
- The artist/album view is generated from the in-memory metadata index, not from filesystem structure
- Double-click is the primary interaction for starting playback
- Single-click selects a track (for context menu or to see details)

## Scenarios

### Scenario 1: Browse by folder structure
A user navigates their music directory tree to find a track.

**Acceptance Criteria:**
- Given the library has been scanned, when the user looks at the tree view, then the root music folder and its subdirectories are displayed as an expandable tree
- Given a folder is collapsed, when the user clicks the expand arrow, then the folder's contents are revealed
- Given the user navigates to /Music/Radiohead/OK Computer, when they look at the track list, then the tracks in that folder are displayed with "Artist - Title" formatting

### Scenario 2: Browse by artist and album
A user wants to see all albums by a specific artist.

**Acceptance Criteria:**
- Given the library has been scanned, when the user switches to the artist/album view, then a list of unique artists is displayed, sorted alphabetically
- Given the user clicks on "Radiohead", when the view updates, then all albums attributed to Radiohead (via Album Artist field) are displayed
- Given the user clicks on "OK Computer", when the view updates, then all tracks from that album are displayed in track number order

### Scenario 3: Initiate playback from explorer
A user finds a track and wants to play it.

**Acceptance Criteria:**
- Given a track is visible in either view, when the user double-clicks it, then the playback queue is replaced with that track's album (or the selected track as a single-item queue) and playback begins
- Given a track is visible, when the user right-clicks and selects "Add to Queue", then the track is appended to the current playback queue
- Given a track is visible, when the user right-clicks and selects "Play Next", then the track is inserted immediately after the current track in the queue

## Implementation Notes

1. **Tree view widget**: Use egui's built-in tree widget or implement a recursive `show_tree` function that renders folders and files. Use `egui::collapsing_header::CollapsingState` for manual control of expand/collapse.
2. **Artist/Album view**: Build a nested data structure from the metadata index: `HashMap<Artist, HashMap<Album, Vec<Track>>>`. Render as two nested scrollable lists (artists on left, albums on right, tracks on far right — or artists top-level, then drill down).
3. **Search filtering**: Apply the search query as a filter to both views. In tree view, show only folders that contain matching tracks (prune empty branches). In artist/album view, show only matching artists/albums.
4. **Selection state**: Track the currently selected `TrackId` and highlight it. Also track the currently playing track (from the playback engine) and highlight it with a different style.
5. **Context menu**: Use `egui::ContextMenu` or a custom popup for right-click actions. Send actions to the playback queue via a channel or shared state.
6. **Performance**: Use `egui::ScrollArea` with `show_rows` for virtualized rendering of large lists. Don't render off-screen items.

## Open Questions

- [ ] Should double-clicking play the entire album or just the single track? (Default: single track, with option to play album via context menu)
- [ ] Should the tree view show all directories or only those containing audio files? (Default: only directories containing at least one supported audio file, with option to show all)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
