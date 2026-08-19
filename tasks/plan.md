# Implementation Plan: riff v0.2.0 — Roadmap Execution

## Overview

This plan covers all recommended improvements from `docs/product/roadmap.md` plus completion of the partial UI surfaces marked in `docs/product/features.md`. It is organized into five phases ordered by dependency: infrastructure first (CI + tests), then new P1 features, then UI polish and accessibility, then cache hardening and partial-feature completion, and finally P2 advanced features.

The plan is optimized for AI-agent execution: each task has explicit acceptance criteria, verification commands, dependency annotations, and file-level scope. An AI agent should be able to pick up any individual task, implement it, verify it, and mark it done without additional context.

## Architecture Reference

- **Domain layer** (`src/domain/`): pure business logic, zero external crate imports
- **Application layer** (`src/app/`): use cases, state (`AppState`), port traits (`AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`), commands
- **Infrastructure layer** (`src/infra/`): trait implementations using symphonia, cpal, lofty, image, walkdir, notify
- **Presentation layer** (`src/ui/`): egui widgets, tray, settings, fonts
- **Composition root** (`src/main.rs`): wires everything together
- **Test suite**: `tests/` directory (5 suites, ~25 tests — **does not currently compile**)
- **Commands**: `cargo test` (test), `cargo check` (type-check), `cargo build --release` (release build), `cargo run` (dev run)

## Key Facts from Current State

1. **Tests do not compile** (confirmed in `docs/engineering/testing-strategy.md`). Tests use `crate::` paths from integration-test location but riff has no `src/lib.rs`.
2. **No CI pipeline** exists — no `cargo fmt --check`, `cargo clippy`, or `cargo test` run automatically.
3. **Player control bar already has a Stop button** (⏹ icon at `app.rs:297`). The known gap is the **mute toggle** (REQ-UI-003-08).
4. **Now Playing view** already shows large cover art (300x300) and full metadata fields. Missing: clickable up-next rows that "play next" (not "jump to"), and an in-view seekable progress bar.
5. **System tray** already works for minimize/restore and playback context menu on macOS/Windows. Remaining gap: configurable close-to-tray behavior.
6. **Cover art display** works in library and Now Playing views; needs sizing/layout polish.

---

## Phase 0: Foundation — Test Suite & CI

**Goal:** Make the existing test suite compile and run, then automate it with CI. This is the highest-leverage investment because it protects every subsequent phase.

### Task 0.1: Make the test suite compile and run

**Description:** The test suite in `tests/` references `crate::domain`, `crate::app`, `crate::ui` paths that do not resolve because riff is a binary-only crate (no `src/lib.rs`). The recommended approach from the testing strategy is to add a thin `src/lib.rs` that re-exports the modules. Fix any additional compilation errors (the strategy notes at least one moved-value bug).

**Acceptance criteria:**
- [ ] `src/lib.rs` exists and re-exports `domain`, `app`, `infra`, `ui` modules
- [ ] All five test suites compile: `domain_tests`, `app_tests`, `infra_tests`, `ui_tests`, `integration_tests`
- [ ] `cargo test --no-run` succeeds without errors
- [ ] `cargo test` runs all ~25 tests and they pass
- [ ] No existing test behavior is changed (only fixes to make them runnable)

**Verification:**
```bash
cargo test --no-run   # must compile
cargo test             # all tests pass
```

**Dependencies:** None

**Files likely touched:**
- `src/lib.rs` (new)
- `tests/mod.rs` (import path fixes)
- `tests/domain_tests.rs` (potential moved-value fix)
- `tests/app_tests.rs`, `tests/infra_tests.rs`, `tests/ui_tests.rs`, `tests/integration_tests.rs` (import path fixes)

**Estimated scope:** S (1-2 files)

---

### Task 0.2: Add GitHub Actions CI pipeline

**Description:** Create a GitHub Actions workflow that runs `cargo fmt --check`, `cargo clippy`, and `cargo test` on Linux (Ubuntu) and Windows. macOS can be deferred to Phase 3 since it shares the same core code paths. The workflow should run on push to `main` and on pull requests.

