# Testing Strategy

This document describes how riff is tested today and where the test suite should go next. It is split into two clearly labeled parts: **Current State**, which documents the verified reality of the repository, and **Recommendations**, which are suggestions for improvement that are not yet implemented. For the commands used to run tests, see [development-setup.md](./development-setup.md).

riff is a Cargo workspace, and its tests live at two levels, mirroring the crate split (ADR 0009):

- **Per-crate suites**, placed with the code they cover. Today that is `riff-infra`, which hosts its own integration-test crate (`riff-infra/tests/mod.rs`, `autotests = false`, one `[[test]]` target named `integration`) for the real-SQLite store tests and the adapter tests. The pure crates (`riff-persistence`, `riff-library`, `riff-playback`) and `riff-backend`/`riff-gui` currently have no in-crate suites; their behavior is exercised through the workspace-root suite at the seams.
- **A single workspace-root integration crate** (`tests/`, package `riff-tests`, `autotests = false`, one `[[test]]` target named `integration`) that holds the cross-crate integration, UI, and golden-image suites and runs green under `cargo test`. It depends on every workspace crate by name (per-crate imports) so each suite reaches the type it needs directly, and it provides the shared `test_utils`/`mocks`/`integration_helpers` modules. The `tempfile` crate is a dev-dependency for tests that need a scratch directory.

## Current State

The workspace currently contains 357 `#[test]` functions: 288 in the workspace-root suite (41 in `domain_tests.rs`, 93 in `app_tests.rs`, 8 in `infra_tests.rs`, 127 in `ui_tests.rs`, 8 in `golden_tests.rs`, 11 in `integration_tests.rs`) and 69 in the `riff-infra` suite (54 in `store_tests.rs`, 15 in `adapter_tests.rs`).

### Build status

The suites compile and run green: `cargo test --all-targets` builds every workspace crate and both integration-test targets and executes all 357 tests (0 failed, 0 ignored). `cargo fmt --check` and `cargo clippy --all-targets` (pedantic, `-D warnings` in CI) are part of the same quality gate, run on Linux and Windows runners by CI (`.github/workflows/ci.yml`).

### The `riff-infra` suite (adapters live with their tests)

`riff-infra/tests/mod.rs` mirrors the workspace-root crate's layout (single crate root, module suites, a prelude of re-exports) and hosts the tests that moved with the adapter crate during the backend crate split:

- `store_tests.rs` (54 tests) — the Application Store at the port seam over real SQLite in tempfile databases: migrations (apply/reopen idempotency, checksum tampering rejected), corruption recovery (quick_check probe, rename-aside, fresh DB), settings/playlists round-trips across restarts, playlist entries with SQL LEFT JOIN validity flags, canonical `all_track_ids` ordering, scan batches committing incrementally, tag refresh preserving history, Clear Library (curation preserved, atomic rollback), and browsing/folder/smart-playlist SQL parity against independent Rust reference oracles.
- `adapter_tests.rs` (15 tests) — real lofty tag round-trips on scratch files, construction smoke tests for the decoder/output/scanner/watcher adapters, and ReplayGain tag parsing.

### The workspace-root suite (cross-crate behavior lives in one place)

`tests/mod.rs` is the single crate root. Beyond declaring the six suite modules, it provides three helper modules:

- `test_utils` — factory functions `create_test_track`, `create_test_track_with_metadata`, and `float_close` (approximate `f32` comparison for audio-parameter assertions).
- `mocks` — scripted implementations of the port traits (`MockAudioDecoder`, `MockAudioOutput`, `MockMetadataReader`, `MockCoverLoader`, `MockMetadataWriter`, `MockTransport`, and store fakes) so app-layer behavior is tested at the seams without real audio hardware or media files. Mocks implement the ports through the `riff-backend` re-export surface.
- `integration_helpers` — paired `PlaybackSession`/`LibrarySession` test fixtures.

Suite modules bring these into scope with `use super::*` and refer to production code through per-crate imports (`riff_backend::`, `riff_infra::`, `riff_library::`, `riff_gui::`).

