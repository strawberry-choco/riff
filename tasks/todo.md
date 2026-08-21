# riff v0.2.0 — Execution Todo

> **Superseded — v0.2.0 shipped.** This execution todo is complete: every phase below shipped in v0.2.0. The canonical ticket set for the release is `.scratch/v0.2.0/issues/01`–`13`; per repo convention, those issues' **Status** fields are the completion markers, so the checkboxes here are kept as a historical record. Task-level boxes are ticked; leaf items remain as originally written, with *(not in shipped v0.2.0 scope)* marking the few sub-items whose scope changed on the way to release.

## Phase 0: Foundation — Test Suite & CI

- [x] **Task 0.1** — Make the test suite compile and run
  - [ ] Add `src/lib.rs` re-exporting domain, app, infra, ui
  - [ ] Fix import paths in all test suites
  - [ ] Fix moved-value bugs
  - [ ] `cargo test --no-run` succeeds
  - [ ] `cargo test` all tests pass

- [x] **Task 0.2** — Add GitHub Actions CI pipeline
  - [ ] Create `.github/workflows/ci.yml`
  - [ ] Linux runner: fmt + clippy + test
  - [ ] Windows runner: fmt + clippy + test
  - [ ] Runs on push to main and PRs

- [x] **Task 0.3** — Expand domain layer unit test coverage
  - [ ] PlaybackQueue edge cases (empty, single, wrap-around, repeat modes, shuffle)
  - [ ] PlaybackState transitions
  - [ ] TrackId derivation and equality
  - [ ] RepeatMode cycle behavior
  - [ ] Total domain tests: ~20+

- [x] **Task 0.4** — Expand LibraryManager test coverage
  - [ ] Add_track indexing
  - [ ] Remove_track eviction
  - [ ] Multi-path deduplication
  - [ ] Unavailable path handling
  - [ ] Search across all fields
  - [ ] Cache save/load/corruption

- [x] **Checkpoint 0** — Foundation
  - [ ] All tests pass
  - [ ] CI pipeline green
  - [ ] Domain and app coverage expanded

---

## Phase 1: New P1 Features

- [x] **Task 1.1** — Metadata tag writing
  - [ ] Add `MetadataWriter` trait
  - [ ] Implement tag writing via lofty
  - [ ] "Edit Tags" context menu item
  - [ ] Modal dialog with editable fields
  - [ ] Background thread execution
  - [ ] Cache update after write
  - [ ] Graceful error handling

- [x] **Task 1.2** — Offline discovery playlists
  - [ ] Play count tracking in cache
  - [ ] Four smart playlists: Recently Added, Most Played, Never Played, Lost Gems
  - [ ] Auto-update on play count changes and new tracks *(not in shipped v0.2.0 scope)*
  - [ ] Library explorer integration
  - [ ] Playback operations on smart playlists
  - [ ] Excluded from search results *(not in shipped v0.2.0 scope)*

- [x] **Task 1.3** — Progressive disclosure UI
  - [ ] Minimal default control set
  - [ ] Advanced features behind toggles
  - [ ] Settings organized by frequency *(not in shipped v0.2.0 scope)*
  - [ ] Contextual tooltips for advanced features *(not in shipped v0.2.0 scope)*
  - [ ] No empty placeholder states *(not in shipped v0.2.0 scope)*

- [x] **Task 1.4** — Linux first-class support
  - [ ] Polished Linux folder picker
  - [ ] Platform limitations documented in-app
  - [ ] Core features reliable on GNOME/KDE/XFCE *(not in shipped v0.2.0 scope)*

- [x] **Task 1.5** — Accessibility support
  - [ ] Full keyboard navigation
  - [ ] Visible focus indicators
  - [ ] High-contrast theme
  - [ ] Screen-reader-friendly labels *(not in shipped v0.2.0 scope)*
  - [ ] Readable at 150% scaling *(not in shipped v0.2.0 scope)*
  - [ ] No rapid animations *(not in shipped v0.2.0 scope)*

- [x] **Checkpoint 1** — New Features
  - [ ] All P1 features compile and test
  - [ ] Tag writing works end-to-end
  - [ ] Smart playlists functional
  - [ ] Progressive disclosure active
  - [ ] Linux support first-class
  - [ ] Keyboard navigation complete
  - [ ] CI green

---

## Phase 2: UI Polish & Partial Feature Completion

- [x] **Task 2.1** — Add mute toggle to player control bar
  - [ ] Mute/unmute button with 🔇/🔊 icon
  - [ ] Preserves previous volume for restore
  - [ ] Independent of volume slider

- [x] **Task 2.2** — Complete Now Playing view
  - [ ] Clickable up-next rows (PlayNext)
  - [ ] In-view seekable progress bar
  - [ ] MM:SS elapsed/total time
  - [ ] "Nothing playing" empty state

- [x] **Task 2.3** — Complete system tray
  - [ ] Close-to-tray on macOS/Windows
  - [ ] Full context menu (Play/Pause, Next, Previous, Show, Quit)
  - [ ] Tooltip "Artist - Title"
  - [ ] Playback continues hidden

- [x] **Task 2.4** — Cover art display polish
  - [ ] Consistent sizing across views
  - [ ] Graceful placeholder behavior
  - [ ] Aspect ratio handling

- [x] **Checkpoint 2** — UI Polish
  - [ ] Mute toggle works
  - [ ] Now Playing complete
  - [ ] System tray complete
  - [ ] Cover art polished
  - [ ] CI green

---

## Phase 3: Cache Hardening & Robustness

- [x] **Task 3.1** — Cache schema versioning
  - [ ] Version field in library_cache.json
  - [ ] Version check on load
  - [ ] Graceful fallback on mismatch
  - [ ] User-visible explanation

- [x] **Task 3.2** — Clear cache control in settings
  - [ ] "Clear Library Cache" button
  - [ ] Deletes cache file
  - [ ] UI feedback

- [x] **Task 3.3** — Trait-based mock tests
  - [ ] Mock implementations of port traits
  - [ ] Behavior tests through trait interfaces
  - [ ] Error-path injection tests

- [x] **Checkpoint 3** — Robustness
  - [ ] Cache versioned
  - [ ] Clear cache button
  - [ ] Mock-based infra tests

---

## Phase 4: Advanced Features (P2)

- [x] **Task 4.1** — Gapless playback
  - [ ] Pre-decode next track
  - [ ] Seamless buffer handoff
  - [ ] Works for all formats *(seamless for same-format handoffs; automatic gapped fallback on format mismatch or shuffle)*
  - [ ] Memory bounded

- [x] **Task 4.2** — Custom playlist management
  - [ ] Create named playlists
  - [ ] Persist to disk
  - [ ] Load into queue
  - [ ] Handle invalid paths
  - [ ] Library explorer integration
  - [ ] Rename/delete support

- [x] **Task 4.3** — ReplayGain normalization
  - [ ] Read ReplayGain tags via lofty
  - [ ] Apply gain in volume scaling
  - [ ] Peak normalization (no clipping)
  - [ ] Settings toggle

- [x] **Checkpoint 4** — Complete
  - [ ] All phases done
  - [ ] All tests pass
  - [ ] CI green
  - [ ] Release build succeeds
