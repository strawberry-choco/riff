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