# nsh UI rewrite plan

riff replaces its egui frontend with an nsh frontend. The five backend crates stay. A second binary, `riff-nsh`, ships beside `riff` until parity, then the three egui crates are deleted. Pixel parity is out of scope. Feature parity is the bar. Work runs from P0 to N16, and each unit names its files and its evidence.

## How to read this

Concrete inventory first, then the work units in execution order, then decisions and risks.
One box is one unit of work, and it names the evidence that checks it.
Check a box only when its evidence exists as a file, log line, screenshot or SHA.
Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.
Evidence lives under `docs/execution-plans/evidence/nsh/<id>/`.
The program is operator reviewed. Nothing merges on an agent's own verdict.

## Concrete inventory

### Workspace edits

Root `Cargo.toml`.
- Add `"crates/riff-gui-nsh"` to `members`.
- Set `rust-version = "1.97"` in `[workspace.package]`.
- Set `resolver = "3"`.
- Add the pinned `nsh-*` entries to `[workspace.dependencies]`.

Exact pins. P0 records the aligned patch set first, because crates.io already carries `nsh-core` 0.6.0 and `nsh-base` 0.6.1.
```toml
[workspace.dependencies]
nsh-core = "=0.6.0"
nsh-base = "=0.6.1"
nsh-engine = "=0.6.1"
nsh-testing = "=0.6.1"
nsh-platform-windows = "=0.6.1"
nsh-platform-linux = "=0.6.1"
nsh-platform-macos = "=0.6.1"
```

Two traps from the research.
- The crates.io crate named `nsh` is an unrelated shell project. Never add it. The framework's libraries are `nsh-core`, `nsh-base`, `nsh-engine`, `nsh-testing` and `nsh-platform-*`.
- The `nsh` command comes from the installer, not from cargo. CI pins it with `NSH_VERSION`.

`crates/riff-gui-nsh/Cargo.toml`.
- Inherit the workspace fields.
- Declare `[[bin]] name = "riff-nsh"`.
- Depend on `riff-backend`, `nsh-core`, `nsh-base`.
- Target-gate `nsh-platform-windows`, `nsh-platform-linux` and `nsh-platform-macos`.
- Do not depend on `riff-gui-theme`. The theme is ported, not shared.

`.github/workflows/ci.yml`.
- Add `libgtk-4-dev clang libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev` to the Linux apt list.
- Add an `Install nsh` step with `NSH_VERSION` pinned and `~/.nsh` cached.
- Add `cargo build --locked -p riff-gui-nsh` and `cargo test --locked -p riff-tests nsh_`.
- Keep every current step until N15, because `riff` still ships.

`.github/workflows/release.yml`, edited at N16.
- Move the Linux leg from `ubuntu-22.04` to `ubuntu-24.04`, because the nsh Linux runtime needs glibc 2.39.
- Add the same Linux packages and the pinned `nsh` install.
- Replace `cargo build -p riff-gui` and the three hand-rolled packaging steps with the shape P0 proves. The likely shape is `nsh package --target <slug>` plus an archive of `.nsh/out/riff/`.

Docs.
- New ADR `docs/adr/0022-nsh-ui-stack-replaces-egui.md`.
- Amend `docs/adr/0004-dual-theme-tokens.md` for the coexistence window.
- Update `AGENTS.md`, `README.md`, `docs/README.md`, `docs/technical/architecture.md`, `docs/engineering/development-setup.md`, `docs/engineering/testing-strategy.md` and `docs/engineering/release-and-packaging.md`.
- Retire `docs/engineering/golden-image-testing.md` at N16.

### New crate tree

