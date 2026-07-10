---
feature: library-cache-persistence
requirement_doc: .lattice/requirements/features/library-cache-persistence.md
created: 2025-07-10
design_status: approved
---

# Library Cache Persistence

Design blueprint for persisting scanned track data (tracks, artists, albums) to a JSON cache file so the library loads instantly on startup without re-scanning, and keeping the cache in sync after scans and path removals.

---

## Design: Level 1 — Capabilities

### User-Facing Capabilities

1. **Instant Library on Startup** — When the app launches, the previously scanned library (tracks, artists, albums) is loaded from a local cache file. No manual scan required.
2. **Cache Updated After Scan** — After any scan (single path or Scan All) completes successfully, the cache is automatically rewritten with the latest data.
3. **Cache Updated After Path Removal** — When a library path is deleted from settings, the cache is rewritten to exclude tracks from that path.
4. **Graceful Cache Miss** — If no cache exists (first launch) or the cache is corrupt, the app starts with an empty library. The user can scan normally to populate and create the cache.

### Non-User-Facing / System Capabilities

5. **Platform-Appropriate Cache Path** — Resolve the cache file location using the `directories` crate (`ProjectDirs::data_local_dir()`), which maps to the correct OS-specific directory (XDG on Linux, Application Support on macOS, Local AppData on Windows).
6. **Automatic Cache Directory Creation** — Create the cache directory (e.g. `~/.local/share/riff/`) if it does not exist when saving.
7. **Corrupt Cache Resilience** — On load failure (parse error, truncated file, I/O error), log a warning and return an empty library. Never crash.
8. **Write Failure Resilience** — On save failure (permissions, disk full), log a warning and continue. The in-memory library is unaffected.

---

## Design: Level 2 — Components

### Layer Mapping

| Component | Layer | New/Existing | Files | Description |
|-----------|-------|-------------|-------|-------------|
| CachePersistence | Application | New | `src/app/library_manager.rs` | `save_cache()` / `load_cache()` methods on `LibraryManager`. Serialises/deserialises the full library state to/from a JSON file. |
| Album | Domain | Change | `src/domain/track.rs` | Added `serde::Serialize` and `serde::Deserialize` derives for cache serialisation. |
| Artist | Domain | Change | `src/domain/track.rs` | Added `serde::Serialize` and `serde::Deserialize` derives for cache serialisation. |
| LibraryManager | Application | Change | `src/app/library_manager.rs` | Added `#[derive(Serialize, Deserialize)]` so the full library can be serialised as a single JSON object. |
| CacheTrigger (ScanComplete) | Presentation | Change | `src/ui/app.rs` | Calls `state.library.save_cache()` when `LibraryUpdate::ScanComplete` is received in `poll_library_updates()`. |
| CacheTrigger (Path Removal) | Presentation | Change | `src/ui/settings.rs` | Calls `state.library.save_cache()` after `remove_tracks_by_root()` in the delete button handler. |
| CacheTrigger (Startup Load) | Presentation | Change | `src/ui/app.rs` | Calls `LibraryManager::load_cache()` in the `first_frame` handler and assigns the result to `state.library`. |

### Component Diagram

