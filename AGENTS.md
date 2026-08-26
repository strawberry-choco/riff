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

Threads are spawned with `std::thread::spawn`:

- **Main thread** — egui event loop. Must not block.
- **Audio engine thread** — decode + output loop. Reads `PlaybackCommand` from channel, sends `PlaybackUpdate` back.
- **Update processor thread** — receives `PlaybackUpdate` from engine, writes to shared `Arc<Mutex<AppState>>`, drives auto-advance on track end.
- **Library scan thread** — scans filesystem with `walkdir`, sends `LibraryUpdate` back.
- **Cover loader thread** — decodes cover images in background, sends result via channel.
- **Tag-write worker thread** — writes tag edits via lofty in background, sends result back.

Cross-thread communication: `crossbeam_channel::unbounded()` for all message passing. Shared state via `Arc<Mutex<AppState>>`. There is also an `Arc<AtomicBool>` cancel flag for library scans.

## Platform-Specific Code

- **macOS / Windows**: System tray icon (`tray-icon` + `muda`), native folder picker (`rfd`).
- **Linux**: No tray icon (no-op). Folder picker is a text input field (no native file dialog). Conditional via `#[cfg(target_os = "linux")]` / `#[cfg(not(target_os = "linux"))]`.

## Commands (Dev Workflow)

```bash
cargo fmt                                  # format
cargo check --all-targets                  # type/borrow checking across all targets
cargo clippy --all-targets -- -D warnings  # lint with warnings as errors
cargo test --all-targets                   # run all unit and integration tests
cargo run                                  # run in dev mode
cargo build --release                      # release build (LTO, stripped)
```

**Test suite**: a single integration crate rooted at `tests/mod.rs` (`autotests = false`, one `[[test]]` target named `integration`), organized into `domain_tests`, `app_tests`, `infra_tests`, `ui_tests`, `golden_tests`, `integration_tests` with shared `test_utils`/`mocks`/`integration_helpers`. Run with `cargo test`. No inline `#[cfg(test)]` modules in `src/`. See `docs/engineering/testing-strategy.md`; golden-image snapshot workflow in `docs/engineering/golden-image-testing.md`.

**CI**: `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test --all-targets` on push/PR to `main` (Linux + Windows matrix). No pre-commit hooks.

## State Persistence

- Library: the authoritative indexed collection lives in the Application Store (`riff.sqlite3`, same data-local dir) — artists, albums identified by `(album artist, title)`, and tracks keyed by path, with strict foreign keys and a derived lowercased `search_text` column. Scans commit in ~10-track batches (an interrupted scan keeps committed batches) with a store-backed freshness filter, and the scan thread never touches `AppState`. There is no in-memory mirror: every view reads the store through bounded Session Projections invalidated by a session-local generation counter (bumped inside the mutation adapter on each committed mutation). Per-track play history (`play_count`, `last_played`, `date_added`) lives in the store's `tracks` table. The legacy `library_cache.json` is never read or written.
- Playlists: user data in the Application Store (`riff.sqlite3`, same data-local dir) via the `PlaylistStore` port — every mutation commits as one immediate durable transaction. The legacy `playlists.json` is never read or written.
- Library paths and watch states: persisted in the Application Store's typed settings tables (the former `eframe::Storage` path was removed).
- TrackId: string key derived from `PathBuf::to_string_lossy()` — track identity is its full file path.

## Important Gotchas

- **msrv**: `rust-version = "1.95"` in Cargo.toml (edition 2021). CI uses the stable toolchain.
- **egui pinned to 0.35**: egui 0.36 regressed headless texture rendering — kittest golden snapshots lose all user-loaded textures (`ctx.load_texture` + painter/image widgets paint nothing; text/shapes still render). The app itself renders fine windowed, but goldens would bake in icon-less UI. Revisit when upgrading past 0.35 (check upstream fix status first).
- **Release profile**: LTO, codegen-units=1, strip=true. `cargo build --release` takes longer but produces smaller binaries.
- **Audio device**: Falls back to device default sample rate if the track's rate is unsupported (common on Windows WASAPI shared mode at 48 kHz).
- **Tests live in `tests/`** — one integration crate rooted at `tests/mod.rs` (per-suite files are modules of it, not separate crates). App-layer tests drive the port traits via the shared mocks module; store tests run against real SQLite in `tempfile` scratch dirs at the infra seam.
- **`AppState`** is a single large struct behind `Arc<Mutex<>>` — contains library, queue, playback, theme, UI state all together. Plan lock ordering carefully to avoid deadlocks (current code only uses one Mutex for AppState, no nested locking).
- **Cover art LRU**: Max 50 cached textures in `cover_textures` HashMap with manual LRU eviction in `cover_lru_keys` Vec.
- **No DI framework** — manual constructor injection in `main.rs` only.
- **Buffer management**: `SymphoniaDecoder` buffers oversize decoded packets in `pending_samples`. The `CpalAudioOutput` uses a `VecDeque<f32>` ring buffer shared between producer (decode loop) and consumer (cpal callback).

## Config Files

`clippy.toml` configures Clippy (msrv, tool-level options). Lint levels are set in `Cargo.toml` under `[lints.clippy]` (pedantic with selected allowances). CI config is `.github/workflows/ci.yml`; no `rustfmt.toml` (defaults apply). Architecture rules live in `docs/technical/architecture.md`. Feature requirements live in `docs/product/requirements.md`, statuses in `docs/product/features.md`, per-surface specs in `docs/product/specs/`. The full documentation index is in `docs/README.md`.

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

### Workflow

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes_tool` for code review.
3. Use `get_affected_flows_tool` to understand impact.
4. Use `query_graph_tool` pattern="tests_for" to check coverage.
<!-- /code-review-graph MCP tools -->
>>>>>>> b8b0d6a (use sqlite)
