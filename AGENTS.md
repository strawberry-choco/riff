# riff — Music Player (Rust + egui)

A lightweight, offline-first desktop music player. A Cargo workspace: five backend capability crates, the frontend crate, and the integration-test crate.

## Quick Start

```bash
cargo run -p riff-gui              # dev build of the `riff` binary (opt-level=1)
cargo build --release -p riff-gui  # LTO, stripped, optimized release
cargo check --workspace            # fast type-check without codegen
```

No special features or feature flags. No codegen step, no migrations to run by hand.

## Architecture

Six source crates in a vertical capability split with a strict, compiler-enforced dependency chain (full reference in `docs/technical/architecture.md`, decision record in `docs/adr/0009-vertical-crate-split-of-the-backend.md`):

```
riff-gui (frontend, `riff` binary)
    -> riff-backend (application API + Composition Root)
        -> riff-infra (adapters + native deps)
            -> riff-library / riff-playback -> riff-persistence
```

- **`riff-persistence`** — the stored entities (`Track`, `TrackId`, `TrackMetadata`, `Album`, `Artist`, `Playlist`) and the Application Store contract (store ports, DTOs, `StoreError`). Zero dependencies: `std` only. Membership criterion: types that cross the persistence boundary.
- **`riff-library`** — the collection capability: scan-side Track construction, the Library Scan Service, Library Session Projections, playlist management, cover resolution and service, its port traits (`MetadataReader`, `MetadataWriter`, `CoverLoader`, `FilesystemWatch`), and `LibraryError`. Sibling of `riff-playback` — no edge between them.
- **`riff-playback`** — the playback capability: the Playback Queue, playback command/update types, the audio engine (pure Rust over its ports), gapless math, the Playback Coordinator, the `Transport` trait + `ChannelTransport`/`FacadeTransport`, its port traits (`AudioDecoder`, `DecoderFactory`, `AudioOutput`), `PlaybackError`, the `PlaybackSession`, and the Up Next read model.
- **`riff-infra`** — every port implementation and every native/external dependency (`rusqlite` bundled, `cpal`, `symphonia`, `lofty`, `image`, `walkdir`, `notify`), with internal seams store / audio / media / filesystem. Membership rule: an item belongs here iff it implements a port defined in another crate or wraps a native/external dependency.
- **`riff-backend`** — the application API: the Backend Facade (typed events and notices), the facade-adjacent services (Session Views, Tag Edit service, Watcher Manager), the `LibrarySession`, the re-export surface that keeps historical `riff_backend::` paths resolving, and the Composition Root (`composition.rs` — the only place that names both ports and concrete adapters, and the owner of the worker threads).
- **`riff-gui`** — the frontend: egui UI, tray icon, native dialogs, fonts, and the `riff` binary, which is a thin composition over `riff_backend::composition::AppRuntime::spawn`.

Domain types (`Track`, `TrackId`, `PlaybackQueue`, …) live in `riff-persistence` and `riff-playback` and import nothing from app, infra, or UI code. Each slice codes against its own port traits; `riff-infra` implements them; dependency arrows point adapters → slices.

Inside the slices, the layering is preserved as module convention: `domain/` (pure types), `app/` (use cases, session state, projections), `infra/` (port traits the adapter crate implements).

## Threading Model

Worker threads are spawned by the Composition Root (`riff-backend/src/composition.rs`):

- **Main thread** — egui event loop (`riff-gui`). Must not block.
- **Audio engine thread** — `AudioEngine::run` (`riff-playback/src/infra/audio_engine.rs`). Reads `PlaybackCommand` from a channel, sends `PlaybackUpdate` back.
- **Playback Coordinator thread** — `PlaybackCoordinator::spawn` (`riff-playback`). Applies `PlaybackUpdate`s to the playback session, commits play history, and owns auto-advance; playback failures surface as typed notices through the facade.
- **Library scan worker thread** — runs the `ScanService` worker (`riff-library`); the scan flow never touches the sessions directly.
- **Filesystem-event forwarder thread** — forwards `notify` events to the `WatcherManager` (`riff-backend`), which debounces and triggers rescans through the scan service.
- **Tag-edit worker thread** — `TagEditWorker` writes tag edits via lofty and commits store facts as one durable change.
- **Cover worker thread** — `CoverService` worker resolves and decodes cover art in the background.

