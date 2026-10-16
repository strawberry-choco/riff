# Store Query Model

**Status**: Accepted
**Date**: 2026-08-22

Views read through bounded Session Projection queries rather than whole-library snapshots. A query signature identifies mode/filter/sort; each projection caches its total count and only the currently visible row windows (`LIMIT/OFFSET`) until invalidated by the session-local Store generation counter. If very large libraries make deep offsets slow in practice, keyset pagination is a targeted follow-up, not part of v1.

Canonical SQL ordering uses byte-wise text comparison unless stated otherwise:

- Flat/all-tracks and whole-folder subtree results: track path ascending.
- Direct folder tracks and Album tracks: track number ascending with missing numbers last, then filename/path tiebreak.
- Artists: name ascending. Albums within artist: year descending, then title ascending.

Search parity is preserved by storing a derived Rust-lowercased search-text column at Track write time; queries lowercase user input in Rust and use substring lookup over that column. Changing the derived algorithm requires an explicit migration/reindex.

Track paths are stored as raw lossy strings, preserving today’s identity behavior, including platform normalization quirks; changing that is a separate domain decision.

## Amendment — 2026-09-22 (Listing Pages, and what still sits outside them)

The decision above stands: bounded reads over a query signature, not
whole-library snapshots, with keyset pagination still a follow-up. One detail of
the second sentence no longer describes the code, and one exception to it is now
recorded explicitly:

- A paged listing's total and its visible window are no longer two cached reads.
  Every paged listing reads one **Listing Page** (`Page<T>` on the
  Application Store query port): total and window under a single connection
  acquisition, so no committed write can interleave the two halves; the page
  carries no generation. "each projection caches its total count and only the
  currently visible row windows" describes what a projection *holds*, not what
  the store answers with. See
  [ADR 0002](0002-ui-reads-the-store-through-session-projections.md) for the
  staleness side.
- The genre-scoped hit reads (`hit_albums_in_genre`, `hit_artists_in_genre`,
  `album_hit_tracks_in_genre`, `hit_genre_counts`) are **not** covered by this
  record's bounded-window description. They materialise every genre-bearing
  track and every album and filter in Rust, with no offset-and-total counterpart at all.
  Making them bounded is a separately tracked query optimisation with its own
  result-set semantics, deliberately not part of the Listing Page change.

## Amendment — 2026-09-29 (what crosses the port, and what the seam asks for)

The decision and both 2026-09-22 bullets stand as descriptions of the **port**.
What changed is that the Session Views seam no longer reads a whole Listing Page
as one fact and hands it to a caller; it now asks for the two halves it needs,
separately. See [ADR 0002](0002-ui-reads-the-store-through-session-projections.md)
for the seam side.

- The port is unchanged. Every paged listing still declares one `*_page` method
  returning `Page<T>` — a total and a window under a single connection
  acquisition, carrying no generation. That property is still what the adapter
  guarantees and what its own store suite pins.
- A **count read** is a `*_page` call with a limit of **zero**: the seam asks
  for the listing's total and no rows, so learning a total costs no window. The
  window half of the answer is discarded unread, not fetched and thrown away.
  (A store adapter may later serve this with a bare `SELECT COUNT(*)`; nothing
  above the port would change.)
- A **row read** asks for one window at the window size the Session Projection
  owns. The port's own rule is unchanged and still binding: a page read takes
  an offset, never a window length it invents.
- The consequence, stated rather than concealed: a count and a window are now
  two reads, so a committed write can land between them. The guarantee that
  survives is at the seam — both reads are stamped at the same generation — not
  here. A caller relying on the old "one fact" pairing must now read them
  through the seam, which is where the generation stamping lives.
- The genre-scoped hit reads' `offset`/`limit` parameters are now advertised to
  no production caller: the two seam reads that walked those windows were
  deleted, so nothing asks for a partial window. Narrowing those signatures
  needs the `riff-infra` adapter and is not part of this change.
## Amendment — 2026-10-16 (a Custom Smart Playlist listing is a covered shape)

The ordering enumeration above is extended with one listing shape, and its
canonical `ORDER BY` is the shared `TrackListOrder` spelling rather than a new one:

- **Custom Smart Playlist listing**: a Rule tree compiled to one SQL predicate
  over `tracks`, ordered by the track sort the listing was opened with, which
  ends on `path` in every direction. The tree is ANDed with the listing's search
  needle *inside* the read — never applied after paging — and the read answers
  the window and the total as one Listing Page (`custom_smart_playlist_page`),
  under a single connection acquisition, with a count read being the same call
  with a limit of zero.

Two things this shape deliberately does **not** do, both decisions rather than
omissions. It never joins to the `albums` or `artists` entity tables: a Rule
tests the Track's own tag, so an album-grain fact is out of reach by
construction, and the column names come from a static field-id table inside the
adapter (an id absent from it is refused before any SQL is produced). And its
text matching is `lower(column)` against a Rust-lowercased bound needle rather
than the derived `search_text` substring lookup above, which is the reversal
[ADR 0020](0020-rule-matching-lowercases-at-query-time.md) records: a Rule must
be able to ask about any column, not only the four the search blob carries.

The six **Built-in Smart Playlist** reads stay outside this record. They remain
unbounded `Vec` reads with no total and no offset — a list with no per-rule cap
has nothing to page — and this shape's existence is not a reason to migrate them.
