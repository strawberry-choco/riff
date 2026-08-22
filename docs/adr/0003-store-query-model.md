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