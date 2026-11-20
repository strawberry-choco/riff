# P0 feasibility memo

Branch `p0/nsh-feasibility`. Started 2026-11-20. Machine: Windows 11 22631, x64, 12 logical cores, rustc 1.98.1 MSVC.

Status: in progress. Verdicts below are filled as each proof lands. Nothing here is an operator decision; the plan says the program is operator reviewed.

Two evidence levels are used and kept apart. **Compiled** means the spike built or ran against the claim on this machine. **Researched** means a subagent read the nsh.dev docs and the tagged source at `v0.6.1` (`2f51d86`) and reported it, but the spike has not yet exercised it. A researched claim is not a pass.

## Verdicts

| P0 item | Verdict | Evidence |
| --- | --- | --- |
| Install the nsh CLI, record the version on Windows | VERIFIED | `nsh 0.6.1 (2f51d86)`, release zip SHA256 `7952c84b…aff4be` verified before extract |
| `nsh init --template rust`, `cargo run`, `nsh run` | VERIFIED for init and cargo run | init generated the template; stock `cargo build` 6m31s, window at 1384 ms, `shots/s1-spike.png`. `nsh run` not separately exercised; `nsh package` proven instead |
| Prove the bridge | VERIFIED | `cargo test --test bridge` green: play, pause, resume, seek over the real 50k-store runtime |
| Idle CPU, playback CPU, cold start, RSS | VERIFIED | see Measurements; sampler self-tested, 12 cores |
| Prove the list at 50k rows | VERIFIED functionally, perf NOT in budget | 57 visible rows, jump to 40000, selection; scroll ~3086 ms/step headless, limiter named in correction 22 |
| Prove covers at 500 | VERIFIED | `shots/s9-covers.png` 500 tiles, `load_ms Some(1037)` warm, RSS 90.49 MB |
| Prove chrome | VERIFIED | `shots/s3-chrome.png`, widths top 56 left 0 right 138; custom Close click exited the process |
| Prove the tray | PARTIAL | icon created in the overflow; close quits (no close-to-hide); on_click INCONCLUSIVE |
| Prove keyboard input | VERIFIED | `cargo test --test interactive` mod+k, escape, space, down |
| Prove drop and reorder | drop VERIFIED, reorder NOT CONFIRMED | folder drop reaches the handler with a desktop idiom; no built-in list reorder API |
| Prove runtime palette switching | VERIFIED | signal-driven dark/light/dark through bindings, headless |
| Prove headless tests on Windows and Linux | Windows VERIFIED, Linux INCONCLUSIVE | 8 headless tests green on Windows; WSL arch lacks gtk4/clang/gstreamer |
| Prove packaging | VERIFIED Windows | exe 29.5 MB, zip 11.9 MB, `.iss` written, installer optional; Linux/macOS paths from docs only |
| Record the exact aligned `nsh-*` versions | VERIFIED | all seven at 0.6.1, `.scratch/nsh-p0/versions.json` |
| Write this memo | done | this file |

## Version alignment

All seven framework crates publish `0.6.1`, and it is the newest non-yanked version of each. The release tag `v0.6.1` published 2026-11-20T03:52:35Z, the same day this spike ran.

The plan's pin block is wrong in two ways. It pins `nsh-core = "=0.6.0"` while pinning the other six at `0.6.1`, so the set is misaligned. And `0.6.1` supersedes it. N1 should pin all seven at `=0.6.1`.

```toml
nsh-core = "=0.6.1"
nsh-base = "=0.6.1"
nsh-engine = "=0.6.1"
nsh-testing = "=0.6.1"
nsh-platform-windows = "=0.6.1"
nsh-platform-linux = "=0.6.1"
nsh-platform-macos = "=0.6.1"
```

Version history confirms the plan's maturity risk. `nsh-core` has nine releases since 2026-10-17, and `nsh-base` seven since 2026-10-22. The exact pin is the right control.

The crates.io crate named `nsh` is the unrelated shell project the plan warns about. It was last updated 2021-07-21 and carries the keyword `shell`. Confirmed, never add it.

