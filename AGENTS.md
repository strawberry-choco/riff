# riff — Music Player (Rust)

A lightweight, offline-first desktop music player. A Cargo workspace of seven members: five backend capability crates, one frontend crate on the nsh UI stack (which also owns the ported design system), and an integration-test crate. The `riff` binary renders through nsh (ADR 0022). See `docs/technical/architecture.md` for the split.

## Quick Start

```bash
cargo run                          # dev build of the `riff` binary (opt-level=1)
cargo build --release              # LTO, stripped, optimized release
cargo check --workspace            # fast type-check without codegen
git config core.hooksPath .githooks  # wire the commit-message hook (CI is the real gate)
```

No codegen step, no migrations to run by hand. The `riff` binary builds with no feature flags. The nsh platform crates carry a trimmed feature list (`shader` and `svg`), single-sourced in the root manifest (ADR 0022).

## Architecture

A vertical capability split with a strict, compiler-enforced dependency chain (full reference in `docs/technical/architecture.md`; decision records in `docs/adr/0009-vertical-crate-split-of-the-backend.md`, `docs/adr/0018-gui-widgetry-splits-into-theme-and-component-crates.md` and `docs/adr/0022-nsh-ui-stack-replaces-egui.md`):

```
riff-gui-nsh (frontend, `riff` binary)
    -> riff-backend (application API + Composition Root)
        -> riff-infra (adapters + native deps)
            -> riff-library / riff-playback -> riff-persistence
    -> nsh-core / nsh-base / nsh-engine, one nsh-platform-* crate per OS
```

Each crate's contents and membership criterion, one line each; `docs/technical/architecture.md#crate-definitions-and-membership-criteria` is the full reference.

- **`riff-persistence`** — types that cross the persistence boundary, and shared `std`-only utilities that belong to no single capability; the standing case is `MutexExt` (poison recovery), defined once in `riff-persistence/src/sync.rs` and re-exported as `riff_backend::app::MutexExt`.
- **`riff-library`** — the collection capability: collection use cases and the ports they consume. Sibling of `riff-playback` — no edge between them.
- **`riff-playback`** — the playback capability: playback use cases and the ports they consume.
- **`riff-infra`** — an item belongs here iff it implements a port defined in another crate or wraps a native/external dependency — nothing else.
- **`riff-backend`** — the frontend-facing event surface, the app-layer application services, and the one place that knows both ports and adapters (the Composition Root).
- **`riff-gui-nsh`** — the frontend, on the nsh UI stack: rendering, input, platform integration, the ported design system (`src/theme/`), and the `riff` binary, a thin composition over `riff_backend::composition::AppRuntime::spawn`. Its single `riff-*` edge is `riff-backend`; it names no egui crate.

Domain types (`Track`, `TrackId`, `PlaybackQueue`, …) live in `riff-persistence` and `riff-playback` and import nothing from app, infra, or UI code. Each slice codes against its own port traits; `riff-infra` implements them; dependency arrows point adapters → slices.

Inside the slices, the layering is preserved as module convention: `domain/` (pure types), `app/` (use cases, session state, projections), `infra/` (port traits the adapter crate implements).

## Threading Model

The Composition Root (`crates/riff-backend/src/composition.rs`) spawns and joins worker threads: `AppRuntime::spawn` returns `(AppRuntime, RuntimeLifecycle)` — the handles the frontend renders with, and the worker threads plus the flags that end them. `RuntimeLifecycle::shutdown` is explicit and idempotent, and joins in dependency order (audio engine first, since its exit disconnects the coordinator). The thread inventory — one entry per worker, with the channels between them — lives in `docs/technical/threading-model.md`.

Cross-thread communication: `crossbeam_channel::unbounded()` for all message passing. Shared state: `Arc<Mutex<PlaybackSession>>` and `Arc<Mutex<LibrarySession>>` (one mutex per session — never nested), `Arc<Mutex<BackendEvents>>`, an `Arc<AtomicBool>` cancel flag for library scans, one `Arc<AtomicBool>` stop flag per request-channel worker (engine, scan, tag-edit, cover), and a quit flag. The audio ring buffer between decode loop and cpal callback lives inside `riff-infra`'s output adapter.

