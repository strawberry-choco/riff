---
feature: library-explorer-panel
requirement_doc: .lattice/requirements/features/library-explorer-panel.md
created: 2026-07-12
design_status: approved
---

# Library Explorer Panel

Design blueprint for adding a dual Library/Folders view toggle to the library explorer panel, with folder tree navigation, folder-level playback, context menus, and view-state preservation.

## Design: Level 1 — Capabilities

### User-Facing Capabilities

1. **Toggle Between Library and Folder Views** — A toggle in the library sidebar switches the left panel between metadata hierarchy (Artist → Album → Track) and filesystem tree (Folder → Subfolder → Track). Each view preserves its own selection state when switching.

2. **Browse by Folder Structure** — Display configured library paths as virtual roots in an expandable/collapsible tree. Only directories containing audio files (directly or in subdirectories) are shown. Lazy-loaded: children populate only when expanded. Selected folder shows its direct tracks in the right-side track panel.

3. **Browse by Artist and Album** — Existing behavior preserved: expand artists to see albums, expand albums to see tracks. Sorted alphabetically (artists), by year then title (albums), by track number (tracks).

4. **Play a Folder as Ad-Hoc Playlist** — Double-click or right-click "Play" on a folder node replaces the playback queue with all tracks from that folder and its subdirectories, starting playback from the first track. Right-click "Play Next" / "Append to Queue" insert or append the folder's tracks.

5. **Initiate Playback from a Track** — Double-click a track in either view replaces the queue with its containing album (or just the track) and plays. Right-click context menu provides Play, Play Next, Add to Queue.

6. **Search Across Both Views** — The existing search bar filters both views: in Library view by metadata fields, in Folder view by track filename/title within the selected folder's subtree, with empty branches pruned.

7. **Now Playing Indicator** — The currently-playing track is highlighted with a ▶ prefix and distinct styling in whichever view is active, updating as the queue advances.

### System Capabilities

8. **View State Preservation** — Per-view selection state (`selected_folder`, `selected_artist`) survives view toggling. The selected track in the right-side details panel is view-independent.

9. **Folder Tree from Library Index** — The folder tree is built from `LibraryManager.tracks` (path-to-track mapping), not re-reading the filesystem per interaction.

### Decisions Made (Level 1)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| Folder tree built from library index, not filesystem | The path-to-track mapping already exists in `LibraryManager.tracks`. No need for a separate tree data structure or filesystem re-reads. | Re-reading filesystem per expansion (slow for large libraries); separate in-memory tree index |
| Per-view selection state | `selected_folder` and `selected_artist` survive view toggling. `selected_track` is view-independent (always shown in right panel). | Single shared selection (loses context on switch); full layout re-render on switch |
| Library view reuses existing `render_artist_view` / `render_flat_view` | These already implement Artist → Album → Track hierarchy. No rewrite needed. | New widget from scratch |

### Constraints

- MUST build folder tree from `LibraryManager.tracks`, not from filesystem reads during UI interaction
- MUST NOT introduce new domain types — reuse existing `Track`, `TrackId`, `PathBuf`
- MUST follow existing egui patterns: `CollapsingHeader` for hierarchy, `selectable_label` for tracks, `context_menu` for right-click
- MUST keep the right-side track details panel unchanged (still shows selected track metadata + cover)
- MUST reuse existing `PlaybackCommand` variants (`Play`, `PlayNext`, `AddToQueue`)

## Design: Level 2 — Components

### Layer Mapping

| Component | Layer | New/Existing | Files | Description |
|-----------|-------|-------------|-------|-------------|
| BrowseMode enum | Application (State) | New | `src/app/state.rs` | `Library \| Folders` enum controlling which view the sidebar renders |
| FolderViewState | Application (State) | New | `src/app/state.rs` | New fields: `selected_folder: Option<PathBuf>`, `selected_artist: Option<String>` — per-view selection |
| LibraryManager queries | Application | New | `src/app/library_manager.rs` | `tracks_in_folder()`, `subdirs_with_audio()` — query methods derived from existing `tracks` map |
| FolderTreeWidget | Presentation | New | `src/ui/app.rs` | Renders expandable/collapsible folder tree using `egui::CollapsingHeader` with lazy-loaded children |
| ViewToggle | Presentation | Change | `src/ui/app.rs` | Replaces "All Tracks" / "Artists" toggle with "Library" / "Folders" selectable labels |
| FolderContextMenu | Presentation | New | `src/ui/app.rs` | Right-click context menu on folder nodes: Play, Play Next, Append to Queue |
| Existing renderers | Presentation | No change (ref) | `src/ui/app.rs` | `render_artist_view()` and `render_flat_view()` reused for Library mode |

