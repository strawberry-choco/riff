---
feature: Folder Watching
epic: Music Library
status: implemented
priority: P1
depends_on: [Library Scanning]
personas: [Active Collector, Library Maintainer]
source_docs: []
---

# Folder Watching

## Problem Statement

Users add new music to their library folders through other applications — downloads, CD rips, file transfers from external drives. Currently, they must manually trigger a rescan in Settings every time they add or remove files. This creates friction: a user who downloads an album into their library folder expects to see it in riff without extra steps. Similarly, tracks from deleted files remain in the library as dead entries until manually cleaned up.

## User / Personas

**Active Collector**: Frequently adds new music to their library folders via downloads, rips, or file transfers. Expects new tracks to appear automatically without navigating to Settings and clicking "Scan."

**Library Maintainer**: Periodically reorganizes or deletes music files. Expects removed tracks to disappear from the library index without manual cleanup, and does not want stale entries cluttering search results.

## Scope

**In scope:**
- Per-folder toggle in Settings: "Watch for changes" on each configured library path
- On filesystem change (file added or deleted in a watched folder or its subdirectories), trigger an automatic incremental rescan of the affected directory
- Debounce: multiple rapid changes within a 2-second window coalesce into a single rescan
- Library cleanup: tracks whose files no longer exist on disk are removed from the index after a change-triggered rescan
- Watch state is persisted alongside library paths in `eframe::Storage` and restored on startup
- Graceful degradation: if the OS cannot watch a directory, show a clear warning state instead of failing silently

**Out of scope:**
- Watching individual files (only directories)
- Real-time metadata refresh of existing tracks when tags are edited externally (only new/removed files trigger changes)
- Watching network paths that do not support native filesystem events (NFS, SMB)
- Cross-platform file locking or conflict resolution when files are being written during a scan
- Automatic rescan on application startup (already covered by cache loading)

## Boundary Conditions

- Inotify limit on Linux: the kernel imposes a per-process limit on watched inodes. If the user has many watched directories (typically > 8,192 subdirectories), new watch registrations may fail. Show a clear error message suggesting the user reduce the number of watched paths or increase the kernel limit.
- Network mounts (NFS, SMB): filesystem events are not reliably delivered. The watcher must detect this (via registration failure or missing events) and transition to a warning state rather than silently doing nothing.
- Mass file operations: copying 500 files at once must not trigger 500 separate rescans. The debounce window must collect all events before firing.
- A rescan is already in progress when a new change event arrives: queue the follow-up rescan to start after the current one completes.
- Application is closed while watching: all watchers are stopped cleanly. On next launch, watching resumes for paths that had it enabled.
- A watched directory is unmounted or becomes inaccessible: the watcher transitions to a warning state and stops triggering rescans.

## Assumptions

- The `notify` crate (cross-platform filesystem watcher) is added as a project dependency
- The watcher runs on the existing library scan thread, reusing the `walkdir` + `crossbeam_channel` infrastructure for incremental rescans
- The debounce window is fixed at 2 seconds, matching the industry standard established by Strawberry and Clementine
- Watch state (enabled/disabled per path) is persisted as part of the library paths configuration in `eframe::Storage`
- Watching is opt-in per folder, not a global application setting — matching the MusicBee and Clementine pattern
- Only directories that exist and are accessible can be watched; unavailable paths default to watch-off with a warning

## Scenarios

### Scenario 1: Enable folder watching

A user enables automatic change detection on a library folder.

**Acceptance Criteria:**
- Given the user is in Settings with at least one library path configured, when they look at a library path row, then a "Watch" toggle is visible, enabled by default for paths that exist and are on a local filesystem
- Given the Watch toggle is on for a path, when the user toggles it off, then the filesystem watcher for that path is stopped and no further automatic rescans occur for changes in that directory tree
- Given the Watch toggle is off for a path, when the user toggles it on, then a filesystem watcher is registered for that path and its subdirectories, and future changes will trigger rescans
- Given the user changes the watch state for any path, when the application is closed and reopened, then the watch state is restored from persisted storage and watching resumes for all enabled paths