Cross-thread communication: `crossbeam_channel::unbounded()` for all message passing. Shared state: `Arc<Mutex<PlaybackSession>>` and `Arc<Mutex<LibrarySession>>` (one mutex per session — never nested), `Arc<Mutex<BackendFacade>>`, an `Arc<AtomicBool>` cancel flag for library scans, and a quit flag. The audio ring buffer between decode loop and cpal callback lives inside `riff-infra`'s output adapter.

## Platform-Specific Code

- **macOS / Windows**: System tray icon (`tray-icon` + `muda`), native folder picker (`rfd`) — all in `riff-gui`.
- **Linux**: No tray icon (no-op). Folder picker is a text input field (no native file dialog). Conditional via `#[cfg(target_os = "linux")]` / `#[cfg(not(target_os = "linux"))]`.

## Commands (Dev Workflow)

```bash
cargo fmt                                  # format
cargo check --all-targets                  # type/borrow checking across all targets
cargo clippy --all-targets -- -D warnings  # lint with warnings as errors
cargo test --all-targets                   # run all unit and integration tests
cargo run -p riff-gui                      # run in dev mode
cargo build --release -p riff-gui          # release build (LTO, stripped)
```

**Test suite**: per-crate suites where the code they cover lives (currently `riff-infra`, which hosts the real-SQLite store tests and the adapter tests), plus a single integration crate at the workspace root (`tests/`, package `riff-tests`, `autotests = false`, one `[[test]]` target named `integration`) organized into `domain_tests`, `app_tests`, `infra_tests`, `ui_tests`, `golden_tests`, `integration_tests` with shared `test_utils`/`mocks`/`integration_helpers`. Run with `cargo test`. No inline `#[cfg(test)]` modules in `src/`. See `docs/engineering/testing-strategy.md`; golden-image snapshot workflow in `docs/engineering/golden-image-testing.md`.

**CI**: `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test --all-targets` on push/PR to `main` (Linux + Windows matrix). No pre-commit hooks.

## State Persistence

