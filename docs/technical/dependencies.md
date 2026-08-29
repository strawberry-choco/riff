# Dependencies

riff is a Cargo workspace: five backend crates, the frontend crate, and the integration-test crate. This page lists every dependency grouped by **owning crate**, with its version, purpose, and any relevant notes. All versions are taken from the crate manifests. Every crate targets Rust edition 2024 with a minimum supported Rust version (`rust-version`) of **1.95**.

Two workspace-level build profiles apply to every member: the dev profile uses `opt-level = 1` for faster iterative builds with usable performance, and the release profile uses `opt-level = 3`, `lto = true`, `codegen-units = 1`, and `strip = true` for a small, fully optimized binary. Release builds therefore take noticeably longer than debug builds.

Clippy is configured in the root `Cargo.toml` under `[workspace.lints.clippy]` (inherited by every crate via `[lints] workspace = true`): `pedantic` warnings are enabled, `nursery` is allowed, and a handful of noisy lints (`needless_pass_by_value`, `module_name_repetitions`, `missing_errors_doc`, `missing_panics_doc`, `must_use_candidate`) are allowed.

The ordering below is the dependency chain: everything above `riff-infra` is pure Rust and needs no C compiler or platform audio libraries; all native dependencies live in `riff-infra` (plus the frontend's UI/platform crates).

## `riff-persistence`

**No dependencies.** The persistence contract is `std`-only — it implements nothing and drags in no audio, image, or database crate. This is the property that lets both capability slices speak the persistence language without native dependencies of their own.

## `riff-playback`

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `riff-persistence` | path | Stored entities and store ports | The only workspace edge the slice has. |
| `crossbeam-channel` | 0.5 | Channel message types | Pure Rust. |
| `fastrand` | 2 | Shuffle permutation | Random indices for `PlaybackQueue` shuffling. |
| `thiserror` | 2 | Error type | Defines `PlaybackError`. |

## `riff-library`

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `riff-persistence` | path | Stored entities and store ports | The only workspace edge the slice has. |
| `crossbeam-channel` | 0.5 | Channel message types | Pure Rust. |
| `thiserror` | 2 | Error type | Defines `LibraryError`. |
| `tracing` | 0.1 | Structured logging | Scan and service warnings. |

## `riff-infra` (all native/external dependencies live here)

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `riff-persistence` | path | Store contract | `SqliteStore` implements the store ports. |
| `riff-library` | path | Library ports | Metadata, cover, scanner, watcher adapters. |
| `riff-playback` | path | Playback ports | Decoder and output adapters. |
| `symphonia` | 0.6 | Pure-Rust audio decoding | Enabled with the `all` features (mp3, flac, ogg, wav, and more). |
| `symphonia-adapter-libopus` | 0.3 | Opus codec adapter | Registered into the codec registry so Opus files decode. |
| `cpal` | 0.18 | Cross-platform audio output | Wraps WASAPI (Windows), CoreAudio (macOS), and ALSA (Linux). Owns the device-default sample-rate fallback. |
| `ringbuf` | 0.5 | Lock-free SPSC ring buffer | Between the decoder thread and the cpal callback. |
| `lofty` | 0.25 | Pure-Rust tag reading/writing | Backs `LoftyMetadataReader` and `LoftyMetadataWriter`. |
| `image` | 0.25 | Cover art decoding | Default features disabled; only `jpeg` and `png` enabled. Backs `ImageCoverLoader`. |
| `walkdir` | 2 | Recursive directory traversal | Used by `AudioFileScanner` during library scans. |
| `rusqlite` | 0.40.2 | Embedded SQLite | `bundled` feature — needs a C compiler; the reason adapters are quarantined in this crate. |
| `notify` | 8 | Filesystem watching | Auto-selects inotify/FSEvents/ReadDirectoryChangesW. |
| `notify-debouncer-full` | 0.7 | Watch-event debouncing | Feeds the watcher forwarder thread. |
| `directories` | 6 | OS data paths | Resolves the Application Store location (`riff.sqlite3`). |
| `crossbeam-channel` | 0.5 | Channels | Worker and watcher plumbing. |
| `tracing` | 0.1 | Structured logging | Adapter warnings and errors. |

Dev-dependency: `tempfile` 3.8 — scratch directories for the real-SQLite and tag round-trip tests in `riff-infra/tests/`.

## `riff-backend`

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `riff-persistence` / `riff-library` / `riff-playback` / `riff-infra` | path | The full backend stack | `riff-infra` is named only by the Composition Root. |
| `thiserror` | 2 | Facade error types | |
| `tracing` | 0.1 | Structured logging | |
| `crossbeam-channel` | 0.5 | Channels | Facade event inbox and worker plumbing. |
| `fastrand` | 2 | Shuffle | Queue helpers the facade surface exposes. |

No native dependencies of its own, and no UI crate dependencies. Dev-dependency: `tempfile` 3.8 for the `scan_bench` example's synthetic library fixture.

## `riff-gui` (frontend)

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `riff-backend` | path | The application API | The frontend's only backend dependency. |
| `egui` | 0.35 | Immediate-mode UI framework | Pinned — egui 0.36 regressed headless texture rendering (see the note in `riff-gui/Cargo.toml`). |
| `eframe` | 0.35 | Windowing and app shell | Enables the `persistence` feature. |
| `egui_extras` (aliased `epi`) | 0.35 | Image support in egui | Imported under the name `epi` with the `image` feature. |
| `resvg` | 0.48 | SVG rasterization | Lucide icon glyphs for the shell chrome. |
| `image` | 0.25 | Cover-byte decoding for texture upload | The on-disk cover loader adapter lives in `riff-infra`; decoding in-memory bytes for egui textures is UI work. |
| `tracing` / `tracing-subscriber` | 0.1 / 0.3 | Logging | Subscriber initialized in `main.rs` with `env-filter`. |
| `crossbeam-channel` | 0.5 | Channels | Frontend-local visibility channel. |

Platform-conditional (non-Linux only, under `[target.'cfg(not(target_os = "linux"))'.dependencies]` — see [./platform-support.md](./platform-support.md)):

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `tray-icon` | 0.24 | System tray icon | Requires GTK dev libraries on Linux, so it is excluded there. |
| `muda` | 0.19 | Cross-platform menu support | Used with `tray-icon` to build the tray menu. |
| `rfd` | 0.17 | Native file dialogs | Native folder picker on macOS/Windows; Linux uses a plain text input instead. |

Dev-dependencies: `egui_kittest` 0.35 (`wgpu` + `snapshot`) and `tempfile` 3.8.

## `tests` (workspace-root integration crate, package `riff-tests`)

| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `riff-backend` / `riff-infra` / `riff-library` / `riff-playback` / `riff-gui` | path | Per-crate imports | Each suite reaches the type it needs directly. |
| `egui` / `egui_kittest` | 0.35 / 0.35 | Golden-image rendering | kittest with `wgpu` + `snapshot` renders real frames headlessly. |
| `tempfile` | 3.8 | Scratch directories | Store tests and restart simulations. |
| `crossbeam-channel` | 0.5 | Test-runtime channels | Driving service seams. |
| `image` | 0.25 | Decoding canned cover bytes | PNG only, for the UI cache test. |

## See also

- [./platform-support.md](./platform-support.md) — the platform-conditional dependency declarations.
- [./architecture.md](./architecture.md) — which crate each dependency is allowed to appear in.
