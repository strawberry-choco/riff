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

// --- The N6 list and inspector probes ---------------------------------------

use riff_backend::app::traits::{DecodedCover, RequestedSize};
use riff_backend::domain::Track;
use riff_gui_nsh::components::track_row::{TrackRowView, row_of};
use riff_gui_nsh::screens::library::list::{PREFETCH_LOOKAHEAD, PREFETCH_LOOKBACK};

/// How many rows the list probe's store carries: enough that scrolling
/// recycles every visible cell several times over, small enough that a
/// cover answer can be queued for every row.
const PROBE_ROWS: usize = 200;

/// One probe track: the id encodes its row index, so the covers mock's
/// request log can be read as row numbers.
fn probe_row(i: usize) -> Track {
    crate::test_utils::create_test_track_with_metadata(
        &format!("/music/probe/{i:03}.wav"),
        &format!("/music/probe/{i:03}.wav"),
        "Probe Artist",
        &format!("Probe {i:03}"),
        "Probe Album",
    )
}

/// The covers mock with handles the test can reach after the runtime moved
/// it: the request log and the answer queue share via `Arc`s.
#[derive(Clone)]
struct SharedCovers(std::sync::Arc<crate::mocks::RecordingCovers>);

impl riff_backend::app::cover_service::Covers for SharedCovers {
    fn request(&self, track_id: TrackId, path: std::path::PathBuf, size: RequestedSize) {
        self.0.request(track_id, path, size);
    }

    fn request_folder(&self, folder: &std::path::Path, size: RequestedSize) {
        self.0.request_folder(folder, size);
    }

    fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
        self.0.poll()
    }

    fn clear_cache(&self) {
        self.0.clear_cache();
    }

    fn poll_cache_clear(&self) -> Option<riff_backend::app::cover_service::ClearCacheOutcome> {
        self.0.poll_cache_clear()
    }
}

