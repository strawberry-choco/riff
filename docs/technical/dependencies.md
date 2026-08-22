# Dependencies

riff is a single Cargo crate. This page lists every dependency grouped by concern, with its version, purpose, and any relevant notes. All versions are taken from `Cargo.toml`. The crate targets Rust edition 2021 with a minimum supported Rust version (`rust-version`) of **1.92**.

Two build profiles are configured: the dev profile uses `opt-level = 1` for faster iterative builds with usable performance, and the release profile uses `opt-level = 3`, `lto = true`, `codegen-units = 1`, and `strip = true` for a small, fully optimized binary. Release builds therefore take noticeably longer than debug builds.

Clippy is configured in `Cargo.toml` under `[lints.clippy]`: `pedantic` warnings are enabled, `nursery` is allowed, and a handful of noisy lints (`needless_pass_by_value`, `module_name_repetitions`, `missing_errors_doc`, `missing_panics_doc`, `must_use_candidate`) are allowed.

## UI

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `egui` | 0.34.3 | Immediate-mode UI framework | Provides the widget set and the `Context`. |
| `eframe` | 0.34.3 | Windowing and app shell for egui | Enables the `persistence` feature for `eframe::Storage`. |
| `egui-elegance` | 0.13 | Theming | Supplies the dark (slate) and light (frost) themes. |
| `epi` (alias for `egui_extras`) | 0.34.3 | Image support in egui | Imported under the name `epi` with the `image` feature for loading image data into egui. |

## Audio decoding

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `symphonia` | 0.5 | Pure-Rust audio decoding | Enabled with the `all` features (mp3, flac, ogg, wav, and more). |
| `symphonia-adapter-libopus` | 0.2 | Opus codec adapter | symphonia 0.5 has no native Opus decoder, so this adapter is registered into the codec registry to provide Opus support. |

The audio engine builds a `CodecRegistry`, registers symphonia's default codecs, and then registers the Opus adapter on top before constructing the `SymphoniaDecoder`.

## Audio output

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `cpal` | 0.18 | Cross-platform audio output | Wraps WASAPI (Windows), CoreAudio (macOS), and ALSA (Linux). Falls back to the device default sample rate when a track's rate is unsupported (common under Windows WASAPI shared mode at 48 kHz). |

## Metadata

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `lofty` | 0.19 | Pure-Rust tag and metadata reading | Reads tags, duration, audio format, and embedded cover art. Backs `LoftyMetadataReader`. |

## Image

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `image` | 0.25 | Image decoding for cover art | Default features disabled; only the `jpeg` and `png` decoders are enabled. Backs `ImageCoverLoader`. |

## Filesystem

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `walkdir` | 2 | Recursive directory traversal | Used by `AudioFileScanner` to find audio files during a library scan. |
| `notify` | 7 | Cross-platform filesystem watching | Backs `FilesystemWatcher`; auto-selects inotify/FSEvents/ReadDirectoryChangesW. Used for automatic rescans of watched folders. |

## Errors

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `thiserror` | 1 | Ergonomic error types | Defines the application-layer `AppError` enum with per-variant display messages. |

## Logging
| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `tracing` | 0.1 | Structured logging | Used throughout for warnings and errors (store failures, cover decode failures, playback errors). |
| `tracing-subscriber` | 0.3 | Logging subscriber | Enables the `env-filter` feature; initialized in `main.rs` via `tracing_subscriber::fmt::init()`. |

## Configuration directories

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `directories` | 5 | OS-specific config/data paths | `ProjectDirs::from("", "", "riff")` resolves the platform-appropriate data directory for the Application Store (`riff.sqlite3`). |

## Threading and synchronization

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `crossbeam-channel` | 0.5 | Multi-producer/multi-consumer channels | All cross-thread messaging uses unbounded crossbeam channels. |
| `crossbeam-queue` | 0.3 | Concurrent queue primitives | Available for lock-free queue structures. |
| `parking_lot` | 0.12 | Faster synchronization primitives | Available alongside `std::sync`; the application state itself uses `std::sync::Mutex` with a poison-recovering helper. |

## Random

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `rand` | 0.8 | Random number generation | Used by `PlaybackQueue` to shuffle track indices. |

## Testing

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `tempfile` | 3.8 | Temporary files/directories in tests | A dev-dependency. The test suite lives in `tests/` (`mod.rs`, `domain_tests.rs`, `app_tests.rs`, `infra_tests.rs`, `ui_tests.rs`, `integration_tests.rs`). |

## Platform-conditional (non-Linux only)

These dependencies are declared under `[target.'cfg(not(target_os = "linux"))'.dependencies]` and are compiled only on macOS and Windows. See [./platform-support.md](./platform-support.md) for the rationale.

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `tray-icon` | 0.19 | System tray icon | Requires GTK dev libraries on Linux, so it is excluded there. |
| `muda` | 0.15 | Cross-platform menu/tray menu support | Used with `tray-icon` to build the tray menu. |
| `rfd` | 0.14 | Native file dialogs | Provides the native folder picker on macOS/Windows; Linux uses a plain text input instead. |

## See also

- [./platform-support.md](./platform-support.md) — platform feature matrix and conditional compilation.
- [./architecture.md](./architecture.md) — which layer each crate is allowed to appear in.