**Acceptance criteria:**
- [ ] `.github/workflows/ci.yml` exists
- [ ] Runs on push to `main` and pull requests
- [ ] Linux runner executes: `cargo fmt --check && cargo clippy && cargo test`
- [ ] Windows runner executes: `cargo fmt --check && cargo clippy && cargo test`
- [ ] Matrix strategy covers at least Linux + Windows
- [ ] Workflow uses `actions/checkout` and `actions-rs/toolchain`
- [ ] Workflow passes in CI (green status badge optional)

**Verification:**
- [ ] `git push` triggers the workflow
- [ ] Workflow passes on the current codebase state

**Dependencies:** Task 0.1 (tests must compile first)

**Files likely touched:**
- `.github/workflows/ci.yml` (new)

**Estimated scope:** XS (1 file)

---

### Task 0.3: Expand domain layer unit test coverage

**Description:** The domain layer is pure Rust with no external dependencies — the cheapest place to add dense test coverage. The existing `domain_tests.rs` covers only 5 tests. Add tests for queue edge conditions and playback state transitions.

**Acceptance criteria:**
- [ ] `PlaybackQueue` tests cover: empty queue (next/previous return None), single-track queue (next wraps or returns None depending on repeat), wrap-around behavior, repeat-one mode, repeat-all mode, shuffle preserves multiset
- [ ] `PlaybackState` tests cover: all state transitions (Stopped → Playing → Paused → Stopped)
- [ ] `TrackId` tests cover: derivation from PathBuf, equality
- [ ] `RepeatMode` tests cover: cycle behavior (None → All → One → None)
- [ ] New tests compile and pass
- [ ] Total domain test count increases from ~5 to at least ~20

**Verification:**
```bash
cargo test domain_tests -- --nocapture
```

**Dependencies:** Task 0.1

**Files likely touched:**
- `tests/domain_tests.rs`
- `tests/test_utils.rs` (if helpers needed)

**Estimated scope:** M (2-3 files)

---

### Task 0.4: Expand LibraryManager test coverage

**Description:** The existing app tests touch search and cache round-tripping but leave much of `LibraryManager` untested. Add tests for track add/remove, artist/album indexing, multi-path handling, and cache corruption recovery.

**Acceptance criteria:**
- [ ] Tests cover: add_track indexing (track appears in all_tracks, artists, albums), remove_track eviction, multi-path deduplication, unavailable path handling
- [ ] Tests cover: search across all metadata fields, folder-based queries, case-insensitive search
- [ ] Tests cover: cache save/load round-trip with multiple paths, cache corruption fallback
- [ ] Tests use `tempfile` for temporary directories
- [ ] New tests compile and pass

**Verification:**
```bash
cargo test app_tests -- --nocapture
```

**Dependencies:** Task 0.1

**Files likely touched:**
- `tests/app_tests.rs`
- `tests/integration_helpers.rs`

**Estimated scope:** M (2-3 files)

---

### Checkpoint 0: Foundation
- [ ] All tests pass: `cargo test` succeeds
- [ ] CI workflow passes on push: green on GitHub
- [ ] Domain and app-layer coverage meaningfully expanded
- [ ] No regressions in existing behavior

---

## Phase 1: New P1 Features

**Goal:** Implement the four new P1 features from the roadmap. These are independent features that can be parallelized across agents.

### Task 1.1: Metadata tag writing

**Description:** Implement the ability to write metadata tags to audio files, initiated from a right-click "Edit Tags" context menu item on tracks. Tag editing opens a modal dialog with current values and save/cancel buttons, runs on a background thread, and updates the cache.

**Acceptance criteria (from REQ-ML-008):**
- [ ] Tag writing uses lofty's write capabilities
- [ ] Supported tags: title, artist, album, album artist, genre, year, track number
- [ ] "Edit Tags" appears in track context menu
- [ ] Modal dialog shows current values with editable fields and Save/Cancel buttons
- [ ] Tag writing runs on a background thread (channel-based)
- [ ] Cache is updated immediately after successful write
- [ ] Write never happens without explicit user confirmation (Save button)
- [ ] Graceful fallback on write errors (permission denied, disk full)
- [ ] File tags are source of truth; cache is derived view

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: right-click a track → "Edit Tags" opens modal with current values
- [ ] Manual: edit a tag → Save → file metadata changes on disk, cache updates