```text
crates/riff-gui-nsh/
  Cargo.toml
  nsh.json
  theme.json
  src/
    lib.rs
    main.rs
    app.rs
    model.rs
    bridge/
      mod.rs
      runtime.rs
      state.rs
      pump.rs
      projections.rs
    theme/
      mod.rs
      palette.rs
      tokens.rs
      fonts.rs
      icons.rs
    shell/
      mod.rs
      layout.rs
      title_bar.rs
      sidebar.rs
      player_bar.rs
    screens/
      mod.rs
      library/
        mod.rs
        browse.rs
        list.rs
        folders.rs
        search.rs
      inspector.rs
      inspector/
        tag_editor.rs
      now_playing.rs
      queue.rs
      playlists.rs
      settings/
        mod.rs
        library.rs
        playback.rs
        appearance.rs
        advanced.rs
        about.rs
      smart_playlists/
        mod.rs
        editor.rs
        rules.rs
        date_picker.rs
    components/
      mod.rs
      track_row.rs
      entity_row.rs
      cover.rs
      playing_indicator.rs
      empty_state.rs
    menus/
      mod.rs
      track.rs
      collection.rs
    platform/
      mod.rs
      library_paths.rs
      window_state.rs
      tray.rs
      notifications.rs
  assets/
    fonts/Inter-Regular.ttf
    fonts/Inter-Medium.ttf
    fonts/Inter-SemiBold.ttf
    fonts/Inter-Bold.ttf
    icons/*.svg
    brand/riff-1024.png
```

### App metadata and entry point

`nsh.json`.
```json
{
  "name": "riff",
  "id": "io.github.strawberry-choco.riff",
  "version": "0.2.0",
  "build": 1,
  "icon": "assets/brand/riff-1024.png",
  "rust": ".",
  "root": "App",
  "theme": "theme.json",
  "appearance": "system"
}
```
- `version` and `build` mirror the workspace version and bump per release.
- The icon is a new 1024x1024 PNG exported once from the existing brand mark.

`src/main.rs`.
- Copy the file `nsh init --template rust` generates.
- Point the root at `riff_gui_nsh::App` through `Root::of::<App>()`.
- Set `AppOptions` with `app_id`, `cache`, `login_item: false` and `fps` gated on `NSH_FPS`.
- `cache` points the nsh image cache at the same data-local directory riff already uses.

`src/lib.rs`.
- Root `#[component] pub fn App()`.
- `#[native_module("riff/native")] fn native(registry: &mut Registry)`.
- Keep the registry even with no TSX. It is where assets ship.

### Bridge surface

The bridge owns `AppRuntime` and is the only holder of signals.

```rust
pub struct UiModel {
    rt: AppRuntime,
    views: RefCell<SessionViews>,
    state: UiState,
}

pub struct UiState {
    pub playback: Signal<PlaybackView>,
    pub library_revision: Signal<u64>,
    pub playlist_revision: Signal<u64>,
    pub sidebar: Signal<SidebarView>,
    pub settings: Signal<SettingsSnapshot>,
    pub palette: Signal<Palette>,
    pub notices: Signal<Vec<NoticePayload>>,
    pub covers: Signal<CoverMap>,
}

pub struct PlaybackView {
    pub state: PlaybackState,
    pub position_secs: f32,
    pub duration_secs: Option<f32>,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub current: Option<Arc<Track>>,
    pub up_next: Arc<[Track]>,
    pub queue_index: usize,
    pub queue_len: usize,
}
```

Exact reads.
- `rt.backend_events.lock_or_recover().events()` gives the `BackendEvent` batch.
- `rt.playback.lock_or_recover()` gives `PlaybackSession` with `queue`, `playback_state`, `current_position`, `current_volume`, `muted`.
- `rt.session_views.track_count(query, sort)` and `track_row(query, sort, index)` serve the virtual list.
- `rt.session_views.artist_albums(artist)` and `album_tracks(album_artist, album_title)` serve the drill-down.
- `rt.session_views.playlists()`, `playlist_view(&id)`, `genres()`, `last_scan()` and `try_custom_smart_playlists()`.
- `rt.session_views.playback_current()` and `playback_up_next()`.
- `rt.covers.request(track_id, path, size)` and `rt.covers.poll()` returning `DecodedCover { rgba, width, height }`.
- `Preferences::hydrate(...)` once at startup, and `Preferences::commit_if_changed(...)` after any settings write.

Exact writes.
- `rt.ui_transport` with `play`, `play_random`, `pause`, `resume`, `stop`, `seek`, `set_volume`, `toggle_mute`, `next`, `previous`, `play_next`, `add_to_queue`, `add_many`, `play_many`, `play_album`, `toggle_shuffle`, `toggle_repeat` and `play_pause`.
- `rt.settings`, `rt.playlists`, `rt.library_mutations`, `rt.tag_edits`, `rt.passes`, `rt.scans` and `rt.thumbnail_cache` behind their existing ports.

