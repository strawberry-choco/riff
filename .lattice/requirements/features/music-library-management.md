---
feature: Music Library Management
epic: Music Library
status: implemented
priority: P0
depends_on: ["Library Scanning"]
personas: ["Music Listener"]
source_docs: []
implementation_notes: |
  Current: A single library path is entered via a plain text input (pending_scan_path)
  in the top bar. There is no list of libraries, no add/delete, and no native file
  picker. The new feature replaces this with a proper library management UI.
---

# Music Library Management

## Problem Statement

The current music library path is a plain text input field in the top bar where users manually type or paste a directory path. This is error-prone (typos, invisible whitespace), offers no discovery (users must know the exact path), and only supports a single library at a time. Users with music spread across multiple directories (e.g., `~/Music`, an external drive, a network mount) have no way to combine them into one unified library. Deleting a library also requires manual path clearing — there is no persistent list.

## User / Personas

- **Music Listener**: A person with a large local music collection who may have music distributed across multiple directories — an internal drive, an external SSD, a NAS share, or multiple OS-standard music folders. They want to manage these locations visually, add new ones by browsing with the OS file picker, and remove old ones without touching the keyboard.

## Scope

**In scope:**
- A **settings page** (separate view/panel) where library paths are managed
- Display a list of registered music library paths in the settings page
- Add a new library path using the **OS native file/folder picker dialog** (macOS, Windows) or a **plain text input field** (Linux)
- Delete an existing library path from the list with a single click
- Persist the list of library paths across application restarts
- **Per-library scan button** (scan that single library) plus a **"Scan All" button** (scan all registered libraries)
- Show status per library path (e.g., number of tracks, last scan time, or scan progress)
- Allow the user to trigger a rescan of all libraries or individual libraries

**Out of scope:**
- Drag-and-drop to reorder libraries
- Renaming a library (it is identified by its path)
- Per-library scan scheduling or auto-watch (inotify/FSEvents)
- Cloud storage or network mount management (the library path is a local filesystem path; network mounts are treated as local)
- Library-level enable/disable toggle (delete + re-add is sufficient for MVP)

## Boundary Conditions

- The library list must survive application restarts (persist to disk via eframe Storage or a config file)
- Adding a path that already exists in the list must be idempotent (no duplicates)
- Deleting a library must only remove the path from the list — it must NOT delete the files on disk
- The native file picker must default to the user's home directory or the OS-standard music folder
- A library path that no longer exists on disk (ejected drive, unmounted NAS) should be displayed as unavailable but not auto-deleted — the user should decide to remove it
- Empty library list should show a helpful placeholder message
- At least one library path must be present before the scan button becomes active (or the button should show an appropriate message)

## Assumptions

- The OS native file picker provides a folder-selection dialog (macOS, Windows)
- Linux uses plain text input fields for entering library paths (no native file picker dependency)
- Users understand that removing a library path from the list does not delete the underlying files
- The list of library paths is small (typically 1–5 entries) — no performance optimization needed for long lists
- egui supports integration with a native file dialog crate for macOS/Windows (e.g., `rfd` / `native-dialog` / `tinyfiledialogs`)

## Scenarios

### Scenario 1: Add a music library
A user wants to add a music directory to the library.

**Acceptance Criteria (macOS / Windows):**
- Given the library list is empty, when the user clicks the "Add Library" button, then the OS native folder picker dialog opens
- Given the folder picker is open, when the user selects a directory and confirms, then the chosen path appears in the library list
- Given the folder picker is open, when the user cancels, then no path is added and the list stays unchanged
- Given a path is already in the library list, when the user tries to add the same path again via the picker, then it is silently ignored (no duplicate entry)

**Acceptance Criteria (Linux):**
- Given the library list is empty, when the user clicks the "Add Library" button, then a text input field appears (inline or in a small dialog)
- Given the text input is visible, when the user types a valid directory path and confirms, then the path appears in the library list
- Given the text input is visible, when the user cancels/dismisses, then no path is added and the list stays unchanged
- Given a path is already in the library list, when the user submits the same path via text input, then it is silently ignored (no duplicate entry)

### Scenario 2: Delete a music library
A user wants to remove a previously added music directory.

