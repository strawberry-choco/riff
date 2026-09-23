# Testing Strategy

This document describes how riff is tested today and where the test suite should go next. It is split into two clearly labeled parts: **Current State**, which documents the verified reality of the repository, and **Recommendations**, which are suggestions for improvement that are not yet implemented. For the commands used to run tests, see [development-setup.md](./development-setup.md).

riff is a Cargo workspace, and its tests live at two levels, mirroring the crate split (ADR 0009):

- **Per-crate suites**, placed with the code they cover. `riff-infra` hosts its own integration-test crate (`riff-infra/tests/mod.rs`, `autotests = false`, one `[[test]]` target named `integration`) for the real-SQLite store tests and the adapter tests. `riff-persistence` has a `tests/` suite for the generation-keyed cache primitive its read path rides on, and `riff-persistence`, `riff-library`, `riff-playback`, and `riff-backend` additionally carry inline `#[cfg(test)]` modules beside the pure logic they pin. `riff-gui` has no in-crate suite; its behavior is exercised through the workspace-root suite's UI and golden seams.
- **A single workspace-root integration crate** (`tests/`, package `riff-tests`, `autotests = false`, one `[[test]]` target named `integration`) that holds the cross-crate integration, UI, and golden-image suites and runs green under `cargo test`. It depends on every workspace crate by name (per-crate imports) so each suite reaches the type it needs directly, and it provides the shared `test_utils`/`mocks`/`integration_helpers` modules. The `tempfile` crate is a dev-dependency for tests that need a scratch directory.

## Current State

The workspace currently contains 742 `#[test]` functions: 570 in the workspace-root suite (41 in `domain_tests.rs`, 181 in `app_tests.rs`, 8 in `infra_tests.rs`, 251 in `ui_tests.rs`, 73 in `golden_tests.rs`, 16 in `integration_tests.rs`), 128 in the `riff-infra` suite (99 in `store_tests.rs`, 29 in `adapter_tests.rs`), and 44 across the per-crate suites — `riff-playback` (28), `riff-backend` (9), `riff-persistence` (5, of which 3 in its `tests/` suite), and `riff-library` (2).

### Build status

The suites compile and run green: `cargo test --all-targets` builds every workspace crate and all three integration-test targets (the workspace root, `riff-infra`, and `riff-persistence`) and executes all 742 tests (0 failed, 0 ignored). `cargo fmt --check` and `cargo clippy --all-targets` (pedantic, `-D warnings` in CI) are part of the same quality gate, run on Linux and Windows runners by CI (`.github/workflows/ci.yml`). One exception is known and open: `riff-infra`'s `adapter_tests::test_filesystem_watcher_forwards_debounced_audio_batches` is timing-sensitive and intermittently fails under whole-suite load while passing in isolation.

### The `riff-infra` suite (adapters live with their tests)

`riff-infra/tests/mod.rs` mirrors the workspace-root crate's layout (single crate root, module suites, a prelude of re-exports) and hosts the tests that moved with the adapter crate during the backend crate split:

- `store_tests.rs` (99 tests) — the Application Store at the port seam over real SQLite in tempfile databases: migrations (apply/reopen idempotency, checksum tampering rejected), corruption recovery (quick_check probe, rename-aside, fresh DB), settings/playlists round-trips across restarts, playlist entries with SQL LEFT JOIN validity flags, canonical `all_track_ids` ordering, scan batches committing incrementally, tag refresh preserving history, Clear Library (curation preserved, atomic rollback), browsing/folder/smart-playlist SQL parity against independent Rust reference oracles, and the one narrow Listing Page test the façade cannot show: a page read holds a single connection acquisition, exercised against a concurrent writer.
- `adapter_tests.rs` (29 tests) — real lofty tag round-trips on scratch files, construction smoke tests for the decoder/output/scanner/watcher adapters, and ReplayGain tag parsing.

### The workspace-root suite (cross-crate behavior lives in one place)

`tests/mod.rs` is the single crate root. Beyond declaring the six suite modules, it provides three helper modules:

