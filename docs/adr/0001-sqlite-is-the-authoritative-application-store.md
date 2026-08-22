# SQLite Is the Authoritative Application Store

**Status**: Accepted
**Date**: 2026-08-22

The former design persisted a non-authoritative Library Cache and Playlists as whole-file JSON snapshots rewritten on each save, which lost all uncommitted work on corruption or exit and made frequent durable writes expensive. We will use an embedded SQLite database through `rusqlite` as the authoritative **Application Store** for the Library, user Playlists, and Settings, committing one transaction per logical change (scans may batch adjacent track changes). This supersedes [ADR 004: Library Cache as JSON File](../product/decisions/004-library-cache-as-json.md); existing Legacy JSON Files are ignored, never imported or deleted, and a fresh database is created at the existing data-local location (`riff.sqlite3`).

## Considered Options

- **Keep whole-file JSON**: simple and human-readable, but every write rewrites the full collection and corruption loses the entire snapshot.
- **JSON blobs inside SQLite**: gains crash-safe container semantics but retains opaque, unqueryable data and misses the relational benefits.
- **Embedded SQLite (chosen)**: incremental durable writes, enforceable relationships, typed queries, and established recovery behavior at the cost of a native dependency and explicit schema migrations.

## Consequences

- Persistence moves behind focused application-layer ports implemented by infrastructure; the UI never imports rusqlite.
- The schema uses natural text keys (track path, artist name, album composite key, playlist ID), typed Settings tables, nullable UTC epoch-nanosecond timestamps, WAL journaling with `synchronous=NORMAL`, `foreign_keys=ON`, and a ~5-second `busy_timeout`.
- Library relationships are strictly enforced. Playlist entries intentionally remain dangling-capable references to Tracks because invalid entries are a current product behavior, not corruption; they are validated at read time.
- Schema evolution uses ordered, checksummed entries in a `schema_migrations` table rather than ad-hoc checks or destructive recreation; migration execution is embedded application code over rusqlite.
- Startup runs an open plus `PRAGMA quick_check`; if either fails, `riff.sqlite3` and its `-wal`/`-shm` siblings are renamed beside the original with a Unix-nanosecond suffix and a fresh Store is created. If the Store still cannot be opened after that recovery, startup fails fast with a clear error. Migration checksum mismatch or migration failure is also fatal.
- “Clear Library” deletes Library collection tables while preserving Playlists and Settings; it is not a whole-database reset.
- Documentation that describes the Library Cache, JSON persistence, “no migrations,” or eframe-backed settings must be updated before this decision is considered implemented.

# The UI Reads the Store Through Session Projections

**Status**: Accepted
**Date**: 2026-08-22

With SQLite as the authoritative Application Store, the UI must stop treating `AppState` as the owner of the full Library. Views read through application-layer store/query ports into small bounded in-memory results (**Session Projections**), invalidate them by a Store generation counter after writes, and render last-known data while reloading. Writes commit to the Store first, then refresh affected projections. A single SQLite connection guarded by a mutex is acceptable initially because egui frame work reads projections rather than issuing arbitrary SQL while holding it.

## Considered Options

- **Keep the full library resident in memory** and merely mirror it to SQLite: preserves today’s UI code but leaves two competing authorities.
- **Query SQLite synchronously from widgets every frame**: always-fresh data but exposes UI rendering to database and lock latency.
- **Session projections with generation-based invalidation (chosen)**: responsive frames, one persistent authority, and bounded reload cost.

## Consequences

- `AppState` shrinks to playback/session/UI concerns plus current projections; repository methods define the view shapes the UI needs.
- Stale reads are possible only between a committed write and the corresponding projection refresh, which generation invalidation makes explicit.
- Lock hold times and result-set sizes become deliberate API constraints rather than incidental implementation details.
- The generation counter is session-local and resets on launch; projections compare their loaded generation against the Store’s current in-memory generation.

# Store Query Model

**Status**: Accepted
**Date**: 2026-08-22

Views read through bounded Session Projection queries rather than whole-library snapshots. A query signature identifies mode/filter/sort/generation; each projection caches its total count and only the currently visible row windows (`LIMIT/OFFSET`) until invalidated by the session-local Store generation counter. If very large libraries make deep offsets slow in practice, keyset pagination is a targeted follow-up, not part of v1.

Canonical SQL ordering uses byte-wise text comparison unless stated otherwise:

- Flat/all-tracks and whole-folder subtree results: track path ascending.
- Direct folder tracks and Album tracks: track number ascending with missing numbers last, then filename/path tiebreak.
- Artists: name ascending. Albums within artist: year descending, then title ascending.

Search parity is preserved by storing a derived Rust-lowercased search-text column at Track write time; queries lowercase user input in Rust and use substring lookup over that column. Changing the derived algorithm requires an explicit migration/reindex.

Track paths are stored as raw lossy strings, preserving today’s identity behavior, including platform normalization quirks; changing that is a separate domain decision.