### DDD Classification

- **BrowseMode** — Value Object enum (two states: `Library`, `Folders`). No identity, simple discriminant.
- **FolderViewState** — Not a domain concept. Pure UI view state living in `AppState` alongside other UI state.
- **LibraryManager** — Existing Aggregate (unchanged). New query methods are read-only projections over the existing `tracks` map.
- **FolderNode** — Not a domain entity. The folder tree is a UI projection of the track index. No separate data structure needed in domain.

No new domain types needed. The folder tree is purely a UI projection over the existing `LibraryManager` aggregate.

### Architecture Validation

- ✅ All new components in correct layers: BrowseMode/AppState in Application; FolderTreeWidget/ViewToggle/FolderContextMenu in Presentation
- ✅ No domain layer changes — no new domain types, no external dependency imports in domain
- ✅ Dependency direction inward: UI → App State → LibraryManager (Application) → Domain types
- ✅ No infrastructure changes needed — no new external crates

### Decisions Made (Level 2)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| No domain types for folder tree | Folder tree is a UI projection of the track index. Adding a `FolderNode` entity would duplicate the path-to-track mapping already in `LibraryManager.tracks`. | `FolderNode` entity in domain; separate `FolderTree` data structure |
| BrowseMode as flat enum in AppState | Simple two-state discriminant. No need for a state machine or separate struct. | Nested enum with per-mode state; separate widget state structs |
| Query methods on LibraryManager | `tracks_in_folder()` and `subdirs_with_audio()` are read-only projections. Natural fit on the existing aggregate. | Separate `FolderQueryService`; compute in UI layer from raw tracks map |

## Decisions Log

| Date | Decision | Reasoning | Alternatives Considered |
|------|----------|-----------|------------------------|
| 2026-07-12 | Folder tree built from LibraryManager.tracks, not filesystem | Path-to-track mapping already exists. No separate tree data structure needed. | Re-reading filesystem per expansion; separate in-memory tree index |
| 2026-07-12 | Per-view selection state: selected_folder + selected_artist | Preserves context when toggling views. selected_track is view-independent. | Single shared selection; full layout re-render on switch |
| 2026-07-12 | Library view reuses existing render_artist_view / render_flat_view | Already implements Artist→Album→Track hierarchy. No rewrite. | New widget from scratch |
| 2026-07-12 | No domain types for folder tree | UI projection over existing index. Adding domain entity would duplicate data. | FolderNode entity in domain; separate FolderTree structure |
| 2026-07-12 | BrowseMode as flat enum in AppState | Simple two-state discriminant. Minimal state change. | Nested enum with per-mode state; separate widget state structs |
| 2026-07-12 | Query methods on LibraryManager | Read-only projections, natural fit on existing aggregate. | Separate FolderQueryService; compute in UI layer |
| 2026-07-12 | Double-click folder replaces queue (not appends) | Matches double-click track behavior (replaces queue with album). Consistent mental model. | Append on double-click; play only first track |
| 2026-07-12 | Folder track sort: track number then filename | Mixes metadata-aware sorting with filesystem fallback. Matches album track sort. | Filesystem order only; alphabetical by title |
| 2026-07-12 | Search in Folders view prunes empty branches | Reduces visual noise. User sees only paths containing matches. | Show all folders with empty-state labels |
| 2026-07-12 | Play Next inserts in reverse order | PlayNext inserts before next track. Reverse preserves folder order. | Insert forward (reverses order); batch command |
| 2026-07-12 | `subdirs_with_audio` returns flat PathBufs | UI constructs tree from flat list. Keeps domain flat. | Nested tree struct; pre-build in LibraryManager |
| 2026-07-12 | No trait for folder queries | Pure read operations on existing data. No test seam needed for MVP. | FolderQuery trait for testability |
| 2026-07-12 | "All Tracks" sub-toggle stays in Library mode | Existing behavior preserved. Only top-level toggle changes. | Remove All Tracks view |
| 2026-07-12 | Playback commands sent individually | Existing channel handles one-at-a-time. Simpler. | Batch PlaybackCommand; queue replacement cmd |
| 2026-07-12 | Design approved at Level 4 — blueprint complete | All levels defined, contracts specified, ready for implementation. | — |

