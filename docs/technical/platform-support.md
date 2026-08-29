# Platform Support

riff targets the three major desktop platforms — macOS, Windows, and Linux — from a single codebase. Most of the application is platform-independent: the persistence contract, the capability slices, and the adapter/API crates behave identically everywhere, and audio output is abstracted by cpal in `riff-infra`. The platform differences are confined to a few frontend integrations — the system tray and the folder picker — which are conditionally compiled, and to the OS-specific location of the Application Store. This document captures the feature matrix, the conditional-compilation strategy, and the platform-specific behaviors worth knowing.

For the crates behind each feature, see [./dependencies.md](./dependencies.md). For where state is persisted, see [./persistence.md](./persistence.md).

## Feature Matrix

| Feature | macOS | Windows | Linux |
|---------|-------|---------|-------|
| System tray icon | Yes | Yes | No (no-op) |
| Folder picker | Native (`rfd`) | Native (`rfd`) | Plain text input |
| Window chrome | Custom titlebar (frameless) | Custom titlebar (frameless) | Custom titlebar (frameless) |
| Audio backend | CoreAudio via cpal | WASAPI via cpal | ALSA via cpal |
| Filesystem watching | FSEvents via `notify` | ReadDirectoryChangesW via `notify` | inotify via `notify` |
| Application Store path | `~/Library/Application Support/com.riff.riff/riff.sqlite3` | `%LOCALAPPDATA%\riff\riff\riff.sqlite3` | `~/.local/share/riff/riff.sqlite3` |
| Window / UI | egui via eframe | egui via eframe | egui via eframe |

Playback, library scanning, metadata reading, cover art, search, the queue, and persistence all work the same on every platform. Only the tray and the folder-picker affordance differ, plus the frameless window's platform-specific caveats: the custom titlebar ships everywhere from one code path (`riff-gui/src/ui/chrome.rs`), validated end-to-end on Windows; macOS and Linux carry documented risk with a per-platform native-decorations fallback — see the [spike findings](../engineering/spikes/frameless-window-chrome-spike.md).

## Conditional Compilation

Platform-specific code is gated with `#[cfg(target_os = "linux")]` and `#[cfg(not(target_os = "linux"))]`. The non-Linux-only dependencies — `tray-icon`, `muda`, and `rfd` — are declared under `[target.'cfg(not(target_os = "linux"))'.dependencies]` in `Cargo.toml`, so they are not even compiled on Linux.

The split shows up in two places:

- **`riff-gui/src/main.rs`** constructs the tray icon and passes it into `RiffApp::new` only on non-Linux targets. The `RiffApp::new` signature itself differs: the non-Linux build takes an extra `tray_icon` argument, while the Linux build omits it.
- **`riff-gui/src/ui/settings.rs`** uses the `rfd` native folder picker on non-Linux targets and falls back to an inline egui text input on Linux, where the user types a path and confirms it.

The `RiffApp` struct carries the tray handle behind the same `cfg`, so the field simply does not exist in a Linux build.

### The folder picker on Linux

On macOS and Windows, adding a library path opens the native OS folder dialog through `rfd`. On Linux the settings view instead shows an inline egui text input with confirm and cancel controls. The user types an absolute path; on confirm, the path is checked for existence and canonicalized for de-duplication before being added to `library_paths`. If the path does not exist or is not a directory, an inline error message is shown. This keeps the Linux build free of any native-dialog dependency while still providing a complete workflow.

### The tray menu

