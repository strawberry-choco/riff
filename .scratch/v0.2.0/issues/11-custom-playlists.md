# 11 — Land custom playlist management

**What to build:** Users can build their own playlists: create a named list from tracks, play it, rename or delete it, and find it again after restart — with missing files surfaced as invalid entries instead of crashes. Playlists persist independently of the rebuildable library cache. The implementation already exists; verify it against the acceptance criteria, fix any gaps, and commit it.

**Blocked by:** 01 — Baseline green gate; 04 — Land offline discovery playlists (establishes the library-explorer integration pattern).

**Status:** done

- [ ] User can create a named playlist containing an ordered list of tracks, accessible from the library explorer alongside smart playlists
- [ ] Playlists persist in their own file, separate from the library cache — clearing the cache never destroys a playlist — and load on next launch
- [ ] A playlist can be loaded into the queue for playback
- [ ] Rename and delete work for any playlist
- [ ] Tracks whose file moved or was deleted display as invalid entries (clearly marked, not playable) without crashing
- [ ] Tests cover create/dedupe/persistence round-trip and invalid-entry handling; feature committed
