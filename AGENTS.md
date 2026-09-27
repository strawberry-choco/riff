# riff — Music Player (Rust + egui)

A lightweight, offline-first desktop music player. A Cargo workspace: backend capability crates, a frontend crate, and an integration-test crate. See `docs/technical/architecture.md` for the split.

## Quick Start

```bash
cargo run -p riff-gui              # dev build of the `riff` binary (opt-level=1)
cargo build --release -p riff-gui  # LTO, stripped, optimized release
cargo check --workspace            # fast type-check without codegen
```

No special features or feature flags. No codegen step, no migrations to run by hand.

## Architecture

A vertical capability split with a strict, compiler-enforced dependency chain (full reference in `docs/technical/architecture.md`, decision record in `docs/adr/0009-vertical-crate-split-of-the-backend.md`):

```
riff-gui (frontend, `riff` binary)
    -> riff-backend (application API + Composition Root)
        -> riff-infra (adapters + native deps)
            -> riff-library / riff-playback -> riff-persistence
```

Each crate's contents and membership criterion, one line each; `docs/technical/architecture.md#crate-definitions-and-membership-criteria` is the full reference.

- **`riff-persistence`** — types that cross the persistence boundary, and shared `std`-only utilities that belong to no single capability; the standing case is `MutexExt` (poison recovery), defined once in `riff-persistence/src/sync.rs` and re-exported as `riff_backend::app::MutexExt`.
- **`riff-library`** — the collection capability: collection use cases and the ports they consume. Sibling of `riff-playback` — no edge between them.
- **`riff-playback`** — the playback capability: playback use cases and the ports they consume.
- **`riff-infra`** — an item belongs here iff it implements a port defined in another crate or wraps a native/external dependency — nothing else.
- **`riff-backend`** — the frontend-facing event surface, the app-layer application services, and the one place that knows both ports and adapters (the Composition Root).
- **`riff-gui`** — the frontend: rendering, input, and platform integration, plus the `riff` binary, which is a thin composition over `riff_backend::composition::AppRuntime::spawn`. `crates/riff-gui/src/ui/theme.rs` is the design system's single store: every design value is written there and read from there, enforced by source sweeps (ADR 0004; rules in `docs/engineering/coding-standards.md#design-tokens`).

Domain types (`Track`, `TrackId`, `PlaybackQueue`, …) live in `riff-persistence` and `riff-playback` and import nothing from app, infra, or UI code. Each slice codes against its own port traits; `riff-infra` implements them; dependency arrows point adapters → slices.

Inside the slices, the layering is preserved as module convention: `domain/` (pure types), `app/` (use cases, session state, projections), `infra/` (port traits the adapter crate implements).

## Threading Model

Worker threads are spawned and joined by the Composition Root (`crates/riff-backend/src/composition.rs`): `AppRuntime::spawn` returns `(AppRuntime, RuntimeLifecycle)` — the handles the frontend renders with, and the worker threads plus the flags that end them. `RuntimeLifecycle::shutdown` is explicit and idempotent, and joins in dependency order (audio engine first, since its exit disconnects the coordinator). The thread inventory — one entry per worker, with the channels between them — lives in `docs/technical/threading-model.md`.

Cross-thread communication: `crossbeam_channel::unbounded()` for all message passing. Shared state: `Arc<Mutex<PlaybackSession>>` and `Arc<Mutex<LibrarySession>>` (one mutex per session — never nested), `Arc<Mutex<BackendEvents>>`, an `Arc<AtomicBool>` cancel flag for library scans, one `Arc<AtomicBool>` stop flag per request-channel worker (engine, scan, tag-edit, cover), and a quit flag. The audio ring buffer between decode loop and cpal callback lives inside `riff-infra`'s output adapter.

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

**Test suite**: Per-crate suites sit with the code they cover; cross-crate integration, UI and golden suites sit in the single workspace-root `riff-tests` crate. `docs/engineering/testing-strategy.md` is authoritative on placement, and the golden-image snapshot workflow is in `docs/engineering/golden-image-testing.md`.

**CI**: `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test --all-targets` on push/PR to `master` (Linux + Windows matrix). No pre-commit hooks.

## State Persistence