## Platform-Specific Code

- **macOS / Windows / Linux**: the tray, the native folder picker and OS notifications all go through nsh's desktop services (`set_tray`, `pick_paths`, `show_notification`) in `riff-gui-nsh/src/platform/`.
- **Linux**: No tray icon (no-op). Folder picker is a text input field (no native file dialog). Conditional via `#[cfg(target_os = "linux")]` / `#[cfg(not(target_os = "linux"))]`.
- **nsh frontend**: Platform integration comes from the `nsh-platform-*` crates, one per desktop OS. The N1 scaffold opens a window only. Tray, picker and the rest of the platform parity land at N14.

## Commands (Dev Workflow)

```bash
cargo fmt                                  # format
cargo check --all-targets                  # type/borrow checking across all targets
cargo clippy --all-targets -- -D warnings  # lint with warnings as errors
cargo test --all-targets                   # run all unit and integration tests
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --document-private-items  # build docs, rustdoc warnings are errors
cargo run                                  # run in dev mode
cargo build --release                      # release build (LTO, stripped)
```

**Test suite**: Per-crate suites sit with the code they cover; cross-crate integration and the nsh bridge, shell and flow suites sit in the single workspace-root `riff-tests` crate. The frontend's scaffold tests sit in `crates/riff-gui-nsh/tests/` and the theme sweeps in `tests/token_sweeps.rs`. `docs/engineering/testing-strategy.md` is authoritative on placement.