**Dependencies:** None (uses existing lofty dependency)

**Files likely touched:**
- `src/app/traits.rs` (new `MetadataWriter` trait)
- `src/infra/metadata_reader.rs` (extend to support writing, or new `metadata_writer.rs`)
- `src/app/commands.rs` (new `WriteTags` command)
- `src/ui/app.rs` (context menu item + modal dialog)
- `src/main.rs` (wire new trait impl)
- `tests/` (new tests for tag writing flow)

**Estimated scope:** L (5-6 files)

---

### Task 1.2: Offline discovery playlists

**Description:** Generate smart playlists from metadata: Recently Added (by file mtime), Most Played, Never Played, and Lost Gems (not played in 90+ days). Play counts are persisted in the library cache. Smart playlists appear in the library explorer alongside regular artists/albums.

**Acceptance criteria (from REQ-ML-009):**
- [ ] Smart playlists are generated without internet access
- [ ] Four built-in playlists: Recently Added, Most Played, Never Played, Lost Gems
- [ ] Play count tracking stored in library cache, persists across restarts
- [ ] Play count increments when a track finishes playing (TrackEnded signal)
- [ ] Smart playlists accessible from library explorer
- [ ] Smart playlists are read-only (auto-generated, not editable)
- [ ] Smart playlists update automatically when play counts change or new tracks are added
- [ ] Smart playlists support Play, Play Next, Append to Queue
- [ ] Smart playlists excluded from library search results

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: library explorer shows smart playlist entries
- [ ] Manual: play tracks → Most Played updates
- [ ] Manual: add new track → Recently Added updates

**Dependencies:** None

**Files likely touched:**
- `src/domain/track.rs` (add play_count field or separate tracking)
- `src/app/library_manager.rs` (smart playlist generation logic)
- `src/app/state.rs` (smart playlist state in AppState)
- `src/ui/app.rs` (render smart playlists in library explorer)
- `src/main.rs` (play count tracking wiring)
- `tests/` (new tests)

**Estimated scope:** L (5-6 files)

---

### Task 1.3: Progressive disclosure UI

**Description:** Restructure the UI to default to minimal controls with advanced features behind explicit toggles. Settings organized by frequency of use. Contextual help for advanced features.

**Acceptance criteria (from REQ-UI-006):**
- [ ] UI defaults to minimal controls: play/pause, library, search
- [ ] Advanced features (tag editing, smart playlist rules, equalizer placeholder) hidden behind explicit toggle
- [ ] Settings page organized by frequency of use (common options easily discoverable)
- [ ] No empty or placeholder states for unconfigured features
- [ ] Contextual tooltips for advanced features on first access
- [ ] Clear visual path from simple to advanced usage

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: app opens with minimal control set
- [ ] Manual: advanced features revealed via toggle
- [ ] Manual: settings organized logically

**Dependencies:** Tasks 1.1, 1.2 (progressive disclosure should include toggles for tag editing and smart playlists)

**Files likely touched:**
- `src/ui/app.rs` (main layout, toggle state)
- `src/ui/settings.rs` (settings organization)
- `src/app/state.rs` (disclosure state fields)
- `tests/` (UI tests)

**Estimated scope:** M (3-4 files)

---

### Task 1.4: Linux first-class support

**Description:** Treat Linux as equal to Windows/macOS. Provide a polished folder picker experience even without native dialogs (current Linux version uses plain text input). Document platform-specific limitations. Ensure reliability on GNOME, KDE, XFCE.

**Acceptance criteria (from REQ-SI-002-09 through -12):**
- [ ] Linux folder picker is polished: clear input field with browse button that opens a directory tree view (or improved text input with autocomplete/path validation)
- [ ] Platform-specific limitations documented in settings/about dialog
- [ ] All core features work reliably on Linux (scan, play, cover art, search)
- [ ] Linux build passes `cargo test` with the same results as other platforms
- [ ] No `#[cfg(target_os = "linux")]` code introduces regressions

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] CI Linux runner green
- [ ] Manual: Linux folder picker is usable
- [ ] Manual: limitations documented in-app

