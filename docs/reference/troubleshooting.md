# Troubleshooting

This page collects common issues reported while building or running riff, each broken down into **Symptom**, **Cause**, and **Fix**. Most of the runtime items are expected behaviors or platform realities rather than bugs; the goal here is to help you tell the difference quickly. For platform-specific design decisions, see [../technical/platform-support.md](../technical/platform-support.md); for where riff stores its state, see [./configuration.md](./configuration.md).

## Audio and Playback

### No audio output, or silent playback

**Symptom:** A track appears to play (progress advances, state shows Playing) but no sound is produced, or playback fails to start on some files.

**Cause:** riff decodes audio at the track's native sample rate, but an output device may not support that rate. This is especially common on Windows, where WASAPI shared mode frequently runs at 48 kHz. When the requested rate is unsupported, riff falls back to the device's default sample rate.

**Fix:** This fallback is automatic and normally transparent. If a specific device misbehaves, try selecting a different output device or changing the system default format. Persistent silence across all tracks and devices is more likely a system-level audio configuration problem than a riff bug.

### Windows audio crackling or dropouts

**Symptom:** Playback on Windows sounds glitchy — clicks, crackle, or brief dropouts — particularly on high-sample-rate files.

**Cause:** This is typically a WASAPI shared-mode sample-rate mismatch. The device runs at one rate (often 48 kHz) while the source material is at another, and the rate conversion or buffering under shared mode introduces artifacts.

**Fix:** Confirm the behavior is rate-related by testing files of different sample rates. Setting the Windows device default format to match your most common source rate can reduce conversion artifacts. This is a known characteristic of shared-mode audio on Windows rather than a defect in the decode path.

### Missing cover art

**Symptom:** A track or album plays correctly but shows no cover image in the UI.

**Cause:** riff resolves cover art with a two-step priority: embedded art in the file's metadata first, then a filesystem fallback looking for `cover.jpg` or `cover.png` (case-insensitive) in the track's directory. If neither source has an image, there is nothing to display.

**Fix:** Embed cover art in the file's tags using a tag editor, or place a `cover.jpg`/`cover.png` in the same folder as the audio files. After adding filesystem art, a rescan (or restarting) ensures the change is picked up.

## Library

### Slow first scan of a large library

**Symptom:** The first time you add a large music folder, scanning takes a long time.

**Cause:** Scanning walks the directory tree, reads metadata from every audio file, and builds the in-memory index. This is genuinely I/O- and CPU-bound work proportional to library size, and it happens in full on the first scan.

**Fix:** This is expected. Subsequent launches load the persisted library cache instead of re-scanning, so startup is fast. You only pay the full scan cost again if the cache is missing or you add new library paths. See [./configuration.md](./configuration.md) for the cache location.

### Duplicate tracks after a rescan

**Symptom:** After renaming or moving files and rescanning, the same music appears more than once in the library.

**Cause:** A track's identity (`TrackId`) is its full file path as a string. When you rename or move a file, its path changes, so riff treats it as a brand-new track rather than the same track at a new location. The old path entry and the new path entry coexist.

**Fix:** This is inherent to path-based identity. Remove the stale entries for the old paths, or clear the library cache and rescan from scratch so only the current paths are indexed. Be aware that any operation changing file paths will reproduce the effect.

### Corrupt or missing library cache

**Symptom:** The library appears empty on startup even though you have scanned folders before, or a previously populated library resets.

**Cause:** The library cache is a JSON file on disk. If it is missing, unreadable, or corrupted, riff recovers by starting from an empty library rather than failing.

**Fix:** This auto-recovery is by design. Add your library folders and rescan; the cache is rebuilt and saved after the scan completes. If corruption recurs, check that the cache location (see [./configuration.md](./configuration.md)) is on a healthy, writable filesystem.

## Platform Behavior

### No system tray icon on Linux

**Symptom:** On Linux, riff does not show a system tray icon and has no minimize-to-tray behavior, while the same build on Windows or macOS does.

**Cause:** This is by design. The tray icon depends on the `tray-icon` and `muda` crates, which require GTK development libraries on Linux. Rather than impose that dependency, riff compiles the tray as a no-op on Linux via `#[cfg(target_os = "linux")]`.

**Fix:** No fix is needed; this is intended behavior. On Windows and macOS the tray icon is available. See [../technical/platform-support.md](../technical/platform-support.md) for the full platform matrix.

### "Add library" is a text field on Linux

**Symptom:** On Linux, adding a library folder requires typing a path into a text input, whereas on Windows and macOS a native folder-picker dialog opens.

**Cause:** This is by design. The native file dialog is provided by the `rfd` crate, which is only compiled on non-Linux platforms. Linux avoids that dependency by offering a plain text field for the path instead.

**Fix:** Enter the absolute path to your music folder in the text field. On Windows and macOS the native picker is used automatically.

## Build

### Release build compiles slowly

**Symptom:** `cargo build --release` takes much longer than `cargo build` or `cargo run`.

**Cause:** The release profile enables link-time optimization (`lto = true`) and single-codegen-unit compilation (`codegen-units = 1`), both of which trade compile-time parallelism for whole-program optimization. Combined with `strip = true`, the result is a small, fully optimized binary at the cost of a longer build.

**Fix:** This is expected and normal. Use `cargo run` (the `dev` profile) for day-to-day iteration and reserve `cargo build --release` for producing distributable binaries. See [../engineering/release-and-packaging.md](../engineering/release-and-packaging.md) for details on the release profile.

### Linux build fails inside cpal or alsa-sys

**Symptom:** On Linux, the build fails while compiling `cpal` or an `alsa-sys`-related crate.

**Cause:** `cpal` uses ALSA on Linux and needs the ALSA development headers to compile. These headers are not installed by default on many distributions.

**Fix:** Install the ALSA development package (for example `sudo apt-get install libasound2-dev pkg-config` on Debian/Ubuntu, or `alsa-lib-devel` on Fedora), then rebuild. See [../engineering/development-setup.md](../engineering/development-setup.md) for the full prerequisites.
