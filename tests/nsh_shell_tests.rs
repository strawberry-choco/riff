//! N4's app shell, driven headless over the mock runtime: the real
//! `shell::install` mounted over the same mock ports the bridge tests
//! compose, so the suite asserts the production tree, not a stand-in.
//!
//! Three proofs, one per behavior the shell owns that a headless engine can
//! see. Space toggles playback through the shell's key map and the real
//! transport. The window-frame loop runs whole: a fresh store's shell
//! reports the headless window's frame and the pump's frame-end diff-commit
//! lands it (read back through the port), and a store that carries a saved
//! frame restores it onto the headless window at mount. A press is what
//! drives `catch_up`, so the persist proof presses the reader twice: the
//! first press settles the commit, the second reads the settled row.
//!
//! What is not assertable headless is recorded in the N4 evidence: the
//! search field's real input focus (mod+k) has no headless text surface.

use std::sync::{Arc, Mutex};

use crossbeam_channel::unbounded;
use nsh_base::button;
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use riff_backend::app::StubThumbnailCache;
use riff_backend::app::events::BackendEvents;
use riff_backend::app::preferences::{Preferences, WindowGeometry};
use riff_backend::app::state::PlaybackSession;
use riff_backend::app::store::StoreChanged;
use riff_backend::app::transport::ChannelTransport;
use riff_backend::app::views::SessionViews;
use riff_backend::composition::AppRuntime;
use riff_backend::domain::PlaybackCommand;
use riff_gui_nsh::bridge::{UiModel, install};

use riff_backend::app::MutexExt;

use crate::mocks::{
    FakeTagEdits, MockLibraryMutationStore, MockLibraryQueryStore, MockPlaylistStore, MockScans,
    MockSettingsStore, RecordingCovers, RecordingPasses, SettingsCall,
};
use crate::nsh_bridge_tests::spawn_command_applier;

/// The frame the restore probe's store carries, small enough to fit the
/// headless display's visible bounds untouched.
const RESTORED_FRAME: WindowGeometry = WindowGeometry {
    x: Some(64.0),
    y: Some(32.0),
    width: Some(800.0),
    height: Some(600.0),
    maximized: false,
};

fn desktop() -> EngineConfig {
    let mut config = EngineConfig::new(Root::of::<ShellProbeApp>());
    config.environment.os = nsh_core::platform::Os::Windows;
    config
}

fn desktop_restore() -> EngineConfig {
    let mut config = EngineConfig::new(Root::of::<ShellRestoreProbeApp>());
    config.environment.os = nsh_core::platform::Os::Windows;
    config
}

/// The readout the store probe renders once a press reads the settings row
/// back through the port.
fn geometry_text(geometry: WindowGeometry) -> String {
    format!(
        "geom {:?} {:?} {:?} {:?} {}",
        geometry.x, geometry.y, geometry.width, geometry.height, geometry.maximized
    )
}

/// The probe's one body: the mock runtime (the bridge tests' composition)
/// under the real shell, plus a press that counts the settings commits the
/// pump's diff-commit made and renders the persisted row. The handle is
/// hydrated first so the sessions and the snapshot agree — the one diff a
/// resting probe can produce is the shell's own window-frame report.
fn probe(preset: Option<WindowGeometry>) -> impl IntoElement {
    let playback = Arc::new(Mutex::new(PlaybackSession::default()));
    let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
    spawn_command_applier(cmd_rx, Arc::clone(&playback), Vec::new());

    let backend_events = Arc::new(Mutex::new(BackendEvents::default()));
    let (_change_tx, change_rx) = unbounded::<StoreChanged>();
    backend_events
        .lock_or_recover()
        .subscribe_to_backend_changes(change_rx);
    let (_notice_tx, notice_rx) = unbounded::<String>();
    backend_events
        .lock_or_recover()
        .subscribe_playback_notices(notice_rx);

    let shared_row: Arc<Mutex<riff_backend::app::store::Settings>> =
        Arc::new(Mutex::new(Default::default()));
    let shared_row_read = Arc::clone(&shared_row);
    let shared_calls: Arc<Mutex<Vec<SettingsCall>>> = Arc::new(Mutex::new(Vec::new()));
    let shared_calls_read = Arc::clone(&shared_calls);
    let mut settings_store = MockSettingsStore {
        shared_calls: Some(shared_calls),
        shared_state: Some(shared_row),
        ..MockSettingsStore::default()
    };
    if let Some(geometry) = preset {
        settings_store.state.scalars.window_x = geometry.x;
        settings_store.state.scalars.window_y = geometry.y;
        settings_store.state.scalars.window_width = geometry.width;
        settings_store.state.scalars.window_height = geometry.height;
        settings_store.state.scalars.window_maximized = geometry.maximized;
    }
    let library = Arc::new(Mutex::new(
        riff_backend::app::state::LibrarySession::default(),
    ));
    let preferences = Preferences::hydrate(
        &playback,
        &library,
        &settings_store,
        &ChannelTransport::new(unbounded().0),
        &Arc::new(Mutex::new(None)),
    );

    let views = SessionViews::new(
        Box::new(MockLibraryQueryStore {
            // The settled-library seam: a store the scan finished on long
            // ago, whose counts the sidebar must show without any
            // LibraryChanged event firing.
            library_counts: riff_persistence::store::LibraryCounts {
                tracks: 2,
                artists: 1,
                albums: 1,
                genres: 1,
            },
            ..MockLibraryQueryStore::default()
        }),
        Box::new(MockPlaylistStore::default()),
    );

    let runtime = AppRuntime {
        playback,
        library,
        backend_events,
        ui_transport: Box::new(ChannelTransport::new(cmd_tx)),
        tray_transport: ChannelTransport::new(unbounded().0),
        preferences,
        scans: Box::new(MockScans::default()),
        watcher_manager: Arc::new(Mutex::new(None)),
        quit_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        settings: Box::new(settings_store),
        playlists: Box::new(MockPlaylistStore::default()),
        library_mutations: Box::new(MockLibraryMutationStore::new()),
        session_views: views,
        tag_edits: Box::new(FakeTagEdits::new()),
        passes: Box::new(RecordingPasses::default()),
        covers: Box::new(RecordingCovers::new()),
        thumbnail_cache: Arc::new(StubThumbnailCache),
    };

    let model = stored(UiModel::new(runtime, None));
    install(model);
    let state = model.with(|m| m.signals());
    let row = signal(String::new());
    let row_line = row;
    let commits = signal(0_usize);
    let commits_line = commits;

    view()
        .flex_1()
        .flex_col()
        .child(riff_gui_nsh::shell::install(model))
        .child(
            text(move || {
                let view = state.sidebar.get();
                format!(
                    "sidebar {} {} {} {} {}",
                    view.tracks, view.artists, view.albums, view.genres, view.smart_lists
                )
            })
            .test_id("sidebar-view"),
        )
        .child(
            button()
                .test_id("read-store")
                .on_press(move || {
                    let count = shared_calls_read
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .len();
                    commits.set(count);
                    let settings = shared_row_read
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let scalars = settings.scalars.clone();
                    row.set(geometry_text(WindowGeometry {
                        x: scalars.window_x,
                        y: scalars.window_y,
                        width: scalars.window_width,
                        height: scalars.window_height,
                        maximized: scalars.window_maximized,
                    }));
                })
                .child(text("read store")),
        )
        .child(text(move || row_line.get()).test_id("store-geometry"))
        .child(text(move || format!("commits {}", commits_line.get())).test_id("commit-count"))
}

