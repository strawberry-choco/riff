# riff — Music Player (Rust + egui)

A lightweight, offline-first desktop music player. Single Cargo crate.

## Quick Start

```bash
cargo run                      # dev build (opt-level=1)
cargo build --release          # LTO, stripped, optimized release
cargo check                    # fast type-check without codegen
```

No special features or feature flags. No codegen step, no migrations.

## Architecture

Four-layer layout enforced by convention — `main.rs` is the only file that touches all four:

```
src/main.rs          # Composition root: wires channels, threads, UI
src/domain/          # Pure business logic. Zero external crate imports.
src/app/             # Use cases, state, trait interfaces (ports)
src/infra/           # Trait implementations using external crates
src/ui/              # egui widgets, tray icon, native file dialogs
```

Domain (`Track`, `PlaybackQueue`, `PlaybackState`, `TrackId`) must not import anything from `app/`, `infra/`, or `ui/`. App defines traits (`AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`) that infra implements. Full architecture reference in `docs/technical/architecture.md`. (The older `.lattice/` docs referenced some module names that differ from actual filenames; the `docs/` tree uses the real source filenames.)

## Threading Model

Threads are spawned directly in `main.rs` with `std::thread::spawn`:

- **Main thread** — egui event loop. Must not block.
- **Audio engine thread** — decode + output loop. Reads `PlaybackCommand` from channel, sends `PlaybackUpdate` back.
- **Update processor thread** — receives `PlaybackUpdate` from engine, writes to shared `Arc<Mutex<AppState>>`.
- **Library scan thread** — scans filesystem with `walkdir`, sends `LibraryUpdate` back.
- **Cover loader thread** — decodes cover images in background, sends result via channel.

Cross-thread communication: `crossbeam_channel::unbounded()` for all message passing. Shared state via `Arc<Mutex<AppState>>`. There is also an `Arc<AtomicBool>` cancel flag for library scans.

## Platform-Specific Code

- **macOS / Windows**: System tray icon (`tray-icon` + `muda`), native folder picker (`rfd`).
- **Linux**: No tray icon (no-op). Folder picker is a text input field (no native file dialog). Conditional via `#[cfg(target_os = "linux")]` / `#[cfg(not(target_os = "linux"))]`.

## Key Dependencies

| Crate | Use |
|---|---|
| `egui` 0.28 / `eframe` 0.28 | UI framework and windowing |
| `egui_extras` 0.28 | Image loading in egui |
| `symphonia` 0.5 (all features) | Audio decoding (mp3, flac, ogg, wav, etc.) |
| `cpal` 0.15 | Cross-platform audio output |
| `lofty` 0.19 | Metadata reading (tags, cover art) |
| `image` 0.24 | JPEG/PNG decoding for cover art |
| `walkdir` 2 | Filesystem scanning |
| `crossbeam-channel` 0.5 | Thread-safe message passing |
| `tray-icon` 0.19 + `muda` 0.15 | System tray (non-Linux only) |
| `rfd` 0.14 | Native file dialogs (non-Linux only) |

## Commands (Dev Workflow)

```bash
cargo fmt                        # format
cargo clippy                     # lint (pedantic + selected strict lints)
cargo check                      # fast type-check only
cargo run                        # run in dev mode
cargo build --release            # release build (LTO, stripped)
```

There is **no CI pipeline**, **no test suite** (zero `#[test]` or `#[cfg(test)]` anywhere), and no pre-commit hooks.

## State Persistence

- Library cache: serialized to `directories::ProjectDirs` data local dir under `riff/library_cache.json`. Loaded on startup, saved after each scan completes.
- Library paths: persisted via `eframe::Storage` (key `library_paths` as JSON string array).
- TrackId: string key derived from `PathBuf::to_string_lossy()` — track identity is its full file path.

## Important Gotchas

- **msrv**: 1.75. CI does not enforce it; `rust-version` in Cargo.toml is informational.
- **Release profile**: LTO, codegen-units=1, strip=true. `cargo build --release` takes longer but produces smaller binaries.
- **Audio device**: Falls back to device default sample rate if the track's rate is unsupported (common on Windows WASAPI shared mode at 48 kHz).
- **No tests exist** — don't look for a test directory or test runner. Any test infrastructure must be created from scratch.
- **`AppState`** is a single large struct behind `Arc<Mutex<>>` — contains library, queue, playback, theme, UI state all together. Plan lock ordering carefully to avoid deadlocks (current code only uses one Mutex for AppState, no nested locking).
- **Cover art LRU**: Max 50 cached textures in `cover_textures` HashMap with manual LRU eviction in `cover_lru_keys` Vec.
- **No DI framework** — manual constructor injection in `main.rs` only.
- **Buffer management**: `SymphoniaDecoder` buffers oversize decoded packets in `pending_samples`. The `CpalAudioOutput` uses a `VecDeque<f32>` ring buffer shared between producer (decode loop) and consumer (cpal callback).

## Config Files

`clippy.toml` configures Clippy (msrv, tool-level options). Lint levels are set in `Cargo.toml` under `[lints.clippy]` (pedantic with selected allowances). No rustfmt or CI config files exist. Architecture rules live in `docs/technical/architecture.md`. Feature requirements live in `docs/product/features.md`. The full documentation index is in `docs/README.md`.
