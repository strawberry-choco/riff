# Persistence

riff is offline-first and keeps its authoritative persistent state in one embedded `SQLite` database: the **Application Store** (`riff.sqlite3`), which holds the Library, Playlists, and Settings. The store is the single authority — the UI never owns a second copy of persisted state; it reads through bounded **Session Projections** that are invalidated by a session-local generation counter after every committed mutation. This document describes where the data lives, when it is written, how the schema evolves, and how corruption is handled.

For the port traits that define what the store provides see `riff-persistence/src/store.rs` (the persistence contract); for the `rusqlite` implementation see `riff-infra/src/store/sqlite.rs`. For the stored entity types see [./data-model.md](./data-model.md), and for the scan flow that populates the Library see [./threading-model.md](./threading-model.md).

## The Application Store (`riff.sqlite3`)

One database file holds every authoritative table. The path is resolved at runtime with the `directories` crate, mirroring where the legacy files lived:

| Platform | Store path |
|----------|------------|
| Linux | `~/.local/share/riff/riff.sqlite3` (honors `$XDG_DATA_HOME`) |
| macOS | `~/Library/Application Support/com.riff.riff/riff.sqlite3` |
| Windows | `%LOCALAPPDATA%\riff\riff\riff.sqlite3` |

### Schema shape

Natural keys throughout: track path, artist name, `(album_artist, title)` composite for albums, playlist ID string. UTC epoch-nanosecond integers for timestamps, nullable where unknown. Typed Settings tables (a single-row scalar settings table plus explicit library-path and watch-state tables). A derived Rust-lowercased `search_text` column on tracks carries exact substring-search parity. Strict foreign keys chain tracks → albums → artists inside the Library; playlist entries intentionally have no enforced link to tracks — dangling references are valid product behavior validated at read time.

### Connection setup

The shared connection is configured for durability and determinism: WAL journal mode, `synchronous = NORMAL`, `foreign_keys = ON`, a ~5-second busy timeout, and case-sensitive `LIKE` so folder prefix queries match paths byte-for-byte. One mutex-guarded connection serves every store port for v1; WAL allows a future read/write split if profiling demands it.

## Save timing

Every logical change commits as one small durable transaction at the moment it happens — there is no whole-file rewrite, no debouncing, and no save-on-quit step:

| Event | Transaction |
|-------|-------------|
| Finished play | One transaction bumps `play_count` and stamps `last_played` together |
| Playlist create/rename/delete/add/remove | One immediate durable transaction per mutation |
| Settings change (volume, toggles, paths, watch states) | One small durable transaction per change |
| Tag edit | One transaction upserts metadata, preserves history, re-derives album year/genre |
| Library-path removal | One transaction removes that root's tracks, orphaned parents, and the path record |
| Scan batches | Adjacent chunk work (~10 tracks) commits per transaction, so an interrupted scan keeps everything already committed |
| Clear Library | One transaction wipes all collection tables; playlists and settings are untouched |

Because each event commits before it is reported done, a crash right afterward cannot lose it.

## Migrations

Schema evolution uses ordered, checksummed migrations embedded in `riff-infra/src/store/sqlite.rs`. Each migration has a stable version number and a SHA-256 content checksum recorded in a `schema_migrations` table. On open, applied versions are verified against their embedded checksums and skipped; pending ones apply exactly once, each inside its own transaction. Editing a shipped migration (or its checksum) makes already-migrated stores fail to open with a clear error instead of silently diverging.

## Corruption recovery

Opening the store follows a deliberate sequence:

1. **Probe read-only** and run `PRAGMA quick_check`. A missing file is a normal fresh start.
2. **Healthy file**: open writable and run migrations.
3. **Corrupt or unreadable**: rename the database plus its `-wal`/`-shm` siblings beside themselves with a Unix-nanosecond suffix (preserved for recovery tools), create a fresh database, and continue.
4. **Recovery itself fails**: fatal startup error with a clear message — never silent data loss.

## Session Projections

The UI does not query SQLite arbitrarily while rendering. Views read through bounded **Session Projections** (library-side in `riff-library/src/app/projection.rs`, playback-side in `riff-playback/src/app/projection.rs`, reached through the Session Views facade in `riff-backend`): small in-memory caches of store query results — visible row windows for the flat list and search, per-folder listings for the folder tree, per-kind lists for smart playlists, and the playback-side projection (current Track, Up Next window, details-panel selection) — stamped with the value of a session-local generation counter. Every committed store mutation bumps that counter inside the store, so projections refetch on the next frame without any restart. Stale reads are possible only between a commit and the next refresh, which generation invalidation makes explicit. There is no other in-memory copy of the library: the Application Store is the single implementation of collection semantics.

## Clear Library

The maintenance action wipes the Library collection section as one transaction: every track (with its play history), album, and artist. Playlists and Settings tables are untouched — playlist entries referencing wiped tracks survive dangling and stay listed until the files return via a rescan. See [Clear Library](../reference/glossary.md) in the glossary.

## Legacy JSON Files

The former design persisted a non-authoritative Library Cache and Playlists as whole-file JSON snapshots rewritten on each save. Those Legacy JSON Files (`library_cache.json`, `playlists.json`) are ignored: never read, never written, never imported, never deleted. This decision is recorded in [ADR 0001](../adr/0001-sqlite-is-the-authoritative-application-store.md), which supersedes [decision 004](../product/decisions/004-library-cache-as-json.md).

## Cover Art LRU (in-memory only)

Decoded cover art is cached in memory, not on disk, in two bounded LRU caches: the cover service (`riff-library`) caches decoded RGBA images (cap 50) so repeated requests never re-decode, and the UI (`riff-gui`) keeps a `cover_textures` map of egui `TextureHandle`s plus a `cover_lru_keys` vector that records recency, capped at 50 entries with least-recently-used eviction. There is no disk cache for covers — cover bytes are re-read and re-decoded from the source file on a cache miss.

## Architectural Constraints

- **Single authority.** The Application Store is the only persisted application state. In-memory structures are caches or session state, never a second authority.
- **Per-event durability.** One transaction per logical event; scans batch adjacent chunk work (~10 tracks).
- **No auto-scan on startup.** Scans remain user-triggered; the app never scans on launch.
- **Ports over drivers.** The store ports (`StoreMigrations`, `SettingsStore`, `PlaylistStore`, `LibraryQueryStore`, `LibraryMutationStore`) are defined in the `riff-persistence` contract and implemented by `SqliteStore` in `riff-infra` over the shared connection; no crate above `riff-infra` imports `rusqlite`.
- **Append-only migrations.** Shipped migrations are immutable; schema changes arrive as new versions.

## See also

- [ADR 0001](../adr/0001-sqlite-is-the-authoritative-application-store.md) — why SQLite is the authoritative Application Store.
- [ADR 0002](../adr/0002-ui-reads-the-store-through-session-projections.md) — the Session Projection model.
- [ADR 0003](../adr/0003-store-query-model.md) — canonical query shapes and orderings.
- [./data-model.md](./data-model.md) — the domain types and their on-disk encodings.
- [./threading-model.md](./threading-model.md) — the scanner thread that feeds the store.
