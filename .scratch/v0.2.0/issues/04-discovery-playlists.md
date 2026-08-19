# 04 — Land offline discovery playlists (REQ-ML-009)

**What to build:** Music discovery without the internet: the library explorer offers four auto-generated playlists — Recently Added, Most Played, Never Played, and Lost Gems — that stay correct as the user plays music, and whose play counts survive restarts. The implementation already exists; verify it against the acceptance criteria, fix any gaps, and commit it.

**Blocked by:** 01 — Baseline green gate.

**Status:** ready-for-agent

- [ ] All four smart playlists appear in the library explorer and are generated entirely from local metadata (no network access)
- [ ] Finishing a track increments its play count and stamps its last-played time; both persist across restarts via the library cache
- [ ] Recently Added orders by date added, Most Played by play count, Never Played shows only unplayed tracks, Lost Gems surfaces tracks unheard for 90+ days
- [ ] Play, Play Next, and Append to Queue all work from each smart playlist
- [ ] Smart playlists are read-only (no rename/delete affordances) and never appear in search results
- [ ] Tests cover ordering rules and play-count increment; feature committed
