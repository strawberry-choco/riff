# Smart playlist rules are collection data; Settings is only where they are edited

**Status**: Accepted
**Date**: 2026-10-15

A Custom Smart Playlist's Rules are authored on a Settings tab, but they are not settings: they are
user-authored facts about Tracks, so they are persisted as collection data in their own typed tables in
the Application Store (`smart_playlists`, `smart_rule_boxes`, `smart_rules`), while `Settings` keeps its
glossary meaning of "preferences that are not music-collection data". The tab is a surface, not a
statement about what kind of data the thing is.

## Considered Options

- **A serialised rule tree in the settings row.** Rejected on the store's own recorded intent: the
  migration that introduced the typed settings tables rules out opaque blobs, and a packed tree cannot be
  searched for the rows that name a field a later migration removes.
- **Rules as a discriminator on the `playlists` table.** Rejected because `Playlist` is defined by its
  stored, user-ordered Track references with dangling entries permitted and validated on use, while a
  Smart Playlist stores no Tracks at all; the glossary makes that distinction explicitly ("a Playlist
  stores the user's Track references, a Smart Playlist does not") and sharing a table would make every
  playlist read special-case the other kind.
- **A `matchable_fields` table describing the field catalog in the database.** Rejected because only code
  can say which comparison operators a field allows and which value control the editor draws, so the table
  would be a second source of the same list. The catalog is one Rust structure; the database enumerates
  the legal ids in CHECK constraints, and per-field legality is enforced before anything reaches the query
  builder.

## Consequences

- Adding or removing a Matchable Field is a store migration that rebuilds `smart_rules` to widen its CHECK
  list — the same shape migrations 010 and 012 used for the settings table — and Rules reference stable
  field ids so a display name can change with no data change at all.
- Sidebar-wide name uniqueness cannot be expressed as one table constraint, so the store's mutation path
  reads both `playlists` and `smart_playlists` to check it, and the ordinary-playlist create and rename
  paths gain that check too. Two features that were deliberately independent now know each other's names.
  The playlist port's own contract documents the opposite today — "duplicate names are allowed" — and that
  sentence is part of this change, not a leftover: a port whose rustdoc contradicts what it now enforces
  will be "fixed" back by the next reader.
- A Rule naming a field id this build does not recognise still reads back, because a CHECK constraint is
  enforced on write and not on read, so an older build opening a newer store shows "riff can't understand
  this rule" and matches nothing instead of failing.
- Rules survive **Clear Library**, which deletes collection data while preserving Playlists and Settings:
  an unrecoverable hand-built tree is not collateral of a re-scan.
