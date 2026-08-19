# riff v0.2.0 — Execution Todo

## Phase 0: Foundation — Test Suite & CI

- [ ] **Task 0.1** — Make the test suite compile and run
  - [ ] Add `src/lib.rs` re-exporting domain, app, infra, ui
  - [ ] Fix import paths in all test suites
  - [ ] Fix moved-value bugs
  - [ ] `cargo test --no-run` succeeds
  - [ ] `cargo test` all tests pass

- [ ] **Task 0.2** — Add GitHub Actions CI pipeline
  - [ ] Create `.github/workflows/ci.yml`
  - [ ] Linux runner: fmt + clippy + test
  - [ ] Windows runner: fmt + clippy + test
  - [ ] Runs on push to main and PRs

- [ ] **Task 0.3** — Expand domain layer unit test coverage
  - [ ] PlaybackQueue edge cases (empty, single, wrap-around, repeat modes, shuffle)
  - [ ] PlaybackState transitions
  - [ ] TrackId derivation and equality
  - [ ] RepeatMode cycle behavior
  - [ ] Total domain tests: ~20+

- [ ] **Task 0.4** — Expand LibraryManager test coverage
  - [ ] Add_track indexing
  - [ ] Remove_track eviction
  - [ ] Multi-path deduplication
  - [ ] Unavailable path handling
  - [ ] Search across all fields
  - [ ] Cache save/load/corruption

- [ ] **Checkpoint 0** — Foundation
  - [ ] All tests pass
  - [ ] CI pipeline green
  - [ ] Domain and app coverage expanded

---

## Phase 1: New P1 Features

- [ ] **Task 1.1** — Metadata tag writing
  - [ ] Add `MetadataWriter` trait
  - [ ] Implement tag writing via lofty
  - [ ] "Edit Tags" context menu item
  - [ ] Modal dialog with editable fields
  - [ ] Background thread execution
  - [ ] Cache update after write
  - [ ] Graceful error handling

- [ ] **Task 1.2** — Offline discovery playlists
  - [ ] Play count tracking in cache
  - [ ] Four smart playlists: Recently Added, Most Played, Never Played, Lost Gems
  - [ ] Auto-update on play count changes and new tracks
  - [ ] Library explorer integration
  - [ ] Playback operations on smart playlists
  - [ ] Excluded from search results

- [ ] **Task 1.3** — Progressive disclosure UI
  - [ ] Minimal default control set
  - [ ] Advanced features behind toggles
  - [ ] Settings organized by frequency
  - [ ] Contextual tooltips for advanced features
  - [ ] No empty placeholder states

- [ ] **Task 1.4** — Linux first-class support
  - [ ] Polished Linux folder picker
  - [ ] Platform limitations documented in-app
  - [ ] Core features reliable on GNOME/KDE/XFCE

- [ ] **Task 1.5** — Accessibility support
  - [ ] Full keyboard navigation
  - [ ] Visible focus indicators
  - [ ] High-contrast theme
  - [ ] Screen-reader-friendly labels
  - [ ] Readable at 150% scaling
  - [ ] No rapid animations

- [ ] **Checkpoint 1** — New Features
  - [ ] All P1 features compile and test
  - [ ] Tag writing works end-to-end
  - [ ] Smart playlists functional
  - [ ] Progressive disclosure active
  - [ ] Linux support first-class
  - [ ] Keyboard navigation complete
  - [ ] CI green

---

## Phase 2: UI Polish & Partial Feature Completion

- [ ] **Task 2.1** — Add mute toggle to player control bar
  - [ ] Mute/unmute button with 🔇/🔊 icon
  - [ ] Preserves previous volume for restore
  - [ ] Independent of volume slider

- [ ] **Task 2.2** — Complete Now Playing view
  - [ ] Clickable up-next rows (PlayNext)
  - [ ] In-view seekable progress bar
  - [ ] MM:SS elapsed/total time
  - [ ] "Nothing playing" empty state

- [ ] **Task 2.3** — Complete system tray
  - [ ] Close-to-tray on macOS/Windows
  - [ ] Full context menu (Play/Pause, Next, Previous, Show, Quit)
  - [ ] Tooltip "Artist - Title"
  - [ ] Playback continues hidden

- [ ] **Task 2.4** — Cover art display polish
  - [ ] Consistent sizing across views
  - [ ] Graceful placeholder behavior
  - [ ] Aspect ratio handling

- [ ] **Checkpoint 2** — UI Polish
  - [ ] Mute toggle works
  - [ ] Now Playing complete
  - [ ] System tray complete
  - [ ] Cover art polished
  - [ ] CI green

---

## Phase 3: Cache Hardening & Robustness

- [ ] **Task 3.1** — Cache schema versioning
  - [ ] Version field in library_cache.json
  - [ ] Version check on load
  - [ ] Graceful fallback on mismatch
  - [ ] User-visible explanation

- [ ] **Task 3.2** — Clear cache control in settings
  - [ ] "Clear Library Cache" button
  - [ ] Deletes cache file
  - [ ] UI feedback

- [ ] **Task 3.3** — Trait-based mock tests
  - [ ] Mock implementations of port traits
  - [ ] Behavior tests through trait interfaces
  - [ ] Error-path injection tests

- [ ] **Checkpoint 3** — Robustness
  - [ ] Cache versioned
  - [ ] Clear cache button
  - [ ] Mock-based infra tests

---

## Phase 4: Advanced Features (P2)

- [ ] **Task 4.1** — Gapless playback
  - [ ] Pre-decode next track
  - [ ] Seamless buffer handoff
  - [ ] Works for all formats
  - [ ] Memory bounded

- [ ] **Task 4.2** — Custom playlist management
  - [ ] Create named playlists
  - [ ] Persist to disk
  - [ ] Load into queue
  - [ ] Handle invalid paths
  - [ ] Library explorer integration
  - [ ] Rename/delete support

- [ ] **Task 4.3** — ReplayGain normalization
  - [ ] Read ReplayGain tags via lofty
  - [ ] Apply gain in volume scaling
  - [ ] Peak normalization (no clipping)
  - [ ] Settings toggle

- [ ] **Checkpoint 4** — Complete
  - [ ] All phases done
  - [ ] All tests pass
  - [ ] CI green
  - [ ] Release build succeeds