## Plan corrections

These are edits N1 must make to `docs/execution-plans/nsh-ui-rewrite.md`. Each names its evidence.

### 1. The pump cannot be event-driven for playback

Compiled-adjacent, read directly. `BackendEvent` has exactly two variants, `TypedNotice(NoticePayload)` and `LibraryChanged { generation: u64 }`, at `crates/riff-backend/src/app/events.rs:66-71`. There is no playback or queue event. The module's own doc comment at `events.rs:61-64` says scan progress and tag edit outcomes reach the frontend through separate polls.

The plan's pump step says "Drain events, rebuild `PlaybackView`". Draining the inbox cannot produce a playback view. The pump must lock `PlaybackSession` and read it every tick. The inbox still serves notices and library generations, and `LibraryChanged` is coalesced to about four emissions per second (`COALESCE_WINDOW`, `events.rs:109`), so it is not a 60 Hz signal source either.

### 2. `SessionViews` playback reads need `sync_playback` first

Researched against `crates/riff-backend/src/app/views.rs:1225,1237,1245`. `playback_current()` and `playback_up_next()` return stale or empty values unless the pump first calls `sync_playback(&mut self, queue: &PlaybackQueue, up_next_limit: usize)`. The plan omits this call. Without it the Now Playing and Up Next screens show nothing.

### 3. `covers.poll()` returns triples, not covers

Read directly. `crates/riff-library/src/app/cover_service.rs:58` declares `fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)>`. `None` is the artless verdict, which the plan's `CoverCache` negative path needs. `request` takes `PathBuf` by value and a `RequestedSize` struct, not a number (`cover_service.rs:51`). The plan writes `covers.request(track_id, path, size)` and `poll()` returning `DecodedCover`, both wrong.

### 4. `AppRuntime::spawn` returns a `Result`

Researched against `crates/riff-backend/src/composition.rs:259`. The real signature is `pub fn spawn(store_path: &Path) -> Result<(Self, RuntimeLifecycle), StoreError>`. The plan shows a bare tuple. `spawn` constructs every real adapter itself, so the spike points it at a synthetic store path and never at the live one.

### 5. `Preferences` is already hydrated, and lives on its own field

Researched against `composition.rs:412` and `crates/riff-backend/src/app/preferences.rs:60,131`. `spawn` calls `hydrate` itself. The plan's "call `Preferences::hydrate(...)` once at startup" would hydrate twice. The handle is `rt.preferences`, not `rt.settings`. `commit_if_changed` takes `&mut self` plus both sessions and `&mut dyn SettingsStore`.

### 6. `session_views` is an owned value, not a shared mutex

Researched against `composition.rs:92`. It is a plain `SessionViews` field and most methods take `&mut self`. It cannot be shared across threads. The plan's `UiModel` sketch already puts `RefCell<SessionViews>` beside `rt`, which is the right instinct, but the reason is stronger than convenience: nsh signals are `!Send`, so the whole model lives on one thread anyway.

### 7. nsh has a tray API, so the plan's tray risk is false

Researched against `nsh-core/src/element/desktop.rs` and `nsh.dev/modules/nsh-desktop.md#tray`, with `examples/tray` in the repo. `set_tray(tray().icon(..).tooltip(..).menu([..]).on_click(..))` exists. One tray per app. Windows notification area, macOS menu-bar status item, Linux StatusNotifierItem, where GNOME needs AppIndicator. It requires the capability `"application": true`.

The plan's risk section says "nsh's desktop docs list no tray API" and asks the operator to accept shipping without a tray before N1. That gate should be removed. The `tray-icon` crate and the `run_on_main_thread` workaround are not needed. Still to be proven live: whether tray menu handlers can reach `rt.ui_transport`, and whether close-to-hide works.

### 8. The desktop backend is not GPUI

