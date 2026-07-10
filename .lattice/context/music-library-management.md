---
feature: music-library-management
requirement_doc: .lattice/requirements/features/music-library-management.md
created: 2025-07-10
design_status: approved
---

# Music Library Management

Design blueprint for replacing the single text-input library path with a full library management system: multiple library paths, OS file picker, persistence, per-library scan + scan all, and a dedicated settings page.

---

## Design: Level 1 — Capabilities

### User-Facing Capabilities

1. **View Registered Libraries** — Display a list of all registered music library paths in a dedicated settings page, each showing path, status indicator, and action buttons.
2. **Add Library Path (macOS/Windows)** — Open OS native folder picker dialog to select a directory. Path appears in the list on confirmation.
3. **Add Library Path (Linux)** — Show text input field for manual path entry (no native file picker dependency). Path appears in the list on confirmation.
4. **Delete Library Path** — Remove a library path from the list with a single click. Does NOT delete files on disk.
5. **Scan Single Library** — Trigger a rescan of one specific library path, scanning only that directory.
6. **Scan All Libraries** — Trigger a rescan of every registered library path in sequence.
7. **Persist Library List** — Save/load the library path list across application restarts (survive quit/launch).
8. **Display Library Status** — Show per-library status: idle, scanning (with progress), scanned N tracks, unavailable (path missing on disk).
9. **Show Empty State** — When no libraries are configured, display a helpful placeholder message: "No music libraries configured. Add one to get started."
10. **Prevent Duplicate Paths** — Silently ignore duplicate library path additions (same canonical path already exists in the list).
11. **Handle Unavailable Paths** — Show a grayed-out/warning indicator for library paths pointing to no-longer-available locations (ejected drive, unmounted NAS). Allow deletion but skip scanning.

### Non-User-Facing / System Capabilities

12. **Native File Dialog Integration** — Conditionally compile `rfd` (Rusty File Dialogs) on macOS/Windows for the OS folder picker. No-op on Linux.
13. **Path Canonicalization** — Canonicalize paths before dedup to handle trailing slashes, symlinks, and equivalent path representations.
14. **State Migration** — Migrate from `library_path: Option<PathBuf>` (single) to `library_paths: Vec<PathBuf>` (multiple) in `AppState`.

---

## Design: Level 2 — Components

### Layer Mapping

| Component | Layer | Directory | Files | Description |
|-----------|-------|-----------|-------|-------------|
| LibraryPaths | App State | `src/app/state.rs` | `state.rs` | Replaces `library_path: Option<PathBuf>` with `library_paths: Vec<PathBuf>`. Added `library_statuses: HashMap<PathBuf, LibraryStatus>` |
| LibraryCommand | App Commands | `src/app/commands.rs` | `commands.rs` | Adds `AddLibrary(PathBuf)` and `RemoveLibrary(PathBuf)` variants |
| LibraryStatus | App State | `src/app/state.rs` | `state.rs` | New enum: `Idle | Scanning{found: usize} | Scanned(n: usize) | Unavailable` |
| ViewMode | App State | `src/app/state.rs` | `state.rs` | Adds `Settings` variant |
| LibrarySettingsPanel | UI | `src/ui/settings.rs` | `settings.rs` (NEW) | New UI panel: settings page with library list management |
| FileDialogHelper | UI | `src/ui/settings.rs` | `settings.rs` (NEW) | Conditionally compiled: wraps `rfd::FileDialog` on macOS/Windows, text input on Linux |
| LibraryPersistence | Application | `src/app/library_manager.rs` or new `src/app/library_persistence.rs` | (extend existing) | Save/load library paths list from disk via `eframe::Storage` or config file |
| LibraryScanner | Application (existing) | `src/infra/scanner.rs` | (existing) | Already handles `ScanDirectory`. Will be invoked per-library or all-libraries. |