The pump.
- One `set_interval(Duration::from_millis(16), ...)` on the engine thread.
- Drain events, rebuild `PlaybackView`, compare and write only changed signals, bump revisions when generations move.
- Worker threads never touch a signal.

Covers.
- Reuse the decision half in `crates/riff-gui/src/ui/cover_cache.rs` after moving `CoverCacheKey`, `cover_cache_key` and `COVER_IN_FLIGHT_CAP` out of `ui/artwork.rs` into a module that names no egui type.
- On poll, wrap each `DecodedCover` as `ImageSrc::rgba(width, height, rgba.into())`. nsh caches the decoded image.
- Delete the texture LRU, byte budget and placeholder texture code in `ui/artwork.rs`.

### Screen map

- `crates/riff-gui/src/main.rs` becomes `crates/riff-gui-nsh/src/main.rs`.
- `ui/app.rs` splits into `src/model.rs`, `src/bridge/*` and the screen modules. No line-for-line port.
- `ui/frame.rs` becomes `src/shell/layout.rs` plus the per-screen composers.
- `ui/chrome.rs` becomes `src/shell/title_bar.rs`. Use `current_window().set(WindowAttributes { title_bar: Some(TitleBarOptions { style: TitleBarStyle::Hidden, height: Some(px(56.0)), .. }) })`, `.window_drag(true)`, `title_bar()` for the reserved control widths and `.window_control(WindowControl::Minimize)`, `.window_control(WindowControl::Maximize)` or `.window_control(WindowControl::Close)`.
- `ui/sidebar.rs` becomes `src/shell/sidebar.rs`.
- `ui/playerbar.rs` becomes `src/shell/player_bar.rs`.
- `ui/now_playing.rs` becomes `src/screens/now_playing.rs`.
- `ui/detail.rs` becomes `src/screens/inspector.rs`.
- `ui/artwork.rs` and `ui/cover_cache.rs` become `src/components/cover.rs`.
- `ui/browser.rs`, `ui/app/browser_pane.rs`, `ui/column.rs`, `ui/selection.rs`, `ui/app/selection_pane.rs` and `ui/scroll_memory.rs` become `src/screens/library/`.
- `ui/settings.rs` becomes `src/screens/settings/`.
- `ui/smart_playlists_page.rs`, `ui/rule_row.rs`, `ui/rule_box.rs` and `ui/rule_date.rs` become `src/screens/smart_playlists/`.
- `ui/app/tag_editor.rs` becomes `src/screens/inspector/tag_editor.rs`.
- `ui/app/track_menu.rs` and `ui/app/collection_menu.rs` become `src/menus/`.
- `ui/app/library_picker.rs` and `ui/drop_overlay.rs` become `src/platform/library_paths.rs`.
- `ui/tray.rs` becomes `src/platform/tray.rs`.
- `ui/traffic_lights.rs` is deleted. `title_bar()` reports the widths.
- `ui/window_visibility.rs` becomes `src/platform/window_state.rs`.
- `ui/notifications.rs` becomes `src/platform/notifications.rs`.
- `ui/feedback.rs` becomes `ToastManager` from `nsh_base`.
- `ui/scroll_memory.rs` is deleted. `virtual_list` keeps its scroll while mounted, and `scroll_to_index` restores it.
- The whole `riff-gui-components` crate maps to `nsh_base` controls plus `src/components/*` composites.

### Theme map

Port from `crates/riff-gui-theme/src/theme.rs` into `src/theme/`.

`Palette`, 17 fields.
`background`, `surface_row`, `surface`, `surface_2`, `surface_3`, `row_hover`, `ink`, `ink_2`, `ink_3`, `line`, `border`, `brand_primary`, `on_brand`, `success`, `warning`, `error`, `info`, `focus_ring`. The translation source names each one in its doc comment, and the three variants are `dark`, `light` and the two high-contrast sets.

