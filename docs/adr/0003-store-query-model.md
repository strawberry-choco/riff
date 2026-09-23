# Store Query Model

**Status**: Accepted
**Date**: 2026-08-22

Views read through bounded Session Projection queries rather than whole-library snapshots. A query signature identifies mode/filter/sort/generation; each projection caches its total count and only the currently visible row windows (`LIMIT/OFFSET`) until invalidated by the session-local Store generation counter. If very large libraries make deep offsets slow in practice, keyset pagination is a targeted follow-up, not part of v1.

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
  The flat list, search, the two entity hit listings, the three browse roots, and
  the two genre drill-downs each read one **Listing Page** (`Page<T>` on the
  Application Store query port): total and window under a single connection
  acquisition, stamped with the generation captured inside it. "each projection
  caches its total count and only the currently visible row windows" describes
  what a projection *holds*, not what the store answers with. See
  [ADR 0002](0002-ui-reads-the-store-through-session-projections.md) for the
  staleness side.
- The genre-scoped hit reads (`hit_albums_in_genre`, `hit_artists_in_genre`,
  `album_hit_tracks_in_genre`, `hit_genre_counts`) are **not** covered by this
  record's bounded-window description. They materialise every genre-bearing
  track and every album and filter in Rust, with no offset-and-total twin at all;
  the `hit_albums_in_genre`/`hit_artists_in_genre` windows are the only survivors
  of the window-without-a-page shape. Making them bounded is a separately tracked
  query optimisation with its own result-set semantics, deliberately not part of
  the Listing Page change.