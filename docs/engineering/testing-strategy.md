# Testing Strategy

This document describes how riff is tested today and where the test suite should go next. It is split into two clearly labeled parts: **Current State**, which documents the verified reality of the repository, and **Recommendations**, which are suggestions for improvement that are not yet implemented. For the commands used to run tests, see [development-setup.md](./development-setup.md).

riff is tested by a single integration-test crate rooted at `tests/mod.rs` (declared in `Cargo.toml` as one `[[test]]` target named `integration` with `autotests = false`), organized into one suite per architectural layer plus an integration suite, running green under `cargo test`. The production module tree lives in the library crate (`src/lib.rs`), which the test crate references through `riff::` re-exports. The `tempfile` crate is declared as a dev-dependency for tests that need a scratch directory.

## Current State

`tests/mod.rs` declares the six suites and provides shared helpers. The suite currently contains 301 `#[test]` functions: 36 in `domain_tests.rs`, 70 in `app_tests.rs`, 68 in `infra_tests.rs`, 115 in `ui_tests.rs`, 8 in `golden_tests.rs`, and 4 in `integration_tests.rs`.

### Build status

The suite compiles and runs green: `cargo test --all-targets` builds the library, the binary, and the `integration` test crate and executes all 301 tests (0 failed, 0 ignored). `cargo fmt --check` and `cargo clippy --all-targets` (pedantic, `-D warnings` in CI) are part of the same quality gate.

This was not always the case: the original test files referenced application internals through `crate::` paths while riff was a binary-only crate, so nothing under `tests/` could compile. The resolution was path (a) from the old P0 recommendations: the module tree moved into a thin library crate (`src/lib.rs` re-exporting `domain`, `app`, `infra`, `ui`), the binary became a wrapper over it, and the tests were wired into a single integration crate with shared `test_utils`/`mocks`/`integration_helpers` modules. The CI workflow (`.github/workflows/ci.yml`) now runs the full gate on Linux and Windows runners.

### Test organization

`tests/mod.rs` is the single crate root. Beyond declaring the six suite modules, it provides three helper modules:

- `test_utils` — factory functions `create_test_track`, `create_test_track_with_metadata`, and `float_close` (approximate `f32` comparison for audio-parameter assertions).
- `mocks` — scripted implementations of the port traits from `src/app/traits.rs` (`MockAudioDecoder`, `MockAudioOutput`, `MockMetadataReader`, `MockCoverLoader`) so app-layer behavior is tested at the seams without real audio hardware or media files.
- `integration_helpers` — `create_test_app_state` (an `Arc<Mutex<AppState>>`).

Suite modules bring these into scope with `use super::*` and refer to production code through the crate-root re-exports.

### What each suite contains

The table summarizes the verified contents of each suite.