Tokens.
- Text sizes `TEXT_XS` 12, `TEXT_SM` 14, `TEXT_XL` 20, `TEXT_3XL` 30.
- Radii `XS` 3, `SM` 6, `MD` 10, `LG` 14, `XL` 18, `FULL` 999.
- Spaces `XS` 4, `SM` 6, `MD` 8, `LG` 12.
- Motion `DEFAULT` 0.18, `HOVER` 0.10, `REDUCED` 0.0.
- Layout `TITLEBAR_H` 56, `SIDEBAR_W` 280, `COLUMN_WIDTH` 280, `COLUMN_MIN_W` 200, `LAST_COLUMN_MIN_W` 320, `INSPECTOR_WIDTH` 300, `PLAYERBAR_H` 88.
- Shadows and the brand gradient port only where nsh supports them. Drop the egui glow.

Fonts. Copy the four Inter files from `crates/riff-gui-theme/assets/fonts/`. Register them with `register_font(include_bytes!(...))` at startup, or through `assets!()`. P0 records which keeps the binary self-contained.

Icons. Copy the 35 Lucide SVGs from `crates/riff-gui-theme/assets/icons/`. First choice is nsh SVG assets with tint. Fallback is the existing `resvg` rasterizer into `ImageSrc::rgba`.

Colors.
- The authoritative store is the `Signal<Palette>` in `UiState`, not `theme.json`. Every riff element reads `move || palette.get().surface` and so on, which also gives runtime high contrast and dark switching without an nsh theme API.
- `theme.json` only sets the `nsh_base` defaults so menus, dialogs and toasts sit close to riff's palette.

### Commands

Development.
```bash
cargo run -p riff-gui-nsh
cargo test -p riff-tests nsh_
```

nsh CLI, run from `crates/riff-gui-nsh`.
```bash
nsh run
nsh package --target windows
nsh package --target linux
nsh package --target macos
```

CI install.
```bash
export NSH_VERSION=v0.6.1
curl -fsSL https://nsh.dev/install.sh | sh
```
```powershell
$env:NSH_VERSION = 'v0.6.1'; irm https://nsh.dev/install.ps1 | iex
```

Artifact paths to verify in P0.
- Windows `.nsh/out/riff/riff.exe`, `.nsh/out/riff.zip`, and `riff-setup.exe` when Inno Setup is installed.
- macOS `.nsh/out/riff.app`.
- Linux `.nsh/out/riff/` with its `nsh/` directory. Confirm the exact path.

### Test map

Survives. `tests/domain_tests.rs` (55 test functions), `tests/app_tests.rs` (243) and `tests/integration_tests.rs` (17). Verify `app_tests.rs` names no egui type before counting it safe.
Replaced. `tests/ui_tests.rs` (457 test functions) and `tests/golden_tests.rs` (92 test functions over 87 baselines), plus `tests/docs_images_tests.rs` and `tests/readme_screenshots.rs`.
New. `tests/nsh_bridge_tests.rs` and `tests/nsh_flow_tests.rs` in the root `riff-tests` crate, using `nsh_testing::Headless` and selectors by `testID`. The existing mock ports in `tests/mod.rs` supply the runtime.
Perf fixture. `tests/nsh_perf_fixture.rs`, ignored by default, seeds 50k tracks and 500 covers and prints cold start, scroll and RSS numbers.

### Scope ledger

- Main Application Window goes to N4 and N14.
- Library Explorer Panel goes to N5 and N7.
- Column Browser Navigation goes to N6 and N7.
- Titlebar Search goes to N7.
- Player Control Bar goes to N4.
- Cover Art Display goes to N6.
- Now Playing View goes to N9.
- Progressive Disclosure goes to N13.
- Keyboard Accessibility and High Contrast go to N2 and N14.
- Reduce Motion goes to N2 and N9.
- System Tray Icon goes to N14, or is deferred by the P0 decision.
- Linux Folder Picker goes to N14. nsh dialogs may remove the split.
- Drag-and-Drop Library Paths go to N14.
- Custom Playlists go to N10.
- Tag Editing goes to N11.
- Smart Playlists and Custom Smart Playlists go to N12.
- Settings pages go to N13.
- Cross-platform Support goes to N15 and N16.

## Work units

### P0 Feasibility spike

No production code. One branch, one memo.

