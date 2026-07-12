---
feature: Library Explorer Panel
epic: User Interface
status: implemented
priority: P0
depends_on: [Library Search]
personas: [Folder Navigator, Metadata Browser]
source_docs: []
---

# Library Explorer Panel

## Problem Statement

Users need to discover and play music through two mental models — by metadata (Artist → Album → Track) and by filesystem location (Folder → Subfolder → Track) — and switch between them seamlessly. Currently, only the metadata view exists. Without a folder view, users who organize music by directory structure have no way to navigate their library as they see it on disk. Without folder-level playback, a user who wants to play everything in a folder must manually select each track.

## User / Personas

**Folder Navigator**: A user who organizes music by directory structure (e.g., /Music/Artist/Album/Track.mp3). They want to browse and play exactly what's on disk, mirroring their file manager.

**Metadata Browser**: A user who wants to browse by artist name and see all albums by that artist, regardless of where the files are stored on disk.

## Scope

**In scope:**
- A toggle button in the library sidebar that switches between "Library" (metadata hierarchy) and "Folders" (filesystem tree) views
- Folder tree: virtual roots from configured library paths, expandable/collapsible, lazy-loaded children
- Tree shows only directories containing at least one audio file (in that directory or any subdirectory)
- Double-click a folder → replace queue with all tracks in that folder and subdirectories, start playback
- Right-click a folder → context menu: Play, Play Next, Append to Queue
- Right-click a track → context menu: Play, Play Next, Add to Queue
- Search filters both views: in Library view by metadata fields, in Folders view by path/title within the selected folder's subtree
- Visual "now playing" indicator (▶) on the currently-playing track in both views
- Selection state persists when toggling between views

**Out of scope:**
- Drag-and-drop reordering or folder reorganization
- In-place rename, delete, or file management on disk
- Recursive display toggle ("show all tracks in subfolders" as a per-folder switch)
- Album art thumbnails in the folder tree (performance concern for large libraries)
- OS file manager integration ("Show in File Explorer" context menu action)
- Breadcrumb navigation bar above the track list

## Boundary Conditions

- Library with 50,000+ tracks must not freeze the UI when expanding a folder node
- Deep directory structures (10+ levels) render correctly with proper indentation
- Empty folders appear in the tree but show "No audio files" in the track panel when selected
- If the currently-playing track exists in the active view, it is highlighted with a ▶ indicator and distinct styling
- When switching views, the previously-selected item (folder or artist) is remembered per-view
- Selected track in the right-side details panel remains selected regardless of view toggle

## Assumptions

- Folder tree is built from the library index (`LibraryManager`), not by re-reading the filesystem on every interaction — the path-to-track mapping already exists via `Track.file_path`
- Search in Folders view filters by filename, title, and path within the currently selected folder's subtree
- Folder hierarchy mirrors the physical directory structure, not a virtual reorganization
- Single-click selects a track or folder; double-click initiates playback
- The "Library" view uses the existing artist/album hierarchy already implemented

## Scenarios

### Scenario 1: Toggle between library and folder views

A user switches the sidebar between metadata-based and filesystem-based browsing, with each view preserving its own state.

**Acceptance Criteria:**
- Given the user is in the Library view with an artist selected, when they click the view toggle to switch to Folders view, then the sidebar shows the folder tree and the previously-selected folder (if any) is highlighted
- Given the user is in the Folders view with a folder selected, when they click the view toggle to switch to Library view, then the sidebar shows the artist/album hierarchy and the previously-selected artist (if any) is highlighted
- Given the user has a track selected in either view, when they toggle views, then the selected track remains selected in the right-side track details panel
- Given a track is currently playing, when the user toggles views, then the playing track is highlighted with a ▶ indicator in whichever view is currently active

### Scenario 2: Browse by folder structure

A user navigates their music directory tree to find tracks.

