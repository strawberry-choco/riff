// Thin composition over the backend's Composition Root: `AppRuntime::spawn`
// owns every adapter, port wiring, and worker thread (backend-crate-split
// issue 08), so the binary only opens the Application Store at its default
// location, spawns the runtime, and hands the returned handles to the UI and
// the tray before running the eframe event loop.
use riff_backend::composition::AppRuntime;
use riff_gui::ui::RiffApp;
use riff_gui::ui::window_visibility::spawn_visibility_listener;

fn main() {
    tracing_subscriber::fmt::init();
    let store_path = riff_backend::composition::default_store_path().unwrap_or_else(|e| {
        panic!(
            "fatal: could not resolve the Application Store location \
             (no data-local directory available): {e}"
        )
    });
    let rt = AppRuntime::spawn(&store_path).unwrap_or_else(|e| panic!("fatal: {e}"));

    let options = eframe::NativeOptions {
        // Frameless launch (Issue 04, ADR 0005): OS decorations are replaced
        // by riff's custom titlebar with a drag region and window controls.
        viewport: riff_gui::ui::chrome::viewport_builder(),
        ..Default::default()
    };

    // Frontend-local visibility channel (Issue 03): the tray pushes
    // `Show Window` requests here, the UI thread drains them between frames.
    // The tray never constructs backend commands on this path.
    let (visibility_tx, visibility_listener) = spawn_visibility_listener();

    #[cfg(not(target_os = "linux"))]
    let tray_icon = match riff_gui::ui::tray::create_tray(
        rt.tray_transport,
        rt.playback.clone(),
        rt.quit_flag.clone(),
        visibility_tx,
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
        rt.quit_flag,
        rt.settings,
        rt.playlists,
        rt.library_mutations,
        rt.session_views,
        rt.tag_edits,
        rt.covers,
        rt.facade,
        visibility_listener,
    );

    #[cfg(target_os = "linux")]
    let app = RiffApp::new(
        rt.playback,
        rt.library,
        rt.ui_transport,
        Box::new(rt.scans.clone()),
        rt.watcher_manager,
        rt.quit_flag,
        rt.settings,
        rt.playlists,
        rt.library_mutations,
        rt.session_views,
        rt.tag_edits,
        rt.covers,
        rt.facade,
        visibility_listener,
    );

    run_native_app(app, options);
}

/// Hand the composed [`RiffApp`] to eframe: frameless native window with the
/// app's font configuration installed before the first frame.
fn run_native_app(app: RiffApp, options: eframe::NativeOptions) {
    eframe::run_native(
        "riff",
        options,
        Box::new(|cc| {
            riff_gui::ui::fonts::configure_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .expect("Failed to run eframe");
}
