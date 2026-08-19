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
| `egui` 0.34.3 / `eframe` 0.34.3 | UI framework and windowing (persistence feature on) |
| `egui_extras` 0.34.3 (aliased `epi`) | Image loading in egui |
| `egui-elegance` 0.13 | Theming/styling |
| `symphonia` 0.5 (all features) + `symphonia-adapter-libopus` 0.2 | Audio decoding (mp3, flac, ogg, wav, aac, opus, etc.) |
| `cpal` 0.18 | Cross-platform audio output |
| `lofty` 0.19 | Metadata reading and tag writing |
| `image` 0.25 (jpeg+png only) | JPEG/PNG decoding for cover art |
| `walkdir` 2 + `notify` 7 | Filesystem scanning and folder watching |
| `crossbeam-channel` 0.5 / `crossbeam-queue` 0.3 / `parking_lot` 0.12 | Message passing and concurrency |
| `thiserror` 1 / `tracing` 0.1 | Errors and structured logging |
| `serde` 1 / `serde_json` 1 / `directories` 5 | Persistence |
| `rand` 0.8 | Shuffle |
| `tray-icon` 0.19 + `muda` 0.15 | System tray (non-Linux only) |
| `rfd` 0.14 | Native file dialogs (non-Linux only) |
| `tempfile` 3.8 (dev) | Scratch directories for tests |

## Commands (Dev Workflow)

```bash
cargo fmt                        # format
cargo clippy                     # lint (pedantic + selected strict lints)
cargo check                      # fast type-check only
cargo run                        # run in dev mode
cargo build --release            # release build (LTO, stripped)
```

**Test suite**: ~151 tests in a single integration crate rooted at `tests/mod.rs` (`autotests = false`, one `[[test]]` target named `integration`), organized into `domain_tests`, `app_tests`, `infra_tests`, `ui_tests`, `integration_tests` with shared `test_utils`/`mocks`/`integration_helpers`. Run with `cargo test`. No inline `#[cfg(test)]` modules in `src/`. See `docs/engineering/testing-strategy.md`.

**CI**: `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test --all-targets` on push/PR to `main` (Linux + Windows matrix; macOS planned per `tasks/plan-v2.md`). No pre-commit hooks.

## State Persistence

- Library cache: serialized to `directories::ProjectDirs` data local dir under `riff/library_cache.json`. Loaded on startup, saved after each scan completes. Carries a `schema_version`; incompatible versions fall back to an empty library. Includes per-track play history (`play_count`, `last_played`, `date_added`).
- Playlists: `playlists.json` in the same data-local dir — user data, deliberately separate from the rebuildable cache.
- Library paths: persisted via `eframe::Storage` (key `library_paths` as JSON string array).
- TrackId: string key derived from `PathBuf::to_string_lossy()` — track identity is its full file path.

## Important Gotchas

- **msrv**: `rust-version = "1.92"` in Cargo.toml (edition 2021). CI uses the stable toolchain.
- **Release profile**: LTO, codegen-units=1, strip=true. `cargo build --release` takes longer but produces smaller binaries.
- **Audio device**: Falls back to device default sample rate if the track's rate is unsupported (common on Windows WASAPI shared mode at 48 kHz).
- **Tests live in `tests/`** — one integration crate rooted at `tests/mod.rs` (per-suite files are modules of it, not separate crates). App-layer tests drive the port traits via the shared mocks module; settings tests use a `MockStorage` for `eframe::Storage`; persistence tests use `tempfile` scratch dirs.
- **`AppState`** is a single large struct behind `Arc<Mutex<>>` — contains library, queue, playback, theme, UI state all together. Plan lock ordering carefully to avoid deadlocks (current code only uses one Mutex for AppState, no nested locking).
- **Cover art LRU**: Max 50 cached textures in `cover_textures` HashMap with manual LRU eviction in `cover_lru_keys` Vec.
- **No DI framework** — manual constructor injection in `main.rs` only.
- **Buffer management**: `SymphoniaDecoder` buffers oversize decoded packets in `pending_samples`. The `CpalAudioOutput` uses a `VecDeque<f32>` ring buffer shared between producer (decode loop) and consumer (cpal callback).

## Config Files

`clippy.toml` configures Clippy (msrv, tool-level options). Lint levels are set in `Cargo.toml` under `[lints.clippy]` (pedantic with selected allowances). CI config is `.github/workflows/ci.yml`; no `rustfmt.toml` (defaults apply). Architecture rules live in `docs/technical/architecture.md`. Feature requirements live in `docs/product/requirements.md`, statuses in `docs/product/features.md`, per-surface specs in `docs/product/specs/`. The full documentation index is in `docs/README.md`.

## Agent Skills

Agent Skills (`~/.config/opencode/skills/agent-skills/`) provide production-grade engineering workflows. They are auto-discovered — the `skill` tool loads `SKILL.md` from the relevant directory.

**Use the `using-agent-skills` meta-skill first** when starting a session to decide which skill applies. Mapping by intent:

- **Design / spec a feature** → `spec-driven-development`
- **Plan work into tasks** → `planning-and-task-breakdown`
- **Implement a feature** → `incremental-implementation` (+ `test-driven-development`, `frontend-ui-engineering`)
- **Fix a bug** → `debugging-and-error-recovery`
- **Review code** → `code-review-and-quality`
- **Simplify code** → `code-simplification`
- **Security / performance / CI work** → `security-and-hardening`, `performance-optimization`, `ci-cd-and-automation`
- **Ship / release** → `shipping-and-launch`, `git-workflow-and-versioning`

**Rules:**
- If a skill applies, invoke it with the `skill` tool and follow it exactly.
- Do not jump directly to implementation — spec before code, plan before build, test before ship.
- Never partially apply a skill — follow the workflow to its exit criteria.

### Slash Commands

Commands live in `.opencode/commands/`. Invoke by name; each loads a structured prompt backed by the corresponding skill:

| Command | Skill | What it does |
|---|---|---|
| `/spec` | spec-driven-development | Write a structured spec before writing code |
| `/planning` | planning-and-task-breakdown | Break work into verifiable tasks |
| `/build` | incremental-implementation + test-driven-development | Implement one task (RED→GREEN→commit). Add `auto` to run the whole plan |
| `/test` | test-driven-development | TDD loop or Prove-It for bugs |
| `/review` | code-review-and-quality | Five-axis code review |
| `/code-simplify` | code-simplification | Reduce complexity, preserve behavior |
| `/ship` | shipping-and-launch | Parallel fan-out review + go/no-go decision |
| `/webperf` | (web-performance-auditor) | Web performance audit (web apps only) |

### Issue tracker

Issues and specs are tracked as local markdown under `.scratch/` (no git remote configured yet). See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical five-label vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.
