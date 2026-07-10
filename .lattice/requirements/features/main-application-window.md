---
feature: Main Application Window
epic: User Interface
status: partial
priority: P0
depends_on: []
personas: []
source_docs: []
implementation_gaps: |
  Done: Basic eframe window with minimum size (800x600), three-panel layout.
  Missing: Window position persistence, theme support (light/dark toggle),
  window title showing current track info, keyboard shortcuts.
---

# Main Application Window

## Problem Statement

The player needs a graphical user interface that works on Linux, Windows, and macOS with minimal external dependencies. The UI framework must compile to a single binary with zero runtime dependencies (no Qt, no GTK, no web engine). It must provide a responsive, native-feeling experience for browsing and playing music.

## User / Personas

**Cross-platform User**: A user who switches between Linux at home and macOS at work and wants the same music player with the same interface on both.

**Minimalist User**: A user who dislikes heavy applications with long startup times. They want a lightweight player that opens instantly.

## Scope

**In scope:**
- Single-window application with a main panel layout
- egui as the immediate-mode GUI framework (pure Rust, minimal deps)
- Cross-platform window creation and management via winit/eframe
- Resizable window with minimum dimensions (800x600)
- Native window decorations (close, minimize, maximize buttons)
- Keyboard focus management between panels
- Light and dark theme support (follow system theme or manual toggle)
- Window title shows current track info when playing

**Out of scope:**
- Multiple independent windows (playlists in separate windows, etc.)
- Full-screen or kiosk mode
- Custom window chrome / borderless window
- HiDPI / fractional scaling issues are handled by egui automatically
- Touch/gesture support (mouse and keyboard only in MVP)

## Boundary Conditions

- Window must not be smaller than 800x600
- Application must handle window close event gracefully (save state, stop audio)
- Application must handle display scale changes (moving between monitors with different DPI)
- Window position and size should be persisted across sessions
- Application must remain responsive during heavy operations (library scan, cover loading) via background threads

## Assumptions

- `egui` + `eframe` provides sufficient UI primitives for all required player controls (buttons, sliders, lists, trees, images)
- Users are comfortable with an immediate-mode GUI aesthetic (slightly different from native widgets but functional)
- The application starts in a reasonable time (<2 seconds) even with a large library

## Scenarios

### Scenario 1: Launch the application
A user opens the music player for the first time.

**Acceptance Criteria:**
- Given the application binary is executed, when it launches, then a window appears within 2 seconds
- Given the application launches, when the window appears, then it has the correct title, minimum size, and default layout
- Given the application launches for the first time, when the window appears, then it starts at a default centered position on the primary monitor
- Given the application has been run before, when it launches, then the window restores to its previous position, size, and maximized state

### Scenario 2: Resize the window
A user resizes the application window.

**Acceptance Criteria:**
- Given the window is at any size >=800x600, when the user resizes it, then all panels reflow appropriately without clipping or overlap
- Given the window is resized to be smaller than 800x600, when the resize is attempted, then the window is clamped to the minimum size
- Given the window is resized, when the resize completes, then the new size is saved for the next session

### Scenario 3: Close the window
A user closes the application.

**Acceptance Criteria:**
- Given the application is running, when the user clicks the window close button, then the application saves its state (window geometry, current queue position, volume) and exits cleanly
- Given audio is playing, when the user closes the window, then audio output stops and the audio device is released before the process exits
- Given the library scan is running in the background, when the user closes the window, then the scan is gracefully cancelled and partial results are saved

## Implementation Notes

1. **Framework**: Use `egui` for all UI rendering and `eframe` for the application framework (event loop, window management, persistence). Eframe handles the winit boilerplate and provides `NativeOptions` for window configuration.
2. **Window persistence**: Use `eframe`'s built-in `Storage` mechanism to save/restore window position, size, and maximized state across sessions.
3. **Theme**: Implement a `Theme` enum (Light, Dark, System). Use `egui::Style` to configure colors. The system theme can be detected via `dark-light` crate or egui's built-in detection.
4. **Layout**: Use a three-panel layout:
   - Left sidebar: Library explorer (collapsible)
   - Center panel: Content area (track list, album grid, search results)
   - Bottom bar: Player controls (transport, progress, volume)
   - Top right: Cover art display (can be a floating panel or part of center)
5. **Threading**: Run the library scanner, cover art loader, and audio decoder on background threads. Use `std::sync::mpsc` or `crossbeam` channels to communicate with the UI thread.

## Open Questions

- [ ] Should we support a compact/minimal player mode (just controls + cover) for small screens? (Non-blocking: can be a future enhancement)
- [ ] Do we need to handle monitor disconnection gracefully? (Non-blocking: eframe/winit handles most cases)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