**CI**: `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test --all-targets` and a `cargo build --locked --workspace --bins` link step on push/PR to `master` (Linux + Windows matrix, Linux packages from the shared `.github/actions/linux-deps` action), plus a warning-free rustdoc build (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --document-private-items`) on the Linux leg, plus the `commit-lint` jobs that validate every commit message against the Conventional Commits profile (`docs/engineering/commit-conventions.md`). A commit-message hook is available locally (`git config core.hooksPath .githooks`) — optional, with the CI jobs as the real gate.

## State Persistence

- Library: the authoritative indexed collection lives in the Application Store (`riff.sqlite3`, same data-local dir) — artists, albums identified by `(album artist, title)`, and tracks keyed by path, with strict foreign keys and a derived lowercased `search_text` column. Scans commit in ~10-track batches (an interrupted scan keeps committed batches) with a store-backed freshness filter. There is no in-memory mirror: every view reads the store through bounded Session Projections invalidated by a session-local generation counter (bumped inside the store's mutation impls on each committed mutation). Per-track play history (`play_count`, `last_played`, `date_added`) lives in the store's `tracks` table. The legacy `library_cache.json` is never read or written.
- Playlists: user data in the Application Store via the `PlaylistStore` port — every mutation commits as one immediate durable transaction. The legacy `playlists.json` is never read or written.
- Library paths and watch states: persisted in the Application Store's typed settings tables.
- TrackId: string key derived from `PathBuf::to_string_lossy()` — track identity is its full file path.

## Important Gotchas

- **msrv**: `rust-version = "1.97"` in every crate manifest (edition 2024), and no job enforces it. The floor is the nsh framework's, whose crates declare 1.97. The same string lives in `clippy.toml`, `docs/engineering/development-setup.md`, `CONTRIBUTING.md` and the `ci.yml` MSRV comment, and all six declarations move together. CI pins one exact toolchain (`toolchain:` in `.github/workflows/ci.yml`) and is bumped deliberately, so a Rust release cannot make CI fail on its own.
- **nsh pinned to 0.6.1**: all seven framework crates are pinned exactly and aligned as one set. nsh makes no compatibility promise for its Rust crates, so an upgrade is its own PR that runs the full suite and the perf fixture.
- **Release profile**: workspace-level LTO, codegen-units=1, strip=true. Profiles and the release process live in `docs/engineering/release-and-packaging.md`.
- **Audio device**: The output stream always opens at the **device default** sample rate. `build_stream_config` (in `riff-infra`'s `audio_output.rs`) takes the track's requested rate as `_requested_rate` and never reads it, so the "falls back when the track's rate is unsupported" behaviour (common on Windows WASAPI shared mode at 48 kHz) is really "always the default" — the requested rate is not consulted. The rate achieved is readable only as a concrete `pub fn CpalAudioOutput::effective_sample_rate`, which **no production code calls**; it is *not* a method on the `AudioOutput` port, so the Audio Engine cannot see it through its interface. Wiring the fact through the port belongs in the same change as the consumer that reads it, never on its own.
- **Session state is two structs**: `PlaybackSession` (`riff-playback`) and `LibrarySession` (`riff-backend`), each behind its own `Arc<Mutex<>>`. Plan lock ordering carefully; never hold one session's lock while acquiring the other's. The one cross-slice interaction (a playback failure setting a scan-status message) is a typed notice through the event inbox, not a state write.
- **Cover caches**: two in RAM and one on disk, and they are not interchangeable. The RAM one is deliberately **split in two halves that never merge**: the decision half — the cache key, the requested size, the in-flight set and its LRU order, what to request, what arrived — is `CoverCache` in `crates/riff-gui-nsh/src/bridge/covers.rs` and holds no picture (the `CONTEXT.md` term as written); the View half is the `ImageSrc` map and its byte budget, in `crates/riff-gui-nsh/src/model.rs` and `src/bridge/covers.rs`. The map is bounded twice over — `COVER_VIEWS_CAP` = 200 entries **and** `COVER_VIEWS_BYTE_BUDGET` = 64 MB of RGBA, with the shared placeholder tile exempt from both the accounting and the eviction — and the two bounds are not redundant: the canonical boxes span 80x in bytes, so 200 entries is 2.4 MB of thumbnails or ~200 MB of heroes. Both halves die with the process, and they meet in the pump's `settle` drain and the cover tile's lookup: the map has one writer, so it is the only thing that can report which arrivals its own bounds dropped, and that report is what keeps the two from drifting. The decision half is assertable with no UI context at all (`crates/riff-gui-nsh/tests/cover_cache_tests.rs`). The Cover Service (`riff-library`) keeps **no decoded-cover cache**: its only cache is `negative`, an artless-verdict LRU of the same cap. What survives a restart is the **Thumbnail cache** — encoded rungs under `<data_local_dir>/covers/`, keyed on the cover *Source's* path with its `(mtime, len)` carried in the filename, so there is no index to keep in sync and no automatic eviction; Settings → Library → "Clear Thumbnail cache" is the only reclaim, and it runs on the cover worker and reports a polled outcome like Tag Edit. There is deliberately no content hash (reading the sources to decide what to re-decode defeats the point) and no decoded-pixel cache in RAM.
- **No DI framework** — manual constructor injection in `crates/riff-backend/src/composition.rs` only.
- **Buffer management**: `SymphoniaDecoder` (`riff-infra`) buffers oversize decoded packets in `pending_samples`. `CpalAudioOutput` uses a lock-free SPSC ring buffer (`ringbuf`) between the producer (decode loop) and the consumer (cpal callback).

## Config Files

`clippy.toml` configures Clippy (msrv, tool-level options). Lint levels are set in the root `Cargo.toml` under `[workspace.lints.clippy]` (pedantic with selected allowances) and `[workspace.lints.rustdoc]` (`private_intra_doc_links` allowed), inherited by every crate via `[lints] workspace = true`. The nsh frontend's app metadata lives in `crates/riff-gui-nsh/nsh.json`, and its `nsh_base` theme file in `crates/riff-gui-nsh/theme.json` (ADR 0022). CI config is `.github/workflows/ci.yml`; no `rustfmt.toml` (defaults apply). Architecture rules live in `docs/technical/architecture.md`. Feature statuses live in `docs/product/features.md`. The full documentation index is in `docs/README.md`.

## Domain docs

Single-context: `CONTEXT.md` and `docs/adr/` at the repo root.

## Agent skills

### Issue tracker

Local markdown: issues and specs live as files under `.scratch/` (gitignored). See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical roles, recorded as a `Status:` line in each issue file. See `docs/agents/triage-labels.md`.

### Domain docs

single-context. See `docs/agents/domain.md`.