Researched against `nsh.dev/docs/windows.md`, `/docs/macos.md`, `/docs/linux.md`, `/docs/gpui.md`. Windows renders with Win32 and Direct2D, macOS with AppKit, Linux with GTK 4. GPUI is a separate opt-in `--target gpui`. This changes the platform floors and the CI package list, and it means the Linux leg needs GTK 4.14 rather than a GPUI stack.

### 9. Signals are `!Send` and engine-thread-only

Researched against `nsh-core/src/element/reactive.rs`. The reactive graph is `thread_local!`. `Signal<T>` is `Copy` but not `Send` or `Sync`. `signal<T: PartialEq + 'static>` dedups by equality on `set`, so the plan's "compare and write only changed signals" is partly free. `update` always notifies.

The owning pattern for non-`Send` state is `stored(value)`, which is `Signal<Rc<RefCell<T>>>` with `.with(|v| ..)`, `.get()` and `.replace()`. `UiModel` should use `stored`, not a bare `RefCell` passed across threads. riff's worker threads hand results over by channel, drained inside the interval closure.

### 10. Deleting riff's cover byte budget would remove the only bound

Researched against `nsh.dev/docs/images-and-assets.md`. nsh caches decoded images in RAM keyed by URL, decode size and blur, and purges under memory pressure. There is no documented numeric RAM cap and no LRU. Its disk cache is 256 MiB per app with LRU, configurable through `CacheOptions`. For RGBA buffers the docs say to give the view a new source for a new frame and let the caches drop the old one once no view shows it, which is reference counting, not a bound.

The plan says "Delete the texture LRU, byte budget and placeholder texture code in `ui/artwork.rs`". The texture LRU and the placeholder texture are egui-specific and do go. The byte budget does not have an nsh equivalent, so deleting it leaves 500 hero covers unbounded. N6 must keep an explicit eviction policy on the nsh side.

### 11. `scroll_to_index` takes four arguments

Researched against `nsh-core/src/element/element_ref.rs:299`. The signature is `scroll_to_index(&self, index: u32, view_position: Percentage, view_offset: Points, animated: bool)`. The index is `u32`, not `usize`. The plan's N6 and N7 text implies a one-argument call.

### 12. `virtual_list` row builders must read the index inside the binding

Researched against `nsh-core/src/element/virtual_list.rs:210` and `nsh.dev/docs/lists.md`. The plan's `virtual_list(move || total, move |at| track_row(at))` is correct. The row function runs once per cell, not per frame or per item. Cell reuse changes `at.index()`, and only bindings that read it re-run. Copying the index out at the top of the row function freezes it.

The bound is `+ 'static` only, with no `Clone` and no `Send`. Capturing `Rc` is the documented idiom, and a `RefCell` or `stored` capture is safe on the single engine thread. This resolves the plan's "windowed reads in a recycled list" risk in favour of the direct pattern, so the bounded prefetch fallback is not needed yet. `estimated_item_size` is a trait method on `ListLayout` and defaults to 50 points.

### 13. Headless defaults to a phone, and cannot load images

Researched against `nsh-testing/src/headless/mod.rs` and `nsh.dev/docs/testing-and-trace.md`. `EngineConfig::new` is 390x844 and defaults to `platform: "ios"`. `drop_files` and `hover` error unless the app is opened with a desktop idiom. Images never load headless, so `onLoad` and `onError` do not fire and cover art cannot be asserted this way.

This costs the plan part of its test strategy. The 87 golden baselines were going to be replaced by a behaviour suite, but cover rendering cannot be covered headless. N16 needs a windowed screenshot path for covers, or an explicit statement that cover pixels are not tested.

The test id setter is `.test_id("..")`, not `testID`. Selectors are `id:`, `label:`, `text:`, `role:`. Headless drives `press_key("mod+k")`, `drag`, and `drop_files`, so the keyboard and drop proofs are assertable without a window.

### 14. `PanResponder` is TSX-only and list reorder is unconfirmed

Researched against `nsh.dev/docs/touch-and-gestures.md` and `elements/view.md`. File drop is available as `.on_file_drop(|event: &FileDropEvent| event.uris)`, with directories included, plus `on_file_drag_enter` and `on_file_drag_leave`. Rust drag gestures use `.claim_touches(TouchClaim::..)` and `.on_responder_*` with `ResponderEvent::gesture: PanGesture`. There is no built-in list-row reorder prop.