```
┌────────────────────────────────────────────────────────────────────┐
│  Presentation Layer (src/ui/)                                       │
│                                                                     │
│  ┌─────────────────────────────────────────┐                       │
│  │  RiffApp::update() — first_frame handler │                      │
│  │  Calls: LibraryManager::load_cache()     │                      │
│  │  Sets: state.library = loaded_cache      │                      │
│  └──────────────┬──────────────────────────┘                       │
│                 │                                                  │
│  ┌──────────────▼──────────────────────────┐                       │
│  │  poll_library_updates()                 │                       │
│  │  On ScanComplete: state.library.save_cache()                    │
│  └─────────────────────────────────────────┘                       │
│                                                                     │
│  ┌─────────────────────────────────────────┐                       │
│  │  show_settings_view()                    │                       │
│  │  On delete: state.library.save_cache()   │                       │
│  └─────────────────────────────────────────┘                       │
└──────────────────────┬────────────────────────────────────────────┘
                       │ calls
                       ▼
┌────────────────────────────────────────────────────────────────────┐
│  Application Layer (src/app/)                                       │
│                                                                     │
│  ┌─────────────────────────────────────────┐                       │
│  │  LibraryManager (library_manager.rs)     │                       │
│  │  ┌─────────────────────────────────────┐ │                       │
│  │  │  save_cache(&self)                   │ │                       │
│  │  │  → directories::ProjectDirs         │ │                       │
│  │  │  → serde_json::to_string(self)      │ │                       │
│  │  │  → std::fs::write(path, json)       │ │                       │
│  │  ├─────────────────────────────────────┤ │                       │
│  │  │  load_cache() -> Self               │ │                       │
│  │  │  → directories::ProjectDirs         │ │                       │
│  │  │  → std::fs::read_to_string(path)    │ │                       │
│  │  │  → serde_json::from_str(&json)      │ │                       │
│  │  └─────────────────────────────────────┘ │                       │
│  └─────────────────────────────────────────┘                       │
│                                                                     │
│  ┌─────────────────────────────────────────┐                       │
│  │  LibraryManager (serialised as JSON)     │                       │
│  │  ┌─────────────────────────────────────┐ │                       │
│  │  │  tracks: HashMap<TrackId, Track>    │ │                       │
│  │  │  artists: HashMap<String, Artist>   │ │                       │
│  │  │  albums: HashMap<String, Album>     │ │                       │
│  │  └─────────────────────────────────────┘ │                       │
│  └─────────────────────────────────────────┘                       │
└──────────────────────┬────────────────────────────────────────────┘
                       │ resolves path via
                       ▼
┌────────────────────────────────────────────────────────────────────┐
│  Infrastructure Layer (src/infra/) — directories crate              │
│                                                                     │
│  directories::ProjectDirs::from("", "", "riff")                    │
│    → data_local_dir()                                              │
│    → join("library_cache.json")                                    │
│  Platform mappings:                                                │
│    Linux:   ~/.local/share/riff/library_cache.json                 │
│    macOS:   ~/Library/Application Support/com.riff.riff/...        │
│    Windows: C:\Users\<user>\AppData\Local\riff\riff\...            │
└────────────────────────────────────────────────────────────────────┘
```

### DDD Classification

- **LibraryCache** — Repository (persists and restores the `LibraryManager` aggregate from a JSON file on disk)
- **LibraryManager** — Aggregate (the full library state — tracks, artists, albums — serialised atomically as a single unit)
- **CachePath** — Value Object (wraps the platform-resolved `PathBuf` to the cache file, computed from `directories::ProjectDirs`)

The cache is the serialised representation of the entire `LibraryManager` aggregate. It has no own identity — it is always derived from the in-memory library state and restored back into it.

---

## Design: Level 3 — Interactions

### Flow 1: Application Startup — Load Cache

```
User launches the app
  → eframe::run_native() → RiffApp::new()
  → On first frame of RiffApp::update():
  →   state.library = LibraryManager::load_cache()
  →   load_cache():
  →     cache_path = ProjectDirs::data_local_dir() / "library_cache.json"
  →     if !cache_path.exists(): return empty LibraryManager
  →     json = std::fs::read_to_string(cache_path)
  →     if read fails: log warning, return empty LibraryManager
  →     serde_json::from_str::<LibraryManager>(&json)
  →     if deserialize fails: log warning, return empty LibraryManager
  →     return deserialized library
  →   state.library = loaded LibraryManager  // tracks, artists, albums populated
  →   persisted_paths = load_persisted_paths()  // library path list (existing)
  →   state.library_paths = persisted_paths
  →   for each path: set status to Idle or Unavailable
  →   Library view renders with restored tracks from cache
  →   No scan is triggered — library is ready immediately
```

### Flow 2: Scan Completes — Save Cache

```
User clicks "Scan" or "Scan All"
  → Scan proceeds on background thread (existing flow)
  → scan_and_add_tracks() populates LibraryManager.tracks, .artists, .albums
  → LibraryUpdate::ScanComplete sent to UI thread
  → poll_library_updates() receives ScanComplete:
  →   state.library_statuses.insert(path, Scanned(total_files))
  →   state.scan_status = "Scan complete: N tracks"
  →   state.library.save_cache()
  →   save_cache():
  →     cache_path = ProjectDirs::data_local_dir() / "library_cache.json"
  →     create_dir_all(cache_path.parent())  // auto-create directory
  →     if create fails: log warning, return
  →     json = serde_json::to_string(self)  // serialize entire LibraryManager
  →     if serialize fails: log warning, return
  →     std::fs::write(cache_path, json)
  →     if write fails: log warning, return
  →   Cache is now up to date with latest scan results
```

