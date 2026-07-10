---
feature: Cross-platform Support
epic: System Integration
status: implemented
priority: P0
depends_on: [Main Application Window]
personas: []
source_docs: []
implementation_notes: |
  Uses eframe (winit) for cross-platform window creation, cpal for audio output
  on Linux (ALSA/PulseAudio/PipeWire), Windows (WASAPI), macOS (CoreAudio).
  Platform-specific code is minimized; tray-icon is macOS/Windows only.
---

# Cross-platform Support

## Problem Statement

The music player must run natively on Linux (including Arch Linux), Windows, and macOS with minimal platform-specific code. The goal is a single codebase that compiles and runs identically across all three platforms, with only environment-specific integrations (audio output, tray icon, file paths) abstracted.

## User / Personas

**Multi-platform User**: A developer or power user who uses Linux at home, Windows at work, and occasionally macOS. They want the same music player experience everywhere.

## Scope

**In scope:**
- Linux support (X11 and Wayland display servers)
- Windows support (Windows 10 and 11)
- macOS support (macOS 12 Monterey and later)
- Audio output via cpal on all platforms
- System tray via tray-icon on all platforms
- File path handling using `std::path::PathBuf` (cross-platform)
- Config directory using `directories` crate (cross-platform: ~/.config/riff on Linux, %APPDATA%\riff on Windows, ~/Library/Application Support/riff on macOS)
- Single binary compilation target per platform

**Out of scope:**
- Mobile platforms (iOS, Android)
- WebAssembly / browser version
- Linux-specific audio backends directly (ALSA, PulseAudio, JACK) — cpal abstracts these
- Windows-specific audio backends (WASAPI exclusive mode, ASIO)
- macOS-specific audio backends (Core Audio directly)

## Boundary Conditions

- The application must compile on all three platforms from the same source code
- Platform-specific features must be behind cfg flags and gracefully degrade if unavailable
- Audio must work on the default device without manual configuration on all platforms
- The application must respect platform conventions (config directory locations, window behavior, etc.)
- Linux: must work on both X11 and Wayland without code changes (handled by winit/cpal)
- Windows: must not require additional runtime DLLs beyond what Windows provides
- macOS: must produce a .app bundle for distribution (build script, not runtime requirement)

## Assumptions

- `cpal` supports all target platforms reliably
- `eframe` (winit) supports X11, Wayland, Windows, and macOS
- `tray-icon` supports all three platforms
- Users on each platform have the standard development/runtime libraries available
- Arch Linux users can install the required system libraries (libayatana-appindicator3-1, etc.) via pacman

## Scenarios

### Scenario 1: Launch on Linux (Arch)
An Arch Linux user launches the application.

**Acceptance Criteria:**
- Given the application binary is executed on Arch Linux with X11 or Wayland, when it launches, then a window appears and audio playback works through the default PulseAudio/PipeWire device
- Given the application is running on Linux, when the user minimizes to tray, then a tray icon appears in the system tray/StatusNotifier area

### Scenario 2: Launch on Windows
A Windows user launches the application.

**Acceptance Criteria:**
- Given the application binary is executed on Windows 10/11, when it launches, then a window appears and audio playback works through the default audio output device
- Given the application is running on Windows, when the user minimizes to tray, then a tray icon appears in the system tray

### Scenario 3: Launch on macOS
A macOS user launches the application.

**Acceptance Criteria:**
- Given the application binary is executed on macOS 12+, when it launches, then a window appears and audio playback works through the default Core Audio device
- Given the application is running on macOS, when the user minimizes to tray, then a tray icon appears in the menu bar

## Implementation Notes

1. **Platform abstraction**: Keep platform-specific code minimal. Use cfg flags only for:
   - Tray icon initialization (slightly different on macOS vs Linux/Windows)
   - Config directory resolution (handled by `directories` crate)
   - Audio device enumeration edge cases (cpal handles most)
2. **Build configuration**: Use `Cargo.toml` features to conditionally compile platform-specific dependencies. For example, `tray-icon` might have different backend features.
3. **Linux note**: On Arch Linux, ensure the PKGBUILD or install instructions mention `libayatana-appindicator3-1` as an optional dependency for tray support. The application should compile and run without it (tray disabled).
4. **CI/CD**: Set up GitHub Actions with matrix builds for ubuntu-latest, windows-latest, and macos-latest to ensure cross-platform compilation.
5. **Testing**: Test audio output on real hardware for each platform. cpal is generally reliable but occasionally has platform-specific quirks (e.g., device enumeration on Linux).

## Open Questions

- [ ] Do we need to handle HiDPI scaling differently on each platform? (Non-blocking: egui/eframe handles this automatically)
- [ ] Should we provide platform-specific packages (.deb, .rpm, .msi, .dmg) or just raw binaries? (Non-blocking: raw binaries for MVP, packages for releases)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
