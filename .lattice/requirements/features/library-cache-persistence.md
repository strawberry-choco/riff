---
feature: Library Cache Persistence
epic: Music Library
status: implemented
priority: P1
depends_on: ["Library Scanning", "Music Library Management"]
personas: ["Music Listener"]
source_docs: []
implementation_notes: |
  Implemented in app/library_manager.rs (save_cache/load_cache on
  LibraryManager), triggered from ui/app.rs (load on first_frame, save on
  ScanComplete) and ui/settings.rs (save on path removal). Uses the
  directories crate for platform-appropriate cache paths. Serialises the
  full LibraryManager (tracks, artists, albums) as JSON.
---

# Library Cache Persistence

## Problem Statement

After adding library paths and scanning for tracks, the user expects their
library (the full list of scanned tracks, artists, and albums) to be
available on the next application launch. Currently, only the library path
list is persisted — the scanned track data lives only in memory. Every
restart requires a full re-scan, which is wasteful and slow for large
collections (tens of thousands of files).

## User / Personas

- **Music Listener**: A person with a large local music collection who opens
  the app frequently. They expect their music library to be ready instantly
  when they launch the app, not after waiting for a full directory scan.

## Scope

**In scope:**
- Persist the full scanned library (tracks, artists, albums) to a JSON
  cache file on disk
- Load the cache on startup so the library is immediately available
- Re-save the cache after every successful scan
- Re-save the cache when a library path is removed (to evict orphaned
  tracks)
- Use platform-appropriate cache directory via the `directories` crate
- Graceful degradation: corrupt or missing cache falls back to an empty
  library (user just clicks Scan)

**Out of scope:**
- Incremental cache updates (always rewrite full cache after scan)
- Compression of the cache file
- Cache invalidation based on filesystem modification times
  (full re-scan is still needed for that)
- Cloud sync of the cache

## Boundary Conditions

- A library of 50,000 tracks should load from cache in under 1 second
  (JSON deserialisation of ~10 MB)
- Corrupt cache file (truncated, invalid JSON) must not crash the
  application — fall back to empty library silently
- Missing cache file (first launch) must not log errors — just start
  with an empty library
- Cache write failures (permissions, disk full) must not crash the
  application — log a warning and continue
- The cache directory must be created automatically if it does not exist

## Assumptions

- JSON serialisation is fast enough for the expected library sizes
  (tested against ~10,000 tracks)
- The cache file is local and reasonably sized (< 50 MB)
- The `directories` crate resolves the correct platform path on all
  supported OSes
- The user triggers scans manually; the cache is only written after
  a scan completes

## Scenarios

### Scenario 1: First launch — no cache exists
A user installs the app and runs it for the first time.

**Acceptance Criteria:**
- Given the app is launched for the first time, when the library view
  opens, then it shows an empty library (no tracks, no artists, no albums)
- Given the library is empty, when the user navigates to settings, then
  the library path list is also empty
- No "cache not found" errors or warnings are logged

### Scenario 2: Scan then relaunch — cache restores library
A user scans their library, quits, and relaunches.

**Acceptance Criteria:**
- Given the user has configured a library path and completed a scan, when
  they quit and relaunch the application, then all previously scanned
  tracks appear in the library immediately
- Given the library is populated from cache, when the user browses
  artists and albums, then all artists and album groupings are preserved
  exactly as after the scan
- Given the cache is loaded, when the user views track details, then
  all metadata fields (title, artist, album, genre, year, track number)
  are present

### Scenario 3: Re-scan updates the cache
A user adds new music and rescans.

**Acceptance Criteria:**
- Given a cached library exists, when the user rescans a library path,
  then the cache is updated with any new tracks
- Given a cached library exists, when the user rescans a library path
  after deleting files, then the cache no longer contains the deleted
  tracks
- Given the cache is updated by a scan, when the user relaunches, then
  the updated track list is loaded

### Scenario 4: Remove library path updates the cache
A user removes a library path from settings.

**Acceptance Criteria:**
- Given a library path was scanned and cached, when the user deletes
  the path from settings, then all tracks belonging to that path are
  removed from the in-memory library and the cache is rewritten
- Given the cache was updated after path removal, when the user
  relaunches, then the removed path's tracks are absent

### Scenario 5: Corrupt cache does not crash
The cache file is manually corrupted or truncated.

**Acceptance Criteria:**
- Given the cache file contains invalid JSON, when the app launches,
  then it starts with an empty library (no crash)
- Given the cache file ended with incomplete JSON data, when the app
  launches, then it starts with an empty library
- A warning is logged to help with debugging, but the UI is unaffected

### Scenario 6: Cache write failure is non-fatal
The cache directory is read-only or disk is full.

**Acceptance Criteria:**
- Given the cache directory cannot be written to, when a scan completes,
  then the scan result is still applied to the in-memory library
- Given a cache write failed, when the user looks at the UI, then no
  error dialog is shown (a warning is logged)

## Implementation Notes

1. **Storage path**: Use `directories::ProjectDirs::data_local_dir()`
   joined with `library_cache.json`. The `directories` crate already
   in the dependency tree resolves the OS-appropriate path:
   - Linux: `~/.local/share/riff/library_cache.json`
   - macOS: `~/Library/Application Support/com.riff.riff/library_cache.json`
   - Windows: `C:\Users\<user>\AppData\Local\riff\riff\library_cache.json`

2. **Serialisation format**: JSON via `serde_json`. The full
   `LibraryManager` struct (tracks, artists, albums maps) is serialised
   as a single JSON object. All track types already derive
   `Serialize`/`Deserialize`.

3. **Load timing**: Load cache in the `first_frame` handler of
   `RiffApp::update()`, before any UI rendering dependant on track data.

4. **Save timing**: Save cache immediately after each `ScanComplete`
   event in `poll_library_updates()`, and immediately after removing
   tracks by root in the settings delete handler.

5. **Error handling**: Both save and load use `tracing::warn!` for
   non-fatal errors (corrupt cache, write failure, directory creation
   failure). No unwrap, no expect, no crash.

6. **Cache exclusion from `Music Library Management`**: The path list
   (managed by `eframe::Storage`) and the track cache (managed as a
   separate JSON file) are independent concerns. The former persists
   which directories to scan; the latter persists the scan result.

## Resolved Decisions

| Question | Decision |
|---|---|
| Cache format | **JSON** — simple, debuggable, no new dependency, serde_json already in the tree |
| Single file or per-path | **Single file** — simpler to manage; rewriting the whole cache is fast enough |
| Auto-scan vs cache on startup | **Cache load** — auto-scan on every startup is wasteful for large libraries |
| Albust/Artist rebuild vs persist | **Persist all three maps** — serialising artists/albums is cheap and avoids recomputation |

## Open Questions

- [ ] Should we add a "Clear Cache" button in settings for debugging?
  *(deferred — deleting the cache file manually or running a scan both work)*

## Links

- Epic index: [index.md](../index.md)
- Library Scanning: [library-scanning.md](library-scanning.md)
- Music Library Management: [music-library-management.md](music-library-management.md)