### Component Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│  UI Layer (src/ui/)                                              │
│                                                                   │
│  ┌────────────────────────────────────────────┐                  │
│  │  LibrarySettingsPanel (NEW: settings.rs)   │                  │
│  │  ┌─────────────┐  ┌────────────────────┐   │                  │
│  │  │ LibraryList  │  │ ScanAllButton      │   │                  │
│  │  │  per-entry:  │  │ AddLibraryButton   │   │                  │
│  │  │  - status    │  │ BackButton         │   │                  │
│  │  │  - scan btn  │  └────────────────────┘   │                  │
│  │  │  - delete    │  ┌────────────────────┐   │                  │
│  │  │  - path text │  │ FileDialogHelper    │   │                  │
│  │  └─────────────┘  │  (cfg-gated)        │   │                  │
│  │                    │  macOS/Win: rfd     │   │                  │
│  │                    │  Linux: text input  │   │                  │
│  │                    └────────────────────┘   │                  │
│  └────────────────────────────────────────────┘                  │
│                                                                   │
│  ┌────────────────────────────────────────────┐                  │
│  │  RiffApp (existing: app.rs)                 │                  │
│  │  - ViewMode::Settings route in update()     │                  │
│  │  - Top bar gear icon → navigates to settings│                  │
│  └────────────────────────────────────────────┘                  │
└──────────────────────┬──────────────────────────────────────────┘
                       │ sends LibraryCommand
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│  Application Layer (src/app/)                                    │
│                                                                   │
│  ┌─────────────────────┐  ┌────────────────────────────────┐     │
│  │  commands.rs         │  │  LibraryManager (existing)      │     │
│  │  AddLibrary(PathBuf) │  │  library_paths: Vec<PathBuf>   │     │
│  │  RemoveLibrary(Path) │  │  scan_directory(path): impl    │     │
│  └─────────────────────┘  │  scan_all(): impl               │     │
│                            │  save_paths(): io::Result       │     │
│  ┌─────────────────────┐  │  load_paths(): impl             │     │
│  │  state.rs            │  └────────────────────────────────┘     │
│  │  library_paths        │                                        │
│  │  library_statuses     │  ┌────────────────────────────────┐     │
│  │  view_mode: Settings  │  │  LibraryPersistence            │     │
│  └─────────────────────┘  │  - save/load via eframe::Storage │     │
│                            │  - JSON array of path strings    │     │
│  ┌─────────────────────┐  └────────────────────────────────┘     │
│  │  errors.rs (extend)   │                                        │
│  │  LibraryPersistence   │                                        │
│  └─────────────────────┘                                          │
└──────────────────────┬──────────────────────────────────────────┘
                       │ implements traits
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│  Infrastructure Layer (src/infra/)                               │
│                                                                   │
│  ┌─────────────────────────────────────────┐                     │
│  │  AudioFileScanner (existing: scanner.rs) │                     │
│  │  - scan(path) → Result<Vec<PathBuf>>     │                     │
│  └─────────────────────────────────────────┘                     │
│                                                                   │
│  ┌─────────────────────────────────────────┐                     │
│  │  [rfd] (NEW dependency, cfg-gated)       │                     │
│  │  - rfd::FileDialog::pick_folder()        │                     │
│  │  - Only on macOS/Windows targets         │                     │
│  └─────────────────────────────────────────┘                     │
└─────────────────────────────────────────────────────────────────┘
```

### DDD Classification

- **LibraryPath** — Value Object (wraps `PathBuf`, canonicalized, used for identity/dedup)
- **LibraryPaths** — Aggregate (collection of LibraryPath entries with status tracking)
- **LibraryStatus** — Value Object enum (Idle, Scanning, Scanned, Unavailable)
- **LibraryManager** — Domain Service (manages scanning, indexing, search across all registered paths)

Identity for a library path: canonicalized `PathBuf`. Two paths that canonicalize to the same filesystem location are the same library.

---

## Design: Level 3 — Interactions

### Flow 1: Navigate to Settings Page

```
User clicks gear icon (⛭) in top bar
  → RiffApp::update() detects gear icon click
  → state.view_mode = ViewMode::Settings
  → egui re-renders, CentralPanel shows LibrarySettingsPanel
  → LibrarySettingsPanel renders:
      - Title: "Settings"
      - Library path list (iterates state.library_paths)
      - Per-entry status, scan, delete
      - Add Library button, Scan All button
      - Back button → state.view_mode = ViewMode::Library
