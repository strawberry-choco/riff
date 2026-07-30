# System Tray — Specification

## Feature

**System Tray Icon** — on macOS and Windows, a system tray icon that allows playback control without opening the main window.

**Status**: Partial. Tray icon, tooltip, left-click window toggle, and right-click menu are implemented. Configurable close-to-tray behavior is unfinished.

---

## Overview

On macOS and Windows, riff runs with a system tray icon so that playback continues with the main window hidden. The tray provides a minimal playback interface: a tooltip showing the current track, left-click to toggle the window, and a right-click context menu with transport and application controls. On Linux, the system tray does not exist — riff runs as a normal window-only application.

---

## Components

| Component | Description |
|-----------|-------------|
| Tray icon | Small icon in the system tray / notification area. |
| Tooltip | Hover text showing the current track. |
| Left-click action | Toggles main window visibility. |
| Right-click context menu | Menu with playback and application controls. |
| Menu: Play/Pause | Toggles playback. |
| Menu: Next Track | Advances to the next track. |
| Menu: Previous Track | Returns to the previous track. |
| Menu: Show Window | Shows the main window. |
| Menu: Quit | Stops playback and exits the application. |

---

## Behavior

### Tray Presence

1. On macOS, riff shall show a system tray icon.
2. On Windows, riff shall show a system tray icon.
3. On Linux, riff shall not show a system tray icon.
4. The tray icon shall be present from the moment the application starts.

### Tooltip

5. The tray icon's tooltip shall show the current track in the format "Artist - Title".
6. If no track is playing, the tooltip shall show "riff" or an appropriate default (e.g. "No track playing").
7. The tooltip shall update when the current track changes.

### Left-Click

8. Left-clicking the tray icon shall toggle the main window:
   - If the window is hidden, left-click shall show it.
   - If the window is visible, left-click shall hide it.
9. Left-clicking shall not affect playback state (does not pause or stop).

### Right-Click Context Menu

10. Right-clicking the tray icon shall show a context menu.
11. The menu shall contain a Play/Pause entry that toggles playback.
12. The menu shall contain a Next Track entry that advances to the next track.
13. The menu shall contain a Previous Track entry that returns to the previous track.
14. The menu shall contain a Show Window entry that shows the main window.
15. The menu shall contain a Quit entry that stops playback and exits the application.

### Close-to-Tray

16. On macOS and Windows, closing the main window (clicking the close button) shall minimize the app to the tray rather than quitting.
17. Playback shall continue with the window hidden.
18. The configurable close-to-tray behavior (e.g. an option to always quit on close) is planned but not yet implemented.

### Quit from Tray

19. Selecting Quit from the tray menu shall stop playback.
20. Selecting Quit from the tray menu shall exit the application.
21. No confirmation dialog shall be shown on quit.

---

## States

| State | Description |
|-------|-------------|
| Tray active — Playing | Icon shown. Tooltip shows "Artist - Title". Menu available. |
| Tray active — Paused | Icon shown. Tooltip shows "Artist - Title". Menu available. |
| Tray active — No track | Icon shown. Tooltip shows default text. Menu available. |
| Window hidden | Main window is not visible. Tray is the only visible interface. |
| Window visible | Both main window and tray icon are visible. |
| Quitted | Icon removed from tray. Application process terminated. |

---

## Edge Cases

- **Track changes while tray is active**: Tooltip updates immediately.
- **Playback ends (queue exhausted)**: Tray icon remains; tooltip reflects "no track" state.
- **App launched with tray icon already present**: A single tray icon is shown (no duplicates).
- **Multiple tray icons**: The system shall not show duplicate tray icons for a single running instance.

---

## Empty States

- **No track playing**: Tooltip shows "No track playing" or "riff." Context menu still available with transport controls (Play/Pause is a no-op when nothing is loaded).

---

## Error States

- **Tray icon not available**: On Linux, no tray icon is shown. The app runs normally without one. No error is raised.
- **Tooltip cannot update**: Fallback to "No track playing." Logged at WARN level.

---

## Platform Differences

| Feature | macOS | Windows | Linux |
|---------|-------|---------|-------|
| Tray icon | Yes | Yes | No |
| Tooltip | "Artist - Title" | "Artist - Title" | N/A |
| Left-click toggle window | Yes | Yes | N/A |
| Right-click context menu | Yes | Yes | N/A |
| Close window → tray | Yes | Yes | No (close = quit) |
| Tray technology | tray-icon + muda | tray-icon + muda | N/A |

---

## Out of Scope

- Configurable close-to-tray behavior (always quit vs. always minimize). (Planned.)
- Tray icon theme selection (light/dark).
- Notification popups from the tray.
- Media session integration (OS-level media controls in lock screen / notification center).

---

## Verification Checklist

- [ ] Tray icon is visible on macOS.
- [ ] Tray icon is visible on Windows.
- [ ] No tray icon on Linux.
- [ ] Tooltip shows "Artist - Title" for the current track.
- [ ] Tooltip shows default text when no track is playing.
- [ ] Tooltip updates when track changes.
- [ ] Left-click toggles window visibility.
- [ ] Left-click does not affect playback state.
- [ ] Right-click shows context menu with all five entries.
- [ ] Play/Pause menu entry toggles playback.
- [ ] Next Track menu entry advances playback.
- [ ] Previous Track menu entry returns to prior track.
- [ ] Show Window menu entry shows the main window.
- [ ] Quit menu entry stops playback and exits the app.
- [ ] Closing the main window minimizes to tray (macOS/Windows).
- [ ] Closing the main window quits the app (Linux).
- [ ] No duplicate tray icons appear on launch.
