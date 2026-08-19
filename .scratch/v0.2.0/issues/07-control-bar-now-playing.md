# 07 — Land player control bar + Now Playing view (REQ-UI-003, REQ-UI-005)

**What to build:** The two most-touched playback surfaces finished to spec: a control bar with working mute that restores the previous volume, and a Now Playing view where you can click an upcoming track to play it next and scrub through the current one on an in-view progress bar. Both already exist in the working tree; verify against the acceptance criteria, fix any gaps, and commit.

**Blocked by:** 01 — Baseline green gate.

**Status:** ready-for-agent

- [ ] Mute toggle sits in the control bar: muting silences output, unmuting restores the exact previous volume, and mute state survives track changes without fighting the volume slider
- [ ] The Now Playing view shows large cover art and full metadata (album artist, year, genre, track number)
- [ ] Clicking an upcoming-queue row queues that track as the next track (Play Next), not as a jump-to
- [ ] The in-view progress bar shows elapsed/total in MM:SS, updates continuously during playback, and clicking it seeks to that position; seeking past the end clamps to the end
- [ ] The view handles the nothing-playing state gracefully
- [ ] Tests cover mute/restore and seek clamping; feature committed