```

### Flow 2: Add Library Path (macOS/Windows)

```
User clicks "Add Library" in settings
  → FileDialogHelper (cfg-gated) invokes:
  → #[cfg(not(target_os = "linux"))]:
  →   rfd::AsyncFileDialog::new().pick_folder().await
  →   If user selects folder:
  →     path = result.path().to_path_buf()
  →     canonical = std::fs::canonicalize(&path) or path
  →     if !state.library_paths.contains(&canonical):
  →       state.library_paths.push(canonical.clone())
  →       LibraryManager.add_path(canonical) // no-op beyond tracking
  →       save_paths_to_persistence()
  →     else: silently ignore
  →   If user cancels: no-op
```

### Flow 3: Add Library Path (Linux)

```
User clicks "Add Library" in settings
  → FileDialogHelper (cfg-gated) invokes:
  → #[cfg(target_os = "linux")]:
  →   Show inline text input + confirm/cancel buttons
  →   User types path and confirms
  →   path = PathBuf::from(input)
  →   if path.exists() && path.is_dir():
  →     canonical = std::fs::canonicalize(&path)
  →     if !state.library_paths.contains(&canonical):
  →       state.library_paths.push(canonical)
  →       save_paths_to_persistence()
  →     else: silently ignore
  →   else: show error "Path does not exist or is not a directory"
  →   If user cancels: hide input, no-op
```

### Flow 4: Delete Library Path

```
User clicks delete (✕) next to a library path in settings
  → Confirm? (optional: brief confirmation toast, or instant delete)
  → state.library_paths.retain(|p| p != &target_path)
  → state.library_statuses.remove(&target_path)
  → Remove all tracks associated with this path from LibraryManager:
      tracks_to_remove: Vec<TrackId> = all tracks whose file_path starts with target_path
      for id in tracks_to_remove: library.remove_track(&id)
  → save_paths_to_persistence()
  → UI updates to reflect removed entry
  → If list is now empty: show empty state placeholder
```

### Flow 5: Scan Single Library

```
User clicks "Scan" on a specific library entry
  → state.library_statuses.insert(path, LibraryStatus::Scanning { found: 0 })
  → lib_cmd.send(LibraryCommand::ScanDirectory(path))
  → Library scan thread (existing) processes:
  →   WalkDir finds audio files
  →   For each chunk: LibraryManager.scan_and_add_tracks(paths, reader)
  →   LibraryUpdate::ScanProgress → UI updates status
  →   LibraryUpdate::ScanComplete { total_files } →
  →     state.library_statuses.insert(path, Scanned(total_files))
  →   LibraryUpdate::ScanError(e) →
  →     state.library_statuses.insert(path, Idle)
  →     state.scan_status = Some(format!("Error: {}", e))
```

### Flow 6: Scan All Libraries

```
User clicks "Scan All"
  → For each path in state.library_paths:
  →   state.library_statuses.insert(path, Scanning { found: 0 })
  →   lib_cmd.send(LibraryCommand::ScanDirectory(path))
  → Each scan proceeds independently on the library scan thread (sequential by channel)
  → UI updates status per-path as each scan progresses/completes
  → After all scans complete: show aggregate notification
```

### Flow 7: Application Startup — Load Persisted Libraries

```
On app launch:
  → RiffApp::new() or first frame:
  →   load library_paths from eframe::Storage or config file
  →   if paths exist:
  →     state.library_paths = loaded_paths
  →     for each path: check if it exists on disk
  →       if exists: status = Idle
  →       if not: status = Unavailable
  →   else: empty state
  →   (Auto-scan on startup: deferred — user triggers scan manually)
```

### Flow 8: Check Path Unavailability

```
When library list is rendered:
  → For each path in state.library_paths:
  →   if !path.exists():
  →     display path with Unavailable indicator (grayed, warning icon)
  →   else:
  →     display normally with current status
  → Unavailable check happens on render (cheap stat call) or cached periodically
```

---

## Design: Level 4 — Contracts

### New / Changed Types

```rust
// === src/app/state.rs changes ===

/// Per-library status for UI display.
#[derive(Debug, Clone, PartialEq)]
pub enum LibraryStatus {
    Idle,
    Scanning { files_found: usize },
    Scanned(usize),        // total tracks
    Unavailable,
}

impl Default for LibraryStatus {
    fn default() -> Self { Self::Idle }
}

/// AppState changes:
/// REMOVE: pub library_path: Option<PathBuf>,
/// ADD:
pub library_paths: Vec<PathBuf>,
pub library_statuses: HashMap<PathBuf, LibraryStatus>,

