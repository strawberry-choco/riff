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
- Schema evolution uses ordered, checksummed entries in a `schema_migrations` table rather than ad-hoc checks or destructive recreation; migration execution is embedded application code over rusqlite. Each migration's expected checksum is **derived from that migration's own SQL text** (SHA-256 of the text after line-ending and trailing-whitespace normalisation, recorded as lowercase hex) rather than typed beside it, so a checksum cannot drift from the statements it describes and editing a shipped migration changes what an already-migrated store is expected to have recorded.
- Startup runs an open plus `PRAGMA quick_check`; if either fails, `riff.sqlite3` and its `-wal`/`-shm` siblings are renamed beside the original with a Unix-nanosecond suffix and a fresh Store is created. If the Store still cannot be opened after that recovery, startup fails fast with a clear error. Migration failure is also fatal, and **so is a migration checksum mismatch — but a mismatch is not recovered**. A checksum mismatch is an integrity signal: the database is intact and only this build's expectations reject it, so the store is left exactly as it is (no rename-aside, no fresh Store) and the user is told to delete the file and relaunch. This fulfils the consequence originally recorded here; the recovery path is entered only for an actually-corrupt store.
- Because the digests were previously hand-typed and seven of fifteen did not describe their own migration, deriving them correctly means **every store migrated by an earlier build now refuses to open** with the mismatch error. That is accepted rather than papered over: there is no re-stamp path, no legacy-value table and no permanent special case, and a populated Store must be deleted by its user. The file is left intact for them to inspect or move aside first.
- “Clear Library” deletes Library collection tables while preserving Playlists and Settings; it is not a whole-database reset.