/// The persist probe: a store with no saved frame, over a headless desktop
/// window the shell reports.
#[component]
pub(crate) fn ShellProbeApp() -> impl IntoElement {
    probe(None)
}

/// The restore probe: a store carrying [`RESTORED_FRAME`], which the shell
/// must put back onto the headless window at mount.
#[component]
pub(crate) fn ShellRestoreProbeApp() -> impl IntoElement {
    probe(Some(RESTORED_FRAME))
}

#[test]
fn space_toggles_playback_over_the_shell() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:player-bar").unwrap();
    app.wait_for("label:Play").unwrap();

    app.press_key("space").unwrap();
    app.wait_for("label:Pause").unwrap();
    app.press_key("space").unwrap();
    app.wait_for("label:Play").unwrap();
}

#[test]
fn the_shell_persists_its_window_frame_through_the_settings_commit() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:player-bar").unwrap();

    let state = app
        .window_state("main")
        .unwrap()
        .expect("a headless desktop has a main window");
    let expected = geometry_text(riff_gui_nsh::shell::layout::geometry_of(state.placement));

    // The first press's catch_up runs the pump ticks that settle the
    // commit; the second reads the settled row through the port.
    app.press("id:read-store").unwrap();
    app.press("id:read-store").unwrap();
    app.wait_for(&format!("text:{expected}")).unwrap();
}

#[test]
fn the_frame_commit_is_the_shell_report_alone() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:player-bar").unwrap();

    // Hydrate seeded the snapshot, the sessions rest untouched, so the one
    // scalar commit the pump can make is the shell's window-frame report.
    app.press("id:read-store").unwrap();
    app.press("id:read-store").unwrap();
    app.wait_for("text:commits 1").unwrap();
}

#[test]
fn the_sidebar_primes_on_a_settled_library() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:sidebar-view").unwrap();

    // No LibraryChanged event ever fires in this probe: the counts the
    // readout settles on are the first tick's priming read off the mock's
    // seeded LibraryCounts, not an event's payload. The default view would
    // read all zeros and this would time out.
    app.wait_for("text:sidebar 2 1 1 1 0").unwrap();
}

#[test]
fn the_shell_restores_the_persisted_frame_at_launch() {
    let mut app = Headless::start(desktop_restore()).unwrap();
    app.wait_for("id:player-bar").unwrap();

    let state = app
        .window_state("main")
        .unwrap()
        .expect("a headless desktop has a main window");
    assert_eq!(state.bounds.origin.x.0, RESTORED_FRAME.x.unwrap());
    assert_eq!(state.bounds.origin.y.0, RESTORED_FRAME.y.unwrap());
    assert_eq!(state.bounds.size.width.0, RESTORED_FRAME.width.unwrap());
    assert_eq!(state.bounds.size.height.0, RESTORED_FRAME.height.unwrap());
    assert_eq!(
        state.mode,
        nsh_core::desktop::WindowMode::Normal,
        "the restored frame is not maximized"
    );
}