/// ViewMode changes:
/// ADD: Settings variant
pub enum ViewMode {
    Library,
    NowPlaying,
    Settings,
}
```

```rust
// === src/app/commands.rs changes ===

/// ADD variants to LibraryCommand:
#[derive(Debug, Clone)]
pub enum LibraryCommand {
    ScanDirectory(PathBuf),
    CancelScan,
    // NEW:
}

// Note: Library additions and removals are handled synchronously in the UI
// thread (mutating AppState directly) since they're fast operations.
// No channel command needed — the scan thread only handles scanning.
```

```rust
// === src/app/library_manager.rs changes ===

/// ADD methods:
impl LibraryManager {
    /// Remove all tracks whose file_path starts with the given root,
    /// and clean up orphaned artists/albums.
    pub fn remove_tracks_by_root(&mut self, root: &Path) -> usize {
        let ids_to_remove: Vec<TrackId> = self.tracks
            .iter()
            .filter(|(_, t)| t.file_path.starts_with(root))
            .map(|(id, _)| id.clone())
            .collect();
        let count = ids_to_remove.len();
        for id in ids_to_remove {
            self.remove_track(&id);
        }
        count
    }

    /// Replace the entire library with a fresh scan of the given paths.
    /// Used when "Scan All" replaces existing tracks.
    pub fn rescan_all(&mut self, paths: &[PathBuf], reader: &dyn MetadataReader) -> usize {
        self.clear();
        let mut total = 0;
        for path in paths {
            // Note: actual scan is async over channel; this is the data path
            // Tracks are added incrementally via scan_and_add_tracks
        }
        total
    }
}
```

### Persistence Contract

```rust
/// Library paths serialization format (JSON):
/// Saved to eframe::Storage via serde_json as a JSON array of path strings.
///
/// Example:
/// ```json
/// ["/home/user/Music", "/mnt/external/Music"]
/// ```

/// Key used for eframe::Storage:
const LIBRARY_PATHS_STORAGE_KEY: &str = "library_paths";

/// In RiffApp::new() or first frame:
fn load_persisted_paths(storage: &dyn eframe::Storage) -> Vec<PathBuf> {
    storage.get_string(LIBRARY_PATHS_STORAGE_KEY)
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| v.into_iter().map(PathBuf::from).collect())
        .unwrap_or_default()
}

/// On library change (add/remove):
fn save_persisted_paths(storage: &mut dyn eframe::Storage, paths: &[PathBuf]) {
    let strings: Vec<String> = paths.iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let json = serde_json::to_string(&strings).unwrap_or_default();
    storage.set_string(LIBRARY_PATHS_STORAGE_KEY, &json);
}
```

### File Dialog Contract (Platform-gated)

```rust
/// Platform-specific library path addition.
/// macOS/Windows: uses rfd native folder picker.
/// Linux: uses inline text input.

#[cfg(not(target_os = "linux"))]
mod file_dialog {
    use rfd::AsyncFileDialog;
    
    /// Returns a future that resolves to Option<PathBuf>.
    /// - Some(path) when user selects a directory
    /// - None when user cancels
    pub async fn pick_folder() -> Option<PathBuf> {
        let handle = AsyncFileDialog::new()
            .set_title("Add Music Library")
            .pick_folder()
            .await;
        handle.map(|f| f.path().to_path_buf())
    }
}

#[cfg(target_os = "linux")]
// No file_dialog module — handled inline in settings.rs
// with egui TextEdit + confirm/cancel buttons.
```

### UI Component Contracts

```rust
/// SettingsPanel renders inside CentralPanel when ViewMode == Settings.
/// Layout (top to bottom):
/// 1. Top bar: "Settings" heading + Back button
/// 2. Library list: scrollable rows, each with:
///    - Folder icon + path text (truncated)
///    - Status label
///    - Scan button (disabled while scanning)
///    - Delete button
/// 3. Bottom actions:
///    - Add Library button
///    - Scan All button
/// 4. Empty state (when no libraries): placeholder message

// In RiffApp::update() — ViewMode routing:
match state.view_mode {
    ViewMode::Library => self.show_library_view(ctx, &mut state, &cmd),
    ViewMode::NowPlaying => self.show_now_playing_view(ctx, &mut state, &cmd),
    ViewMode::Settings => self.show_settings_view(ctx, &mut state, &cmd, &lib_cmd),
}