N10's playlist drag reorder has to be built from responders. The plan lists `PanResponder` as an option, which does not exist in Rust mode. Budget for this.

### 15. Packaging adds a Linux tarball, and the glibc floor is unconfirmed

Researched against `nsh.dev/docs/packaging.md`. Windows writes `.nsh\out\<Name>\<Name>.exe`, `.nsh\out\<Name>.zip`, and always `.nsh\out\<Name>.iss`. It compiles `<Name>-setup.exe` only when Inno Setup 6 is installed, so the installer is optional. Linux writes the directory plus `.nsh/out/<Name>.tar.gz`, which the plan omits. macOS writes `.nsh/out/<Name>.app`, ad-hoc signed.

Floors are Windows 10 1809, macOS 14, GTK 4.14, WebKitGTK 6.0 at 2.42 or newer, and GStreamer 1.14. WebKitGTK is required by the default release runner. The plan's claim that nsh needs glibc 2.39, and therefore an Ubuntu 24.04 release runner, is not confirmed by the docs. Verify it before N16 moves the runner.

### 16. The cargo fork is recommended, not required

Researched against `nsh.dev/docs/installation.md` and `/docs/cli.md`. `huacnlee/cargo` shares one build directory across projects and worktrees and caps its size. It changes no semantics. Stock `cargo build`, `cargo run` and `cargo test` work, and `nsh run`, `package`, `test` and `check` invoke plain `cargo` from PATH. CI does not need the fork.

### 17. Fonts ship through the asset macros

Researched against `nsh.dev/docs/fonts.md` and `nsh-core/src/element/ambient.rs:883`. `register_font(data: impl Into<Vec<u8>>)` is async and returns `Answer<Result<String, String>>`, for fonts unknown at build time. The self-contained path is `const F: FontAsset = asset!("fonts/Inter-Regular.ttf")` plus `registry.assets(assets!())` in the native module. Both compile the bytes into the binary, and `asset!` reads the family at compile time. This answers the plan's open question about which keeps the binary self-contained.

### 18. Build weight is far above the docs' estimate

Compiled. `cargo generate-lockfile` on the spike resolved 637 packages in 7.2 s. The Rust doc says the first build compiles "about 60 crates". The lock includes `nsh-js`, `nsh-package`, `nsh-i18n` and `rsa v0.10.0-rc.19`. Whether `nsh-js` links into a Rust-only binary is still open, and it matters because the plan's Rust-only decision rests on shipping no JavaScript VM.

### 19. The platform crates must disable default features

Compiled. `nsh init --template rust` generates platform dependencies with `default-features = false` and an explicit feature list (`shader`, `editor`, `emoji-picker`, `highlight`, `math`, `plot`, `rich-editor`, `term`, `websocket`, `updater`, `svg`, `net`, and `sound` on Linux). The spike's first manifest enabled defaults and resolved 637 packages including wgpu, webview2 and QuickJS. N1 should copy the generated feature list and trim it to what riff uses, then re-measure the lock and the binary. Whether QuickJS still links with defaults off is being checked against the package build log.

The generated `src/main.rs` also carries a `compile_error!` for non-desktop targets and sets `windows_subsystem = "windows"`. The spike omits the subsystem attribute so `cargo run` keeps stdout; N1 must decide which it wants, because a GUI-subsystem binary writes no console logs.

### 20. Never write a signal while holding the `stored` borrow

Compiled. The first pump wrote `m.playback.set(snap)` inside `model.with(|m| ..)`. The write notifies a binding that reads the same store, which re-borrows the `RefCell` and panics `RefCell already borrowed` at `nsh-core-0.6.1/src/element/reactive.rs:552`. The fix is to copy the `Copy` signal handles out of the store once, read the runtime inside the borrow, and write the signals after the borrow ends. N3's pump and every handler must follow this shape. The headless bridge test is the regression gate.