Where the tray is present (macOS and Windows), it is built with `tray-icon` and `muda` and offers quick playback controls — play/pause, next, previous, show/hide window, and quit. Tray menu events are dispatched on a dedicated thread (`riff-gui/src/ui/tray.rs`) that translates them into `PlaybackCommand`s on the shared command channel (through the tray's `FacadeTransport`), so the tray drives exactly the same playback path as the main window. A shared `AtomicBool` quit flag coordinates shutdown between the tray and the egui event loop.

## Why Linux Has No Tray

The system tray on Linux depends on GTK development libraries being present at build and run time. Requiring those would add a heavyweight system dependency to an otherwise lightweight, self-contained application, and tray behavior on Linux desktops is inconsistent across environments. Rather than impose that dependency, riff treats the tray as a no-op on Linux: the application runs entirely through its main window, and there is no tray icon or tray menu. macOS and Windows have first-party tray support with no extra system libraries, so the tray is enabled there.

The same reasoning applies to the folder picker. `rfd` provides a native dialog on macOS and Windows, but on Linux it would again pull in GTK. The Linux build therefore uses a plain text input for adding a library path, which has no native-dialog dependency.

## Audio Backend Notes

Audio output is handled by cpal, which selects the platform backend automatically: CoreAudio on macOS, WASAPI on Windows, and ALSA on Linux. No platform-specific code is needed in riff for this selection.

One Windows-specific behavior is worth noting: under WASAPI **shared mode**, the audio device commonly runs at 48 kHz. If a track's sample rate is not supported by the device, `CpalAudioOutput` falls back to the device's default sample rate rather than failing. This fallback is transparent to the rest of the application — the decoder still produces samples at the track's rate, and the output stream is configured to whatever the device accepts.

## Filesystem Watching

Folder watching uses the `notify` crate, which auto-selects the platform mechanism: FSEvents on macOS, ReadDirectoryChangesW on Windows, and inotify on Linux. This is cross-platform with no conditional compilation in riff's code. On Linux specifically, the kernel inotify watch limit can be reached on very large directory trees; when watching cannot be enabled (permission errors, network mounts, or the inotify limit), the affected path's watch state becomes `Warning(reason)` and the UI shows a warning indicator with the reason as a tooltip. Watching is non-fatal everywhere: if it cannot be enabled, the app continues to work with manual scans. See [./persistence.md](./persistence.md) for how watch state is persisted.

## Building per Platform

The build commands are identical on every platform:

```bash
cargo build --release -p riff-gui    # optimized binary (LTO, codegen-units=1, stripped)
cargo run -p riff-gui                # dev build (opt-level=1)
```

The practical differences are in the toolchain prerequisites:

- **macOS**: Xcode command-line tools provide the CoreAudio and system frameworks cpal and the tray link against. No extra configuration is needed.
- **Windows**: a standard Rust toolchain (MSVC or GNU) is sufficient. WASAPI is part of the OS, and the tray uses Win32 APIs directly.
- **Linux**: because the tray and `rfd` are excluded, the Linux build avoids the GTK dependency entirely. Audio output via ALSA may require the usual ALSA development headers depending on the distribution and the cpal backend selected at build time.

The release profile produces a stripped, link-time-optimized binary on all platforms; it is slower to build but substantially smaller.

## Common Platform Issues

A few issues recur often enough to note here; full diagnostics are in the troubleshooting guide linked below.

- **No tray icon on Linux.** This is by design, not a bug. Use the main window; see the "Why Linux Has No Tray" section above.
- **Wrong sample rate / no audio on Windows.** Usually WASAPI shared mode locked to 48 kHz. riff falls back to the device default rate automatically, but if another application has exclusive control of the device, cpal may fail to open a stream; closing the other application frees the device.
- **Watch warning on Linux.** A `Warning` watch state typically means the inotify watch limit was hit or the path is a network mount. Watching degrades gracefully to manual scanning.
- **Store location confusion.** The Application Store lives in the OS data directory, not next to the binary or in the working directory. See the path table above and [./persistence.md](./persistence.md).

## Troubleshooting

For platform-specific diagnostics - missing GTK libraries on Linux, audio device selection on Windows, or locating the store file - see the troubleshooting guide at [../reference/troubleshooting.md](../reference/troubleshooting.md).

## See also

- [./dependencies.md](./dependencies.md) — the platform-conditional dependency declarations.
- [./persistence.md](./persistence.md) - platform-specific store path resolution.
- [./architecture.md](./architecture.md) — how platform-specific code is kept in the frontend crate.