### Flow 3: Path Removed — Save Cache

```
User clicks delete (✕) on a library path in settings
  → state.library.remove_tracks_by_root(&path)  // removes tracks in this path
  → state.library_paths.retain(|p| p != &path)
  → state.library_statuses.remove(&path)
  → state.library.save_cache()  // persist after removal
  → save_library_paths(storage, &state.library_paths)  // persist path list (existing)
  → UI updates to reflect removed entry
  → Cache now excludes tracks from the deleted path
```

### Flow 4: Corrupt or Missing Cache (First Launch)

```
User launches app with no cache file
  → load_cache() returns empty LibraryManager (no error, no warning)
  → Library view shows empty state (no tracks, no artists, no albums)
  → User navigates to settings, adds path, clicks Scan
  → Scan runs and populates library
  → ScanComplete → save_cache() creates cache file for next launch

User launches app with corrupted cache file
  → load_cache():
  →   serde_json::from_str() fails with parse error
  →   log warning: "Failed to deserialize library cache: {error}"
  →   return empty LibraryManager
  → Library view shows empty state
  → User can either delete the cache manually or just rescan
```

---

## Design: Level 4 — Contracts

### New / Changed Types

```rust
// === src/domain/track.rs changes ===

/// Added serde derives to existing types:
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<TrackId>,
    pub year: Option<u32>,
    pub genre: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Artist {
    pub name: String,
    pub albums: Vec<String>,
}
```

```rust
// === src/app/library_manager.rs changes ===

/// Added serde derives to existing type:
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LibraryManager {
    pub tracks: HashMap<TrackId, Track>,
    pub artists: HashMap<String, Artist>,
    pub albums: HashMap<String, Album>,
}

/// Added methods to LibraryManager:
impl LibraryManager {
    /// Compute the platform-appropriate cache file path.
    fn cache_path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("", "", "riff")
            .map(|d| d.data_local_dir().join("library_cache.json"))
    }

    /// Save the full library state to the JSON cache file.
    /// Silently handles directory creation, serialisation, and write errors
    /// by logging warnings and returning. Never panics.
    pub fn save_cache(&self) {
        let path = match Self::cache_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!("Failed to create cache directory: {e}");
                return;
            }
        }
        let json = match serde_json::to_string(self) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize library cache: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, json) {
            tracing::warn!("Failed to write library cache: {e}");
        }
    }

    /// Load the library state from the JSON cache file.
    /// If the file does not exist, is corrupt, or any I/O error occurs,
    /// returns an empty LibraryManager. Never panics.
    pub fn load_cache() -> Self {
        let path = match Self::cache_path() {
            Some(p) => p,
            None => return Self::new(),
        };
        if !path.exists() {
            return Self::new();
        }
        let json = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to read library cache: {e}");
                return Self::new();
            }
        };
        match serde_json::from_str(&json) {
            Ok(lib) => lib,
            Err(e) => {
                tracing::warn!("Failed to deserialize library cache: {e}");
                Self::new()
            }
        }
    }
}
```

### Persistence Contract

```rust
/// Cache file location:
/// Resolved via directories::ProjectDirs::data_local_dir() / "library_cache.json"
///
/// Platform resolution:
///   Linux:   $XDG_DATA_HOME/riff/library_cache.json
///            (default ~/.local/share/riff/library_cache.json)
///   macOS:   ~/Library/Application Support/com.riff.riff/library_cache.json
///   Windows: C:\Users\<user>\AppData\Local\riff\riff\library_cache.json
///
/// Serialisation format:
/// The full LibraryManager struct serialised as a single JSON object with
/// three top-level keys: tracks, artists, albums.
///
/// Example (abbreviated):
/// {
///   "tracks": {
///     "/music/song.mp3": {
///       "id": "/music/song.mp3",
///       "file_path": "/music/song.mp3",
///       "metadata": {
///         "title": "Song Title",
///         "artist": "Artist Name",
///         "album": "Album Name",
///         "album_artist": "Artist Name",
///         "track_number": 1,
///         "disc_number": 1,
///         "genre": "Rock",
///         "year": 2024,
///         "composer": null,
///         "comment": null
///       },
///       "duration": { "secs": 240, "nanos": 0 },
///       "sample_rate": 44100,
///       "channels": 2
///     }
///   },
///   "artists": {
///     "Artist Name": {
///       "name": "Artist Name",
///       "albums": ["Artist Name - Album Name"]
///     }
///   },
///   "albums": {
///     "Artist Name - Album Name": {
///       "title": "Album Name",
///       "artist": "Artist Name",
///       "tracks": ["/music/song.mp3", ...],
///       "year": 2024,
///       "genre": "Rock"
///     }
///   }
/// }
```

