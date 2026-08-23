# Spike Findings: Frameless Window Chrome (Issue 04)

**Status**: Complete — **GO**
**Date**: 2026-08-23
**Decision record**: [ADR 0005](../../adr/0005-frameless-window-chrome-on-all-platforms.md)
**Implementation**: `src/ui/chrome.rs` (custom titlebar), wired in `src/main.rs` and `src/ui/app.rs`

## Go / No-Go

**GO.** The frameless approach works end-to-end on Windows (the primary dev platform) using only egui/eframe public APIs — no platform-specific code, no new dependencies. The same single implementation ships on all three platforms per ADR 0005; macOS and Linux carry documented, bounded risk with a cheap fallback (below), not a blocker.

## What Was Validated on Windows

All checks were run against the real app (`cargo build`, launch, drive the actual window):

| Check | Method | Result |
|---|---|---|
| Launches frameless | Screenshot of the running window | No native title bar/caption buttons; custom strip renders edge-to-edge |
| Drag region present | Accessibility tree + egui interact over the full 56 px strip | Registered; wordmark left, controls right |
| Custom minimize works | Clicked the real `–` button via UI automation | Window minimized (`IsIconic = TRUE`); restores cleanly |
| Custom close works | Clicked the real `✕` button via UI automation | Close-to-tray honored (REQ-SI-001): window hides, process stays alive |

Headless contracts are pinned by tests in `tests/ui_tests.rs`: the launch viewport is undecorated with unchanged size/min-size, close routes through `ViewportCommand::Close` (the vetoable path — never a hard exit), minimize through `ViewportCommand::Minimized(true)`, and drag-region gestures decide between window-move and maximize-toggle.

## Recommended Implementation Approach (keep this)

1. **Launch config** — `chrome::viewport_builder()`: `ViewportBuilder::with_decorations(false)` carrying over the decorated window's inner/min sizes. Single source for `main.rs`.
2. **Titlebar layout** — an exact-height top panel (`theme::TITLEBAR_H`, 56 px token) with `Frame::NONE`, so the drag region covers the full strip.
3. **Drag region vs controls z-order** — register the full-strip `ui.interact(rect, id, Sense::click_and_drag())` FIRST, then draw the buttons after it inside a right-to-left scope over the same rect. Later widgets sit on top and win clicks; no manual rect exclusion needed. This is the pattern egui's own `custom_window_frame` example validates.
4. **Gestures** — primary drag-start → `ViewportCommand::StartDrag` (winit `drag_window`); double-click → toggle `Maximized`; buttons → `Minimized(true)` / `Close`.
5. **Close semantics** — always `ViewportCommand::Close`. The existing close-to-tray veto in `RiffApp::logic` then treats the custom button exactly like the native X on macOS/Windows, and quits on Linux where there is no tray.

## Platform Status & Risk

### Windows — confirmed ✅

Validated end-to-end (table above). Resize borders still work on the undecorated window because winit implements invisible resize hit-testing for resizable undecorated windows; snapping/Aero behavior follows the OS.

### macOS — risk documented, pending hands-on confirmation ⚠️

- The mechanism is portable: `decorations(false)` + `StartDrag` map to winit's `drag_window`, which is supported on macOS.
- Risks: traffic-light conventions disappear (users may expect them); Cmd+Q and the dock close path already flow through the same close-requested veto, so close-to-tray semantics should hold; fullscreen zoom button behavior differs from Windows maximize.
- Confirmation needed before Phase 1 signs off: drag, minimize/close clicks, and resize edges on a real Mac.
- **Fallback if it fails**: gate `with_decorations(false)` behind `#[cfg(not(target_os = "macos"))]` and keep native chrome there — one cfg, no architectural change, since all chrome rendering lives in one module.

### Linux — risk documented, pending hands-on confirmation ⚠️

- Mechanism is supported: winit implements `drag_window` on both X11 and Wayland, and client-side decorations are the norm under Wayland compositors.
- Risks: resize affordances and maximize snapping vary by window manager; some X11 tiling WMs ignore undecorated hints; there is no tray on Linux already, so the custom close button genuinely quits (consistent with today's native-X behavior).
- Confirmation needed before Phase 1 signs off: drag, close, and resize under GNOME (Wayland) and a representative X11 WM.
- **Fallback if it fails**: same one-cfg gate as macOS (`#[cfg(not(target_os = "linux"))]`), matching the existing tray/picker platform split.

## Consequences Carried Into Phase 1

- Drag regions, resize handles, and maximize snapping are riff's responsibility on every target (ADR 0005).
- Issue 06 integrates these controls into the unified 56 px titlebar; the spike's strip is deliberately minimal (default-styled buttons, duplicated wordmark) and will be restyled there.
