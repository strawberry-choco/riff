---
feature: folder-watching
requirement_doc: .lattice/requirements/features/folder-watching.md
created: 2026-07-12
design_status: approved
---

# Folder Watching

Design blueprint for automatic filesystem change detection in library folders, triggering incremental rescans when files are added or deleted, with per-path toggle, debounce coalescing, and graceful degradation.

## Design: Level 1 — Capabilities

### User-Facing Capabilities

1. **Enable/Disable Per-Folder Watching** — In Settings, each library path row shows a "Watch" toggle. Defaults to enabled for local paths, disabled/unavailable for network/unavailable paths. State persists across restarts.

2. **Automatic Rescan on New Files** — When audio files are added or modified in a watched folder, an incremental rescan of that library path triggers automatically after a 2-second quiet period.

3. **Automatic Cleanup on Deleted Files** — When audio files are deleted from a watched folder, the corresponding tracks are removed from the library index after the triggered rescan completes.

4. **Graceful Warning on Unsupported Filesystem** — When watching cannot be enabled (network mount, permissions error, inotify limit on Linux), the toggle shows a ⚠ warning state with a descriptive tooltip.

### System Capabilities

5. **Debounced Change Coalescing** — Multiple rapid filesystem events within a 2-second window coalesce into a single rescan trigger per library path.

6. **Rescan Queueing** — If a rescan is in progress when a new change event arrives, the follow-up rescan is queued for after the current one completes.

7. **Watch State Persistence** — Per-path watch enabled/disabled state persists alongside library paths in `eframe::Storage` and restores on startup.

8. **Clean Watcher Shutdown** — All watchers are stopped on application close. Watching resumes for enabled paths on next launch.

### Decisions Made (Level 1)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| Per-path toggle, not global setting | Matches MusicBee/Clementine pattern. Users may want to watch a local SSD library but not a slow external drive. | Global on/off switch |
| Debounce window: 2 seconds | Industry standard (Strawberry, Clementine). Balances responsiveness with avoiding redundant rescans. | 1s (too aggressive), 5s (too slow) |
| Watch state persisted with library paths | Natural colocation — watch state is per-path metadata. Same storage key or sibling key. | Separate storage key; separate config file |
| `notify` crate for cross-platform FS events | De facto Rust ecosystem standard. Supports inotify (Linux), FSEvents (macOS), ReadDirectoryChangesW (Windows). | `inotify` directly (Linux-only); polling-based approach |

## Design: Level 2 — Components

### Layer Mapping

| Component | Layer | New/Existing | Files | Description |
|-----------|-------|-------------|-------|-------------|
| WatchState enum | Application (State) | New | `src/app/state.rs` | `Enabled \| Disabled \| Warning(String)` — per-path watch status |
| WatchToggle widget | Presentation | Change | `src/ui/settings.rs` | Toggle button per library path row with warning state display |
| FilesystemWatcher | Infrastructure | New | `src/infra/watcher.rs` | Wraps `notify::Watcher`, manages per-path watch registration/unregistration |
| WatcherManager | Application | New | `src/app/watcher_manager.rs` | Orchestrates watcher lifecycle: start/stop, debounce, rescan triggering |
| LibraryPath persisted data | Application | Change | `src/ui/settings.rs` | Extend persisted path data model to include watch state |
| DebounceTimer | Application | New | `src/app/watcher_manager.rs` (inline) | Per-path 2-second timer using `std::time::Instant` |

### Component Diagram

