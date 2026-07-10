---
feature: System Tray Icon
epic: System Integration
status: partial
priority: P1
depends_on: [Main Application Window]
personas: []
source_docs: []
implementation_notes: |
  Implemented for macOS and Windows (not Linux). Uses tray-icon + muda crates.
  Gated behind `#[cfg(not(target_os = "linux"))]`. On Linux, tray is absent;
  tray-icon requires libayatana-appindicator which isn't universally available.
---

# System Tray Icon

## Problem Statement

Users want the music player to continue running in the background without taking up taskbar/dock space. A system tray icon allows the application to run minimized, provide quick playback control via a context menu, and be restored to the foreground when needed.

## User / Personas

**Background Listener**: A user who listens to music while working and doesn't want a window cluttering their workspace. They control playback via global shortcuts or the tray menu.

## Scope

**In scope:**
- System tray icon visible when the application is running
- Tray icon tooltip showing current track ("Artist - Title")
- Left-click tray icon: toggle window visibility (hide/show)
- Right-click tray icon: context menu with options:
  - Play/Pause
  - Next Track
  - Previous Track
  - Show Window
  - Quit
- Window close button minimizes to tray (default behavior, configurable in settings)
- Tray icon persists when window is hidden
- Tray icon removed when application quits

**Out of scope:**
- Custom tray icon animation (e.g., bouncing to the beat)
- Tray icon showing album cover thumbnail
- Balloon/toast notifications on track change
- Media key handling (separate feature, not in MVP)
- macOS menu bar extra (use standard tray for cross-platform consistency)

## Boundary Conditions

- Tray icon must work on Linux (X11 and Wayland via StatusNotifier/AppIndicator), Windows (System Tray), and macOS (Menu Bar / NSStatusItem)
- If the system tray is not available (rare on modern desktops), the application falls back to running as a normal window-only app
- The tray icon must not prevent the application from quitting (no zombie processes)
- Tray menu must be localized (English only for MVP)

## Assumptions

- The `tray-icon` crate provides cross-platform tray functionality in pure Rust
- Users expect the close button to minimize to tray (standard behavior for music players)
- The system tray is available on all target platforms (it is on modern Linux, Windows, and macOS)

## Scenarios

### Scenario 1: Minimize to tray
A user closes the window but wants the music to keep playing.

**Acceptance Criteria:**
- Given the "close to tray" setting is enabled (default), when the user clicks the window close button, then the window hides but music continues playing and a tray icon appears
- Given the application is in the tray, when the user looks at the system tray, then the icon is visible with a tooltip showing the current track

### Scenario 2: Restore from tray
A user wants to bring the player window back.

**Acceptance Criteria:**
- Given the application is minimized to the tray, when the user left-clicks the tray icon, then the window is restored to its previous position and size
- Given the application is in the tray, when the user selects "Show Window" from the right-click menu, then the window is restored

### Scenario 3: Control playback from tray
A user wants to skip a track without opening the window.

**Acceptance Criteria:**
- Given the application is in the tray, when the user right-clicks and selects "Next Track", then playback advances to the next track
- Given the application is in the tray, when the user right-clicks and selects "Play/Pause", then playback state toggles

### Scenario 4: Quit from tray
A user wants to fully close the application.

**Acceptance Criteria:**
- Given the application is in the tray, when the user right-clicks and selects "Quit", then the application stops playback, removes the tray icon, and exits cleanly
- Given the application is running with a window visible, when the user selects "Quit" from the tray menu, then the application exits cleanly

## Implementation Notes

1. **tray-icon crate**: Use the `tray-icon` crate which wraps platform-specific tray APIs. On Linux, it uses `libappindicator` or `StatusNotifierItem` via `libayatana-appindicator` (or the older `libappindicator`).
2. **Icon**: Embed a default application icon as bytes in the binary (e.g., a simple 32x32 PNG). Load it via `tray_icon::icon::Icon::from_rgba()` or `tray_icon::icon::Icon::from_file()`.
3. **Event loop integration**: The tray icon runs its own event loop or integrates with `winit`'s event loop. Use `winit`'s `EventLoopProxy` to send tray events to the main application thread.
4. **Window visibility toggle**: Maintain the `window_visible` state. On left-click, toggle it by calling `window.set_visible(!window_visible)`.
5. **Menu items**: Use `tray_icon::menu` or `muda` crate for the context menu. Connect menu item clicks to the playback engine via channels.
6. **Linux note**: On Arch Linux (and most modern Linux), `libayatana-appindicator3-1` is the modern replacement for the deprecated `libappindicator`. The `tray-icon` crate may need `libayatana-appindicator` feature enabled. Ensure the application gracefully degrades if neither library is available (fallback to window-only mode).

## Open Questions

- [ ] Should we include a "Preferences" item in the tray menu? (Non-blocking: defer to settings dialog)
- [ ] Should we support Linux StatusNotifierItem directly or rely on `tray-icon`'s abstraction? (Non-blocking: use `tray-icon` abstraction)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
