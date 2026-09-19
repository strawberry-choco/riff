// Thin composition over the backend's Composition Root: `AppRuntime::spawn`
// owns every adapter, port wiring, and worker thread (backend-crate-split
// issue 08), so the binary only opens the Application Store at its default
// location, spawns the runtime, and hands the returned handles to the UI and
// the tray. The tray and the app are composed inside eframe's creation
// closure — the only place the egui context exists, and the tray needs a
// clone of it on its own thread to wake a sleeping event loop — before the
// first frame runs.
use riff_backend::composition::AppRuntime;
use riff_gui::ui::RiffApp;
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
        // Frameless launch (Issue 04, ADR 0005): OS decorations are replaced
        // by riff's custom titlebar with a drag region and window controls.
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

            #[cfg(not(target_os = "linux"))]
            let app = RiffApp::new(
                rt.playback,
                rt.library,
                rt.ui_transport,
                Box::new(rt.scans.clone()),
                rt.watcher_manager,
                tray_icon,
                rt.settings,
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
    // each one to return. Every close now reaches this point the same way —
    // OS close, a Linux custom X, and the tray Quit (which enqueues the real
    // close and wakes the loop) all exit through eframe.
    lifecycle.shutdown();
}
