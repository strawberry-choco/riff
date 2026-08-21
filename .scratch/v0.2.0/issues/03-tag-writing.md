# 03 — Land metadata tag writing (REQ-ML-008)

**What to build:** End-to-end tag editing: a user right-clicks a track, chooses Edit Tags, adjusts values in a modal, saves, and the audio file's tags on disk are updated without freezing the UI — and the library reflects the change immediately. The implementation already exists; verify it against the acceptance criteria, fix any gaps, and commit it.

**Blocked by:** 01 — Baseline green gate.

**Status:** done

- [ ] Right-click context menu on a track offers "Edit Tags" wherever tracks are listed
- [ ] The modal opens pre-filled with the track's current title, artist, album, album artist, genre, year, and track number
- [ ] Save writes tags to the file on a background thread — the UI never blocks on the write
- [ ] After a successful write, the library cache updates immediately without a rescan; re-reading the file confirms it is the source of truth
- [ ] Nothing is written without an explicit Save; Cancel and Escape close the modal unchanged
- [ ] Write failures (permission denied, disk full, unsupported format) produce a graceful, user-visible error and the app keeps working
- [ ] Tests cover the write path including at least one error case; feature committed