### Cache Trigger Points

```rust
// === In RiffApp::update() — first_frame ===

// Startup: load cached library instead of auto-scanning
if self.first_frame {
    state.library = LibraryManager::load_cache();
    // ... rest of first_frame (load paths, set statuses) ...
    self.first_frame = false;
}
```

```rust
// === In poll_library_updates() — ScanComplete handler ===

// Post-scan: persist updated library to cache
LibraryUpdate::ScanComplete { path, total_files } => {
    state.library_statuses.insert(path, LibraryStatus::Scanned(total_files));
    state.scan_status = Some(format!("Scan complete: {} tracks", total_files));
    state.library.save_cache();
}
```

```rust
// === In show_settings_view() — delete button handler ===

// Post-removal: persist updated library to cache
if ui.button("\u{1F5D1}").clicked() {
    state.library.remove_tracks_by_root(&path);
    state.library_paths.retain(|p| p != &path);
    state.library_statuses.remove(&path);
    state.library.save_cache();
    // ... save path list to eframe::Storage ...
}
```

### Error Contract

All cache operations use `tracing::warn!` for non-fatal errors:

| Scenario | Behaviour | Log |
|----------|-----------|-----|
| Cache directory cannot be created | Save skipped, in-memory library unaffected | `"Failed to create cache directory: {e}"` |
| Serialisation fails | Save skipped, in-memory library unaffected | `"Failed to serialize library cache: {e}"` |
| File write fails | Save skipped, in-memory library unaffected | `"Failed to write library cache: {e}"` |
| Cache file does not exist | Return empty library, no error | (none, silent) |
| Cache file unreadable | Return empty library | `"Failed to read library cache: {e}"` |
| Cache file contains invalid JSON | Return empty library | `"Failed to deserialize library cache: {e}"` |

---

## Design Summary

### Components and Layer Assignments

| Component | Layer | New/Existing | Files |
|-----------|-------|-------------|-------|
| LibraryManager serde derive | Application | Change | `src/app/library_manager.rs` |
| `save_cache()` / `load_cache()` | Application | New | `src/app/library_manager.rs` |
| `Album` serde derives | Domain | Change | `src/domain/track.rs` |
| `Artist` serde derives | Domain | Change | `src/domain/track.rs` |
| Cache load on startup | Presentation | Change | `src/ui/app.rs` |
| Cache save on ScanComplete | Presentation | Change | `src/ui/app.rs` |
| Cache save on path removal | Presentation | Change | `src/ui/settings.rs` |

### Key Contracts and Interfaces

1. **Cache path**: `directories::ProjectDirs::data_local_dir() / "library_cache.json"` resolved per OS
2. **Cache format**: Single JSON object containing the full `LibraryManager` struct (tracks, artists, albums maps)
3. **Load call**: `LibraryManager::load_cache()` — called once on first frame before any UI rendering
4. **Save calls**: `state.library.save_cache()` — called after `ScanComplete` and after path removal
5. **Error handling**: All errors logged as warnings, never panics, never shows error to user

### Architectural Constraints

- **No auto-scan on startup**: Cache replaces the need for auto-scan. Scans are still user-triggered.
- **Single-file cache**: All three maps (tracks, artists, albums) serialised atomically in one JSON file. No incremental or per-path cache files.
- **Separate from path persistence**: Library paths continue to use `eframe::Storage` (existing system). The track cache is an independent concern.
- **Silent fallback on error**: If the cache cannot be loaded, the app starts with an empty library. The user experience is identical to first launch.
- **Application-layer persistence**: Cache logic lives in `LibraryManager` (Application layer), not in Infrastructure. It uses `std::fs`, `serde_json`, and `directories` directly, which are general-purpose crates, not domain-specific infrastructure.
- **No cache invalidation**: The cache is always treated as authoritative. After scan or path removal, it is rewritten entirely. There is no staleness detection based on filesystem modification times.

### Domain Model Decisions