**Acceptance Criteria:**
- Given a library path exists in the list, when the user clicks the delete button next to it, then the path is removed from the list immediately
- Given a library path is removed, when the user looks at the list, then the removed path is no longer visible
- Given a library path is removed, when the user inspects the filesystem, then the original files and directories are untouched
- Given the last library path is removed, when the list becomes empty, then a helpful placeholder message is shown (e.g., "Add a music folder to get started")

### Scenario 3: Scan all libraries
A user has multiple library paths and triggers a scan.

**Acceptance Criteria:**
- Given multiple library paths are registered, when the user clicks the scan button, then all paths are scanned in sequence (or parallel) and tracks from all paths appear in the unified library
- Given a library path was scanned before and new files have been added since, when a rescan completes, then the new files appear in the library
- Given a library path was removed and re-added, when scanned, then its tracks appear correctly in the library (no duplicate or stale tracks from the old registration)

### Scenario 4: Persist library list
A user quits and relaunches the application.

**Acceptance Criteria:**
- Given the user has added 3 library paths, when they quit and relaunch the application, then all 3 paths are still present in the library list
- Given the user deleted a library path, when they relaunch the application, then the deleted path is not in the list
- Given no library paths were added (empty list), when the user relaunches, then the list remains empty

### Scenario 5: Unavailable library path
A library path points to a no-longer-available location.

**Acceptance Criteria:**
- Given a library path points to an ejected external drive, when the library list is displayed, then the path is shown with an unavailable indicator (e.g., grayed out or with a warning icon)
- Given a library path is unavailable, when a scan is triggered, then the unavailable path is skipped with a warning shown in the scan status
- Given a library path is unavailable, the user can still delete it from the list normally

## Implementation Notes

1. **Platform-specific path input**:
   - **macOS / Windows**: Use `rfd` (Rusty File Dialogs) for a native OS folder picker dialog (`rfd::AsyncFileDialog::pick_folder()`). Gate behind `#[cfg(not(target_os = "linux"))]`.
   - **Linux**: Use a plain text input field (similar to the current `pending_scan_path` text input) with a "Browse" placeholder label. No native file picker dependency. Gate behind `#[cfg(target_os = "linux")]`.

2. **Persistence**: Store the library path list via `eframe`'s `Storage` trait (`serde_json` under the hood) or in a separate config file at the standard config directory (using the `directories` crate already in the dependency tree). A JSON array of path strings is sufficient.

3. **UI placement**: A **settings page** accessible from the top bar gear icon (⛭). The settings page shows:
   - Page title: "Settings" or "Music Libraries"
   - A list of registered library paths, each row showing:
     - Folder icon + path string (truncated with ellipsis for long paths)
     - Status indicator (idle, scanning, scanned N tracks, unavailable)
     - A **Scan** button (scans only that library)
     - A **Delete** button (trash icon or ✕)
   - At the bottom of the list:
     - An **Add Library** button (opens file picker on macOS/Windows, shows text input on Linux)
     - A **Scan All** button (scans all registered libraries)
   - A **Back** button or navigation to return to the main library view
   - Empty state: "No music libraries configured. Add one to get started."

4. **State changes**: Add `library_paths: Vec<PathBuf>` to `AppState` (replacing the current single `library_path: Option<PathBuf>`). Add commands `AddLibrary(PathBuf)` and `RemoveLibrary(PathBuf)` to `LibraryCommand`.

5. **Scan flow**: When scan is triggered, iterate all registered paths and call the existing `ScanDirectory` logic for each. The scanner already handles multiple paths via `scan_and_add_tracks`.

6. **Idempotency**: Before adding a path, canonicalize it and check against existing entries to prevent duplicates with different string representations (trailing slashes, symlink targets).

## Resolved Decisions

| Question | Decision |
|---|---|
| Library list UI placement | **Settings page** — accessible from the top bar gear icon (⛭), separate from the main library view |
| Individual vs. Scan All | **Both** — per-library scan button on each entry + a "Scan All" button at the bottom |
| Path canonicalization | **OS default** — use the path as returned by the OS/file picker. No additional normalization beyond what the OS provides. |
| Linux file picker dependency | **Plain text input on Linux** — gate `rfd` behind `#[cfg(not(target_os = "linux"))]`. Linux uses a text input field instead. |

## Open Questions

- [ ] Should the library list support drag-to-reorder for scan priority? *(deferred — not in scope for MVP)*

## Links

- Design blueprint: [../../context/music-library-management.md](../../context/music-library-management.md)
- Epic index: [index.md](../index.md)