- `test_utils` — factory functions `create_test_track`, `create_test_track_with_metadata`, and `float_close` (approximate `f32` comparison for audio-parameter assertions).
- `mocks` — scripted implementations of the port traits (`MockAudioDecoder`, `MockAudioOutput`, `MockMetadataReader`, `MockCoverLoader`, `MockMetadataWriter`, `MockTransport`, and store fakes) so app-layer behavior is tested at the seams without real audio hardware or media files. Mocks implement the ports through the `riff-backend` re-export surface. `MockLibraryQueryStore` is the hand-written fake the Session Views suites drive the façade through: every query serves its canned field and records one `LibraryQueryCall`, so a Listing Page read — total and window in one store read — appears in the call log exactly once; failure injection still names the store halves a page composes (`FailingQuery::ArtistsCount`, `ArtistsWindow`, …) and makes that single read return `Err`. Three of the mocks are the service front ends the app shell polls — `MockScans` (a scriptable outcome queue), `MockTagEdits`, and `MockCovers` — and `MockSettingsStore` can additionally record into a caller-held `Arc<Mutex<_>>` via `with_shared_calls`, which is how a test reads the log back after the shell has taken ownership of the store.
- `integration_helpers` — paired `PlaybackSession`/`LibrarySession` test fixtures.

Suite modules bring these into scope with `use super::*` and refer to production code through per-crate imports (`riff_backend::`, `riff_infra::`, `riff_library::`, `riff_gui::`).

| Suite | Test count | What it actually covers |
|---|---|---|
| `domain_tests.rs` | 41 | `PlaybackQueue` edge cases (empty queue, single track, ordered advance to the end, wrap-around with repeat-all incl. single-track wrap, repeat-one stopping, previous at the boundaries, shuffle multiset preservation, clear, upcoming), repeat-mode cycling, `TrackId` derivation/equality/hashing, session defaults, playlist id slugging, smart-playlist kinds, metadata display/search helpers. |
| `app_tests.rs` | 181 | The two session structs and `replaygain_factor` math, gapless eligibility/handoff and frame/duration math, scan-side Track construction (`build_tracks` with a mock reader), the Library Scan Service end to end over real stores (batching, cancellation keeping committed batches, idempotent rescans, failure surfacing), the Tag Edit service outcomes (write/commit failures, play-history preservation), the Cover service (resolution, negative caching, duplicate coalescing, LRU eviction), the Session Views seam over the store ports (bounded windows served as Listing Pages, browsing/folder/smart-playlist and playback-side projections, generation invalidation, stale-cache-on-error, and one page read per listing per frame), the playlist projection and reorder rules, the Listing Page coherence tests that drive the same façade over a real Application Store in a scratch directory, and the Playback Coordinator (history committed before advancing, repeat-one replay, stop at the end, typed error notices). |
| `infra_tests.rs` | 8 | Port-seam boundary behavior driven through the shared mocks: decoder open/decode/seek/EOF scripting, output write/volume/buffer semantics, metadata-reader failure injection, and cover-loader result handling. (The real-SQLite and real-adapter tests live in the `riff-infra` suite.) |
| `ui_tests.rs` | 251 | First-frame restore through the real ports (settings, playlists; legacy JSON ignored — the library collection needs no hydration and is read live from the store), settings round-trips across simulated restarts over real SQLite, playlist mutations committing through the store and patching their projection, high-contrast visuals, seek clamping, duration formatting, tilde expansion, directory autocomplete, Now Playing actions, the cover-texture LRU bound, and the whole-frame suite below. |
| `golden_tests.rs` | 73 | Golden-image snapshot tests: render real egui frames headlessly through `egui_kittest` and pin them pixel-for-pixel against the 71 committed baselines under `tests/snapshots/` — the browser/detail/empty columns in dark, light and high contrast, the dark-palette Play card, the library hero and track list, Now Playing with and without a cover, the queue and selection panels, the titlebar search field, the playerbar matrix, settings, sidebar and shell chrome, the folder tree, the elastic window-size matrix with drilled Artists/Albums/Genres columns and inspector, the icon atlas, and the type/spacing scales. Authoring, re-baselining, and diff-review workflow in [golden-image-testing.md](./golden-image-testing.md). |
| `integration_tests.rs` | 16 | `MutexExt` poison recovery; playback command channel round-trip; a real scan driven through the `ScanService` seam end to end; `WatcherManager` debounce/rescan behavior across burst, deferred, and unwatchable-path scenarios; an audio-buffer write/read simulation; and the Composition Root end-to-end test: `AppRuntime::spawn` wires the real `riff-infra` adapters into the slice-defined ports and the worker threads run. |