// New method:
fn show_settings_view(
    &mut self,
    ctx: &egui::Context,
    state: &mut AppState,
    cmd: &Option<Sender<PlaybackCommand>>,
    lib_cmd: &Option<Sender<LibraryCommand>>,
) {
    // Renders the settings panel
}
```

### Error Contract

```rust
// Extend AppError (src/app/errors.rs) if needed:
// Not strictly needed — operations are simple enough that
// existing error variants (Io, LibraryScan) suffice.
```

---

## Design Summary

### Components and Layer Assignments

| Component | Layer | New/Existing | Files |
|-----------|-------|-------------|-------|
| LibraryPaths state | Application (State) | Change | `src/app/state.rs` |
| LibraryStatus enum | Application (State) | New | `src/app/state.rs` |
| Settings view mode | Application (State) | Change | `src/app/state.rs` |
| LibrarySettingsPanel | Presentation | New | `src/ui/settings.rs` |
| FileDialogHelper | Presentation | New | `src/ui/settings.rs` (cfg-gated) |
| LibraryPersistence | Application | New | `src/app/library_manager.rs` |
| `rfd` dependency | Infrastructure | New | `Cargo.toml` (cfg-gated) |
| Track removal by root | Application | New | `src/app/library_manager.rs` |

### Key Contracts and Interfaces

1. **State**: `library_paths: Vec<PathBuf>`, `library_statuses: HashMap<PathBuf, LibraryStatus>`, `ViewMode::Settings`
2. **Persistence**: JSON array of path strings via `eframe::Storage`
3. **File Dialog**: `rfd::AsyncFileDialog::pick_folder()` (macOS/Win), inline TextEdit (Linux)
4. **LibraryCommand**: `ScanDirectory(PathBuf)` reused; add/delete done synchronously on UI thread
5. **Track cleanup**: `LibraryManager::remove_tracks_by_root(&Path) -> usize`

### Architectural Constraints

- **Layer purity**: Settings panel goes in `src/ui/settings.rs` (Presentation layer), not in `app.rs`. The gear icon routing changes in `app.rs` are minimal.
- **Platform gating**: Native file dialog (`rfd`) only on macOS/Windows via `#[cfg(not(target_os = "linux"))]`. Linux uses text input. This matches the existing pattern in `tray.rs`.
- **No auto-scan on startup**: Library paths load on startup but are not automatically scanned. User triggers scan manually.
- **Delete is non-destructive**: Removing a library path from the list only removes tracks from the in-memory index. Files on disk are never touched.
- **No channel for add/remove**: Library path add/remove operations are fast (Vec push/retain + persistence write) and happen synchronously on the UI thread. No need for background channel commands.
- **Reuse existing scan infrastructure**: `ScanDirectory` command + `AudioFileScanner` + `LibraryManager::scan_and_add_tracks` are reused without changes. The only addition is per-path status tracking.
- **Persistence via eframe::Storage**: Uses eframe's built-in `Storage` trait (serde_json under the hood) which already persists to disk. No separate config file needed.

### Domain Model Decisions

- **LibraryPath** is a value object identified by its canonical `PathBuf`. Equality is filesystem-equality via canonicalization.
- **LibraryStatus** is a value object enum with four states: `Idle`, `Scanning`, `Scanned(N)`, `Unavailable`.
- **No library entity** — the library path is a simple identifier with associated status. No additional behavior beyond scanning.
- **Track ownership** — a track belongs to the library path it was scanned from (determined by `file_path.starts_with(root)`). When a library is removed, all its tracks are removed.

### Open Questions Resolved During Design