### 21. A `dynamic` block re-runs only if it reads a signal, and flex children need `flex_1`

Compiled. Two separate zero-render bugs. The cover grid's `dynamic` closure read only the non-reactive store, so it never re-ran and painted nothing; reading the arrival-count signal inside it fixed that. The track list and the cover grid both collapsed to zero height because a `virtual_list` or grid inside a `flex_col` had no `flex_1`. N6 and N9 must set `flex_1` on scrolling children and read a signal inside every `dynamic` that shows store data.

### 22. Scattered `track_row` reads cost ~10 ms each, so N6 needs the prefetch window

Compiled. `tests/projection_cost.rs` against the 50k store: `track_count` 6552 µs cold, sequential `track_row` 119.5 µs/call, repeated same-index 225.2 µs/call, scattered `track_row` 9990.6 µs/call. A scroll jump invalidates the projection's warm window and every visible row becomes a scattered read. Twenty headless scroll steps of 2000 pt took 61712 ms, about 3086 ms/step, which includes layout passes and is an upper bound rather than a pure read cost. The plan's direct `track_row(at.index())` per-row pattern is therefore too slow to scroll at 50k. The bounded prefetch window driven by `on_viewable_items_change`, which the plan lists as a fallback, is required. Keep a sorted id list or a warm window so row reads stay sequential.

### 23. Tray close-to-hide is not the default, and system caption buttons survive `Hidden`

Compiled. Clicking the window's system Close with a tray set exited the process, so N14's close-to-hide needs explicit configuration, not just `set_tray`. Under `TitleBarStyle::Hidden` the platform still draws the caption buttons inside the reserved `title_bar().right` region, themed for the system appearance; on a light custom bar they become near-invisible. N4 must either use the system buttons and not draw duplicates, or suppress them. The tray `on_click` handler was not verified because coordinate-clicking the overflow flyout missed; it needs a human or UI-automation pass.

### 24. Cold cover throughput is a few per second, and nsh's image cache has no numeric bound

Compiled. First cold run reached 24 covers in 20 s; a second run reached 443 in 120 s (about 3.7/s) as the on-disk Thumbnail cache warmed; a fully warm run loaded all 500 in 1037 ms. RSS with 500 decoded 56x56 covers held was 90.49 MB. Combined with correction 10, N6 must keep its own eviction and should prefetch covers ahead of the viewport rather than on arrival, because cold decode is the limiter.

## Measurements

All numbers are from this Windows machine, 12 logical cores, debug builds unless stated. CPU is percent of one core. The sampler reads `Get-Process` counters every 250 ms; it was self-tested against an idle process (0% CPU, 10.19 MB, 15 samples). Run counts and ranges are in the CSVs under `logs/`.

| Scenario | Cold start to window | Idle CPU median (range) | Working set median / peak | Samples |
| --- | --- | --- | --- | --- |
| s1 spike, empty store | 1384 ms | 0 (0..0) | 62.79 / 62.79 MB | 44 |
| s3 chrome | 964 ms first, 128 ms warm | 0 (0..0) | 47.6 MB | 18-29 |
| s7 bridge, 50k store, pump running | 4758 ms | 15.65 (5.22..36.79) | 78.04 / 78.0 MB | 27 |
| s9 covers, 500 held | 964 ms | 0 (0..5.77) after load | 90.49 / 90.46 MB | 220 |
| s5 packaged release exe | 626 ms | 0 (0..0) | 42.25 / 42.25 MB | 18 |

The bridge's 15.65% idle is the 16 ms pump polling the session every tick even when stopped. That is a standing cost N3 should reduce by sleeping when nothing changed. The covers run's CPU median of 0 with a range to 88 in the first cold run shows the decode worker, not the render loop, is the limiter during load.

Build weight: debug spike 88.1 MB exe, 491 crates compiled in 6m31s with default features; the generated minimal-feature template packaged to a 29.5 MB release exe in 12m48s and compiled no QuickJS. The 637-package lock was the default-feature spike; N1 should re-measure with the trimmed feature list.

