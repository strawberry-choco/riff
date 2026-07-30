# Persistence

riff is offline-first and keeps three distinct pieces of state on disk: the scanned music library (a JSON cache of tracks, artists, and albums), the list of registered library paths, and the per-path folder-watch state. The library cache is by far the most important — it lets the app show a full library instantly on startup without re-scanning. This document describes each mechanism, where the data lives, when it is written, and how errors are handled.

For the types that are serialized, see [./data-model.md](./data-model.md). For the scan flow that populates the cache, see [./threading-model.md](./threading-model.md) and [./data-flow.md](./data-flow.md).

## The Library Cache (`library_cache.json`)

The entire `LibraryManager` aggregate — its `tracks`, `artists`, and `albums` maps — is serialized as a single JSON object and written to one file. The cache is a full snapshot, not an incremental log; it is rewritten wholesale whenever it changes.

### Path resolution

The cache path is resolved at runtime with the `directories` crate, never hardcoded:

```rust
fn cache_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "riff")
        .map(|d| d.data_local_dir().join("library_cache.json"))
}
```

This maps to the platform-appropriate data directory:

| Platform | Cache path |
|----------|------------|
| Linux | `~/.local/share/riff/library_cache.json` (honors `$XDG_DATA_HOME`) |
| macOS | `~/Library/Application Support/com.riff.riff/library_cache.json` |
| Windows | `%LOCALAPPDATA%\riff\riff\library_cache.json` |

If the parent directory does not exist when saving, it is created with `create_dir_all`.

### JSON format

The file is a single JSON object with three top-level keys. An abbreviated example:

```json
{
  "tracks": {
    "/music/song.mp3": {
      "id": "/music/song.mp3",
      "file_path": "/music/song.mp3",
      "metadata": {
        "title": "Song Title",
        "artist": "Artist Name",
        "album": "Album Name",
        "album_artist": "Artist Name",
        "track_number": 1,
        "disc_number": 1,
        "genre": "Rock",
        "year": 2024,
        "composer": null,
        "comment": null
      },
      "duration": { "secs": 240, "nanos": 0 },
      "sample_rate": 44100,
      "channels": 2
    }
  },
  "artists": {
    "Artist Name": { "name": "Artist Name", "albums": ["Artist Name - Album Name"] }
  },
  "albums": {
    "Artist Name - Album Name": {
      "title": "Album Name",
      "artist": "Artist Name",
      "tracks": ["/music/song.mp3"],
      "year": 2024,
      "genre": "Rock"
    }
  }
}
```

Because `TrackId` is a newtype over `String` and the album/artist keys are plain strings, the maps serialize as JSON objects keyed by those strings.

### When the cache is loaded

The cache is loaded once, on the first frame of the UI (`RiffApp::update` with `first_frame == true` in `src/ui/app.rs`):

```rust
state.library = LibraryManager::load_cache();
```

`load_cache` returns an empty `LibraryManager` if the file is missing, unreadable, or contains invalid JSON. A successful load populates the library immediately, so the user sees their tracks without any scan and without any "loading" state. No scan is triggered on startup; the cache fully replaces the need for one.

### When the cache is saved

The cache is written in exactly two situations, both from the UI layer:

1. **Scan complete** — when `poll_library_updates` receives `LibraryUpdate::ScanComplete`, it calls `state.library.save_cache()`.
2. **Library path removal** — when the user deletes a library path in settings, the handler removes the path's tracks (`remove_tracks_by_root`), then calls `state.library.save_cache()` so the cache no longer references them.

`save_cache` creates the cache directory if needed, serializes the whole `LibraryManager` with `serde_json::to_string`, and writes the file with `std::fs::write`.

### Error contract

Every cache operation degrades gracefully. Failures are logged with `tracing::warn!` and never panic, never propagate to the user as an error dialog, and never disturb the in-memory library.

| Scenario | Behavior | Log |
|----------|----------|-----|
| Cache directory cannot be created | Save skipped; in-memory library unaffected | `Failed to create cache directory: {e}` |
| Serialization fails | Save skipped; in-memory library unaffected | `Failed to serialize library cache: {e}` |
| File write fails | Save skipped; in-memory library unaffected | `Failed to write library cache: {e}` |
| Cache file does not exist | Returns empty library; no error | (none — silent) |
| Cache file unreadable | Returns empty library | `Failed to read library cache: {e}` |
| Cache file contains invalid JSON | Returns empty library | `Failed to deserialize library cache: {e}` |

A missing cache (first launch) and a corrupt cache are handled identically: the app starts with an empty library, exactly like a fresh install, and the user can scan to repopulate it.

## Library Path List

The list of registered library root folders is persisted separately from the track cache, using `eframe::Storage` under the key `"library_paths"` as a JSON array of path strings:

```json
["/home/user/Music", "/mnt/external/Music"]
```

The helpers live in `src/ui/settings.rs` (`load_library_paths`, `load_library_paths_immutable`, `save_library_paths`). The list is loaded on the first frame and written whenever a path is added or removed. On load, each path's status is set to `Idle` if it exists on disk or `Unavailable` if it does not (for example, an ejected drive). This mechanism is independent of the track cache: the path list says *where* to scan, while the cache holds *what* was found.

## Folder-Watch State

Per-path folder-watch state is also persisted through `eframe::Storage`, as a JSON map keyed by path. The `WatchState` enum (`Disabled`, `Enabled`, `Warning(String)`) derives serde specifically for this. On startup, paths whose state was `Enabled` are re-registered with the filesystem watcher; if registration fails, the state transitions to `Warning(reason)`. See [./platform-support.md](./platform-support.md) for platform notes on watching.

## Cover Art LRU (in-memory only)

Decoded cover art is cached in memory, not on disk. The UI (`src/ui/app.rs`) keeps a `cover_textures` map of egui `TextureHandle`s plus a `cover_lru_keys` vector that records recency. The cache holds at most **50** entries; when a 51st is inserted, the least-recently-used entry is evicted from both the map and the vector. Every access "touches" the key, moving it to the most-recently-used position. There is no disk cache for covers — cover bytes are re-read and re-decoded from the source file on a cache miss, which keeps the design simple and avoids persisting large binary blobs.

## Architectural Constraints

- **No auto-scan on startup.** The cache is the sole persistence mechanism for track data. Scans remain user-triggered; the app never scans on launch.
- **Single-file, atomic cache.** All three maps are written as one JSON file. There are no per-path cache files and no incremental updates, so there is no merge logic.
- **Application-layer persistence.** The cache logic lives in `LibraryManager` (the application layer), not in infrastructure. It uses only general-purpose crates (`std::fs`, `serde_json`, `directories`) and operates on data the `LibraryManager` already owns.
- **The cache is disposable.** Deleting `library_cache.json` loses nothing that a full re-scan cannot recover. There is no cache versioning or migration; a format mismatch simply falls back to an empty library.
- **The cache is always authoritative.** There is no staleness detection based on file modification times. It is rewritten entirely after each scan and after each path removal.

## See also

- [./data-model.md](./data-model.md) — the serialized types (`LibraryManager`, `Track`, `Album`, `Artist`, `WatchState`).
- [./threading-model.md](./threading-model.md) — the scanner thread that produces the data the cache stores.
- [./platform-support.md](./platform-support.md) — platform-specific cache paths and watch behavior.