A few observations about the current coverage, stated neutrally:

- The real-infrastructure tests live in `riff-infra/tests/` because that is where the adapters live; the root suite reaches real adapters only through the Composition Root test and the UI restore tests.
- The root `infra_tests.rs` suite is mock-driven by design: it pins the port contracts the app layer codes against, while the `riff-infra` suite pins what the real adapters do.
- Decoding real audio end to end would need sample media files, which are not checked in; behavior at the media ports is covered through the mocks and the lofty round-trip tests.

Commands: run everything with `cargo test --all-targets`; run one crate's suite with `cargo test -p riff-infra` or `cargo test -p riff-tests`; run one module with `cargo test domain_tests`; see output with `cargo test -- --nocapture`.

### Whole-frame tests (the app shell, headlessly)

`ui_tests.rs` also carries `whole_frame_tests`, the suite that drives the *real* app shell through its `eframe::App` interface rather than testing a view function in isolation. This is the seam for anything whose contract is the frame loop itself — what one frame does, in order, to state a test can observe.

- **Harness.** `egui_kittest` with its `eframe` feature, via `HarnessBuilder::build_eframe`, which calls the app's `logic` then its `ui` for every step. The default `LazyRenderer` means no wgpu device and no window, so these tests carry none of the golden suite's GPU cost or flakiness.
- **Construction.** `RiffApp::new_for_test` (`#[doc(hidden)]`) fills in what the platform normally supplies — no tray icon, an empty watcher handle, a fresh quit flag, a real visibility channel — and *delegates* to the production constructor so the two cannot drift field-for-field.
- **Timing.** The harness runs the frame body twice inside `build_eframe` (one AccessKit warm-up frame, one settling step), so a test must mutate state after the harness exists and then `step()` once per frame it wants to observe. A selection applied at the end of a frame renders one frame later.
- **Seams asserted through.** The rendered output (accessibility tree), the mock call records, and the live sessions the app actually holds. Note that the titlebar's scan-status line is *painted*, not drawn as a widget, so it has no accessibility node: the session slot the painter reads is that contract's observable seam and is what the titlebar renders verbatim.
- **Prove the test can fail.** Each frame-loop assertion was verified by temporarily no-op'ing the production call it covers — for the playlist reorder test, making `commit_playlist_reorder` a self-drop — and confirming the test fails. A frame test that passes with the behaviour removed is the most expensive kind of green.

## Recommendations

The following are suggestions for further strengthening the test suite and the surrounding automation. The former P0 items (make the suite compile, stand up CI, expand domain coverage, deepen store-query coverage) are done and are now documented under Current State; the remaining items are prioritized to guide the next round of work.

### P1 — Medium priority

- **Keep growing the pure crates' own suites.** `riff-persistence`, `riff-library`, `riff-playback`, and `riff-backend` now carry in-crate tests, and `riff-persistence` has its own `tests/` suite; most slice logic is still covered from the root suite through the re-export surface. As a module's contract is purely internal, move (or add) its tests into the slice's own suite so they run without compiling the adapter stack — that compile isolation is one of the split's payoffs. Cross-crate integration, UI, and golden suites stay at the workspace root.
- **Add real integration tests for cover resolution over real image files.** Scan-to-play has a real end-to-end test; cover resolution against real JPEG/PNG fixtures on disk is still mock-based at the root.
- **Add property tests for queue shuffle.** Shuffle uses `fastrand`; property-based tests (for example with `proptest`) can assert invariants such as "shuffle preserves the multiset of tracks" and "shuffle does not drop or duplicate entries" across many random seeds. The multiset invariant is already covered by a deterministic test.
- **Measure coverage.** Introduce `cargo-llvm-cov` to quantify coverage and highlight untested paths, and report it in CI.

### Suggested next steps, in order

1. Keep moving slice-internal tests into the per-crate suites now seeded for the slices, keeping integration and goldens at the root.
2. Add sample-media fixtures for real cover/metadata integration tests.
3. Add property-based shuffle tests.
4. Wire in coverage reporting.
