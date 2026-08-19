# 08 — Land system tray completion + cover art polish (REQ-SI-001, REQ-UI-004)

**What to build:** riff becomes a proper background companion on Windows/macOS — closing the window tucks it into the tray with playback uninterrupted — and cover art looks consistent everywhere it appears. Both already exist in the working tree; verify against the acceptance criteria, fix any gaps, and commit.

**Blocked by:** 01 — Baseline green gate.

**Status:** ready-for-agent

- [ ] On macOS/Windows, closing the main window minimizes to the tray instead of quitting, and playback continues while hidden
- [ ] Tray tooltip shows "Artist - Title" of the current track; left-click toggles window visibility
- [ ] Tray right-click menu offers Play/Pause, Next Track, Previous Track, Show Window, and Quit — and Quit stops playback and exits the app
- [ ] On Linux, closing the window quits the app (no tray), with no leftover tray code paths active
- [ ] Cover art displays at consistent sizes in the library and Now Playing views through the same LRU-backed pipeline, shows a placeholder when no art exists, and oversized images are clamped rather than breaking layout
- [ ] Feature committed
