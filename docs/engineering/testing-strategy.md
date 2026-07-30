# Testing Strategy

This document describes how riff is tested today and where the test suite should go next. It is split into two clearly labeled parts: **Current State**, which documents the verified reality of the repository, and **Recommendations**, which are suggestions for improvement that are not yet implemented. For the commands used to run tests, see [development-setup.md](./development-setup.md).

riff has test files. They live in the top-level `tests/` directory (not inline in `src/`), are organized into one suite per architectural layer plus an integration suite, and are intended to run under `cargo test`. The `tempfile` crate is declared as a dev-dependency for tests that need a scratch directory. Note the important caveat documented under Build Status below: as of this writing the suite does not compile, so `cargo test` does not yet run green.

## Current State

The `tests/` directory contains a `mod.rs` that declares the five suites and provides shared helpers, plus one file per suite. There are roughly 25 `#[test]` functions in total, all gated behind `#[cfg(test)]`.

### Build status

Verified by running `cargo test --no-run`: the suite does **not currently compile**. The test files sit in `tests/` (the integration-test location, where each file is its own crate root) but reference application internals through `crate::` paths and `use super::*`. riff is a binary-only crate with no `src/lib.rs`, so those `crate::domain`, `crate::app`, and `crate::ui` paths do not resolve from an integration test, producing unresolved-import and unresolved-path errors (E0432/E0433 and friends). At least one suite also has a straightforward moved-value bug (E0382). The net effect is that `cargo test` fails during compilation before any test executes.

This does not change the fact that the test code and its roughly 25 test functions exist and describe the intended coverage; it means making them runnable is the first order of business (see the P0 recommendations below). The descriptions of each suite in the next section reflect what the test code is written to verify.

### Test organization

`tests/mod.rs` is the entry point. Beyond declaring the five modules, it provides two helper modules:

- `test_utils` — factory functions `create_test_track` and `create_test_track_with_metadata` that build `domain::Track` values with default or custom metadata.
- `integration_helpers` — `create_test_app_state` (an `Arc<Mutex<AppState>>`) and `create_mock_library` (a `LibraryManager` pre-populated with two tracks).

### What each suite contains

The table summarizes the verified contents of each suite.

| Suite | Test count | What it actually covers |
|---|---|---|
| `domain_tests.rs` | 5 | `AppState::new` defaults; `PlaybackQueue` append/next/previous and current-index behavior; `PlaybackState` `Display`; `TrackId::from`; `Track` metadata display fallbacks (`display_artist`/`display_title`/`display_album`). |
| `app_tests.rs` | 5 | `MutexExt::lock_or_recover`; `LibraryManager::new` emptiness; `LibraryManager` add-track structure; `LibraryManager::search`; and save/load of the library cache using `tempfile` and the `RIFF_CACHE_PATH` environment variable. |
| `infra_tests.rs` | 5 | Construction-only smoke tests: `SymphoniaDecoder::new`, `LoftyMetadataReader::new`, `ImageCoverLoader::new`, `AudioFileScanner::new`, and `FilesystemWatcher::new`. |
| `ui_tests.rs` | 5 | Settings persistence round-trips (library paths, volume, watch states) and `restore_from_backup_if_corrupted`, all against a local `MockStorage` that implements `eframe::Storage`. |
| `integration_tests.rs` | 5 | `AppState` mutex safety across threads; playback command channel send/receive; a simulated library scan updating state; a simulated settings-persistence round-trip; and a simulated audio buffer write/read. |

A few observations about the current coverage, stated neutrally:

- The `infra_tests.rs` suite verifies only that infrastructure types can be constructed; it does not decode audio, read real metadata, or load real images, because doing so requires sample media files that are not checked in.
- Several integration tests are simulations of the real flows (scan, persistence, audio buffer) rather than end-to-end runs through the actual threads and channels.
- The `domain_tests.rs` suite includes an `AppState` test even though `AppState` lives in the application layer, so the suite boundaries are pragmatic rather than strict.

Once the suite compiles (see Build status and the first P0 item below), the intended commands are: run everything with `cargo test`, a single suite with `cargo test domain_tests`, and see output with `cargo test -- --nocapture`.

## Recommendations

The following are suggestions for strengthening the test suite and the surrounding automation. None of this is implemented yet; items are prioritized to guide the next round of work.

### P0 — High priority

- **Make the test suite compile and run.** This is the prerequisite for everything else. Two viable paths: (a) add a thin `src/lib.rs` that re-exports the `domain`, `app`, `infra`, and `ui` modules so integration tests under `tests/` can reference them via the crate name, or (b) move these tests inline as `#[cfg(test)]` unit modules inside `src/` where `crate::` paths resolve naturally. Either way, fix the unresolved imports and the moved-value bug so `cargo test` builds and runs green.
- **Add a CI pipeline.** There is currently no CI. A GitHub Actions workflow that runs `cargo fmt --check`, `cargo clippy`, and `cargo test` on a Windows / macOS / Linux matrix would catch regressions and platform-specific breakage (especially around `cpal` and the non-Linux tray/dialog dependencies) before they merge — and would have caught the current compile failure.
- **Expand domain unit coverage.** The domain layer is pure and dependency-free, which makes it cheap to test thoroughly. Add cases for queue edge conditions (empty queue, single track, wrap-around next/previous) and for shuffle/repeat behavior.
- **Deepen `LibraryManager` coverage.** The app suite touches search and cache round-tripping but leaves much of the manager untested. Add tests for adding/removing tracks, artist/album indexing, and multi-path handling.

### P1 — Medium priority

- **Add real integration tests for scan-to-play and cover resolution.** Today these flows are simulated. Where feasible, check in tiny sample audio files (or generate them in a test fixture) so that scanning, metadata extraction, and cover resolution can be exercised end to end.
- **Use the port traits to mock infrastructure in app-layer tests.** The `AudioDecoder`, `AudioOutput`, `MetadataReader`, and `CoverLoader` traits exist precisely so the application layer can be tested without `symphonia` or `cpal`. Add mock implementations and drive the app layer through them, turning the construction-only infra smoke tests into meaningful behavior tests at the boundary.
- **Add property tests for queue shuffle.** Shuffle uses `rand`; property-based tests (for example with `proptest`) can assert invariants such as "shuffle preserves the multiset of tracks" and "shuffle does not drop or duplicate entries" across many random seeds.
- **Measure coverage.** Introduce `cargo-tarpaulin` or `cargo-llvm-cov` to quantify coverage and highlight untested paths, and report it in CI once a pipeline exists.

### Suggested next steps, in order

1. Make the existing suite compile and pass (add a `lib.rs` re-export or inline the tests; fix the import and move errors).
2. Stand up the CI workflow (fmt + clippy + test, three-OS matrix) so the suite stays green.
3. Fill in domain-layer edge-case tests, since they are the cheapest wins.
4. Introduce trait-based mocks and convert key infra smoke tests into app-layer behavior tests.
5. Add sample-media fixtures and real scan/cover integration tests.
6. Wire in coverage reporting.