**Dependencies:** None

**Files likely touched:**
- `src/ui/settings.rs` (Linux folder picker UI)
- `src/ui/app.rs` (about/settings documentation)
- `src/main.rs` (Linux-specific initialization)
- `Cargo.toml` (conditional dependencies for Linux)

**Estimated scope:** M (2-4 files)

---

### Task 1.5: Accessibility support

**Description:** Implement full keyboard navigation, high-contrast theme, screen-reader-friendly labels, and cognitive load reduction. This addresses the research finding that all major players ignore accessibility.

**Acceptance criteria (from REQ-UI-007):**
- [ ] Entire UI navigable via keyboard (Tab, Enter, Arrow keys, Escape)
- [ ] All interactive elements have visible focus indicators
- [ ] High-contrast theme option in settings (in addition to existing light/dark)
- [ ] Text readable at standard UI scaling (verify at 150% scaling)
- [ ] Screen-reader-friendly labels on all interactive elements
- [ ] No rapid animations or flashing elements
- [ ] Clear visual hierarchy
- [ ] Error messages are specific and actionable

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: navigate entire app with keyboard only
- [ ] Manual: high-contrast theme works
- [ ] Manual: focus indicators visible

**Dependencies:** Task 1.3 (progressive disclosure changes may affect keyboard navigation)

**Files likely touched:**
- `src/ui/fonts.rs` (font sizing for accessibility)
- `src/ui/app.rs` (keyboard navigation, focus indicators, high-contrast theme)
- `src/ui/settings.rs` (high-contrast toggle)
- `src/app/state.rs` (accessibility state fields)

**Estimated scope:** M (3-4 files)

---

### Checkpoint 1: New Features
- [ ] All new P1 features compile and pass tests
- [ ] Tag writing works end-to-end
- [ ] Smart playlists appear in library explorer and update on play
- [ ] UI defaults to minimal, advanced features toggleable
- [ ] Linux support is first-class
- [ ] Keyboard navigation covers the entire app
- [ ] CI pipeline passes

---

## Phase 2: UI Polish & Partial Feature Completion

**Goal:** Complete the remaining partial features and polish the UI surfaces.

### Task 2.1: Add mute toggle to player control bar

**Description:** Add a mute button to the player control bar that mutes/unmutes audio while preserving the previous volume level for restore.