- *How to handle add/remove — channel or sync?* → **Sync on UI thread**. Fast operations, no need for channel overhead.
- *Should scanning be parallel or sequential?* → **Sequential via existing scan thread**. The scan thread processes one `ScanDirectory` command at a time. "Scan All" enqueues multiple commands sequentially.
- *Single persistence mechanism or separate config file?* → **eframe::Storage only**. Simpler, already integrated, sufficient for MVP.
- *Should unavailable paths trigger auto-delete?* → **No**. Show as unavailable but let user decide to delete.
- *Path canonicalization: always or OS default?* → **Canonicalize on add** using `std::fs::canonicalize` for dedup. If canonicalization fails (path doesn't exist yet), use the path as-is.

### Design Status

**Approved — ready for implementation**

---

## Decisions Log

| Date | Decision | Reasoning | Alternatives Considered |
|------|----------|-----------|------------------------|
| 2025-07-10 | Settings page accessed via gear icon in top bar | Consistent with existing UI pattern (gear icon already exists, currently no-op). Minimal UI changes. | Dedicated settings tab; hamburger menu |
| 2025-07-10 | Add/remove library paths handled synchronously on UI thread | Fast operations (Vec push/retain + JSON persist). No need for channel messaging. No background work. | Channel commands for add/remove; IPC |
| 2025-07-10 | Persist library paths via eframe::Storage | Already integrated (serde_json under the hood). No new dependencies. Simple JSON array format. | Separate config file via `directories` crate; SQLite |
| 2025-07-10 | Scan All enqueues sequential ScanDirectory commands | Existing scan thread is single-consumer. Sequential is simpler and avoids concurrency issues with shared state. | Parallel scan threads per path; batch command |
| 2025-07-10 | `rfd` gated behind `#[cfg(not(target_os = "linux"))]` | Follows existing platform-gating pattern in `tray.rs`. Linux avoids GTK dependency of rfd. | `native-dialog` crate; `tinyfiledialogs`; always use text input |
| 2025-07-10 | LibraryStatus tracked per-path in HashMap | Simple, direct lookup by path. Clear ownership. | Status embedded in a LibraryPath struct; status in separate Vec parallel to paths |
| 2025-07-10 | Delete removes tracks by file_path.starts_with(root) | No need for per-track library origin tracking. Simple prefix match on existing data. | Add `library_root` field to Track; track library origin per-track |
| 2025-07-10 | No auto-scan on startup | User should explicitly trigger scans. Avoids slow startup with large libraries. | Auto-scan all paths on startup; auto-scan last-known paths |
| 2025-07-10 | ViewMode::Settings added to existing enum | Minimal change. No new state machine. | Separate app state for settings; modal overlay |
| 2025-07-10 | Settings panel in new src/ui/settings.rs file | Keeps app.rs manageable. Single responsibility. | Inline in app.rs; extract all panels to separate files |

---

## Open Questions

- [ ] Should delete show a confirmation dialog before removing a library? *(Deferred — instant delete with undo toast TBD)*
- [ ] Should library path entries be editable (rename) or just add/delete? *(Deferred — delete + re-add pattern)*
- [ ] Should the unavailable-path check happen on every render or be cached with periodic refresh? *(Cached on render with stat call — cheap enough)*

---

## Constraints

- MUST NOT delete files on disk when removing a library path from the list.
- MUST use existing `LibraryCommand::ScanDirectory` for triggering scans — no new scan channel.
- MUST follow existing platform-gating pattern (`#[cfg(not(target_os = "linux"))]` for native file dialog).
- MUST NOT introduce new external dependencies beyond `rfd` (macOS/Windows only).
- MUST preserve all existing functionality — the scan path text input in the top bar is replaced, not removed alongside.
- MUST survive application restart — library paths persist via eframe::Storage.
- MUST be idempotent — adding the same path twice silently ignores the duplicate.
- MUST handle unavailable paths gracefully — display warning, skip on scan, allow delete.

---

## Key Files

| File | Action | Description |
|------|--------|-------------|
| `src/ui/settings.rs` | **CREATE** | Settings page UI: library list, add/delete, scan buttons, empty state |
| `src/ui/app.rs` | MODIFY | Add ViewMode::Settings routing, gear icon navigation, settings view rendering |
| `src/app/state.rs` | MODIFY | Replace `library_path` with `library_paths`, add `library_statuses` HashMap, add `ViewMode::Settings` |
| `src/app/commands.rs` | NO CHANGE (optionally extend) | Add/remove handled synchronously; no new channel commands needed |
| `src/app/library_manager.rs` | MODIFY | Add `remove_tracks_by_root()` method |
| `src/infra/scanner.rs` | NO CHANGE | Reused as-is |
| `Cargo.toml` | MODIFY | Add `rfd` dependency behind `[target.'cfg(not(target_os = "linux"))'.dependencies]` |
| `.lattice/requirements/features/music-library-management.md` | NO CHANGE (link context) | Requirement doc source |