## Design: Level 3 — Interactions

### Flow 1: Toggle Between Views

```
User clicks "Folders" selectable label in sidebar
  → state.browse_mode = BrowseMode::Folders
  → state.selected_artist = current artist selection (snapshot before switch)
  → SidePanel re-renders with FolderTreeWidget
  → If state.selected_folder is Some(folder), folder tree highlights previously-selected folder
  → Track details panel (right side) unchanged — still shows selected_track

User clicks "Library" selectable label
  → state.browse_mode = BrowseMode::Library
  → state.selected_folder = current folder selection (snapshot before switch)
  → SidePanel re-renders with Artist→Album hierarchy
  → If state.selected_artist is Some(artist), that artist's CollapsingHeader is opened
```

### Flow 2: Expand Folder Node (Lazy Load)

```
User clicks expand arrow on a folder node
  → egui::CollapsingHeader toggles open state
  → On expand:
    → lib_mgr.subdirs_with_audio(&folder_path)
    → For each direct child dir of folder_path:
        check if any track's file_path starts with that child path
        if yes → include
    → Each child dir: CollapsingHeader with folder name
    → Only dirs with audio files shown (recursive check)
  → On collapse: nothing to compute (egui hides children)
```

### Flow 3: Select Folder → Show Tracks

```
User single-clicks a folder node label
  → state.selected_folder = Some(folder_path)
  → Right-side track panel updates:
    → lib_mgr.tracks_in_folder(&folder_path)
    → Returns direct children tracks sorted by track number, then filename
    → Rendered as selectable_label list with ▶ indicator
```

### Flow 4: Double-Click Folder → Play Folder

```
User double-clicks a folder node
  → Collect all descendant TrackIds:
    lib_mgr.tracks.values()
        .filter(|t| t.file_path.starts_with(&folder_path))
        .map(|t| t.id.clone())
  → Sort by file_path
  → cmd.send(PlaybackCommand::Play(tracks[0]))
  → For remaining: cmd.send(PlaybackCommand::AddToQueue(track_id))
```

### Flow 5: Right-Click Folder Context Menu

```
User right-clicks a folder node
  → context_menu:
    "Play"        → replace queue + play folder (same as double-click)
    "Play Next"   → insert folder tracks after current track (reverse order to preserve)
    "Append to Queue" → add folder tracks to end of queue
```

### Flow 6: Search in Folders View

```
User types in search bar while BrowseMode::Folders
  → query = state.search_query.to_lowercase()
  → For each folder node:
      if no descendant track matches query → skip (prune)
      if at least one matches → render normally
  → Selected folder's track list: only show matching tracks
```

### Flow 7: Now Playing Indicator

```
Every frame:
  → current_track_id = state.queue.current_track()
  → In Library view: ▶ prefix on matching track label (existing behavior)
  → In Folders view: ▶ prefix on matching track label
  → In Folders view: highlight folder nodes that contain the current track
```

### Decisions Made (Level 3)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| Double-click folder replaces queue (not appends) | Matches double-click track behavior (replaces queue with album). Consistent mental model. | Append on double-click; play only the first track |
| Folder track sort: track number then filename | Mixes metadata-aware sorting with filesystem fallback. Matches album track sort in Library view. | Filesystem order only; alphabetical by title |
| Search in Folders view prunes empty branches | Reduces visual noise. User sees only paths that contain matches. | Show all folders with empty-state labels; collapse only |
| Play Next inserts in reverse order | `PlaybackCommand::PlayNext` inserts before the next track. Reversing the iteration preserves original folder order. | Insert in forward order (reverses folder order); batch PlayNext command |

## Design: Level 4 — Contracts

### New Types (src/app/state.rs)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseMode {
    Library,
    Folders,
}

impl Default for BrowseMode {
    fn default() -> Self { Self::Library }
}