- [ ] Install the nsh CLI and record the version on Windows.
- [ ] Run `nsh init riff-spike --template rust`, then `cargo run` and `nsh run`.
- [ ] Prove the bridge. Own a real `AppRuntime`, run the 16 ms pump, and drive play, pause, seek and queue updates.
- [ ] Measure idle CPU, playback CPU, cold start to interactive and RSS.
- [ ] Prove the list. 50k rows through `virtual_list` and `SessionViews::track_row`, with `scroll_to_index` and selection. Measure scroll frame cost.
- [ ] Prove covers. 500 covers through `covers.poll()` and `ImageSrc::rgba`. Measure load time and RSS.
- [ ] Prove chrome. `TitleBarStyle::Hidden`, `.window_drag(true)`, `title_bar()` and `WindowControl`.
- [ ] Prove the tray. `tray-icon` on its own thread with `run_on_main_thread`. Record works or infeasible.
- [ ] Prove keyboard input. Ctrl+K focus, arrows, Space, focus ring and accessibility roles.
- [ ] Prove drop and reorder. Folder drop through the file-drop event, and drag reorder in a list.
- [ ] Prove runtime palette switching through `Signal<Palette>`.
- [ ] Prove headless tests with `nsh_testing::Headless` on Windows and Linux.
- [ ] Prove packaging. `nsh package --target windows`, and record the Linux and macOS output paths.
- [ ] Record the exact aligned `nsh-*` versions.
- [ ] Write `docs/execution-plans/evidence/nsh/p0/memo.md` with a go or no-go per item and the numbers.

Gate. Every item has an answer and a number. A red on tray, chrome or packaging changes the plan before N1.

### N1 Scaffold and CI

Depends on P0.

Files.
- Create `crates/riff-gui-nsh/{Cargo.toml,nsh.json,theme.json,assets/brand/riff-1024.png,src/lib.rs,src/main.rs}`.
- Edit the root `Cargo.toml` for the member, the MSRV, resolver 3 and the pins.
- Edit `.github/workflows/ci.yml` for the Linux packages and the nsh install.
- Add `docs/adr/0022-nsh-ui-stack-replaces-egui.md`.
- Edit `AGENTS.md` and `docs/technical/architecture.md`.

Build.
- Root `App` renders a single token-styled `view()` with the riff name and a version line.
- `cargo run -p riff-gui-nsh` opens a window on Windows and Linux.
- Both binaries build in CI.

Evidence.
- CI run link, `n1-window.png`, and the `nsh --version` line in the log.

### N2 Theme port

Depends on N1.

Files.
- Create `src/theme/{palette,tokens,fonts,icons}.rs` and `src/screens/dev_tokens.rs`.
- Copy the four Inter files and the 35 SVGs into `assets/`.
- Write `theme.json` for `nsh_base` defaults.
- Amend `docs/adr/0004-dual-theme-tokens.md`.

Build.
- Define `Palette` with the 17 color fields and the four color variants. Dark, light, high contrast dark and high contrast light. Reduce motion is a separate flag, not a color.
- Define every token constant from the map above.
- Register four Inter weights.
- Define `Icon` with the 35 names and a size and color API.
- The hidden `dev_tokens` screen shows every color, radius, space, type size and icon.

Done when.
- A unit test resolves every palette slot in all four color variants.
- Screenshots `n2-dark.png`, `n2-light.png`, `n2-contrast.png` show the fixture.
- The fixture is reachable only in debug builds.

### N3 Backend bridge

Depends on N1.

Files.
- Create `src/bridge/{runtime,state,pump,projections}.rs`, `src/model.rs`.
- Create `tests/nsh_bridge_tests.rs` in the root `riff-tests` crate.

Build.
- Move `AppRuntime` into `UiModel` at startup.
- Define the signal shapes above.
- Drain `BackendEvents`, rebuild `PlaybackView`, and write only changed signals.
- Bump `library_revision` and `playlist_revision` when the projections' generations move.
- Wire covers through `CoverCache` and `covers.poll()`.
- Call `Preferences::hydrate` once and `Preferences::commit_if_changed` after settings writes.

Done when.
- `cargo test -p riff-tests nsh_bridge` passes over the mock runtime.
- A play command changes `PlaybackView` within one pump tick.
- A queue change from the engine lands in the signal.
- No worker thread writes a signal.

Evidence.
- Test log, idle CPU number, and a screenshot of the debug playback readout.

