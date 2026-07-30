# 002: No System Tray on Linux

**Status**: Accepted
**Date**: 2026-07-31

## Context

The system tray icon is implemented using the `tray-icon` and `muda` crates, which provide reliable tray integration on macOS and Windows. On Linux, the tray technology depends on `libayatana-appindicator` or a compatible freedesktop notification server, neither of which is reliably available across Linux distributions.

## Decision

Linux builds shall omit the system tray entirely. There shall be no tray icon, tray menu, tooltip, or close-to-tray behavior on Linux. Linux builds run as a normal window-only application: closing the window closes the app.

This decision is enforced via conditional compilation (`#[cfg(target_os = "linux")]`) — the tray code is excluded from the Linux binary at compile time.

## Consequences

**Positive**:
- No runtime dependency on `libayatana-appindicator` or a specific desktop environment.
- No partial or broken tray behavior (missing icon, non-functional menu).
- Consistent experience on Linux: what you see is what you get.
- Simpler binary: fewer conditional branches and platform-specific error paths.

**Negative**:
- Linux users lose the "keep playing with window hidden" feature.
- No way to control playback from the desktop notification area on Linux.
- Linux users must keep the window open to play music in the background.

## Related Documents

- [Features](./features.md) — System Tray Icon feature.
- [Platform Support](../technical/platform-support.md).
- [Architecture](../technical/architecture.md) — Platform-specific code.