// AppState additions:
pub struct AppState {
    // ... existing fields unchanged ...
    pub browse_mode: BrowseMode,           // default: Library
    pub selected_folder: Option<PathBuf>,  // Folders view selection
    pub selected_artist: Option<String>,   // Library view selection
    // selected_track: Option<TrackId> remains view-independent
}
```

### LibraryManager Query Methods (src/app/library_manager.rs)

```rust
impl LibraryManager {
    /// Direct child tracks of a folder, sorted by track number then filename.
    pub fn tracks_in_folder(&self, folder: &Path) -> Vec<&Track>;

    /// Immediate subdirectories containing audio files (recursive check).
    /// Used for lazy-loading folder tree children. Sorted alphabetically.
    pub fn subdirs_with_audio(&self, folder: &Path) -> Vec<PathBuf>;

    /// Whether any track exists under this folder tree.
    pub fn folder_has_audio(&self, folder: &Path) -> bool;

    /// All TrackIds within a folder and subdirectories, sorted by file_path.
    /// Used for folder-level playback (Play/PlayNext/Append).
    pub fn track_ids_in_folder_tree(&self, folder: &Path) -> Vec<TrackId>;
}
```

### UI Method Signatures (src/ui/app.rs)

```rust
impl RiffApp {
    /// Render the folder tree (virtual roots from library_paths).
    fn render_folder_tree(&mut self, ui, state, cmd);

    /// Collect descendant TrackIds and fan out PlaybackCommands.
    fn play_folder(&self, state, folder, cmd);
    fn play_next_folder(&self, state, folder, cmd);
    fn append_folder_to_queue(&self, state, folder, cmd);

