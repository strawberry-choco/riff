# Testing Strategy

This document describes how riff is tested today and where the test suite should go next. It is split into two clearly labeled parts: **Current State**, which documents the verified reality of the repository, and **Recommendations**, which are suggestions for improvement that are not yet implemented. For the commands used to run tests, see [development-setup.md](./development-setup.md).

riff is a Cargo workspace, and its tests live at two levels, mirroring the crate split (ADR 0009):

- **Per-crate suites**, placed with the code they cover. `riff-infra` hosts its own integration-test crate (`riff-infra/tests/mod.rs`, `autotests = false`, one `[[test]]` target named `integration`) for the real-SQLite store tests and the adapter tests. `riff-persistence` has a `tests/` suite for the generation-keyed cache primitive its read path depends on, and `riff-persistence`, `riff-library`, `riff-playback`, and `riff-backend` also carry inline `#[cfg(test)]` modules beside the pure logic they pin. The frontend crate carries its own suite: `crates/riff-gui-nsh/tests/` holds the scaffold tests (the brand asset in the registry, `nsh.json`'s agreement with the bootstrap constants, the manifest fence, and the theme wiring on `Root`), the cover-decision-half probes (`cover_cache_tests.rs`), the four-variant palette resolution (`theme_tests.rs`), and the design-token sweeps (`token_sweeps.rs`).
- **A single workspace-root integration crate** (`tests/`, package `riff-tests`, `autotests = false`, one `[[test]]` target named `integration`) that holds the cross-crate integration, UI, and golden-image suites and runs green under `cargo test`. It depends on every workspace crate by name (per-crate imports) so each suite reaches the type it needs directly, and it provides the shared `test_utils`/`mocks`/`integration_helpers` modules. The `tempfile` crate is a dev-dependency for tests that need a scratch directory.

## Current State

### Build status

The suites compile and run green: `cargo test --all-targets` builds every workspace crate and its test targets. `cargo fmt --check` and `cargo clippy --all-targets` (pedantic, `-D warnings` in CI) are part of the same quality gate, run on Linux and Windows runners by CI (`.github/workflows/ci.yml`), as is the rustdoc build (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --document-private-items`), enforced on the CI Linux leg. CI also links every shipped binary with `cargo build --locked --workspace --bins`, which clippy and test do not do. Two `riff-infra` tests are known and open, both for reasons in the test rather than the product: `adapter_tests::test_filesystem_watcher_forwards_debounced_audio_batches` is timing-sensitive and intermittently fails under whole-suite load while passing in isolation, and `store_tests::a_page_read_never_mixes_generations_with_a_concurrent_writer` asserts `total == rows.len()` on a window whose `LIMIT` the concurrent writer can outgrow — so it fails whenever the writer reaches more rows than the page asked for, and passes only while it has not. Separately from any failing test, the `riff-tests` binary is subject to a known open `STATUS_ACCESS_VIOLATION` crash on Windows that kills the run before it prints a summary; [access-violation-flake.md](./access-violation-flake.md) records what has been ruled out and the next move.

### The `riff-infra` suite (adapters live with their tests)

`riff-infra/tests/mod.rs` mirrors the workspace-root crate's layout (single crate root, module suites, a prelude of re-exports) and hosts the tests that moved with the adapter crate during the backend crate split:

- `store_tests.rs` — the Application Store at the port seam over real SQLite in tempfile databases: migrations (apply/reopen idempotency, checksum tampering rejected), corruption recovery (quick_check probe, rename-aside, fresh DB), settings/playlists round-trips across restarts, playlist entries with SQL LEFT JOIN validity flags, canonical `all_track_ids` ordering, scan batches committing incrementally, tag refresh preserving history, Clear Library (curation preserved, atomic rollback), browsing/folder/smart-playlist SQL parity against independent Rust reference oracles, and the one narrow store-side test the façade cannot show: a `*_page` read holds a single connection acquisition, exercised against a concurrent writer. That is now a property of the store adapter alone — the seam reads a listing's count and its rows separately — so the test moved from being the seam's last guarantee to being the adapter's own.
- `adapter_tests.rs` — real lofty tag round-trips on scratch files, construction smoke tests for the decoder/output/scanner/watcher adapters, and ReplayGain tag parsing.

### The workspace-root suite (cross-crate behavior lives in one place)

`tests/mod.rs` is the single crate root. Beyond declaring the seven suite modules, it provides three helper modules:

- `test_utils` — factory functions `create_test_track`, `create_test_track_with_metadata`, and `float_close` (approximate `f32` comparison for audio-parameter assertions).
- `mocks` — scripted implementations of the port traits (`MockAudioDecoder`, `MockAudioOutput`, `MockMetadataReader`, `MockCoverLoader`, `MockMetadataWriter`, `MockTransport`, and store fakes) so app-layer behavior is tested at the seams without real audio hardware or media files. Mocks implement the ports through the `riff-backend` re-export surface. `MockLibraryQueryStore` is the hand-written fake the Session Views suites drive the façade through: every query serves its canned field and records one `LibraryQueryCall`. A paged listing reaches the same `*_page` port method **twice** when a surface asks for both halves — a count read and a row read, told apart by the `limit` each carried (0 for a count, a whole window for rows) — so `window_calls()` and `count_calls()` report them separately, and failure injection names each read independently (`FailingQuery::ArtistsCount` fails the count read while `ArtistsWindow` still serves rows, and the reverse). Three of the mocks are the service front ends the app shell polls — `MockScans` (a scriptable outcome queue), `MockTagEdits`, and `MockCovers` — and `MockSettingsStore` can also record into a caller-held `Arc<Mutex<_>>` via `with_shared_calls`, which is how a test reads the log back after the shell has taken ownership of the store.
- `integration_helpers` — paired `PlaybackSession`/`LibrarySession` test fixtures.

Suite modules bring these into scope with `use super::*` and refer to production code through per-crate imports (`riff_backend::`, `riff_infra::`, `riff_library::`, `riff_gui_nsh::`).

| Suite | What it covers |
|---|---|
| `domain_tests.rs` | `PlaybackQueue` edge cases (empty queue, single track, ordered advance to the end, wrap-around with repeat-all incl. single-track wrap, repeat-one stopping, previous at the boundaries, shuffle multiset preservation, clear, upcoming), repeat-mode cycling, `TrackId` derivation/equality/hashing, session defaults, playlist id slugging, smart-playlist kinds, metadata display/search helpers. |
| `app_tests.rs` | The two session structs and `replaygain_factor` math, gapless eligibility/handoff and frame/duration math, scan-side Track construction (`build_tracks` with a mock reader), the Library Scan Service end to end over real stores (batching, cancellation keeping committed batches, idempotent rescans, failure surfacing), the Tag Edit service outcomes (write/commit failures, play-history preservation), the Cover service (resolution, negative caching, duplicate coalescing, LRU eviction), the Session Views seam over the store ports (row-at-*i* and the count as two reads, **listing boundaries — a row at a window's first position, at its last, and the row just past a window — asserted at the seam against both the mock port and a real store**, browsing/folder/smart-playlist and playback-side projections, generation invalidation, stale-cache-on-error, and one count read plus one window per listing per frame), the playlist projection and reorder rules, the paged-listing tests that drive the same façade over a real Application Store in a scratch directory, and the Playback Coordinator (history committed before advancing, repeat-one replay, stop at the end, typed error notices). |
| `integration_tests.rs` | `MutexExt` poison recovery; playback command channel round-trip; a real scan driven through the `ScanService` seam end to end; `WatcherManager` debounce/rescan behavior across burst, deferred, and unwatchable-path scenarios; an audio-buffer write/read simulation; and the Composition Root end-to-end test: `AppRuntime::spawn` wires the real `riff-infra` adapters into the slice-defined ports and the worker threads run. |
| `nsh_bridge_tests.rs` | The bridge over the mock runtime: the pump's tick, the signal writes outside the `stored` borrow, playback/queue/cover/preferences round-trips, and the search retarget. |
| `nsh_shell_tests.rs` | The real shell mounted headless over the mock ports: sidebar counts and selection, the traversal ring, the list and prefetch window, the inspector, the browse stages, search, the queue panel, Now Playing, and the double-click play gesture. |
| `nsh_flow_tests.rs` | The end-to-end flows: browse, search, play, queue, playlist, tag edit, smart playlist, settings and library path. |
| `repo_invariant_tests.rs` | The Linux apt list is one fact in three places: the shared `.github/actions/linux-deps` action and both contributor guides parse to the same package names. Both workflows use the shared action and carry no inline `apt-get install`. |

A few observations about the current coverage, stated neutrally:

- The real-infrastructure tests live in `riff-infra/tests/` because that is where the adapters live; the root suite reaches real adapters only through the Composition Root test and the nsh settings round-trip tests.
- The root suite's port-seam tests are mock-driven by design: they pin the port contracts the app layer codes against, while the `riff-infra` suite pins what the real adapters do.
- Decoding real audio end to end would need sample media files, which are not checked in; behavior at the media ports is covered through the mocks and the lofty round-trip tests.

Commands: run everything with `cargo test --all-targets`; run one crate's suite with `cargo test -p riff-infra`, `cargo test -p riff-gui-nsh` or `cargo test -p riff-tests`; run one module with `cargo test domain_tests`; see output with `cargo test -- --nocapture`.

### Headless nsh tests (the app shell)

`tests/nsh_bridge_tests.rs` and `tests/nsh_shell_tests.rs` mount the real production tree (the pump, the shell, the stages) in `nsh_testing::Headless` over the mock ports. Selectors address `testID`s. The rules that keep them honest:

- **Desktop idiom.** `Headless::start` defaults to a 390x844 phone; a shell test sets `environment.os = Os::Windows` and a desktop viewport, or the sidebar and inspector leave no width for the stage.
- **Engine thread only.** Signals are `!Send`; every assertion reads the rendered screen, never a signal cross-thread. A pump write inside the `stored` borrow panics the engine thread and fails the suite.
- **Images never load headless**, so cover pixels are asserted from the decode path and the live screenshot, not the headless tree.
- **Prove the test can fail.** Each behavior test was observed failing with its production call removed or contradicted.

## Recommendations

The following are suggestions for further strengthening the test suite and the surrounding automation, prioritized to guide the next round of work.

### P1 — Medium priority

- **Keep growing the pure crates' own suites.** `riff-persistence`, `riff-library`, `riff-playback`, and `riff-backend` carry in-crate tests, and `riff-gui-nsh` carries suites of its own, so most slice logic still covered from the root suite through the re-export surface has a place to move to. As a module's contract is purely internal, move (or add) its tests into the slice's own suite so they run without compiling the adapter stack — that compile isolation is one of the split's payoffs. Cross-crate integration and the nsh bridge, shell and flow suites stay at the workspace root.
- **Add real integration tests for cover resolution over real image files.** Scan-to-play has a real end-to-end test; cover resolution against real JPEG/PNG fixtures on disk is still mock-based at the root.
- **Add property tests for queue shuffle.** Shuffle uses `fastrand`; property-based tests (for example with `proptest`) can assert invariants such as "shuffle preserves the multiset of tracks" and "shuffle does not drop or duplicate entries" across many random seeds. The multiset invariant is already covered by a deterministic test.
- **Measure coverage.** Introduce `cargo-llvm-cov` to quantify coverage and highlight untested paths, and report it in CI.
