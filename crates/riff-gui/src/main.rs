// Thin composition over the backend's Composition Root: `AppRuntime::spawn`
// owns every adapter, port wiring, and worker thread (backend-crate-split
// issue 08), so the binary only opens the Application Store at its default
// location, spawns the runtime, and hands the returned handles to the UI and
// the tray. The tray and the app are composed inside eframe's creation
// closure — the only place the egui context exists, and the tray needs a
// clone of it on its own thread to wake a sleeping event loop — before the
// first frame runs.
//
// Stops a console window flashing on Windows release builds; it also detaches
// stderr, so `tracing_subscriber::fmt::init()` below is a no-op from the user's
// point of view — intended, not a regression. Debug builds keep the console
// (`not(debug_assertions)`), so logs stay visible.
//
// This has to be an INNER (crate-level) attribute with a leading `!`.
// `windows_subsystem` is a crate-level setting, so writing it as an outer
// attribute on `fn main` applies it to the function instead, which newer rustc
// rejects outright with `unused_attributes` and a non-zero exit — the release
// build never links. The `cfg_attr` only fires in release, so `cargo check`
// and `cargo clippy` run in debug and never see it; a green local gate is not
// evidence that this attribute is well-formed.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use riff_backend::composition::AppRuntime;
use riff_gui::ui::RiffApp;
#[cfg(not(target_os = "linux"))]
use riff_gui::ui::window_visibility::spawn_visibility_listener;

fn main() {
    color_eyre::install().expect("failed to install color_eyre");
    tracing_subscriber::fmt::init();
    let store_path = riff_backend::composition::default_store_path().unwrap_or_else(|e| {
        panic!(
            "fatal: could not resolve the Application Store location \
             (no data-local directory available): {e}"
        )
    });
    let (rt, mut lifecycle) =
        AppRuntime::spawn(&store_path).unwrap_or_else(|e| panic!("fatal: {e}"));

    let options = eframe::NativeOptions {
        // Chrome is a per-OS decision (macos-native-title-bar issue 01,
        // amending ADR 0005), and this is where it is consumed. On Windows and
        // Linux `chrome_mode()` answers `CustomCaption` and the launch is
        // frameless: OS decorations are replaced by riff's custom titlebar
        // with a drag region and window controls. On macOS it answers
        // `NativeTrafficLights` and the AppKit-decorated window ships instead
        // (full-size content view, transparent title bar, hidden title, native
        // traffic lights as the window controls).
        viewport: riff_gui::ui::chrome::viewport_builder(),
        ..Default::default()
    };

    // Frontend-local visibility channel (Issue 03): the tray pushes
    // `Show Window` requests here, the UI thread drains them between frames.
    // The tray never constructs backend commands on this path. Linux has no
    // tray, so the channel exists only on macOS/Windows.
    #[cfg(not(target_os = "linux"))]
    let (visibility_tx, visibility_listener) = spawn_visibility_listener();

    eframe::run_native(
        "riff",
        options,
        Box::new(move |cc| {
            riff_gui::ui::fonts::configure_fonts(&cc.egui_ctx);

            // The egui context exists only inside this closure, and the tray
            // needs a clone of it on its own thread to wake a sleeping event
            // loop (`send_viewport_cmd` only queues — it is applied on the
            // next frame), so tray creation — and therefore the app it feeds —
            // happens here.
            #[cfg(not(target_os = "linux"))]
            let tray_icon = match riff_gui::ui::tray::create_tray(
                cc.egui_ctx.clone(),
                rt.tray_transport,
                rt.playback.clone(),
                // Cloned once here and shared: the tray stores it to declare a
                // quit, the app loads it to tell that quit from an OS window
                // close (they are the same `ViewportEvent::Close`, and only the
                // macOS block below resolves a close at all).
                rt.quit_flag.clone(),
                visibility_tx.clone(),
            ) {
                Ok(tray) => {
                    tracing::info!("Tray icon created");
                    Some(tray)
                }
                Err(e) => {
                    tracing::warn!("Failed to create tray icon: {}", e);
                    None
                }
            };

            // One arm per platform rather than one `not(linux)` arm, because
            // the argument lists no longer agree: the trailing `quit_flag` is
            // `macos`-gated in `RiffApp::new` (Windows resolves no close, so a
            // `not(linux)` gate there would leave the field dead and fail CI's
            // `-D warnings`). An arm's `cfg` must match the gate on the
            // arguments it passes, hence the explicit Windows arm.
            #[cfg(target_os = "macos")]
            let app = RiffApp::new(
                rt.playback,
                rt.library,
                rt.ui_transport,
                Box::new(rt.scans.clone()),
                rt.watcher_manager,
                tray_icon,
                rt.settings,
                rt.preferences,
                rt.playlists,
                rt.library_mutations,
                rt.session_views,
                rt.tag_edits,
                rt.covers,
                rt.backend_events,
                visibility_listener,
                visibility_tx,
                rt.quit_flag,
            );

            #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
            let app = RiffApp::new(
                rt.playback,
                rt.library,
                rt.ui_transport,
                Box::new(rt.scans.clone()),
                rt.watcher_manager,
                tray_icon,
                rt.settings,
                rt.preferences,
                rt.playlists,
                rt.library_mutations,
                rt.session_views,
                rt.tag_edits,
                rt.covers,
                rt.backend_events,
                visibility_listener,
                visibility_tx,
            );

            #[cfg(target_os = "linux")]
            let app = RiffApp::new(
                rt.playback,
                rt.library,
                rt.ui_transport,
                Box::new(rt.scans.clone()),
                rt.watcher_manager,
                rt.settings,
                rt.preferences,
                rt.playlists,
                rt.library_mutations,
                rt.session_views,
                rt.tag_edits,
                rt.covers,
                rt.backend_events,
            );

            Ok(Box::new(app))
        }),
    )
    .expect("Failed to run eframe");

    // The window closed and `app` has been dropped, so nothing renders with
    // this runtime any more: stop the worker threads it spawned and wait for
    // each one to return. Every close that quits reaches this point the same
    // way — a Linux custom X, a Windows frameless OS close, and the macOS red
    // traffic light when "Quit on close" is on all exit through eframe, as
    // does the tray Quit (which enqueues the real close and wakes the loop).
    //
    // The tray Quit is never cancelled on macOS even though the red traffic
    // light's close is: both arrive as the same payload-free
    // `ViewportEvent::Close`, so the app separates them by the shared quit
    // flag the tray stores before enqueueing, and a committed quit is always
    // passed through (see `app::close_resolution`). Otherwise the default
    // preference would turn Quit into "hide to tray" and this line would never
    // run — leaving a running, playback-stopped process with a dead tray menu.
    lifecycle.shutdown();
}