    /// Recursively render a folder node (CollapsingHeader).
    /// Returns true if node/descendants contain the currently-playing track.
    fn render_folder_node(&mut self, ui, state, cmd, path, depth, current_track_id, query) -> bool;
}
```

### Modified show_library_view (replaces All Tracks/Artists toggle)

```rust
fn show_library_view(&mut self, ctx, state, cmd) {
    // ... SidePanel, search bar unchanged ...
    
    // REPLACED: "All Tracks" / "Artists" → "Library" / "Folders"
    ui.horizontal(|ui| {
        ui.selectable_label(state.browse_mode == BrowseMode::Library, "Library");
        ui.selectable_label(state.browse_mode == BrowseMode::Folders, "Folders");
    });
    
    match state.browse_mode {
        BrowseMode::Library => {
            // Existing sub-toggle + render_artist_view / render_flat_view
        }
        BrowseMode::Folders => {
            self.render_folder_tree(ui, state, cmd);
        }
    }
    // Right panel unchanged
}
```

### Decisions Made (Level 4)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| `subdirs_with_audio` returns PathBufs, not a tree struct | Keeps domain layer flat. UI constructs tree from flat list. | Return nested struct; pre-build tree in LibraryManager |
| `track_ids_in_folder_tree` returns sorted Vec<TrackId> | Sorting by file_path matches filesystem order. Consumer sends PlaybackCommands. | Return unsorted; sort in UI |
| No trait/interface for folder queries | Pure read operations on existing data. No external dependency or test seam needed. | FolderQuery trait for testability |
| search_query reuse in Folders view | Prunes folder nodes with no matching descendants; filters track list. Same search bar, different filter logic per mode. | Separate search field per view |
| "All Tracks" sub-toggle remains in Library mode | Existing behavior: Library mode still has flat vs. artist view. Only the top-level toggle changes. | Remove "All Tracks" view; always show artist hierarchy |
| Playback commands sent individually (not batched) | Existing PlaybackCommand channel handles one-at-a-time. Simpler than adding a batch command. | Batch PlaybackCommand; queue replacement command |

### File Change Summary

| File | Action | What Changes |
|------|--------|-------------|
| `src/app/state.rs` | MODIFY | Add `BrowseMode` enum, `selected_folder`, `selected_artist` fields |
| `src/app/library_manager.rs` | MODIFY | Add 4 query methods: `tracks_in_folder`, `subdirs_with_audio`, `folder_has_audio`, `track_ids_in_folder_tree` |
| `src/ui/app.rs` | MODIFY | Replace view toggle, add `render_folder_tree`, `render_folder_node`, `play_folder`/`play_next_folder`/`append_folder_to_queue`, route by `browse_mode` |
| `src/domain/` | NO CHANGE | No new types needed |
| `src/infra/` | NO CHANGE | No new external dependencies |

## Design Summary

### Components and Layer Assignments

| Component | Layer | New/Existing | Files |
|-----------|-------|-------------|-------|
| BrowseMode enum | Application (State) | New | `src/app/state.rs` |
| FolderViewState fields | Application (State) | New | `src/app/state.rs` |
| LibraryManager queries | Application | New | `src/app/library_manager.rs` |
| ViewToggle (Library/Folders) | Presentation | Change | `src/ui/app.rs` |
| FolderTreeWidget | Presentation | New | `src/ui/app.rs` |
| FolderContextMenu | Presentation | New | `src/ui/app.rs` |
| Existing Library renderers | Presentation | No change (ref) | `src/ui/app.rs` |

### Key Contracts and Interfaces

1. **BrowseMode**: `Library | Folders` enum in AppState, defaulting to Library
2. **Per-view selection**: `selected_folder: Option<PathBuf>`, `selected_artist: Option<String>` — preserved across toggles
3. **LibraryManager.tracks_in_folder(path)**: Direct child tracks sorted by track number
4. **LibraryManager.subdirs_with_audio(path)**: Flat list of child dirs with audio, for lazy tree expansion
5. **LibraryManager.track_ids_in_folder_tree(path)**: All descendant TrackIds sorted by file_path, for folder playback
6. **Folder tree rendering**: Recursive `render_folder_node` using `CollapsingHeader`, lazy children via `subdirs_with_audio`
7. **Folder playback**: `play_folder` / `play_next_folder` / `append_folder_to_queue` fan out PlaybackCommands

### Architectural Constraints

- No new domain types — folder tree is a UI projection over LibraryManager aggregate
- No new infrastructure dependencies
- Search bar reused for both views, different filter logic per mode
- Right-side track details panel unchanged
- Existing PlaybackCommand variants reused (no batch/macro commands)
- LibraryManager query methods are read-only, zero side effects

### Domain Model Decisions

- Folder tree has no domain representation — computed on-the-fly from `Track.file_path`
- Track-to-folder relationship is implicit (derived from file_path prefix matching)
- Folder identity = PathBuf (same as library paths)
- No folder metadata stored in domain (folder name derived from path component)

### Open Questions Resolved During Design

- *Should the folder tree be pre-built or lazy?* → **Lazy**. Subdirectories computed on expand via `subdirs_with_audio()`. Pre-building for 50K tracks would be wasteful.
- *Should folder playback replace or append the queue?* → **Replace on Play, insert on Play Next, append on Append to Queue**. Matches existing track-level behavior.
- *Should "All Tracks" view remain?* → **Yes, in Library mode only**. The existing flat/artist sub-toggle stays within Library mode.
- *How to handle search in Folders view?* → **Prune empty branches**. If no descendant matches, folder node is hidden. Track list filtered normally.

### Design Status

**Approved — ready for implementation**

## Constraints

- MUST build folder tree from `LibraryManager.tracks`, not from filesystem reads during UI interaction
- MUST NOT introduce new domain types — reuse existing `Track`, `TrackId`, `PathBuf`
- MUST follow existing egui patterns: `CollapsingHeader` for hierarchy, `selectable_label` for tracks, `context_menu` for right-click
- MUST keep the right-side track details panel unchanged (still shows selected track metadata + cover)
- MUST reuse existing `PlaybackCommand` variants (`Play`, `PlayNext`, `AddToQueue`)
- MUST NOT add new crates to `Cargo.toml`

## Key Files

| File | Action | Description |
|------|--------|-------------|
| `src/app/state.rs` | MODIFY | Add `BrowseMode` enum, `selected_folder: Option<PathBuf>`, `selected_artist: Option<String>` |
| `src/app/library_manager.rs` | MODIFY | Add `tracks_in_folder()`, `subdirs_with_audio()`, `folder_has_audio()`, `track_ids_in_folder_tree()` |
| `src/ui/app.rs` | MODIFY | Replace view toggle, add `render_folder_tree()`, `render_folder_node()`, `play_folder()`/`play_next_folder()`/`append_folder_to_queue()`, route by `browse_mode` |
| `src/domain/` | NO CHANGE | No new domain types needed |
| `src/infra/` | NO CHANGE | No new external dependencies |
