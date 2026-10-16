# Rule matching lowercases at query time, not at write time

**Status**: Accepted
**Date**: 2026-10-15

Custom Smart Playlist Rules match text ("genre is rock", "title contains moon") case-insensitively by
asking SQLite to lowercase the column inside the predicate — `lower(column) = ?` against a needle Rust
has already lowercased — rather than by storing a lowercase twin of every matchable column. This
deliberately reverses the reasoning of the store's two earlier lowercase decisions: migration 004 derives
the tracks' `search_text` in Rust at write time and migration 011 adds `name_lower` /
`album_artist_lower` / `title_lower` for the same reason, both documented as necessary because SQLite's
`instr` is case-sensitive, `case_sensitive_like` is forced on per connection, and SQLite's `lower()`
folds ASCII only while Rust's folds non-Latin script correctly.

We accepted the ASCII limitation because adding lowercase twins for the sixteen matchable fields means a
table rebuild plus a library-wide re-read of every audio file behind a `METADATA_VERSION` bump, to serve
a feature whose reads are full rowid scans anyway — the store has no `CREATE INDEX` at all today. Exact
case-sensitive matching was rejected as the alternative, which would make "Rock" and "rock" two genres to
the listener and would not match how the genres view already behaves.

## Consequences

- Non-Latin tags do not fold: a file tagged `РОК` is not found by a rule for `рок`. The reverse holds for
  the derived lowercase needle, so the mismatch is one-directional and silent. A test fixture pins this
  documented limitation rather than leaving it to be rediscovered.
- No migration backfills anything and `METADATA_VERSION` stays where it is, so enabling this feature
  costs an existing library no re-scan.
- Adding lowercase columns later does not change Rule semantics, only the query shape; if it happens,
  supersede this ADR rather than contradicting it silently.
