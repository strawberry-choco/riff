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
use std::time::Duration;

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
use riff_backend::domain::{PlaybackCommand, PlaylistId, SmartPlaylistKind, TrackId};
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
    let library_read = Arc::clone(&library);
    let preferences = Preferences::hydrate(
        &playback,
        &library,
        &settings_store,
        &ChannelTransport::new(unbounded().0),
        &Arc::new(Mutex::new(None)),
    );

    // The sidebar's seed: every section kind carries a non-default answer —
    // the library totals, one built-in smart list's total, and one user
    // playlist's entry count — so a row that shows a default is visible.
    let playlist_store = MockPlaylistStore::with_playlists(vec![(
        PlaylistId("mix".to_owned()),
        "Mix".to_owned(),
        vec![
            TrackId("/music/probe/one.wav".to_owned()),
            TrackId("/music/probe/two.wav".to_owned()),
            TrackId("/music/probe/three.wav".to_owned()),
        ],
    )]);

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
            smart_list_counts: vec![
                (SmartPlaylistKind::Favorites, 4),
                (SmartPlaylistKind::RecentlyAdded, 7),
            ],
            ..MockLibraryQueryStore::default()
        }),
        Box::new(playlist_store.clone()),
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
        playlists: Box::new(playlist_store),
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
    let session = signal(String::new());
    let session_line = session;
    let nav = state.nav;

    view()
        .flex_1()
        .flex_col()
        .child(riff_gui_nsh::shell::install(model))
        .child(
            text(move || {
                let view = state.sidebar.get();
                format!(
                    "sidebar {} {} {} {} {} smart {} playlists {}",
                    view.tracks,
                    view.artists,
                    view.albums,
                    view.genres,
                    view.folder_roots,
                    view.smart_lists
                        .iter()
                        .find(|(k, _)| *k == SmartPlaylistKind::Favorites)
                        .map_or(0, |(_, n)| *n),
                    view.playlists
                        .iter()
                        .map(|row| format!("{}({})", row.name, row.tracks))
                        .collect::<Vec<_>>()
                        .join(","),
                )
            })
            .test_id("sidebar-view"),
        )
        .child(text(move || format!("nav {:?}", nav.get())).test_id("nav-view"))
        .child(text(move || session_line.get()).test_id("session-view"))
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
        .child(
            button()
                .test_id("read-session")
                .on_press(move || {
                    let library = library_read.lock_or_recover();
                    session.set(format!(
                        "session {:?} {:?} {:?}",
                        library.view_mode, library.browse_mode, library.library_section
                    ));
                })
                .child(text("read session")),
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
    // read all zeros and this would time out. Every section kind carries a
    // seeded answer — the library totals, the built-in smart list's total,
    // the user playlist's entry count.
    app.wait_for("text:sidebar 2 1 1 1 0 smart 4 playlists Mix(3)")
        .unwrap();
    app.wait_for("label:Mix (3)").unwrap();
    app.wait_for("label:All Tracks").unwrap();
    app.wait_for("label:Favorites").unwrap();
}

/// The selection: the artists row's press lands the egui
/// `land_on_library` field set on the session, and the row's band paints
/// the egui selected fill while the other rows stay bare. Both facts
/// survive the pump's ticks — a selection write mutates no store, so the
/// write travels through the nav signal alone.
#[test]
fn a_selection_write_lands_the_land_on_library_field_set() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:sidebar-view").unwrap();

    app.press("id:nav-artists").unwrap();
    app.wait_for("text:/library_section: Artists/")
        .expect("the nav signal carries the selection");

    app.press("id:read-session").unwrap();
    app.wait_for("text:session Library Library Artists")
        .expect("the session holds the land_on_library field set");

    let artists = app.screen().view_style("id:nav-artists").unwrap();
    let genres = app.screen().view_style("id:nav-genres").unwrap();
    assert!(
        artists.is_some_and(|style| style.background_color.is_some()),
        "the active row's band paints"
    );
    assert!(
        genres.is_none_or(|style| style.background_color.is_none()),
        "an inactive row paints no band"
    );

    // A second selection moves the band with it.
    app.press("id:nav-genres").unwrap();
    app.wait_for("text:/library_section: Genres/").unwrap();
    let artists = app.screen().view_style("id:nav-artists").unwrap();
    let genres = app.screen().view_style("id:nav-genres").unwrap();
    assert!(
        artists.is_none_or(|style| style.background_color.is_none()),
        "the old row's band drops"
    );
    assert!(
        genres.is_some_and(|style| style.background_color.is_some()),
        "the new row's band paints"
    );
}

/// The traversal: the arrows walk the chrome ring into the sidebar's rows
/// and Enter activates the focused row, exactly the row's own press. The
/// active section is visible after traversal — the band followed the
/// activation.
#[test]
fn arrows_reach_every_section_and_enter_activates() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:sidebar-view").unwrap();

    // From nothing: search, play, next, mute, shuffle, repeat, then the
    // sidebar's first row — All Tracks — then Artists.
    for _ in 0..7 {
        app.press_key("down").unwrap();
    }
    app.press_key("down").unwrap();
    app.press_key("down").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("text:/library_section: Artists/")
        .expect("Enter activated the focused row");
    app.press("id:read-session").unwrap();
    app.wait_for("text:session Library Library Artists")
        .unwrap();

    // Space activates a focused row the same way — and must not toggle
    // playback while a row holds the traversal focus.
    app.press_key("down").unwrap();
    app.press_key("down").unwrap();
    app.press_key("space").unwrap();
    app.wait_for("text:/library_section: Genres/").unwrap();

    // Stepping back up leaves the rows and the chrome keeps its slots.
    app.press_key("up").unwrap();
    app.press_key("up").unwrap();
    app.press_key("up").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("text:/library_section: AllTracks/").unwrap();
}

/// The fold: the collapse chevron's press flips the persisted flag through
/// the session and the pump's settings diff lands it in the snapshot the
/// rows read, so the smart list rows fold away — and an opened smart list
/// closes with the section.
#[test]
fn the_fold_chevron_collapses_the_smart_lists() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:sidebar-view").unwrap();

    app.wait_for("label:Favorites").unwrap();
    app.press("id:nav-favorites").unwrap();
    app.wait_for("text:/smart_list: Some.Favorites./").unwrap();

    app.press("id:smart-lists-collapse").unwrap();
    app.wait_for_gone("label:Favorites", Duration::from_secs(2))
        .unwrap();
    app.wait_for("text:/smart_list: None/")
        .expect("the fold closed the opened list");
    app.press("id:read-session").unwrap();
    app.wait_for("text:session Library Library AllTracks")
        .unwrap();
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
