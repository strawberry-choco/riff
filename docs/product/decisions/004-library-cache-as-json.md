# 004: Library Cache as JSON File

**Status**: Accepted
**Date**: 2026-07-31

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

- [Features](./features.md) — Library Cache Persistence.
- [Persistence](../technical/persistence.md).
- [Roadmap](./roadmap.md) — Cache schema versioning recommendation.