| Suite | Test count | What it actually covers |
|---|---|---|
| `domain_tests.rs` | 36 | `PlaybackQueue` edge cases (empty queue, single track, ordered advance to the end, wrap-around with repeat-all incl. single-track wrap, repeat-one stopping, previous at the boundaries, shuffle multiset preservation, clear, upcoming), repeat-mode cycling, `TrackId` derivation/equality/hashing, `PlaybackState` distinction, playlist id slugging, smart-playlist kinds, metadata display/search helpers. |
| `app_tests.rs` | 70 | Scan-side Track construction (`build_tracks` with a mock reader: metadata/format round-trip, skipping unreadable files, date-added stamping), playlist entry validity over store-resolved entries (real file, scanned-but-missing, dangling), tag-edit apply semantics + a mock `MetadataWriter`, `ReplayGain` factor math, gapless helpers, mute/effective volume, mutex recovery, and all five Session Projections (bounded windows, browsing/folder/smart-playlist caches, and the playback-side projection: queue-change and generation invalidation, skip-unresolved ids, stale-cache-on-error, selected-track caching). |
| `infra_tests.rs` | 68 | The Application Store at the port seam over real SQLite in tempfile databases: migrations (apply/reopen idempotently, checksum tampering rejected), corruption recovery (quick_check probe, rename-aside with nanosecond suffixes, fresh DB), settings/playlists round-trips across restarts, playlist entries with SQL LEFT JOIN validity flags (dangling refs stay listed, flip invalid on removal), canonical `all_track_ids` ordering, scan batches committing incrementally, tag refresh preserving history, Clear Library (curation preserved, atomic rollback on a simulated mid-clear failure), browsing/folder/smart-playlist SQL parity against independent Rust reference oracles, plus port-seam behavior through the shared mocks and construction smoke tests for the real adapters. |
| `ui_tests.rs` | 115 | First-frame restore through the real ports (settings, playlists; legacy JSON ignored — the library collection needs no hydration and is read live from the store), settings round-trips across simulated restarts over real SQLite, playlist mutations committing through the store and patching their projection, high-contrast visuals, seek clamping, duration formatting, tilde expansion, directory autocomplete, Now Playing actions, and the cover-texture LRU bound. |
| `golden_tests.rs` | 8 | Golden-image snapshot tests: render real egui frames headlessly through `egui_kittest` and pin them pixel-for-pixel against committed baselines under `tests/snapshots/` (dark-palette Play card, library hero, track list, sidebar, shell chrome, Now Playing, settings, playerbar). Authoring, re-baselining, and diff-review workflow in [golden-image-testing.md](./golden-image-testing.md). |
| `integration_tests.rs` | 4 | `AppState` mutex safety across threads; playback command channel send/receive; a simulated library scan updating state; and a simulated audio buffer write/read. |

A few observations about the current coverage, stated neutrally:

- The `infra_tests.rs` suite exercises the real infrastructure types at the store port seam over real SQLite; decoding real audio, reading real tags, and loading real images would need sample media files, which are not checked in. Behavior at the media ports is covered through the mocks instead.
- Several integration tests are simulations of the real flows (scan, audio buffer) rather than end-to-end runs through the actual threads and channels; persistence flows, by contrast, run against real SQLite.
- The `domain_tests.rs` suite includes an `AppState` test even though `AppState` lives in the application layer, so the suite boundaries are pragmatic rather than strict.

Commands: run everything with `cargo test`, a single suite with `cargo test domain_tests`, and see output with `cargo test -- --nocapture`.

## Recommendations

The following are suggestions for further strengthening the test suite and the surrounding automation. The former P0 items (make the suite compile, stand up CI, expand domain coverage, deepen store-query coverage) are done and are now documented under Current State; the remaining items are prioritized to guide the next round of work.

### P1 — Medium priority

- **Add real integration tests for scan-to-play and cover resolution.** Today these flows are simulated. Where feasible, check in tiny sample audio files (or generate them in a test fixture) so that scanning, metadata extraction, and cover resolution can be exercised end to end.
- **Use the port traits to mock infrastructure in app-layer tests.** The `AudioDecoder`, `AudioOutput`, `MetadataReader`, and `CoverLoader` traits exist precisely so the application layer can be tested without `symphonia` or `cpal`. Mock implementations now exist in `tests/mod.rs`; extend their use to drive more app flows through them.
- **Add property tests for queue shuffle.** Shuffle uses `rand`; property-based tests (for example with `proptest`) can assert invariants such as "shuffle preserves the multiset of tracks" and "shuffle does not drop or duplicate entries" across many random seeds. The multiset invariant is already covered by a deterministic test.
- **Measure coverage.** Introduce `cargo-tarpaulin` or `cargo-llvm-cov` to quantify coverage and highlight untested paths, and report it in CI.

### Suggested next steps, in order

1. Add sample-media fixtures and real scan/cover/metadata integration tests.
2. Extend the port mocks to cover more app-layer flows (e.g. queue advancement driven by the update-processor logic).
3. Add property-based shuffle tests.
4. Wire in coverage reporting.
