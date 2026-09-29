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
  track and every album and filter in Rust, with no offset-and-total twin at all.
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
- The consequence, stated rather than papered over: a count and a window are now
  two reads, so a committed write can land between them. The guarantee that
  survives is at the seam — both reads are stamped at the same generation — not
  here. A caller relying on the old "one fact" pairing must now read them
  through the seam, which is where the generation stamping lives.
- The genre-scoped hit reads' `offset`/`limit` parameters are now advertised to
  no production caller: the two seam reads that walked those windows were
  deleted, so nothing asks for a partial window. Narrowing those signatures
  needs the `riff-infra` adapter and is not part of this change.