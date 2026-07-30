# Now Playing View — Specification

## Feature

**Now Playing View** — a focused panel for the current track showing title, artist, album, large cover art, full metadata, the upcoming queue, and an in-view progress bar.

**Status**: Partial. Basic track info (title, artist, album) and upcoming-queue list are implemented. Large cover art, full metadata fields, clickable up-next rows, and in-view progress bar are planned.

---

## Overview

The Now Playing view is a toggleable panel that shifts focus from browsing to listening. It displays the current track prominently with its cover art and metadata, and shows the upcoming tracks in the queue. The view complements — rather than replaces — the library explorer and control bar.

---

## Components

| Component | Description |
|-----------|-------------|
| Toggle button | Opens/closes the Now Playing view. |
| Large cover art | Displays the current track's cover art at large size. |
| Title label | Displays the current track's title. |
| Artist label | Displays the current track's artist. |
| Album label | Displays the current track's album. |
| Album artist label | Displays the current track's album artist. (Planned.) |
| Year label | Displays the current track's year. (Planned.) |
| Genre label | Displays the current track's genre. (Planned.) |
| Track number label | Displays the current track's track number. (Planned.) |
| Upcoming queue list | Displays the tracks that follow the current one in the queue. |
| Progress bar | In-view seekable progress bar for the current track. (Planned.) |

---

## Behavior

### View Toggle

1. The user shall be able to open the Now Playing view with a toggle (button or menu item).
2. The user shall be able to close the Now Playing view with the same toggle.
3. The view shall remember its open/close state between sessions (optional; if implemented, the default on first launch shall be closed).

### Track Display

4. The view shall display the current track's title.
5. The view shall display the current track's artist.
6. The view shall display the current track's album.
7. The view shall display the current track's album artist.
8. The view shall display the current track's year.
9. The view shall display the current track's genre.
10. The view shall display the current track's track number.
11. Fields that do not exist for the current track shall not be displayed (no "Unknown" placeholders).
12. The displayed metadata shall update when the current track changes (track transition).

### Large Cover Art

13. The view shall display the current track's cover art at a large size, significantly larger than the cover art shown in the library explorer.
14. Cover art shall use the same resolution pipeline as the rest of the application (embedded > filesystem fallback > placeholder).
15. If no cover art is available, a placeholder shall be shown.

### Upcoming Queue

16. The view shall display the tracks that follow the current track in the queue.
17. Each queue entry shall show at minimum the track title and artist.
18. Clicking a queue entry shall play that track next (i.e. insert it as the current track or move it to the top of the remaining queue).

### In-View Progress Bar

19. The view shall include a seekable progress bar for the current track.
20. Clicking the progress bar shall seek to that position in the current track.
21. Seeking past the end shall clamp to the end.
22. The progress bar shall update continuously during playback.

---

## States

| State | Description |
|-------|-------------|
| Closed | View is not visible. Toggle button available to open. |
| Open — Track playing | View visible. Metadata and cover art showing. Progress advancing. |
| Open — Track paused | View visible. Metadata and cover art showing. Progress frozen. |
| Open — No track | View visible. Empty state shown (e.g. "Nothing playing"). |
| Open — Cover missing | View visible. Placeholder shown instead of cover art. |

---

## Edge Cases

- **Track changes during viewing**: Metadata, cover art, and upcoming queue shall update immediately.
- **Queue cleared while viewing**: Upcoming queue list shall become empty; metadata still shows the last-played track until a new track starts.
- **Cover art becomes unavailable** (e.g. file deleted): Fall back to placeholder.
- **Very long track titles or album names**: Truncate gracefully; do not break layout.

---

## Empty States

- **No track playing**: The view shall show "Nothing playing" or equivalent text. Cover art area shows placeholder. Progress bar disabled.
- **Queue empty but track playing**: Upcoming queue list shows "No upcoming tracks." Metadata and cover art still display.

---

## Error States

- **Cover art decode failure**: Placeholder shown. No error message in the view (logged at WARN level).
- **Metadata read failure**: Fields that failed to read are simply not displayed. The view does not error out.

---

## Keyboard / Input

- **Click on cover art**: Recommended behavior — advances to next track or opens a larger cover view. (Not yet specified.)
- **Click on queue entry**: Plays that track next.

---

## Platform Differences

- No platform-specific behavior. The Now Playing view is identical across Linux, Windows, and macOS.

---

## Out of Scope

- Lyrics display.
- Related tracks or recommendations.
- Audio visualizations.
- Per-track EQ or effects controls.

---

## Verification Checklist

- [ ] View can be opened and closed with the toggle.
- [ ] Title, artist, and album are displayed for the current track.
- [ ] Album artist, year, genre, and track number are displayed when available. (Planned.)
- [ ] Missing metadata fields are hidden, not shown as "Unknown."
- [ ] Large cover art displays correctly and uses the same resolution pipeline. (Planned.)
- [ ] Placeholder is shown when no cover art is available. (Planned.)
- [ ] Metadata updates when the current track changes.
- [ ] Upcoming queue list shows tracks after the current one.
- [ ] Clicking a queue entry plays that track next. (Planned.)
- [ ] In-view progress bar is seekable and updates continuously. (Planned.)
- [ ] Empty state ("Nothing playing") is shown when no track is playing.
- [ ] Queue empty state is handled gracefully.