- **LibraryCache is a Repository** — its responsibility is to persist and restore the `LibraryManager` aggregate. It is not a domain entity itself.
- **Cache = full state snapshot** — the cache is a serialised copy of the entire in-memory library. There is no diffing or incremental update.
- **Cache file is disposable** — deleting the cache file loses no data that cannot be recovered by a full re-scan. This keeps the design simple and avoids migration concerns.
- **Platform path resolved at runtime** — the `directories` crate handles OS differences. The code does not hardcode any paths.

### Open Questions Resolved During Design

- *Should we cache per-path or as a single file?* → **Single file**. Simpler, atomic writes, no merge logic needed. Rewriting the full cache is fast enough for expected library sizes.
- *Should we update the cache incrementally as tracks are scanned?* → **No**. Write only on ScanComplete. Incremental writes during scanning would add I/O overhead and complexity with no user-visible benefit.
- *Should we compress the cache file?* → **No**. JSON is human-readable for debugging, and gzip would add a dependency for marginal space savings. A library of 50,000 tracks serialises to ~10 MB of JSON.
- *Should we add a "Clear Cache" button in settings?* → **Deferred**. Deleting the cache manually or running a scan both achieve the same result.

### Design Status

**Approved — ready for implementation**

---

## Decisions Log

| Date | Decision | Reasoning | Alternatives Considered |
|------|----------|-----------|------------------------|
| 2025-07-10 | Single JSON cache file for all tracks | Simpler to manage. Atomic writes. No merge logic. Full rewrite is fast enough for expected library sizes (~10 MB for 50k tracks). | Per-path cache files; SQLite database; binary format |
| 2025-07-10 | Cache written only on ScanComplete | Minimises I/O. Scanning is already the natural sync point. Incremental writes would add complexity with no user benefit. | Write after each track chunk; write on timer |
| 2025-07-10 | `directories` crate for platform path | Already a dependency. Correct XDG/AppData resolution on all target OSes. No new dependency. | Hardcoded paths; eframe::Storage (string-length limited); custom path resolution |
| 2025-07-10 | Application-layer persistence in LibraryManager | Cache logic is state serialisation, not domain infrastructure. LibraryManager already owns the data being serialised. | New infra/persistence.rs module; separate CacheService in app layer |
| 2025-07-10 | Silent fallback on cache error | Non-fatal errors should never disrupt the user experience. An empty library with a scan button is better than a crash dialog. | Panic on corrupt cache; auto-delete and re-scan; show error dialog |
| 2025-07-10 | Cache replaces auto-scan on startup | Auto-scan wastes startup time for large libraries. Cache loads in <1 second. User still triggers scans manually when they add music. | Auto-scan on startup (rejected by user); lazy scan in background after UI renders |
| 2025-07-10 | `serde_json` for serialisation format | Already in dependency tree. Human-readable for debugging. Adequate performance. No new dependency. | bincode (faster but binary); ron (Rust-native but niche); msgpack (smaller but new dep) |

---

## Open Questions

- [ ] Should we add a cache version field to the JSON schema to handle future format migrations? *(Deferred — the cache is disposable; a version mismatch can just fall back to empty)*
- [ ] Should we cache cover art image data alongside track metadata? *(Deferred — cover art is already cached in memory via egui textures; disk cache would add significant complexity)*

---

## Constraints

- MUST use the `directories` crate for resolving the cache file path (no hardcoded paths).
- MUST NOT auto-scan on startup — the cache is the sole persistence mechanism for track data.
- MUST survive corrupt or missing cache files without crashing — fall back to empty library.
- MUST write cache after each ScanComplete event.
- MUST write cache after each library path removal.
- MUST NOT introduce new external dependencies beyond what is already in `Cargo.toml`.
- MUST be transparent to the user — no "Loading cache..." UI state; the library should appear instantly.

---

## Key Files

| File | Action | Description |
|------|--------|-------------|
| `src/app/library_manager.rs` | **MODIFY** | Add `#[derive(Serialize, Deserialize)]` to `LibraryManager`. Add `save_cache()` and `load_cache()` methods. Add `cache_path()` helper. |
| `src/domain/track.rs` | **MODIFY** | Add `#[derive(Serialize, Deserialize)]` to `Album` and `Artist`. |
| `src/ui/app.rs` | **MODIFY** | Replace auto-scan with `LibraryManager::load_cache()` in `first_frame`. Add `state.library.save_cache()` in `ScanComplete` handler. |
| `src/ui/settings.rs` | **MODIFY** | Add `state.library.save_cache()` after `remove_tracks_by_root()` in delete button handler. |