```
┌────────────────────────────────────────────────────────────────────┐
│  Presentation Layer (src/ui/)                                       │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  show_settings_view() — per-path row                          │  │
│  │  ┌──────┬─────────────────────────────────┬────────────────┐ │  │
│  │  │ 📁   │ /home/user/Music                 │ Watch [✓]      │ │  │
│  │  │      │ ⚠ Watching not supported         │ Scan    Delete │ │  │
│  │  └──────┴─────────────────────────────────┴────────────────┘ │  │
│  │                                                                │  │
│  │  WatchToggle: checkbox / toggle_ui per path                    │  │
│  │  Warning state: ⚠ icon + tooltip on hover                      │  │
│  └──────────────────────────────────────────────────────────────┘  │
└──────────────────────┬────────────────────────────────────────────┤
                       │ sends LibraryCommand::SetWatchState         │
                       ▼                                             │
┌────────────────────────────────────────────────────────────────────┐
│  Application Layer (src/app/)                                       │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  AppState (state.rs)                                          │  │
│  │  NEW: watch_states: HashMap<PathBuf, WatchState>              │  │
│  │  NEW: watcher_manager: Option<WatcherManager>                 │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  WatcherManager (watcher_manager.rs)                           │  │
│  │  ┌──────────────────────────────────────────────────────┐    │  │
│  │  │  start_watching(paths, watcher, lib_cmd_tx)           │    │  │
│  │  │  stop_watching(path)                                   │    │  │
│  │  │  stop_all()                                            │    │  │
│  │  │  ┌──────────────────────────────────────────────┐    │    │  │
│  │  │  │  DebounceTimer (inline)                       │    │    │  │
│  │  │  │  per_path: Option<Instant>                   │    │    │  │
│  │  │  │  on_event(path):                              │    │    │  │
│  │  │  │    reset timer → 2s sleep → fire rescan      │    │    │  │
│  │  │  │  if timer already active: reset (coalesce)   │    │    │  │
│  │  │  └──────────────────────────────────────────────┘    │    │  │
│  │  └──────────────────────────────────────────────────────┘    │  │
│  └──────────────────────────────────────────────────────────────┘  │
└──────────────────────┬────────────────────────────────────────────┤
                       │ uses notify::Watcher trait                   │
                       ▼                                             │
┌────────────────────────────────────────────────────────────────────┐
│  Infrastructure Layer (src/infra/)                                  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  FilesystemWatcher (watcher.rs) — wraps notify crate          │  │
│  │  notify::recommended_watcher(callback)                        │  │
│  │  watcher.watch(path, RecursiveMode::Recursive)                │  │
│  │  On event: channel.send(path) → debounce in WatcherManager    │  │
│  └──────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────┘
```

### DDD Classification

- **WatchState** — Value Object enum (`Enabled`, `Disabled`, `Warning(reason)`). No identity, just status.
- **WatcherManager** — Application Service. Orchestrates infrastructure watcher lifecycle, debouncing, and rescan triggering. Not a domain concept.
- **FilesystemWatcher** — Infrastructure adapter implementing the `notify` watcher. No domain relevance.
- **LibraryPath watch config** — Persistence concern. Stored as metadata alongside path string in `eframe::Storage`.

No new domain types needed. Folder watching is an infrastructure + application concern — domain entities (Track, LibraryManager) are unaffected.

### Architecture Validation

- ✅ WatcherManager in Application layer — depends only on domain + std (channel sender)
- ✅ FilesystemWatcher in Infrastructure layer — wraps `notify` crate
- ✅ WatchToggle in Presentation layer — egui widget
- ✅ Dependency direction: UI → Application → Infrastructure (trait boundary)
- ✅ `notify` crate added as new dependency behind infrastructure
- ✅ No domain changes — domain entities unaffected