- Library: the authoritative indexed collection lives in the Application Store (`riff.sqlite3`, same data-local dir) — artists, albums identified by `(album artist, title)`, and tracks keyed by path, with strict foreign keys and a derived lowercased `search_text` column. Scans commit in ~10-track batches (an interrupted scan keeps committed batches) with a store-backed freshness filter. There is no in-memory mirror: every view reads the store through bounded Session Projections invalidated by a session-local generation counter (bumped inside the store's mutation impls on each committed mutation). Per-track play history (`play_count`, `last_played`, `date_added`) lives in the store's `tracks` table. The legacy `library_cache.json` is never read or written.
- Playlists: user data in the Application Store via the `PlaylistStore` port — every mutation commits as one immediate durable transaction. The legacy `playlists.json` is never read or written.
- Library paths and watch states: persisted in the Application Store's typed settings tables.
- TrackId: string key derived from `PathBuf::to_string_lossy()` — track identity is its full file path.

## Important Gotchas

- **msrv**: `rust-version = "1.95"` in every crate manifest (edition 2024). CI uses the stable toolchain.
- **egui pinned to 0.35**: egui 0.36 regressed headless texture rendering — kittest golden snapshots lose all user-loaded textures (`ctx.load_texture` + painter/image widgets paint nothing; text/shapes still render). The app itself renders fine windowed, but goldens would bake in icon-less UI. Revisit when upgrading past 0.35 (check upstream fix status first). See the note in `riff-gui/Cargo.toml`.
- **Release profile**: workspace-level LTO, codegen-units=1, strip=true. `cargo build --release` takes longer but produces smaller binaries.
- **Audio device**: Falls back to device default sample rate if the track's rate is unsupported (common on Windows WASAPI shared mode at 48 kHz) — reported through the `AudioOutput::effective_sample_rate` port method.
- **Tests live in `riff-infra/tests/` and `tests/`** — the adapter/store tests live with `riff-infra`; cross-crate integration, UI, and golden tests live in the single workspace-root crate (`tests/mod.rs`, per-suite files are modules of it). App-layer tests drive the port traits via the shared mocks module; store tests run against real SQLite in `tempfile` scratch dirs at the infra seam.
- **Session state is two structs**: `PlaybackSession` (`riff-playback`) and `LibrarySession` (`riff-backend`), each behind its own `Arc<Mutex<>>`. Plan lock ordering carefully; never hold one session's lock while acquiring the other's. The one cross-slice interaction (a playback failure setting a scan-status message) is a typed notice through the facade, not a state write.
- **Cover caches**: the decoded-cover LRU (cap 50) lives in the `CoverService` (`riff-library`); the egui texture LRU (max 50 `TextureHandle`s in `cover_textures` with manual LRU eviction in `cover_lru_keys`) lives in `riff-gui/src/ui/app.rs`.
- **No DI framework** — manual constructor injection in `riff-backend/src/composition.rs` only.
- **Buffer management**: `SymphoniaDecoder` (`riff-infra`) buffers oversize decoded packets in `pending_samples`. `CpalAudioOutput` uses a lock-free SPSC ring buffer (`ringbuf`) between the producer (decode loop) and the consumer (cpal callback).

## Config Files

`clippy.toml` configures Clippy (msrv, tool-level options). Lint levels are set in the root `Cargo.toml` under `[workspace.lints.clippy]` (pedantic with selected allowances) and inherited by every crate via `[lints] workspace = true`. CI config is `.github/workflows/ci.yml`; no `rustfmt.toml` (defaults apply). Architecture rules live in `docs/technical/architecture.md`. Feature requirements live in `docs/product/requirements.md`, statuses in `docs/product/features.md`, per-surface specs in `docs/product/specs/`. The full documentation index is in `docs/README.md`.

<!-- code-review-graph MCP tools -->
## MCP Tools: code-review-graph

**This project has a knowledge graph. Start with the code-review-graph
MCP tools to narrow scope, then read the source.** The graph is cheaper than scanning files and
gives you structural context (callers, dependents, test coverage) that file search cannot.

### When to use graph tools FIRST

- **Exploring code**: `semantic_search_nodes_tool` or `query_graph_tool` instead of Grep
- **Understanding impact**: `get_impact_radius_tool` instead of manually tracing imports
- **Code review**: `detect_changes_tool` + `get_review_context_tool` instead of reading entire files
- **Finding relationships**: `query_graph_tool` with callers_of/callees_of/imports_of/tests_for
- **Architecture questions**: `get_architecture_overview_tool` + `list_communities_tool`

### Workflow

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes_tool` for code review.
3. Use `get_affected_flows_tool` to understand impact.
4. Use `query_graph_tool` pattern="tests_for" to check coverage.

### Verify in the source

- Narrow scope with the graph, then read the source. Do not change code from graph output alone.
- For any non-trivial change, read the implementation and the relevant tests before concluding.
- Verify the exact source when touching behavior, database logic, migrations, retries, fallbacks,
  recovery, or compatibility code.
- When the graph and the source disagree, the source wins. The graph may be stale or may not
  model that relationship.
- An empty graph result can mean "not indexed" or "not statically visible", not "does not exist".

### Key Tools

| Tool | Use when |
| ------ | ---------- |
| `detect_changes_tool` | Reviewing code changes — gives risk-scored analysis |
| `get_review_context_tool` | Need source snippets for review — token-efficient |
| `get_impact_radius_tool` | Understanding blast radius of a change |
| `get_affected_flows_tool` | Finding which execution paths are impacted |
| `query_graph_tool` | Tracing callers, callees, imports, tests, dependencies |
| `semantic_search_nodes_tool` | Finding functions/classes by name or keyword |
| `get_architecture_overview_tool` | Understanding high-level codebase structure |
| `refactor_tool` | Planning renames, finding dead code |
<!-- /code-review-graph MCP tools -->