**Acceptance criteria (from REQ-UI-003-08):**
- [ ] Control bar shows a mute toggle button (🔇/🔊 icon)
- [ ] Muting sets volume to 0
- [ ] Unmuting restores the previous volume level
- [ ] Mute state persists across track changes
- [ ] Mute state is independent of the volume slider (muting doesn't change slider position, or slider reflects muted state)

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: mute button toggles audio on/off, restores volume

**Dependencies:** None

**Files likely touched:**
- `src/ui/app.rs` (mute button in control bar, mute state handling)
- `src/app/state.rs` (muted volume field)

**Estimated scope:** XS (1-2 files)

---

### Task 2.2: Complete Now Playing view — clickable up-next & in-view progress bar

**Description:** The Now Playing view already shows large cover art and full metadata. Add: clickable up-next rows that queue the clicked track as "next" (using PlayNext), and an in-view seekable progress bar.

**Acceptance criteria (from REQ-UI-005):**
- [ ] Clicking an upcoming-queue row sends `PlaybackCommand::PlayNext` for that track
- [ ] In-view seekable progress bar appears in the Now Playing view
- [ ] Progress bar shows elapsed/total time in MM:SS format
- [ ] Clicking the progress bar seeks to that position
- [ ] Seeking past the end clamps to the end
- [ ] Progress bar updates continuously during playback
- [ ] "Nothing playing" empty state works

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: click up-next row → that track plays next
- [ ] Manual: click progress bar → seeks to position

**Dependencies:** None

**Files likely touched:**
- `src/ui/app.rs` (Now Playing view: clickable rows, progress bar)

**Estimated scope:** S (1 file)

---

### Task 2.3: Complete system tray — close-to-tray behavior

**Description:** Finalize the system tray integration so that closing the main window minimizes to tray on macOS/Windows (REQ-SI-001-13), and the tray context menu is fully functional with all specified items.

**Acceptance criteria (from REQ-SI-001):**
- [ ] On macOS/Windows, closing main window minimizes to tray (doesn't quit)
- [ ] Tray tooltip shows "Artist - Title"
- [ ] Left-click toggles window visibility
- [ ] Right-click menu: Play/Pause, Next Track, Previous Track, Show Window, Quit
- [ ] Quit from tray stops playback and exits app
- [ ] Playback continues with window hidden
- [ ] On Linux, closing window quits the app (no tray)

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual (macOS/Windows): close window → app in tray, playback continues
- [ ] Manual: tray right-click menu has all five items

**Dependencies:** None

**Files likely touched:**
- `src/ui/tray.rs` (tray behavior, menu items)
- `src/ui/app.rs` (window close handling → tray minimize)
- `src/main.rs` (viewport command for close-to-tray)

**Estimated scope:** S (2-3 files)

---

### Task 2.4: Cover art display polish

**Description:** Polish cover art sizing and layout across all views. Ensure consistent sizing, proper aspect ratio handling, and graceful placeholder behavior.

**Acceptance criteria (from REQ-UI-004):**
- [ ] Cover art displays in all views where tracks are shown
- [ ] Uses the same resolution pipeline and LRU cache (50 entries, max)
- [ ] Placeholder shown when no cover art available
- [ ] Sizing consistent across library view and Now Playing view
- [ ] Oversized images don't break layout (clamped/resized)

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: cover art displays correctly in all views
- [ ] Manual: placeholder shown when no cover art

**Dependencies:** None

**Files likely touched:**
- `src/ui/app.rs` (cover art sizing/layout in library and Now Playing views)

**Estimated scope:** S (1 file)

---

### Checkpoint 2: UI Polish
- [ ] Mute toggle works in control bar
- [ ] Now Playing view has clickable up-next and progress bar
- [ ] System tray: close-to-tray works, all menu items present
- [ ] Cover art display polished across all views
- [ ] All tests pass, CI green

---

## Phase 3: Cache Hardening & Robustness

**Goal:** Add robustness improvements to the library cache and settings.

### Task 3.1: Cache schema versioning

**Description:** Add a version field to the library cache JSON. When the data model changes in the future, older caches can be detected and handled deliberately (migrate or explain) rather than silently falling back to empty.

**Acceptance criteria (from roadmap recommendation):**
- [ ] `library_cache.json` includes a `schema_version` field
- [ ] On load, cache is checked against expected version
- [ ] If version mismatch, a warning is logged and cache is treated as incompatible (current fallback to empty library behavior preserved)
- [ ] The version number is incremented in code whenever the cache data model changes
- [ ] Version mismatch produces a user-visible explanation (in settings or log) rather than silent failure

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: inspect cache file, confirm version field present
- [ ] Manual: create cache with wrong version → graceful fallback

**Dependencies:** None

**Files likely touched:**
- `src/app/library_manager.rs` (cache save/load with version)
- `tests/app_tests.rs` (cache version tests)

**Estimated scope:** S (1-2 files)

---

### Task 3.2: "Clear cache" control in settings

**Description:** Add a button in the settings page that deletes the library cache file, forcing a full rescan on next scan. Paairs with cache schema versioning.

**Acceptance criteria:**
- [ ] Settings page has a "Clear Library Cache" button
- [ ] Clicking the button deletes `library_cache.json`
- [ ] Confirmation dialog or undo not required (cache is rebuildable)
- [ ] UI shows clear feedback (label changes or toast)
- [ ] Cache file path is shown or documented

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: click Clear Cache → cache file deleted

**Dependencies:** Task 3.1 (pairs naturally)

**Files likely touched:**
- `src/ui/settings.rs` (Clear Cache button + handler)

**Estimated scope:** XS (1 file)

---

### Task 3.3: Expand infra-layer tests with trait-based mocks

**Description:** Convert the construction-only infrastructure smoke tests into meaningful behavior tests using mock implementations of the port traits (`AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`). This allows testing the app layer through the trait boundaries.

**Acceptance criteria:**
- [ ] Mock implementations of `AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader` exist
- [ ] `infra_tests.rs` exercises actual behavior through trait interfaces (not just construction)
- [ ] Mocks allow controlled error injection for error-path testing
- [ ] New tests compile and pass
- [ ] At least 5 new meaningful behavior tests

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test infra_tests -- --nocapture`

**Dependencies:** Task 0.1 (tests must compile)

**Files likely touched:**
- `tests/infra_tests.rs`
- `tests/mod.rs` (mock implementations)

**Estimated scope:** M (2-3 files)

---

### Checkpoint 3: Robustness
- [ ] Cache has schema version
- [ ] Clear cache button in settings
- [ ] Infra-layer tests use trait-based mocks
- [ ] All tests pass, CI green

---

## Phase 4: Advanced Features (P2)

**Goal:** Implement the deferred P2 features from the roadmap. These are larger, higher-risk items that should land on top of a stable, well-tested foundation.

### Task 4.1: Gapless playback

**Description:** Implement seamless transitions between consecutive album tracks. The next track must be decoded and staged in the buffer before the current track ends, eliminating the silence gap.

**Acceptance criteria (from deferred requirements):**
- [ ] No audible silence between consecutive tracks from the same album
- [ ] Next track is pre-decoded and buffered before current track ends
- [ ] Works for all supported formats (MP3, FLAC, AAC, Opus, WAV, OGG Vorbis)
- [ ] Does not affect shuffled or non-consecutive playback
- [ ] Memory usage remains bounded (does not grow unbounded with queue size)
- [ ] Track-end detection is reliable (no premature or late handoff)

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: play album tracks → no gap between consecutive tracks
- [ ] Manual: memory profile stable during long playback

**Dependencies:** All previous phases (needs stable, tested audio engine)

**Files likely touched:**
- `src/infra/decoder.rs` (pre-decode next track, cross-track buffering)
- `src/infra/audio_output.rs` (seamless buffer handoff)
- `src/app/commands.rs` (new commands for gapless mode)
- `src/app/state.rs` (gapless state tracking)
- `src/main.rs` (thread coordination for pre-decoding)

**Estimated scope:** XL (6-8 files) — this is the largest single task. Consider breaking into sub-tasks: (a) pre-decode infrastructure, (b) buffer handoff logic, (c) state tracking and integration.

---

### Task 4.2: Custom playlist management

**Description:** Enable users to create, save, and load named playlists of tracks. Playlists are persisted to disk and loadable back into the queue. Handle file path changes gracefully.

**Acceptance criteria (from deferred requirements):**
- [ ] User can create a named playlist containing an ordered list of tracks
- [ ] Playlists persisted to disk and loaded on next launch
- [ ] User can load a playlist into the queue
- [ ] When a track's file path changes or file is deleted, playlist entry is marked invalid (not crash)
- [ ] Playlists accessible from library explorer
- [ ] Playlists support rename and delete
- [ ] Playlist entries show current/invalid status

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: create playlist → save → restart → playlist loads
- [ ] Manual: move a track file → playlist entry shows invalid

**Dependencies:** Tasks 1.2 (offline discovery playlists establishes the pattern)

**Files likely touched:**
- `src/domain/` (Playlist entity, PlaylistId)
- `src/app/library_manager.rs` (playlist persistence, load, save)
- `src/app/state.rs` (playlist state in AppState)
- `src/ui/app.rs` (playlist UI in library explorer)
- `src/ui/settings.rs` (playlist management in settings)
- `tests/` (playlist tests)

**Estimated scope:** L (6-7 files)

---

### Task 4.3: ReplayGain normalization

**Description:** Read existing ReplayGain tags from audio metadata and apply gain adjustment in the volume-scaling step. For tracks without ReplayGain tags, no adjustment is applied.

**Acceptance criteria (from deferred requirements):**
- [ ] riff reads ReplayGain tags (REPLAYGAIN_TRACK_GAIN, REPLAYGAIN_TRACK_PEAK) from audio metadata via lofty
- [ ] Where ReplayGain tags exist, gain is applied in the same volume-scaling step the engine already uses
- [ ] For tracks without ReplayGain tags, no gain adjustment applied
- [ ] ReplayGain does not cause clipping (peak normalization applied)
- [ ] User can enable/disable ReplayGain in settings

**Verification:**
- [ ] `cargo check` succeeds
- [ ] `cargo test` passes
- [ ] Manual: play tracks with/without ReplayGain tags → consistent loudness
- [ ] Manual: ReplayGain toggle in settings works

**Dependencies:** None (uses existing lofty dependency)

**Files likely touched:**
- `src/infra/metadata_reader.rs` (read ReplayGain tags)
- `src/infra/decoder.rs` (apply gain to decoded samples)
- `src/app/state.rs` (ReplayGain enabled state)
- `src/ui/settings.rs` (ReplayGain toggle)
- `tests/` (ReplayGain tests)

**Estimated scope:** M (4-5 files)

---

### Checkpoint 4: Complete
- [ ] All phases complete
- [ ] Gapless playback works for consecutive album tracks
- [ ] Custom playlists create, save, load, and handle invalid paths
- [ ] ReplayGain normalizes loudness from existing tags
- [ ] All tests pass
- [ ] CI pipeline green
- [ ] `cargo build --release` succeeds on all platforms

---

## Parallelization Opportunities

### Safe to parallelize (independent):
| Tasks | Reason |
|-------|--------|
| 0.3, 0.4 | Both are test expansion, no shared files |
| 1.1, 1.2, 1.4 | Independent features, different code areas |
| 2.1, 2.3, 2.4 | Independent UI polish, different files |
| 4.2, 4.3 | Independent P2 features |

### Must be sequential:
| Chain | Reason |
|-------|--------|
| 0.1 → 0.2 | CI needs compilable tests |
| 0.1 → 0.3, 0.4 | Test expansion needs working test harness |
| 1.1, 1.2 → 1.3 | Progressive disclosure should include toggles for tag editing and smart playlists |
| All Phases → 4.1 | Gapless playback needs stable, tested audio engine |

### Recommended agent dispatch strategy:
1. **Phase 0**: Single agent for 0.1 (test compile fix), then 0.2 (CI). Then parallel: agent A for 0.3 (domain tests), agent B for 0.4 (LibraryManager tests).
2. **Phase 1**: Three parallel agents: (a) 1.1 (tag writing), (b) 1.2 (discovery playlists), (c) 1.4 (Linux support). Then 1.3 (progressive disclosure) depends on 1.1/1.2. Then 1.5 (accessibility) depends on 1.3.
3. **Phase 2**: Parallel: 2.1, 2.2, 2.3, 2.4 (all independent UI polish).
4. **Phase 3**: 3.1 then 3.2 (sequential). 3.3 can run in parallel with 3.2.
5. **Phase 4**: 4.1, 4.2, 4.3 can all run in parallel since they touch different subsystems.

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Tests still don't compile after Task 0.1 | High — blocks CI and all future testing | Add `lib.rs` approach is well-documented; if that fails, move tests inline as `#[cfg(test)]` modules |
| Gapless playback introduces audio glitches | High — core user experience | Gate behind a toggle; extensive manual testing with diverse formats; start with a single format before expanding |
| Trait-based mocks too complex for current architecture | Medium — delays Phase 3 | Start with minimal mocks (one method each); expand iteratively |
| CI flakiness from platform-specific issues | Medium — CI noise | Pin Rust toolchain; use `-j1` for audio tests to avoid resource contention |
| Play count persistence adds cache bloat | Low — negligible (4 bytes per track) | Play count is part of cache, not a separate file |

---

## Open Questions

1. **Gapless playback approach**: Should pre-decoding happen in the same audio thread (simpler, less risk) or a separate thread (better responsiveness)? The current decode loop is single-threaded per track.
2. **Playlist storage format**: JSON file alongside the library cache, or embedded in the library cache? Separate files make playlists shareable; embedded keeps the data model unified.
3. **Accessibility scope**: Should high-contrast theme be a third theme alongside light/dark, or an overlay modifier? This affects how egui themes are composed.
4. **Linux folder picker**: Should we add a native file dialog dependency for Linux (e.g., `dialog` crate), or build a custom directory tree widget? Custom widget keeps Linux dependency-free but requires more UI work.
