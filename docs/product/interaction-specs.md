# Interaction Specifications

This document specifies the behavior of every user-facing widget and interaction in riff. It is the reference for UI implementation PRs: each statement defines what happens when a user interacts with a specific element.

---

## 1. Main Window

| Property | Value |
|----------|-------|
| Framework | egui / eframe (immediate-mode UI) |
| Resizable | Yes |
| Minimum size | Defined by the content (library panel + control bar); shall not shrink below the space needed for core controls. |
| Position | Persisted between sessions. |
| Size | Persisted between sessions. |
| Title bar | Standard platform title bar. |
| Title | "riff" (or application name). |
| Close behavior (macOS/Windows) | Minimizes to system tray. Playback continues. |
| Close behavior (Linux) | Quits the application. Playback stops. |
| Window icon | Standard application icon (platform-default from binary metadata). |

---

## 2. Top Bar

| Element | Behavior |
|---------|----------|
| Gear/settings icon | Click opens the settings panel. |
| Window controls | Standard platform close/minimize buttons. Close behavior depends on platform (see Main Window). |

---

## 3. Library Explorer — View Toggle

| Property | Value |
|----------|-------|
| Toggle | Switches between Library view and Folders view. |
| Default on first launch | Library view. |
| Selection memory | Each view remembers its own selection. Switching views and back restores the previous selection in each view. |
| Visual indicator | The active view is visually distinguished (e.g. highlighted toggle or tab). |

---

## 4. Library Explorer — Library View

| Interaction | Behavior |
|-------------|----------|
| Click on artist name | Expands to show the artist's albums. |
| Click on expanded artist | Collapses back to artist level. |
| Click on album name | Expands to show the album's tracks. |
| Click on expanded album | Collapses back to album level. |
| Double-click on a track | Starts playback. The queue is replaced with the surrounding album context when available. |
| Right-click on a track | Context menu with Play, Play Next, Append to Queue. |
| Right-click on an album | Context menu with Play (plays entire album), Play Next, Append to Queue. |
| Currently playing track | Visually marked (highlighted, icon, or label) in the tree. |

**Sorting rules**:
- Albums grouped by album artist.
- Albums sorted by year within each artist.
- Tracks sorted by track number within each album.

---

## 5. Library Explorer — Folders View

| Interaction | Behavior |
|-------------|----------|
| Click on a library path node | Expands to show top-level subdirectories that contain audio. |
| Click on a subdirectory | Expands children on demand (lazy loading). |
| Click on expanded node | Collapses children. |
| Double-click on a folder | Replaces the queue with every track in that folder and its subfolders. Starts playing from the top. |
| Double-click on a track | Starts playback of that track. |
| Right-click on a folder | Context menu with Play, Play Next, Append to Queue. |
| Right-click on a track | Context menu with Play, Play Next, Append to Queue. |
| Currently playing track | Visually marked. |

**Tree behavior**:
- Only directories that actually contain audio files are shown.
- Children load on demand (lazy loading), so large trees expand quickly.

---

## 6. Search Box

| Property | Value |
|----------|-------|
| Location | Library explorer panel (above the tree). |
| Placeholder text | "Search..." or equivalent. |
| Clear control | A button or icon that clears the query. |
| Filtering behavior | Real-time filtering as the user types. |
| Library view filter | Matches artist, album artist, album, and title. |
| Folders view filter | Prunes the tree to folders containing matching tracks. |
| Selection on filter | The currently selected item is preserved if it matches the filter; otherwise the selection is cleared. |
| Selection on clear | The previous selection (before the filter was applied) is restored. |

---

## 7. Settings Panel

| Element | Behavior |
|---------|----------|
| Library path list | Shows all registered library paths, each as a row. |
| Path row display | Path, status indicator, Scan button, Watch toggle, Delete control. |
| Add Library button | Click opens the native folder picker (macOS/Windows) or text input (Linux). |
| Adding an existing path | No-op — no duplicate entry is created. |
| Removing a path | The path is removed from the list. Index entries for that path are deleted. Files on disk are never deleted. |
| Confirmation on remove | No confirmation dialog — removal is immediate. (Design choice; may change.) |
| Scan button (per path) | Triggers a scan of that single library path. |
| Scan All button | Triggers a scan of all registered library paths. |
| Watch toggle | Three states: On, Off, Warning (with explanation tooltip). |

---

## 8. Player Control Bar

| Element | Behavior |
|---------|----------|
| Previous button | Press: if track has played > few seconds, restart current track. Otherwise, jump to prior track. |
| Play/Pause button | Press: toggle between playing and paused states. |
| Stop button | Press (planned): stop playback, reset position to zero. |
| Next button | Press: advance to next track in queue. |
| Progress bar | Click: seek to clicked position. Elapsed/total time shown as MM:SS / MM:SS. |
| Volume slider | Drag: adjust volume 0–100%. |
| Mute button | Press: mute (volume → 0, remember previous level). Press again: unmute (restore previous level). |
| Shuffle toggle | Press: toggle shuffle on/off. Visual state reflects current setting. |
| Repeat toggle | Press: cycle through no repeat → repeat all → repeat one. Visual state reflects current setting. |
| Queue position indicator | Shows "current / total" (e.g. "3 / 42"). |

---

## 9. Now Playing View

| Element | Behavior |
|---------|----------|
| Toggle button | Press: open/close the Now Playing view. |
| Large cover art | Displays current track's cover at large size. (Planned.) |
| Metadata labels | Display title, artist, album, album artist, year, genre, track number. Missing fields are hidden. |
| Upcoming queue list | Shows tracks after the current one in the queue. |
| Click on queue entry | Plays that track next. (Planned.) |
| Progress bar | Click to seek. Updates continuously. (Planned.) |

---

## 10. System Tray (macOS/Windows only)

| Interaction | Behavior |
|-------------|----------|
| Hover | Tooltip shows "Artist - Title". |
| Left-click | Toggle main window visibility. |
| Right-click | Open context menu. |
| Menu: Play/Pause | Toggle playback. |
| Menu: Next Track | Advance to next track. |
| Menu: Previous Track | Return to previous track. |
| Menu: Show Window | Show the main window. |
| Menu: Quit | Stop playback, exit application. |

**Linux**: No tray icon. Close window = quit app.