Scroll: 20 headless steps of 2000 pt over 50k rows, 61712 ms total, 3086 ms/step, limiter scattered `track_row` at ~10 ms/call (correction 22). This is NOT within an interactive budget and is the single most important perf finding of P0.

## Method

Builds run serially on this one machine. Concurrent cargo would contend for the target directory and corrupt the CPU numbers. The spike lives at `docs/execution-plans/evidence/nsh/p0/spike/` with its own `[workspace]` table, so it never joins the riff workspace and no production file changes. Its target directory is `.scratch/nsh-p0/target`.

Process sampling uses `.scratch/nsh-p0/sample.ps1`, which reads `Get-Process` counters at a fixed interval and writes CSV. It reports CPU as a percent of one core and as a percent of the whole 12-core machine, plus working set, private bytes, threads and handles. Self-tested against an idle process, which read 0% CPU and 10.19 MB across 15 samples.

The library fixture is synthetic and generated under `.scratch/nsh-p0/`. The live store and the user's real collection are never read, scanned or screenshotted.

Rerun the whole spike from the repo root. The levers are committed under `evidence/nsh/p0/tools/`; their generated outputs go to gitignored `.scratch/nsh-p0/`.

```bash
python docs/execution-plans/evidence/nsh/p0/tools/make_fixture.py      # 50k tracks / 500 albums
python docs/execution-plans/evidence/nsh/p0/tools/verify_fixture.py    # independent byte round-trip
export CARGO_TARGET_DIR=$PWD/.scratch/nsh-p0/target
cd docs/execution-plans/evidence/nsh/p0/spike
cargo build --examples
cargo run --example scan -- ../../../../../../.scratch/nsh-p0/library   # populate the store
cargo test                                        # 8 headless proofs
cargo run --example bridge                        # windowed pump proof
```

Windowed shots and CPU/RSS sampling use `tools/run-proof.ps1 -Exe <path> -Tag <name>`, which captures via `PrintWindow` (`tools/shot.ps1`) and samples via `tools/sample.ps1`. Run the PowerShell levers from `tools/` so their relative output paths resolve to `../shots` and `../logs`.

## Go / no-go

Go, with three plan edits before N1. The framework builds, runs, packages and tests on Windows; the bridge, list, covers, chrome, keyboard, palette and drop all work against the real backend. The tray exists natively, so the plan's tray-gap gate is removed.

The three edits that change downstream work:
1. N3's pump must poll `PlaybackSession`, call `sync_playback`, and write signals outside the `stored` borrow (corrections 1, 2, 20).
2. N6 must use a prefetch window for row reads and keep its own cover eviction (corrections 10, 22, 24). The direct per-row pattern does not meet an interactive scroll budget.
3. N1 should pin all seven crates at `=0.6.1` and copy the generated minimal platform feature list, then re-measure build weight (corrections 18, 19).

## Still open

- egui trunk baseline (S6). Not captured. `riff-gui` reads the store from the OS data-local dir and exposes no store-path override, so measuring trunk on a synthetic library would need a production code change, which P0 forbids. Running the real app would also expose the user's live collection. The nsh numbers above stand alone; the trunk comparison is a follow-up that needs a store-path seam.
- Linux headless and Linux/macOS packaging (S13). WSL archlinux has glibc 2.44 and cargo 1.97.0 but no gtk4, clang, gstreamer or nsh. Installing those is a large system change not taken this run. CI's ubuntu leg with the plan's apt list is the intended path; the glibc 2.39 claim behind the Ubuntu 24.04 runner move is still unconfirmed.
- `nsh run` specifically. `cargo run` and `nsh package` are proven; the dev-server `nsh run` loop was not separately exercised.
- Tray `on_click` and close-to-hide configuration (correction 23). Needs a human or UI-automation pass.
- List drag reorder. No built-in API; N10 must build it from touch responders. Budget for it.
- Whether the system caption buttons can be suppressed under `Hidden` (correction 23).