| Suite | Test count | What it actually covers |
|---|---|---|
| `domain_tests.rs` | 41 | `PlaybackQueue` edge cases (empty queue, single track, ordered advance to the end, wrap-around with repeat-all incl. single-track wrap, repeat-one stopping, previous at the boundaries, shuffle multiset preservation, clear, upcoming), repeat-mode cycling, `TrackId` derivation/equality/hashing, session defaults, playlist id slugging, smart-playlist kinds, metadata display/search helpers. |
| `app_tests.rs` | 93 | The two session structs and `replaygain_factor` math, gapless eligibility/handoff and frame/duration math, scan-side Track construction (`build_tracks` with a mock reader), the Library Scan Service end to end over real stores (batching, cancellation keeping committed batches, idempotent rescans, failure surfacing), the Tag Edit service outcomes (write/commit failures, play-history preservation), the Cover service (resolution, negative caching, duplicate coalescing, LRU eviction), the Session Views facade over the store ports (bounded windows, browsing/folder/smart-playlist and playback-side projections, generation invalidation, stale-cache-on-error), the playlist projection and reorder rules, and the Playback Coordinator (history committed before advancing, repeat-one replay, stop at the end, typed error notices). |
| `infra_tests.rs` | 8 | Port-seam boundary behavior driven through the shared mocks: decoder open/decode/seek/EOF scripting, output write/volume/buffer semantics, metadata-reader failure injection, and cover-loader result handling. (The real-SQLite and real-adapter tests live in the `riff-infra` suite.) |
| `ui_tests.rs` | 127 | First-frame restore through the real ports (settings, playlists; legacy JSON ignored — the library collection needs no hydration and is read live from the store), settings round-trips across simulated restarts over real SQLite, playlist mutations committing through the store and patching their projection, high-contrast visuals, seek clamping, duration formatting, tilde expansion, directory autocomplete, Now Playing actions, and the cover-texture LRU bound. |
| `golden_tests.rs` | 8 | Golden-image snapshot tests: render real egui frames headlessly through `egui_kittest` and pin them pixel-for-pixel against committed baselines under `tests/snapshots/` (dark-palette Play card, library hero, track list, sidebar, shell chrome, Now Playing, settings, playerbar). Authoring, re-baselining, and diff-review workflow in [golden-image-testing.md](./golden-image-testing.md). |
| `integration_tests.rs` | 11 | `MutexExt` poison recovery; playback command channel round-trip; a real scan driven through the `ScanService` seam end to end; `WatcherManager` debounce/rescan behavior across burst, deferred, and unwatchable-path scenarios; an audio-buffer write/read simulation; and the Composition Root end-to-end test: `AppRuntime::spawn` wires the real `riff-infra` adapters into the slice-defined ports and the worker threads run. |

A few observations about the current coverage, stated neutrally:

- The real-infrastructure tests live in `riff-infra/tests/` because that is where the adapters live; the root suite reaches real adapters only through the Composition Root test and the UI restore tests.
- The root `infra_tests.rs` suite is mock-driven by design: it pins the port contracts the app layer codes against, while the `riff-infra` suite pins what the real adapters do.
- Decoding real audio end to end would need sample media files, which are not checked in; behavior at the media ports is covered through the mocks and the lofty round-trip tests.

Commands: run everything with `cargo test --all-targets`; run one crate's suite with `cargo test -p riff-infra` or `cargo test -p riff-tests`; run one module with `cargo test domain_tests`; see output with `cargo test -- --nocapture`.

## Recommendations

The following are suggestions for further strengthening the test suite and the surrounding automation. The former P0 items (make the suite compile, stand up CI, expand domain coverage, deepen store-query coverage) are done and are now documented under Current State; the remaining items are prioritized to guide the next round of work.

### P1 — Medium priority

- **Give the pure crates their own suites as they earn them.** `riff-persistence`, `riff-library`, and `riff-playback` currently carry no in-crate tests; their logic is covered from the root suite through the re-export surface. As slice logic grows, move (or add) the pure logic tests into each slice's own suite so they run without compiling the adapter stack — that compile isolation is one of the split's payoffs. When they do, keep the cross-crate integration and golden suites at the workspace root.
- **Add real integration tests for cover resolution over real image files.** Scan-to-play has a real end-to-end test; cover resolution against real JPEG/PNG fixtures on disk is still mock-based at the root.
- **Add property tests for queue shuffle.** Shuffle uses `fastrand`; property-based tests (for example with `proptest`) can assert invariants such as "shuffle preserves the multiset of tracks" and "shuffle does not drop or duplicate entries" across many random seeds. The multiset invariant is already covered by a deterministic test.
- **Measure coverage.** Introduce `cargo-llvm-cov` to quantify coverage and highlight untested paths, and report it in CI.

### Suggested next steps, in order

1. Seed per-crate suites for the slices as their logic grows, keeping integration and goldens at the root.
2. Add sample-media fixtures for real cover/metadata integration tests.
3. Add property-based shuffle tests.
4. Wire in coverage reporting.
