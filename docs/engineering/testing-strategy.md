# Testing Strategy

This document describes how riff is tested today and where the test suite should go next. It is split into two clearly labeled parts: **Current State**, which documents the verified reality of the repository, and **Recommendations**, which are suggestions for improvement that are not yet implemented. For the commands used to run tests, see [development-setup.md](./development-setup.md).

riff is tested by a single integration-test crate rooted at `tests/mod.rs` (declared in `Cargo.toml` as one `[[test]]` target named `integration` with `autotests = false`), organized into one suite per architectural layer plus an integration suite, running green under `cargo test`. The production module tree lives in the library crate (`src/lib.rs`), which the test crate references through `riff::` re-exports. The `tempfile` crate is declared as a dev-dependency for tests that need a scratch directory.

## Current State

`tests/mod.rs` declares the five suites and provides shared helpers. The suite currently contains 155 `#[test]` functions: 39 in `domain_tests.rs`, 71 in `app_tests.rs`, 16 in `infra_tests.rs`, 24 in `ui_tests.rs`, and 5 in `integration_tests.rs`.

### Build status

The suite compiles and runs green: `cargo test --all-targets` builds the library, the binary, and the `integration` test crate and executes all 155 tests (0 failed, 0 ignored). `cargo fmt --check` and `cargo clippy --all-targets` (pedantic, `-D warnings` in CI) are part of the same quality gate.

This was not always the case: the original test files referenced application internals through `crate::` paths while riff was a binary-only crate, so nothing under `tests/` could compile. The resolution was path (a) from the old P0 recommendations: the module tree moved into a thin library crate (`src/lib.rs` re-exporting `domain`, `app`, `infra`, `ui`), the binary became a wrapper over it, and the tests were wired into a single integration crate with shared `test_utils`/`mocks`/`integration_helpers` modules. The CI workflow (`.github/workflows/ci.yml`) now runs the full gate on Linux and Windows runners.

### Test organization

`tests/mod.rs` is the single crate root. Beyond declaring the five suite modules, it provides three helper modules:

- `test_utils` — factory functions `create_test_track`, `create_test_track_with_metadata`, and `float_close` (approximate `f32` comparison for audio-parameter assertions).
- `mocks` — scripted implementations of the port traits from `src/app/traits.rs` (`MockAudioDecoder`, `MockAudioOutput`, `MockMetadataReader`, `MockCoverLoader`) so app-layer behavior is tested at the seams without real audio hardware or media files.
- `integration_helpers` — `create_test_app_state` (an `Arc<Mutex<AppState>>`) and `create_mock_library` (a `LibraryManager` pre-populated with two tracks).

Suite modules bring these into scope with `use super::*` and refer to production code through the crate-root re-exports.

### What each suite contains

The table summarizes the verified contents of each suite.

| Suite | Test count | What it actually covers |
|---|---|---|
| `domain_tests.rs` | 39 | `PlaybackQueue` edge cases (empty queue, single track, ordered advance to the end, wrap-around with repeat-all incl. single-track wrap, repeat-one stopping, previous at the boundaries, shuffle multiset preservation, clear, upcoming), repeat-mode cycling, `TrackId` derivation/equality/hashing, `PlaybackState` distinction, playlists serde, smart-playlist kinds, track play-history serde, metadata display/search helpers. |
| `app_tests.rs` | 71 | `LibraryManager` add/remove indexing (tracks/artists/albums, track-number sorting, album-artist keys, dedup incl. across separate scans), removal + orphan cleanup, removal by root, folder queries against real tempdirs, search across all metadata fields case-insensitively, cache save/load round-trips (serde level and versioned-envelope level), corrupt/wrong-version/malformed cache fallback, scan with a mock reader (dedup, skipping unreadable files, unavailable paths, date-added stamping), play counts, all four smart playlists, playlists CRUD + persistence, tag edits + a mock `MetadataWriter`, `ReplayGain` factor math, gapless helpers, mute/effective volume, mutex recovery. |
| `infra_tests.rs` | 16 | Port-seam behavior through the shared mocks (scripted decoder open/frames/EOF/seek/errors, recording output buffer/errors, canned metadata reader, cover loader results), `ReplayGain` gain-string parsing, plus construction smoke tests for the real `SymphoniaDecoder`, `LoftyMetadataReader`, `ImageCoverLoader`, `AudioFileScanner`, and `FilesystemWatcher`. |
| `ui_tests.rs` | 24 | Settings persistence round-trips (library paths, volume, advanced mode, high contrast, `ReplayGain`, watch states) against a local `MockStorage` implementing `eframe::Storage`, backup restoration on corruption, high-contrast visuals, seek clamping, duration formatting, tilde expansion, and directory autocomplete. |
| `integration_tests.rs` | 5 | `AppState` mutex safety across threads; playback command channel send/receive; a simulated library scan updating state; a simulated settings-persistence round-trip; and a simulated audio buffer write/read. |

A few observations about the current coverage, stated neutrally:

- The `infra_tests.rs` suite exercises the real infrastructure types only for construction and pure parsing; decoding real audio, reading real tags, and loading real images would need sample media files, which are not checked in. Behavior at the ports is covered through the mocks instead.
- Several integration tests are simulations of the real flows (scan, persistence, audio buffer) rather than end-to-end runs through the actual threads and channels.
- The `domain_tests.rs` suite includes an `AppState` test even though `AppState` lives in the application layer, so the suite boundaries are pragmatic rather than strict.

Commands: run everything with `cargo test`, a single suite with `cargo test domain_tests`, and see output with `cargo test -- --nocapture`.

## Recommendations

The following are suggestions for further strengthening the test suite and the surrounding automation. The former P0 items (make the suite compile, stand up CI, expand domain coverage, deepen `LibraryManager` coverage) are done and are now documented under Current State; the remaining items are prioritized to guide the next round of work.

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