/// The list probe: the mock runtime under the real shell, with a
/// [`PROBE_ROWS`]-row store and a covers mock that answers every request
/// with a queued thumbnail. The extra readouts (selection, covers-request
/// count) let a test read facts the engine thread holds.
#[component]
pub(crate) fn ListProbeApp() -> impl IntoElement {
    let playback = Arc::new(Mutex::new(PlaybackSession::default()));
    let tracks: Vec<Track> = (0..PROBE_ROWS).map(probe_row).collect();
    let library_ids: Vec<TrackId> = tracks.iter().map(|t| t.id.clone()).collect();
    let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
    spawn_command_applier(cmd_rx, Arc::clone(&playback), library_ids.clone());

    let backend_events = Arc::new(Mutex::new(BackendEvents::default()));
    let (_change_tx, change_rx) = unbounded::<StoreChanged>();
    backend_events
        .lock_or_recover()
        .subscribe_to_backend_changes(change_rx);
    let (_notice_tx, notice_rx) = unbounded::<String>();
    backend_events
        .lock_or_recover()
        .subscribe_playback_notices(notice_rx);

    let covers_mock = std::sync::Arc::new(crate::mocks::RecordingCovers::new());

    let settings_store = crate::mocks::MockSettingsStore::default();
    let preferences = Preferences::hydrate(
        &playback,
        &Arc::new(Mutex::new(
            riff_backend::app::state::LibrarySession::default(),
        )),
        &settings_store,
        &ChannelTransport::new(unbounded().0),
        &Arc::new(Mutex::new(None)),
    );

    let library_map: std::collections::HashMap<TrackId, Track> =
        tracks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    let views = SessionViews::new(
        Box::new(crate::mocks::MockLibraryQueryStore {
            library: library_map,
            flat: tracks,
            library_counts: riff_persistence::store::LibraryCounts {
                tracks: PROBE_ROWS,
                artists: 1,
                albums: 1,
                genres: 1,
            },
            ..crate::mocks::MockLibraryQueryStore::default()
        }),
        Box::new(crate::mocks::MockPlaylistStore::default()),
    );

    let runtime = AppRuntime {
        playback,
        library: Arc::new(Mutex::new(
            riff_backend::app::state::LibrarySession::default(),
        )),
        backend_events,
        ui_transport: Box::new(ChannelTransport::new(cmd_tx)),
        tray_transport: ChannelTransport::new(unbounded().0),
        preferences,
        scans: Box::new(crate::mocks::MockScans::default()),
        watcher_manager: Arc::new(Mutex::new(None)),
        quit_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        settings: Box::new(settings_store),
        playlists: Box::new(crate::mocks::MockPlaylistStore::default()),
        library_mutations: Box::new(crate::mocks::MockLibraryMutationStore::new()),
        session_views: views,
        tag_edits: Box::new(crate::mocks::FakeTagEdits::new()),
        passes: Box::new(crate::mocks::RecordingPasses::default()),
        covers: Box::new(SharedCovers(covers_mock.clone())),
        thumbnail_cache: Arc::new(StubThumbnailCache),
    };
    let requests_handle = covers_mock.clone();

    let model = stored(UiModel::new(runtime, None));
    install(model);
    let state = model.with(|m| m.signals());
    let requested_line = signal(String::new());
    let requested_out = requested_line;
    let requests_read = requests_handle.clone();
    let serve_handle = requests_handle.clone();
    let prefetch_model = model;
    let prefetch_state = state;

    view()
        .flex_1()
        .flex_col()
        .child(riff_gui_nsh::shell::install(model))
        .child(
            // The viewability report is the live handler's own trigger; the
            // probe's button drives the same window fn with a scripted pair,
            // so the headless test can assert the read discipline.
            button()
                .test_id("prefetch")
                .on_press(move || {
                    riff_gui_nsh::screens::library::list::prefetch(
                        prefetch_model,
                        &prefetch_state,
                        (0, 15),
                    );
                })
                .child(text("prefetch")),
        )
        .child(
            button()
                .test_id("serve")
                .on_press(move || {
                    for (id, _, size) in serve_handle.requested() {
                        serve_handle.serve_cover(
                            id,
                            size,
                            Some(DecodedCover {
                                rgba: [0x40u8, 0x80, 0xC0, 0xFF].repeat(56 * 56),
                                width: 56,
                                height: 56,
                            }),
                        );
                    }
                })
                .child(text("serve")),
        )
        .child(
            button()
                .test_id("read-requests")
                .on_press(move || {
                    let lines = requests_read
                        .requested()
                        .iter()
                        .filter_map(|(id, _, _)| {
                            id.0.rsplit('/')
                                .next()?
                                .trim_end_matches(".wav")
                                .parse::<usize>()
                                .ok()
                        })
                        .collect::<Vec<_>>();
                    requested_line.set(
                        lines
                            .iter()
                            .map(std::string::ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                })
                .child(text("read requests")),
        )
        .child(text(move || requested_out.get()).test_id("requests-count"))
}

/// The missing-row probe: the row component over a synthetic display model
/// the store cannot serve — the flat All Tracks listing carries only
/// scanned-and-present rows, so the marker's proof is synthetic by
/// construction (recorded in the N6 trail).
#[component]
pub(crate) fn MissingRowProbeApp() -> impl IntoElement {
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

    let views = SessionViews::new(
        Box::new(crate::mocks::MockLibraryQueryStore::default()),
        Box::new(crate::mocks::MockPlaylistStore::default()),
    );
    let runtime = AppRuntime {
        playback,
        library: Arc::new(Mutex::new(
            riff_backend::app::state::LibrarySession::default(),
        )),
        backend_events,
        ui_transport: Box::new(ChannelTransport::new(cmd_tx)),
        tray_transport: ChannelTransport::new(unbounded().0),
        preferences: Preferences::default(),
        scans: Box::new(crate::mocks::MockScans::default()),
        watcher_manager: Arc::new(Mutex::new(None)),
        quit_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        settings: Box::new(crate::mocks::MockSettingsStore::default()),
        playlists: Box::new(crate::mocks::MockPlaylistStore::default()),
        library_mutations: Box::new(crate::mocks::MockLibraryMutationStore::new()),
        session_views: views,
        tag_edits: Box::new(crate::mocks::FakeTagEdits::new()),
        passes: Box::new(crate::mocks::RecordingPasses::default()),
        covers: Box::new(crate::mocks::RecordingCovers::new()),
        thumbnail_cache: Arc::new(StubThumbnailCache),
    };
    let model = stored(UiModel::new(runtime, None));
    let state = model.with(|m| m.signals());
    let list_focus = signal(None::<usize>);
    let facts = || TrackRowView {
        index: 0,
        id: TrackId("/music/gone.wav".to_owned()),
        title: "Ghost Track".to_owned(),
        artist: "Ghost Artist".to_owned(),
        album: "Ghost Album".to_owned(),
        duration: Some(Duration::from_secs(61)),
        favorite: true,
        missing: true,
    };
    view()
        .flex_1()
        .flex_col()
        .p(px(8.0))
        .child(row_of(model, state, facts, list_focus))
}

// --- The N6 tests -----------------------------------------------------------

fn list_desktop() -> EngineConfig {
    let mut config = EngineConfig::new(Root::of::<ListProbeApp>());
    config.environment.os = nsh_core::platform::Os::Windows;
    // The N6 row's columns need a desktop stage: the default headless
    // viewport is the phone (correction 13), which leaves no width beside
    // the sidebar and the inspector.
    config.viewport = size(px(1280.0), px(800.0));
    config
}

fn missing_desktop() -> EngineConfig {
    let mut config = EngineConfig::new(Root::of::<MissingRowProbeApp>());
    config.environment.os = nsh_core::platform::Os::Windows;
    config.viewport = size(px(1280.0), px(800.0));
    config
}

#[test]
fn the_stage_renders_the_virtual_track_list() {
    let mut app = Headless::start(list_desktop()).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("text:Probe 000")
        .expect("the first row's pump-primed content");

    // The window is virtualized: some rows, never the whole store.
    let visible = app.screen().count("id:track-row").unwrap();
    assert!(
        visible > 0 && visible < PROBE_ROWS,
        "virtualized, got {visible}"
    );

    // The first row's content comes from the mock store.
    let title = app.screen().texts().join("\n");
    assert!(title.contains("Probe 000"), "the first row's title renders");
    assert!(title.contains("Probe Album"), "the album column renders");
}

#[test]
fn a_row_press_selects_and_the_inspector_follows() {
    let mut app = Headless::start(list_desktop()).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("text:Probe 000").expect("rows primed");
    app.wait_for("text:Nothing selected")
        .expect("the empty inspector");

    app.press("id:track-row >> nth=0").unwrap();
    app.wait_for("text:Probe 000").unwrap();
    app.wait_for_gone("text:Nothing selected", Duration::from_secs(2))
        .unwrap();

    app.press("id:track-row >> nth=1").unwrap();
    app.wait_for("text:Probe 001").unwrap();

    // The band follows the selection: the pressed row paints it, the other
    // does not.
    let first = app.screen().view_style("id:track-row >> nth=0").unwrap();
    let second = app.screen().view_style("id:track-row >> nth=1").unwrap();
    assert!(
        second.is_some_and(|style| style.background_color.is_some()),
        "the selected row's band paints"
    );
    assert!(
        first.is_none_or(|style| style.background_color.is_none()),
        "the deselected row's band drops"
    );
}

#[test]
fn the_selection_survives_recycling() {
    let mut app = Headless::start(list_desktop()).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("text:Probe 000").expect("rows primed");

    app.press("id:track-row >> nth=0").unwrap();
    app.wait_for("text:Probe 000").unwrap();

    // Away: far enough that every cell the selection was bound to is
    // recycled into other rows.
    for _ in 0..8 {
        app.scroll("id:track-list", px(1000.0)).unwrap();
    }
    app.wait_for("text:Probe 160").expect("scrolled deep");

    // Back: the same rows, re-bound by fresh cells.
    for _ in 0..8 {
        app.scroll("id:track-list", px(-1000.0)).unwrap();
    }
    app.wait_for("text:Probe 000").unwrap();
    let first = app.screen().view_style("id:track-row >> nth=0").unwrap();
    assert!(
        first.is_some_and(|style| style.background_color.is_some()),
        "the selection survived the recycle"
    );
    app.wait_for("text:Probe 000").unwrap();
}

#[test]
fn the_prefetch_window_requests_ascending_in_a_bounded_window() {
    let mut app = Headless::start(list_desktop()).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("text:Probe 000").expect("rows primed");

    // Drive the window the way the viewability handler does, with the
    // probe's scripted pair, then read the request log's row order.
    app.press("id:prefetch").unwrap();
    app.press("id:read-requests").unwrap();
    app.wait_for("text:/^0,/")
        .expect("the window asked for the viewable rows first");

    let text = app.screen().texts().join("\n");
    let line = text
        .lines()
        .find(|line| line.contains(','))
        .expect("the request-order readout");
    let indices: Vec<usize> = line
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    assert!(!indices.is_empty(), "the window requested covers");
    let sorted = indices
        .iter()
        .try_fold(0usize, |last, &next| (last <= next).then_some(next))
        .is_some();
    assert!(sorted, "requests ascend: {indices:?}");

    // The window is the bound: one report's widened range, never the store.
    let bound = PREFETCH_LOOKAHEAD + PREFETCH_LOOKBACK + 80;
    assert!(
        indices.len() <= bound,
        "the window stayed bounded: {} <= {bound}",
        indices.len()
    );
    // Ahead of the viewport: the lookahead's reach, not just the visible 28.
    assert!(
        indices.len() > 28,
        "the window prefetched ahead: {} requests",
        indices.len()
    );
}

#[test]
fn cover_arrivals_swap_the_placeholder_for_the_image() {
    let mut app = Headless::start(list_desktop()).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("text:Probe 000").expect("rows primed");

    // The rows open on the placeholder (nothing asked yet).
    let placeholders = app.screen().count("id:cover-placeholder").unwrap();
    assert!(placeholders > 0, "the tiles open on the placeholder");

    // Ask (the viewability handler's window), serve the answers, and the
    // pump's drain files them: the tiles' branch flips to the arrivals.
    // Headless proves the STRUCTURE of the RGBA path — not painted pixels
    // (correction 13: the pixels are the windowed screenshot's proof).
    app.press("id:prefetch").unwrap();
    app.press("id:serve").unwrap();
    app.wait_for_gone("id:cover-placeholder", Duration::from_secs(4))
        .expect("every tile shows its arrival");
}

#[test]
fn the_playing_marker_paints_on_the_current_row() {
    let mut app = Headless::start(list_desktop()).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("text:Probe 000").expect("rows primed");
    app.wait_for("label:Play").unwrap();

    // The applier's PlayRandom loads the store's first row: row 0 becomes
    // the current track and its row paints the marker.
    app.press("id:transport-play").unwrap();
    app.wait_for("label:Pause").unwrap();
    app.wait_for("id:row-playing")
        .expect("the current row carries the marker");
}

#[test]
fn the_missing_marker_renders_on_a_synthetic_row() {
    let mut app = Headless::start(missing_desktop()).unwrap();
    app.wait_for("id:row-missing").expect("the marker paints");
    let texts = app.screen().texts().join("\n");
    assert!(texts.contains("(missing)"), "the marker's word renders");
    assert!(
        texts.contains("Ghost Track"),
        "the row's other columns render"
    );
}

#[test]
fn the_traversal_reaches_the_list_and_enter_selects() {
    let mut app = Headless::start(list_desktop()).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("text:Probe 000").expect("rows primed");
    app.wait_for("text:Nothing selected")
        .expect("the inspector opens empty");

    // Through the chrome ring (7) and the sidebar's ten rows, into the
    // list; the step reveals the focused row and Enter selects it —
    // asserted by the selection's own facts, not by row 0's text (which
    // the mount already shows): the inspector's empty state goes, and the
    // focused row's band paints.
    for _ in 0..(7 + 10) {
        app.press_key("down").unwrap();
    }
    app.press_key("enter").unwrap();
    app.wait_for_gone("text:Nothing selected", Duration::from_secs(2))
        .expect("Enter selected the focused row");
    let first = app.screen().view_style("id:track-row >> nth=0").unwrap();
    assert!(
        first.is_some_and(|style| style.background_color.is_some()),
        "the focused row's band paints"
    );
}

// --- The N6 perf drive (ignored by default) ---------------------------------

/// The P0 scroll-budget drive, replayed against the nsh frontend's list:
/// headless over the REAL 50k fixture store (`RIFF_N3_STORE`, the .scratch
/// fixture — never the live store), jump deep, then 20 scroll steps of
/// 2000 pt timed. The P0 spike's direct per-row pattern measured ~3086
/// ms/step on this machine; the prefetch window is what must hold the
/// budget now. Run: `cargo test -p riff-tests n6_scroll_drive -- --ignored
/// --nocapture`.
/// The drive probe: the REAL runtime over the fixture store (the app's own
/// store path resolution), under the real shell — the production path
/// without the debug readout in front of it.
#[component]
pub(crate) fn DriveApp() -> impl IntoElement {
    riff_gui_nsh::theme::fonts::register_inter_fonts();
    let store_path = riff_gui_nsh::store_path();
    let (rt, lifecycle) = riff_backend::composition::AppRuntime::spawn(&store_path)
        .expect("the fixture store spawns");
    let model = stored(UiModel::new(rt, Some(lifecycle)));
    riff_gui_nsh::bridge::install(model);
    riff_gui_nsh::shell::install(model)
}

#[test]
#[ignore = "needs the synthetic 50k fixture store at RIFF_N3_STORE"]
fn n6_scroll_drive_over_the_fixture_store() {
    if std::env::var_os("RIFF_N3_STORE").is_none() {
        panic!("set RIFF_N3_STORE to the synthetic fixture store");
    }
    let mut config = EngineConfig::new(Root::of::<DriveApp>());
    config.environment.os = nsh_core::platform::Os::Windows;
    config.viewport = size(px(1280.0), px(800.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("id:track-list").unwrap();
    app.wait_for("label:All Tracks")
        .expect("the library primed");

    // Jump deep, settle, then time the steps.
    app.press_key("mod+alt+j").unwrap();
    std::thread::sleep(Duration::from_millis(1500));

    let steps = 20;
    let t0 = std::time::Instant::now();
    for _ in 0..steps {
        app.scroll("id:track-list", px(2000.0)).unwrap();
    }
    let total = t0.elapsed();
    println!(
        "N6 DRIVE: {steps} steps of 2000pt over 50k rows: total {} ms, per-step {:.0} ms (P0 upper bound ~3086 ms/step)",
        total.as_millis(),
        total.as_secs_f64() * 1000.0 / f64::from(steps),
    );
}

/// The read-cost probe behind the drive number: pure `track_row` timings at
/// depth, no UI — what part of a scroll step is the seam's reads.
#[test]
#[ignore = "needs the synthetic 50k fixture store at RIFF_N3_STORE"]
fn n6_read_cost_probe() {
    use riff_backend::app::state::TrackSort;
    if std::env::var_os("RIFF_N3_STORE").is_none() {
        panic!("set RIFF_N3_STORE to the synthetic fixture store");
    }
    let store_path = riff_gui_nsh::store_path();
    let (mut rt, _lifecycle) =
        riff_backend::composition::AppRuntime::spawn(&store_path).expect("fixture store");
    let reads_at = |rt: &mut riff_backend::composition::AppRuntime,
                    start: usize,
                    count: usize|
     -> std::time::Duration {
        let t0 = std::time::Instant::now();
        for i in 0..count {
            let _ = rt
                .session_views
                .track_row("", TrackSort::NumberAsc, start + i);
        }
        t0.elapsed()
    };
    let _ = rt.session_views.track_count("", TrackSort::NumberAsc);
    let cold = reads_at(&mut rt, 40000, 50);
    let warm = reads_at(&mut rt, 40000, 50);
    let next_window = reads_at(&mut rt, 40050, 50);
    let walk = reads_at(&mut rt, 40100, 100);
    println!(
        "N6 READS: depth-40000 cold(50) {cold:?} warm(50) {warm:?} next-window(50) {next_window:?} walk(100) {walk:?}"
    );
}