### Scenario 2: New or modified files trigger automatic rescan

A user adds music files to a watched folder through an external application.

**Acceptance Criteria:**
- Given a folder is being watched, when one or more audio files are added or modified in that folder or any subdirectory, then an incremental rescan of the affected directory begins after a 2-second quiet period with no further change events
- Given a rescan is triggered by filesystem changes, when the scan completes, then new tracks appear in the library index and are visible in both the Library and Folders views without requiring a manual scan
- Given multiple files are added in rapid succession (e.g., copying an album of 12 tracks within the same directory), when the 2-second quiet period elapses, then exactly one rescan is triggered for that directory rather than one per file

### Scenario 3: Deleted files trigger library cleanup

A user removes music files from a watched folder.

**Acceptance Criteria:**
- Given a folder is being watched, when one or more audio files are deleted from that folder or any subdirectory, then an incremental rescan of the affected directory begins after the quiet period
- Given the rescan detects that previously-indexed files no longer exist on disk, when the scan completes, then the corresponding tracks are removed from the library index and no longer appear in search results or either browse view
- Given the currently-playing track is deleted from disk while playing, when the cleanup rescan completes, then playback continues until the track ends naturally, and the track is removed from the queue after it finishes

### Scenario 4: Folder watching degrades gracefully when unavailable

The system cannot watch a directory due to operating system limitations or permissions.

**Acceptance Criteria:**
- Given a library path is on a network mount (NFS, SMB) or a filesystem that does not support native file watching, when the application attempts to register a watcher for it, then the Watch toggle displays a warning state (⚠ icon) with a tooltip reading "Watching not supported on this filesystem" and no watcher is registered
- Given a library path exists but the application lacks read permissions for its contents, when the application attempts to register a watcher, then the Watch toggle displays a warning state with a tooltip explaining the permission issue
- Given the inotify watch limit is reached on Linux, when the user attempts to enable watching on an additional path, then an error message appears stating "Watch limit reached — too many watched directories. Try reducing the number of watched paths or increasing fs.inotify.max_user_watches" and the toggle remains off
- Given a watched directory is unmounted or becomes inaccessible while the watcher is active, when the watcher receives an error event, then the Watch toggle transitions to the warning state, automatic rescans stop for that path, and a status message appears in the Settings row

## Implementation Notes

1. **Watch toggle in Settings** — Add a "Watch" toggle button (checkbox or selectable label) to each library path row in the Settings view. Extend the persisted library path data model to include a watch state boolean per path. Default to enabled for local paths, disabled for network/unavailable paths.
2. **Filesystem watcher integration** — Add the `notify` crate. Spawn a watcher on the library scan thread that registers recursive watches on enabled paths. Use a debounce mechanism (e.g., a 2-second `std::time::Instant` timer) to coalesce rapid events before sending `LibraryCommand::ScanDirectory` for the affected path.
3. **New file detection** — When a change event fires after the debounce window, send `LibraryCommand::ScanDirectory(path)` where `path` is the root library path containing the changed directory. The existing scanner's incremental logic (skip already-indexed paths) handles the rest.
4. **Deleted file cleanup** — After an incremental rescan triggered by a filesystem change, compare the scan results against the existing index. Remove any `TrackId` entries whose file path no longer exists on disk. If the deleted track is in the current queue, mark it for removal after playback.
5. **Graceful degradation** — On watcher registration, catch errors from `notify` (permission denied, unsupported filesystem, inotify limit). Set the watch state to a warning variant that displays in Settings with a descriptive tooltip. Periodically retry registration (e.g., on next app launch) in case the underlying issue has been resolved.

## Open Questions

- None.

## Links

- Design: [folder-watching](../../context/folder-watching.md) — Approved, ready for implementation
- Epic index: [index.md](../index.md)