### N4 App shell

Depends on N2 and N3.

Files.
- Create `src/shell/{layout,title_bar,sidebar,player_bar}.rs`, `src/app.rs`.

Build.
- Titlebar with the hidden native bar, drag region, window controls and the search field placeholder.
- Sidebar skeleton with the section headers from `features.md`.
- Empty stage.
- Player bar with previous, play pause, next, seek, elapsed, total, volume, mute, shuffle, repeat and queue position, all over `rt.ui_transport`.
- Window size and position persistence through the settings store.
- Keyboard map. Space toggles playback, Ctrl+K focuses search, arrows move focus, Escape clears.

Done when.
- A seeded queue plays, pauses, seeks, changes volume, mutes, shuffles and repeats.
- The Windows chrome matches the current app except the tray.
- The Linux window opens and plays.

Evidence.
- Screenshots `n4-shell.png` and `n4-playing.png`.
- A keyboard walkthrough recording.
- Idle and playback CPU numbers.

### N5 Sidebar and navigation

Depends on N4.

Files. `src/shell/sidebar.rs`, `src/model.rs`.

Build.
- Counts from the projection for tracks, artists, albums, genres, built-in smart lists, playlists and folder roots.
- Sections LIBRARY, SMART LISTS, PLAYLISTS, with the Add folder footer and last-scan stamp.
- Selection writes `LibrarySession.library_section`, `browse_mode` and the browser path.
- `testID` on every row and header.

Done when.
- Counts match the store after a scan.
- Keyboard traversal reaches every section.
- The active section is visible.

### N6 Virtual track list and inspector

Depends on N5.

Files. `src/screens/library/list.rs`, `src/screens/inspector.rs`, `src/components/{track_row,cover,empty_state}.rs`.

Build.
- `virtual_list(move || total, move |at| track_row(at))` over `track_count` and `track_row`.
- `estimated_item_size(px(44.0))`, `.on_viewable_items_change(...)` for cover prefetch, and a list ref for `scroll_to_index`.
- Row anatomy. Index or favorite, title, artist, album, duration, missing marker, playing marker.
- Inspector with cover, metadata and the tag section placeholder.
- Selection survives recycling.

Done when.
- 50k rows scroll inside the P0 budget.
- Missing and playing states render.
- Selection updates the inspector.
- Covers appear through the RGBA path.

Evidence. `n6-list.png`, scroll numbers, RSS with 500 covers.

### N7 Browse stages and search

Depends on N6.

Files. `src/screens/library/{browse,folders,search}.rs`.

Build.
- Artists to albums to tracks.
- Albums to tracks.
- Genres to artists to albums to tracks.
- All Tracks.
- The folder tree with lazy children.
- Ctrl+K search over the active section, with the same matching rules the backend already exposes.
- Per-stage scroll restoration through `scroll_to_index`.

Done when.
- Each drill-down lands on the right rows.
- Search filters in real time and Escape restores the full list.
- A deep folder tree stays responsive.
- Per-stage perf numbers are recorded.

### N8 Menus and actions

Depends on N7.

Files. `src/menus/{track,collection}.rs`.

Build.
- Track menu, Play, Play Next, Add to Queue, Shuffle, Add to Playlist, Favorite, Edit Tags, Remove from Playlist.
- Folder and entity menus with Play, Play Next, Add to Queue, Shuffle.
- Right-click through `popup_menu` or `ContextMenu`.
- Double-click play and folder play.

Done when.
- Every action reaches the transport or store and the UI reflects the result.
- The menu is reachable by keyboard.
- Favorite commits without a rescan.

### N9 Queue and Now Playing

Depends on N8.

Files. `src/screens/{queue,now_playing}.rs`, `src/components/playing_indicator.rs`.

Build.
- Queue list with the current row, promote on click, and clear.
- Now Playing with large cover, the full metadata set, a seek slider and an empty state.
- Playing indicator with nsh animation, snapped to its settled tint under reduce motion.

Done when.
- Queue order and the current row match the engine.
- Promote works.
- Reduce motion settles instantly.

### N10 Playlists

Depends on N9.

Files. `src/screens/playlists.rs`.

