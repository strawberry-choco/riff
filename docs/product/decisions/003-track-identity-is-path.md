# 003: Track Identity Is the Full File Path

**Status**: Accepted
**Date**: 2026-07-31

## Context

Every track in the library needs a stable identity for deduplication, queue management, and cache persistence. The options considered were:

- **Content hash**: cryptographically stable, but requires reading the file.
- **Database auto-ID**: requires a database dependency and migration support.
- **User-assigned ID**: fragile, requires user intervention.
- **Full file path**: simple, derived from `PathBuf::to_string_lossy()`, works naturally with JSON serialization.

## Decision

A track's identity (`TrackId`) is its full file path as a string. The same file on disk always has the same identity, regardless of which library path it was scanned through.

## Consequences

**Positive**:
- Automatic deduplication: the same file path cannot appear twice in the index.
- Simple serialization: path strings in a JSON cache, no database.
- Cross-library deduplication is automatic (same file scanned through different paths = one entry).
- No additional infrastructure (no database, no ID assignment logic).

**Negative**:
- If a user moves or renames a file, the old index entry becomes stale (the path no longer exists). A rescan is required to update the index.
- Renaming a directory invalidates all entries under it.
- The identity is fragile to path changes — this is a trade-off for simplicity.

## Related Documents

- [Features](./features.md) — Library Scanning, Library Cache Persistence.
- [Data Model](../technical/data-model.md).
- [Data Flow](../technical/data-flow.md).