### Decisions Made (Level 2)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| WatcherManager as Application service | Orchestrates watcher lifecycle + debounce logic. Depends on channels and watcher trait, not on `notify` directly. | Put in Infrastructure (would couple to notify crate); inline in main.rs |
| Debounce inline in WatcherManager | Simple per-path Instant map. No need for a separate debouncer abstraction for this scope. | Separate DebounceService; use `notify-debouncer-full` crate |
| Watcher runs on existing library scan thread | Reuses existing thread infrastructure. Events → rescan on same thread avoids synchronization complexity. | Dedicated watcher thread |
| Persist watch state alongside path list | Same `eframe::Storage` key or sibling key. Natural colocation. | Separate storage key for watch states; separate config file |
| WatchState as flat enum | Three states cover all cases: enabled (watching), disabled (user off), warning (system can't watch). | Separate error/warning types; nested state machine |

## Design: Level 3 — Interactions

### Flow 1: Enable Watching on a Path

```
User clicks "Watch" toggle on a library path
  → watcher_manager.start_watching(&path, &watcher, &lib_cmd_tx)
  → WatcherManager:
    → FilesystemWatcher::watch(path, Recursive)?
    → Success: state.watch_states[path] = Enabled, persist
    → Error: state.watch_states[path] = Warning("reason"), persist
    → UI shows ✓ or ⚠ accordingly
```

### Flow 2: Filesystem Change → Debounced Rescan

```
File changed in watched folder
  → notify callback fires → event_tx.send(changed_dir)
  → WatcherManager receives:
    → For containing library path:
        if timer active: reset timer (coalesce)
        if timer inactive: spawn debounce loop (2s sleep, then fire)
    → After quiet period:
        if scan in progress: pending_rescan[path] = true
        else: lib_cmd_tx.send(ScanDirectory(path))
```

### Flow 3: Rescan Completes → Cleanup + Queue Check

```
ScanComplete received in UI
  → state.library.save_cache()
  → watcher_manager.mark_scan_complete(path)
  → If pending_rescan[path]: fire queued rescan immediately
  → Else: scan_in_progress[path] = false
```

### Flow 4: Disable Watching

```
User toggles Watch off
  → watcher_manager.stop_watching(&path)
  → unwatch, cancel timers, clear scan flags
  → state.watch_states[path] = Disabled, persist
```

### Flow 5: Watch Fails → Warning State

```
start_watching fails:
  Permission denied → Warning("Permission denied")
  Network mount → Warning("Watching not supported on this filesystem")
  inotify limit → Warning("Watch limit reached")
UI: ⚠ icon with tooltip, toggle in off position
Next launch: retry registration
```

### Flow 6: Startup → Restore Watch State

```
first_frame: load watch_states from eframe::Storage
  For each path with WatchState::Enabled:
    if path exists: start_watching
      Success → stays Enabled
      Failure → transitions to Warning(reason)
    else: Warning("Path unavailable")
  Persist updated states
```

### Flow 7: Shutdown → Clean Stop

```
on_exit / drop: watcher_manager.stop_all()
  → unwatch all, cancel all timers, drop watcher
```

### Decisions Made (Level 3)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| Debounce: reset timer on new event | Coalesce rapid changes. Timer starts fresh after each event, fires after 2s quiet. | Fixed window; count threshold |
| Pending rescan queued, not dropped | Changes during active scan would be missed otherwise. | Drop follow-up events |
| Watch failure non-fatal | App continues normally with manual scan. Warning only. | Disable library path; error dialog |
| Watcher reuses scan infrastructure | Sends LibraryCommand::ScanDirectory to existing scan thread. | New channel; direct call |

## Design: Level 4 — Contracts

### New Types

```rust
// === src/app/state.rs ===

/// Per-path filesystem watch state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WatchState {
    /// User has explicitly disabled watching for this path.
    Disabled,
    /// Watching is active and functional.
    Enabled,
    /// Watching cannot be activated. Contains a human-readable reason.
    Warning(String),
}

impl Default for WatchState {
    fn default() -> Self { Self::Disabled }
}

// AppState additions:
pub struct AppState {
    // ... existing fields ...
    /// Per-path watch state, persisted alongside library_paths.
    pub watch_states: HashMap<PathBuf, WatchState>,
}
```

### Persistence Contract

```rust
// === src/ui/settings.rs ===

const WATCH_STATES_KEY: &str = "watch_states";

/// Save per-path watch states as JSON map: {"path": "Enabled"|"Disabled"|"Warning: reason"}
pub fn load_watch_states(storage: Option<&dyn eframe::Storage>) -> HashMap<PathBuf, WatchState> {
    let Some(storage) = storage else { return HashMap::new(); };
    storage.get_string(WATCH_STATES_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_watch_states(storage: &mut dyn eframe::Storage, states: &HashMap<PathBuf, WatchState>) {
    let json = serde_json::to_string(states).unwrap_or_else(|_| "{}".to_string());
    storage.set_string(WATCH_STATES_KEY, json);
}
```

### FilesystemWatcher (Infrastructure)

```rust
// === src/infra/watcher.rs ===

use notify::{Watcher, RecursiveMode, Event};
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};

/// Wraps notify::Watcher. Sends changed directory paths to a channel.
pub struct FilesystemWatcher {
    inner: notify::INotifyWatcher,  // or RecommendedWatcher
}

impl FilesystemWatcher {
    /// Create a new watcher. Events are sent to `event_tx` as PathBufs.
    pub fn new(event_tx: Sender<PathBuf>) -> Result<Self, notify::Error>;

    /// Start watching a directory recursively.
    pub fn watch(&mut self, path: &Path) -> Result<(), notify::Error>;

    /// Stop watching a directory.
    pub fn unwatch(&mut self, path: &Path) -> Result<(), notify::Error>;
}
```

### WatcherManager (Application)

```rust
// === src/app/watcher_manager.rs ===

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use crossbeam_channel::Sender;
use crate::app::commands::LibraryCommand;
use crate::infra::watcher::FilesystemWatcher;

/// Manages lifecycle: watch registration, debounce, rescan triggering.
pub struct WatcherManager {
    watcher: FilesystemWatcher,
    lib_cmd_tx: Sender<LibraryCommand>,
    /// Per-path debounce: Some(Instant) means a timer is active.
    debounce_timers: HashMap<PathBuf, Option<Instant>>,
    /// Whether a scan is currently running for this path.
    scan_in_progress: HashMap<PathBuf, bool>,
    /// Whether a follow-up rescan is queued (arrived during active scan).
    pending_rescan: HashMap<PathBuf, bool>,
}

impl WatcherManager {
    pub fn new(watcher: FilesystemWatcher, lib_cmd_tx: Sender<LibraryCommand>) -> Self;

    /// Start watching a path. Returns Ok(()) or an error reason string.
    pub fn start_watching(&mut self, path: &Path) -> Result<(), String>;

    /// Stop watching a path. Cancels any active debounce timer.
    pub fn stop_watching(&mut self, path: &Path);

    /// Clean shutdown: unwatch all, cancel all timers.
    pub fn stop_all(&mut self);

    /// Called when a ScanComplete is received for a path.
    /// Fires queued rescan if pending.
    pub fn mark_scan_complete(&mut self, path: &Path);

    /// Internal: handle a filesystem event for a changed directory.
    fn on_fs_event(&mut self, changed_dir: &Path);

    /// Internal: trigger the debounced rescan for a library path.
    fn trigger_rescan(&mut self, lib_path: &Path);
}
```

### Settings UI Changes

```rust
// === src/ui/settings.rs — per-path row addition ===

// In show_settings_view, within the per-path loop, add:
ui.horizontal(|ui| {
    // ... existing: folder icon, path label, status, scan, delete ...
    
    // NEW: Watch toggle
    let watch_state = state.watch_states.get(&path).cloned().unwrap_or_default();
    let can_watch = !matches!(watch_state, WatchState::Warning(_));
    
    let mut watching = watch_state == WatchState::Enabled;
    let toggle = ui.add_enabled(can_watch, egui::Checkbox::new(&mut watching, "Watch"));
    
    if toggle.changed() {
        if watching {
            // start_watching returns Result
            match watcher_manager.start_watching(&path) {
                Ok(()) => {
                    state.watch_states.insert(path.clone(), WatchState::Enabled);
                }
                Err(reason) => {
                    state.watch_states.insert(path.clone(), WatchState::Warning(reason));
                }
            }
        } else {
            watcher_manager.stop_watching(&path);
            state.watch_states.insert(path.clone(), WatchState::Disabled);
        }
        if let Some(storage) = frame.storage_mut() {
            save_watch_states(storage, &state.watch_states);
        }
    }
    
    // Warning tooltip
    if let WatchState::Warning(ref reason) = watch_state {
        ui.label("⚠").on_hover_text(reason);
    }
});
```

### Main.rs Wiring (sketch)

```rust
// In main.rs:
let (fs_event_tx, fs_event_rx) = unbounded::<PathBuf>();
let watcher = FilesystemWatcher::new(fs_event_tx)?;
let mut watcher_manager = WatcherManager::new(watcher, lib_cmd_tx.clone());

// Event processing loop (on scan thread or dedicated thread):
std::thread::spawn(move || {
    while let Ok(changed_dir) = fs_event_rx.recv() {
        watcher_manager.on_fs_event(&changed_dir);
    }
});

// In state:
state.watcher_manager = Some(watcher_manager);
```

### Decisions Made (Level 4)

| Decision | Reasoning | Alternatives Rejected |
|---|---|---|
| WatchState serialized as JSON alongside paths | Simplest persistence story. Same storage mechanism, sibling key. | Binary format; separate config file |
| WatcherManager holds watcher ownership | Manager is the sole lifecycle owner. No shared ownership needed. | Arc-wrapped watcher shared between manager and thread |
| FS events channel: `crossbeam_channel<PathBuf>` | Matches existing channel infrastructure. Sends the changed directory path (not the library path). Manager resolves which library path contains it. | Event struct; raw notify::Event type |
| `notify` crate — platform-specific watcher | `notify::recommended_watcher` auto-selects inotify/FSEvents/ReadDirectoryChangesW. Cross-platform with zero conditional compilation. | Per-platform cfg-gated watcher implementations |
| No trait for WatcherManager | WatcherManager is an application service, not behind a port. No test seam needed for MVP. | WatcherManager trait for testability |

### File Change Summary

| File | Action | What Changes |
|------|--------|-------------|
| `src/app/state.rs` | MODIFY | Add `WatchState` enum, `watch_states: HashMap<PathBuf, WatchState>` |
| `src/app/watcher_manager.rs` | CREATE | New application service: watcher lifecycle, debounce, rescan triggering |
| `src/infra/watcher.rs` | CREATE | FilesystemWatcher wrapping `notify` crate |
| `src/ui/settings.rs` | MODIFY | Watch toggle per library path row, warning state display, persistence |
| `src/main.rs` | MODIFY | Wire watcher channel, spawn FS event processing thread |
| `Cargo.toml` | MODIFY | Add `notify` dependency |
| `src/domain/` | NO CHANGE | |
| `src/app/commands.rs` | NO CHANGE | Reuses existing `LibraryCommand::ScanDirectory` |
| `src/app/library_manager.rs` | NO CHANGE | Reuses existing scan infrastructure |

## Design Summary

### Components and Layer Assignments

| Component | Layer | New/Existing | Files |
|-----------|-------|-------------|-------|
| WatchState enum | Application (State) | New | `src/app/state.rs` |
| WatcherManager | Application | New | `src/app/watcher_manager.rs` |
| FilesystemWatcher | Infrastructure | New | `src/infra/watcher.rs` |
| WatchToggle widget | Presentation | Change | `src/ui/settings.rs` |
| Watch persistence | Presentation | Change | `src/ui/settings.rs` |
| Thread wiring | Main | Change | `src/main.rs` |
| `notify` crate | Infrastructure | New | `Cargo.toml` |

### Key Contracts and Interfaces

1. **WatchState**: `Disabled | Enabled | Warning(String)` — per-path watch status
2. **FilesystemWatcher**: Wraps `notify::Watcher`, sends `PathBuf` events via channel
3. **WatcherManager**: `start_watching(path)`, `stop_watching(path)`, `mark_scan_complete(path)`, `stop_all()`
4. **Persistence**: `load_watch_states` / `save_watch_states` via `eframe::Storage` JSON map
5. **Debounce**: Per-path `Instant` timer, resets on each event, fires after 2s quiet
6. **Rescan trigger**: Sends `LibraryCommand::ScanDirectory(path)` to existing scan channel
7. **Queueing**: `pending_rescan` flag per path → fired on `mark_scan_complete`

### Architectural Constraints

- WatcherManager in Application layer: depends on infrastructure trait (not directly on `notify`)
- No domain changes — watching is an application/infrastructure concern
- Reuses existing `LibraryCommand::ScanDirectory` and scan thread infrastructure
- Watch state persists as JSON alongside library path list in `eframe::Storage`
- One new external dependency: `notify` crate (cross-platform FS events)
- Watcher runs on a dedicated or shared thread, not blocking UI thread
- All error paths result in Warning state, not application crash

### Domain Model Decisions

- WatchState is a Value Object — no identity, enum discriminant only
- No domain concept of "filesystem change" — domain sees only the resulting Track adds/removes
- WatcherManager is a pure Application Service — stateless orchestration over infrastructure
- LibraryManager is unchanged — tracks added/removed via existing scan infrastructure

### Open Questions Resolved During Design

- *Should watching be on by default?* → **Yes for local accessible paths, no for network/unavailable**. Matches user expectation: "I added this folder, I want auto-detection."
- *Where does the watcher thread run?* → **Dedicated thread for FS events, scan thread for rescans**. FS events → channel → WatcherManager (on scan thread) → debounce → ScanDirectory command.
- *Should deleted tracks be removed from the current queue?* → **Keep playing until natural end, then remove**. Matches requirement doc: "playback continues until the track ends naturally."
- *Should we use `notify-debouncer-full` crate?* → **No. Custom 2-second debounce is simple enough.** Avoids adding another dependency for a small amount of code.
- *Should watch state persist separately or with paths?* → **With paths. Sibling JSON key in eframe::Storage.** Natural colocation, same load/save cycle.

### Design Status

**Approved — ready for implementation**

## Open Questions