Build.
- List and detail, create, rename, delete with confirmation.
- Add and remove tracks.
- Drag reorder through `PanResponder` or the list's row drag.
- Missing rows struck through and skipped.
- Duplicate and built-in name rules through the existing backend wording.

Done when.
- A reordered playlist survives restart.
- Missing rows render and are skipped.
- Name conflicts show the backend's message.

### N11 Tag editor

Depends on N10.

Files. `src/screens/inspector/tag_editor.rs`.

Build.
- Inline fields for title, artist, album, album artist, genre, year, track number and the ReplayGain values.
- Album aggregation with `(different)` and `(none)`.
- Save for one track and for the album, over `rt.tag_edits`.
- Failure wording that names the file.

Done when.
- A tag edit writes the file and updates the library without a rescan.
- An album edit updates every track and reports the count.
- A read-only file reports its reason.

### N12 Smart playlists

Depends on N11.

Files. `src/screens/smart_playlists/{editor,rules,date_picker}.rs`.

Build.
- Built-in lists.
- Custom rule editor with rows, nested boxes and the all-of and any-of mode.
- One field, comparison and value per row, with seven levels of nesting.
- Date rule with the new `date_picker` popover, because nsh has no date picker.
- Live count, unsaved guard, Save, Cancel and Back.
- Unreadable rules shown as unreadable and blocking Save.

Done when.
- Every rule kind round-trips through the store.
- Invalid rules block Save with the reason.
- A saved list opens as a track listing.

### N13 Settings

Depends on N12.

Files. `src/screens/settings/{library,playback,appearance,advanced,about}.rs`.

Build every control in `crates/riff-gui/src/ui/settings.rs`.
- Library. Paths, add, remove, rescan, watch toggle, missing path states, Clear Library.
- Playback. ReplayGain toggle and mode.
- Appearance. Dark, light, system, high contrast, reduce motion.
- Advanced. Advanced mode, notifications, close quits, thumbnail cache clear, library-wide ReplayGain pass.
- About. Version, licenses, offline statement.

Done when.
- Every setting persists and takes effect.
- Clear Library asks first and preserves playlists.
- Thumbnail cache clear reports its polled outcome.

### N14 Platform integration

Depends on N13.

Files. `src/platform/{library_paths,window_state,tray,notifications}.rs`.

Build.
- Folder picker through `pick_paths(OpenDialog::folders())` on all three desktops, with the typed-path fallback when the dialog errors.
- Folder drop through the file-drop event, with the same acceptance and auto-scan rules.
- OS notifications through `notify-rust`, or nsh toasts where they fit.
- Tray if P0 proved it. Close to hide, quit on close, playback controls and tooltip.
- Window visibility and close resolution.
- Accessibility pass. Roles, labels, focus ring and keyboard only walkthrough.

Done when.
- Each platform behavior is exercised on its own OS.
- The Linux picker either works through the portal or shows the typed fallback.
- The accessibility walkthrough passes.

### N15 Flip and delete

Depends on N14.

Files.
- Rename the binary to `riff`.
- Delete `crates/riff-gui`, `crates/riff-gui-components` and `crates/riff-gui-theme`.
- Move the fonts and icons into `crates/riff-gui-nsh/assets/`.
- Remove egui and eframe from the root `Cargo.toml` and delete their pins.
- Edit `README.md`, `AGENTS.md`, `docs/README.md`, `docs/technical/architecture.md`, `docs/engineering/development-setup.md`.

Done when.
- `Cargo.lock` has no egui entry.
- `cargo build --workspace` and `cargo test --workspace` pass.
- The nsh binary opens and plays on all three OSes.

### N16 Tests and artifacts

Depends on N15.

Files.
- Replace `tests/ui_tests.rs` and `tests/golden_tests.rs` with `tests/nsh_flow_tests.rs`.
- Add `tests/nsh_perf_fixture.rs`.
- Edit `.github/workflows/release.yml`.
- Edit `docs/engineering/{testing-strategy,release-and-packaging}.md`.
- Regenerate README screenshots.
- Delete `docs/engineering/golden-image-testing.md`.

Build.
- Headless flow tests for browse, search, play, queue, playlist, tag edit, smart playlist, settings and library path.
- Perf fixture with 50k tracks.
- Release packaging through the shape P0 proved.
- README screenshots captured from the nsh app.