**Acceptance Criteria:**
- Given configured library paths exist, when the user opens the Folders view, then each library path is displayed as a top-level root node in the tree, labeled with its folder name (not the full path)
- Given a folder node is collapsed, when the user clicks its expand arrow, then its immediate subdirectories are loaded and displayed, and only directories containing at least one audio file (directly or in any subdirectory) are shown
- Given a folder node is selected, when the track panel updates, then all tracks directly inside that folder are displayed with "Artist - Title" formatting, sorted by track number then filename
- Given the user types a search query in the search bar while in Folders view, when the tree filters, then only folders containing tracks that match the query are displayed and empty branches are pruned

### Scenario 3: Browse by artist and album

A user browses their library by metadata hierarchy.

**Acceptance Criteria:**
- Given the library has been scanned, when the user switches to the Library view, then a list of unique artists is displayed sorted alphabetically with expand/collapse arrows
- Given the user clicks an artist name, when the view expands, then all albums attributed to that artist (via Album Artist field, falling back to Track Artist) are displayed, sorted by year descending then title
- Given the user clicks an album, when the view expands, then all tracks from that album are displayed in track-number order
- Given the user types a search query in the Library view, when the view filters, then only artists, albums, and tracks matching the query are displayed

### Scenario 4: Play a folder as an ad-hoc playlist

A user wants to play everything in a folder without selecting individual tracks.

**Acceptance Criteria:**
- Given a folder node is visible in the tree, when the user double-clicks the folder, then all audio tracks in that folder and its subdirectories are added to the playback queue (replacing the current queue) and playback starts from the first track
- Given a folder node is visible, when the user right-clicks the folder and selects "Play", then all tracks in that folder and its subdirectories replace the queue and playback starts from the first track
- Given a folder node is visible, when the user right-clicks the folder and selects "Play Next", then all tracks in that folder and its subdirectories are inserted immediately after the currently-playing track in the queue
- Given a folder node is visible, when the user right-clicks the folder and selects "Append to Queue", then all tracks in that folder and its subdirectories are added to the end of the queue without interrupting playback

### Scenario 5: Initiate playback from a track

A user finds a specific track and plays it or queues it.

**Acceptance Criteria:**
- Given a track is visible in either view, when the user double-clicks it, then the playback queue is replaced with the album containing that track (or just the track if album context is unavailable) and playback begins
- Given a track is visible, when the user right-clicks and selects "Play", then the track replaces the queue and playback begins
- Given a track is visible, when the user right-clicks and selects "Add to Queue", then the track is appended to the end of the current queue
- Given a track is visible, when the user right-clicks and selects "Play Next", then the track is inserted immediately after the currently-playing track in the queue
- Given a track is currently playing, when the user looks at either view, then the playing track is highlighted with a ▶ indicator and distinct styling that differentiates it from the selected-but-not-playing state

## Implementation Notes

1. **View toggle + state preservation** — Add a toggle button (e.g., "Library" / "Folders" selectable labels) in the library sidebar that switches the left panel's content. Extend `AppState` with per-view selection state (`selected_folder: Option<PathBuf>`, `selected_artist: Option<String>`) that survives view switching.
2. **Folder tree widget** — Build a recursive function rendering directory nodes using `egui::CollapsingHeader`. Use `egui::collapsing_header::CollapsingState` for manual expand/collapse control. Lazy-load children: only add child nodes when a directory is expanded. Derive virtual roots from `library_paths`.
3. **Folder playback** — On double-click of a folder node, collect all `TrackId`s whose `file_path` is a descendant of the folder, fan out into `PlaybackCommand::Play(track_id)` for the first and `PlaybackCommand::AddToQueue` for the rest. Context menu reuses the same submenu widget pattern already used for tracks.
4. **Search coexistence** — Apply the existing search query (`state.search_query`) as a filter. In Library view, reuse the existing `LibraryManager::search()`. In Folders view, filter the tree by checking whether any descendant track matches; prune nodes with no matches.
5. **Track playback indicators** — Track the currently-playing `TrackId` from the playback engine and highlight it in both views with a ▶ prefix and distinct text color. Ensure the indicator updates when the queue advances to the next track.

## Open Questions

- None.

## Links

- Design: [library-explorer-panel](../../context/library-explorer-panel.md) — Approved, ready for implementation
- Epic index: [index.md](../index.md)
