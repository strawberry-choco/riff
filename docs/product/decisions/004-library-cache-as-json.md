# 004: Library Cache as JSON File

**Status**: Superseded by [ADR 0001: SQLite is the authoritative Application Store](../../adr/0001-sqlite-is-the-authoritative-application-store.md), with the query model in [ADR 0003](../../../adr/0003-store-query-model.md)
**Date**: 2026-07-31

> **Superseded.** The JSON library cache described here no longer exists. riff persists the Library, Playlists, and Settings in the Application Store (`riff.sqlite3`); see [ADR 0001](../../adr/0001-sqlite-is-the-authoritative-application-store.md) and [Persistence](../technical/persistence.md). This document is kept for historical context only.

## Context

The library needs to persist between launches so that a large collection is browsable instantly instead of re-scanning the disk on every start. The options considered were:

- **SQLite database**: robust, but adds a dependency and migration complexity.
- **Binary serialization (e.g. bincode)**: smaller files, but not human-readable.
- **JSON file**: human-readable, portable, simple to serialize with serde.

## Decision

The library cache is a JSON file (`library_cache.json`) using serde serialization, stored in the platform's data-local directory via the `directories` crate.

## Consequences

**Positive**:
- Human-readable: users can inspect, backup, or manually edit the cache.
- Portable: works on any platform with serde support.
- No database dependency: zero external DB crates, no migrations, no connection management.
- Simple crash recovery: a missing or corrupt cache falls back to an empty library + rescan.
- Smaller codebase: no database schema or migration logic to maintain.

**Negative**:
- Larger file size for large collections compared to binary formats.
- No schema versioning yet (a known gap — recommended for future work).
- Full file rewrite on every cache update (not incremental).
- JSON parse failures on corrupt files cause a full library loss (fallback to empty).

## Related Documents

- [ADR 0001: SQLite is the authoritative Application Store](../../adr/0001-sqlite-is-the-authoritative-application-store.md) — the superseding decision.
- [Persistence](../technical/persistence.md) — how persistence actually works today.