- Library: the authoritative indexed collection lives in the Application Store (`riff.sqlite3`, same data-local dir) — artists, albums identified by `(album artist, title)`, and tracks keyed by path, with strict foreign keys and a derived lowercased `search_text` column. Scans commit in ~10-track batches (an interrupted scan keeps committed batches) with a store-backed freshness filter. There is no in-memory mirror: every view reads the store through bounded Session Projections invalidated by a session-local generation counter (bumped inside the store's mutation impls on each committed mutation). Per-track play history (`play_count`, `last_played`, `date_added`) lives in the store's `tracks` table. The legacy `library_cache.json` is never read or written.
- Playlists: user data in the Application Store via the `PlaylistStore` port — every mutation commits as one immediate durable transaction. The legacy `playlists.json` is never read or written.
- Library paths and watch states: persisted in the Application Store's typed settings tables.
- TrackId: string key derived from `PathBuf::to_string_lossy()` — track identity is its full file path.

## Important Gotchas

- **msrv**: `rust-version = "1.95"` in every crate manifest (edition 2024). CI uses the stable toolchain.
- **egui pinned to 0.35**: egui 0.36 regressed headless texture rendering — kittest golden snapshots lose all user-loaded textures (`ctx.load_texture` + painter/image widgets paint nothing; text/shapes still render). The app itself renders fine windowed, but goldens would bake in icon-less UI. Revisit when upgrading past 0.35 (check upstream fix status first). See the note in `crates/riff-gui/Cargo.toml`.
- **Release profile**: workspace-level LTO, codegen-units=1, strip=true. Profiles and the release process live in `docs/engineering/release-and-packaging.md`.
- **Audio device**: The output stream always opens at the **device default** sample rate. `build_stream_config` (in `riff-infra`'s `audio_output.rs`) takes the track's requested rate as `_requested_rate` and never reads it, so the "falls back when the track's rate is unsupported" behaviour (common on Windows WASAPI shared mode at 48 kHz) is really "always the default" — the requested rate is not consulted. The rate actually achieved is readable only as a concrete `pub fn CpalAudioOutput::effective_sample_rate`, which **no production code calls**; it is *not* a method on the `AudioOutput` port, so the Audio Engine cannot see it through its interface. Wiring the fact through the port belongs in the same change as the consumer that reads it, never on its own.
- **Session state is two structs**: `PlaybackSession` (`riff-playback`) and `LibrarySession` (`riff-backend`), each behind its own `Arc<Mutex<>>`. Plan lock ordering carefully; never hold one session's lock while acquiring the other's. The one cross-slice interaction (a playback failure setting a scan-status message) is a typed notice through the event inbox, not a state write.
- **Cover caches**: two in RAM and one on disk, and they are not interchangeable. The egui texture LRU (up to `COVER_CACHE_CAP` = 200 `TextureHandle`s **and** `COVER_TEXTURE_BYTE_BUDGET` = 64 MB of RGBA, order tracked in `cover_lru_keys`, shared placeholder tile exempt from both the accounting and eviction) lives in `crates/riff-gui/src/ui/app.rs` and dies with the process. The two bounds are not redundant: the canonical boxes span 80x in bytes, so 200 entries is 2.4 MB of thumbnails or ~200 MB of heroes. The Cover Service (`riff-library`) keeps **no decoded-cover cache**: its only cache is `negative`, an artless-verdict LRU of the same cap. What survives a restart is the **Thumbnail cache** — encoded rungs under `<data_local_dir>/covers/`, keyed on the cover *Source's* path with its `(mtime, len)` carried in the filename, so there is no index to keep in sync and no automatic eviction; Settings → Library → "Clear Thumbnail cache" is the only reclaim, and it runs on the cover worker and reports a polled outcome like Tag Edit. There is deliberately no content hash (reading the sources to decide what to re-decode defeats the point) and no decoded-pixel cache in RAM.
- **No DI framework** — manual constructor injection in `crates/riff-backend/src/composition.rs` only.
- **Buffer management**: `SymphoniaDecoder` (`riff-infra`) buffers oversize decoded packets in `pending_samples`. `CpalAudioOutput` uses a lock-free SPSC ring buffer (`ringbuf`) between the producer (decode loop) and the consumer (cpal callback).

## Config Files

`clippy.toml` configures Clippy (msrv, tool-level options). Lint levels are set in the root `Cargo.toml` under `[workspace.lints.clippy]` (pedantic with selected allowances) and inherited by every crate via `[lints] workspace = true`. CI config is `.github/workflows/ci.yml`; no `rustfmt.toml` (defaults apply). Architecture rules live in `docs/technical/architecture.md`. Feature statuses live in `docs/product/features.md`. The full documentation index is in `docs/README.md`.

## Agent skills

### Issue tracker

Issues and specs are local markdown under `.scratch/<feature-slug>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

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