Done when.
- The flow suite passes on Windows and Linux.
- Each packaged artifact installs, opens and plays a track.
- The release notes describe the new artifact shape.

## Verification protocol

**Unit.** `cargo test -p riff-gui-nsh` for pure mappers, `cargo test -p riff-tests nsh_` for bridge and flow tests, and the surviving backend suites.
**Live.** A manual scenario script per unit, run on Windows and Linux, with screenshots under the evidence folder. macOS gets a CI build and one operator run before N15.
**Perf.** Cold start to interactive, scroll cost at 50k rows, cover load with 500 covers, RSS and idle CPU. Record trunk and head. No ratio between unlike scenarios.
**Accessibility.** Keyboard only walkthrough per phase, focus visible, labels on the core flows.

## Decisions

- **Rust-only nsh.** No TSX. No JavaScript VM, one type boundary, and nsh's docs recommend Rust mode for large business logic.
- **Parallel binary, not a branch.** egui and nsh cannot share a window, so a branch has no shippable state. The second binary keeps master green and keeps the exit open.
- **Backend untouched.** Ports, sessions and channels stay. The rewrite is presentation only.
- **Signals adapt the pull model.** The pump turns generations and events into signals. It does not replace `SessionViews`.
- **Exact nsh pin.** nsh shipped eight versions in its first month and makes no compatibility promise for its Rust crates.
- **Feature freeze on egui.** Bug fixes only from N1. New behavior lands in nsh first.
- **Feature parity, not layout parity.** Screens may be simpler. The scope ledger is the audit.
- **Theme authority moves.** The `Signal<Palette>` is the one store after N2. ADR 0004 is amended for the coexistence window.

## Risks

- **Framework maturity.** P0 numbers and the exact pin are the controls. Upgrade PRs run the full suite.
- **Tray gap.** nsh's desktop docs list no tray API. Default is to ship without a tray and restore it later. The operator must accept this before N1 if P0 cannot make it work.
- **Platform floors.** Windows 10 1809, macOS 14 and GTK 4.14. Document them and decide whether pre-floor systems are out of scope.
- **Runtime theme model.** The `Signal<Palette>` path avoids dependence on an nsh theme API. P0 still verifies the base components restyle.
- **Windowed reads in a recycled list.** P0 verifies the exact pattern. Fallback is a bounded prefetch window driven by viewability callbacks.
- **Test loss.** 457 UI test functions and 87 golden baselines die with egui. The replacement is a smaller behavior suite plus perf fixtures.
- **Two stacks drift.** Feature freeze and backend changes land in the nsh bridge first.
- **Packaging shape.** The current release ships a bare executable, a tar.gz and a dmg. nsh writes a directory with an assets folder. P0 records the new shape and N16 rewrites the workflow and notes.
- **Linux runner version.** nsh needs glibc 2.39, so the release runner moves from Ubuntu 22.04 to 24.04.
- **Build weight and binary size.** P0 measures both.
- **macOS coverage.** CI build plus one operator run before cutover.

## Appendix A. Rejected alternatives

- **In-place rewrite on a branch.** No shippable state for the whole rewrite.
- **TSX or mixed mode.** A VM, a second language and a split boundary, for hot reload alone.
- **egui and nsh in one window.** One event loop per toolkit.
- **Big bang in one PR.** No intermediate evidence and no way to isolate a regression.
- **Restyle egui and call it the rewrite.** Does not meet the goal.

## Appendix B. Links

- [nsh repository](https://github.com/longbridge/nsh)
- [Building with Rust](https://nsh.dev/docs/rust)
- [Project structure](https://nsh.dev/docs/project-structure)
- [Reactivity](https://nsh.dev/docs/reactivity)
- [Lists](https://nsh.dev/docs/lists)
- [Images and assets](https://nsh.dev/docs/images-and-assets)
- [Fonts](https://nsh.dev/docs/fonts)
- [nsh/desktop](https://nsh.dev/modules/nsh-desktop)
- [Testing](https://nsh.dev/docs/testing-and-trace)
- [Windows](https://nsh.dev/docs/windows)
- [Linux](https://nsh.dev/docs/linux)
- [Backward compatibility](https://nsh.dev/docs/compatibility)
