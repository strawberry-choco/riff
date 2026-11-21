//! N3's backend bridge, driven headless over the mock runtime.
//!
//! The bridge's concrete `AppRuntime` composes over this crate's mock ports
//! directly (every field is a port handle the `composed_shell` twin already
//! wires), so no seam was invented: the test component below builds the
//! runtime from the mocks, the pump (`riff_gui_nsh::bridge::install`) is
//! installed once, and `nsh_testing::Headless` drives the buttons and reads
//! the readout the way a user would.
//!
//! The one engine-side stand-in: the mock runtime's playback "engine" is a
//! command-applier thread (the session semantics of `Play`/`Seek`/`Next`/
//! `AddToQueue` written straight onto the shared session). The real
//! `AudioEngine` over `MockAudioOutput` has no consumer backpressure — its
//! decode loop drains the whole scripted track to EOF in milliseconds and
//! `TrackEnded` parks playback — so the `Playing` state it produces is
//! unobservable at a readout. The bridge path under test (command → session
//! → pump → signal) is unchanged by that substitution; the real engine's own
//! semantics stay with `app_tests`' engine-loop suite.
//!
//! Every assertion below is a readout change, and the pump is the only
//! writer of the signals the readout renders (P0 correction 20: writes
//! happen strictly outside the `stored` borrow — this suite is the
//! regression gate; a write inside the borrow panics the engine thread and
//! fails every test here).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use nsh_base::button;
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use riff_backend::app::StubThumbnailCache;
use riff_backend::app::events::BackendEvents;
use riff_backend::app::preferences::Preferences;
use riff_backend::app::state::{LibrarySession, PlaybackQueue, PlaybackSession, PlaybackState};
use riff_backend::app::store::{LibraryCounts, StoreChanged};
use riff_backend::app::transport::{ChannelTransport, Transport};
use riff_backend::app::views::SessionViews;
use riff_backend::composition::AppRuntime;
use riff_backend::domain::{PlaybackCommand, Track, TrackId};
use riff_gui_nsh::bridge::{UiModel, install};

use crate::MutexExt;
use crate::mocks::{
    FakeTagEdits, MockLibraryMutationStore, MockLibraryQueryStore, MockPlaylistStore, MockScans,
    MockSettingsStore, RecordingCovers, RecordingPasses, SettingsCall,
};
use crate::test_utils::create_test_track_with_metadata;

// --- The mock runtime ------------------------------------------------------

/// How long the applier's tracks "are", so a seek has a total to clamp
/// against the way the engine's decoder-provided duration would.
const PROBE_TRACK_DURATION: Duration = Duration::from_secs(120);

/// The two tracks the mock store carries: titles and artists the readout
/// renders, so the tests can assert on words, not ids.
pub(crate) fn probe_tracks() -> (Track, Track) {
    let one = create_test_track_with_metadata(
        "/music/probe/one.wav",
        "/music/probe/one.wav",
        "Probe Artist",
        "Probe One",
        "Probe Album",
    );
    let two = create_test_track_with_metadata(
        "/music/probe/two.wav",
        "/music/probe/two.wav",
        "Probe Artist",
        "Probe Two",
        "Probe Album",
    );
    (one, two)
}

/// The mock runtime's playback engine: drain commands and write their
/// session semantics. Runs parked forever on its own thread, exactly like
/// `app_tests`' engine harness leaves its own.
pub(crate) fn spawn_command_applier(
    cmd_rx: Receiver<PlaybackCommand>,
    state: Arc<Mutex<PlaybackSession>>,
    library_ids: Vec<TrackId>,
) {
    std::thread::spawn(move || {
        while let Ok(command) = cmd_rx.recv() {
            let mut session = state.lock_or_recover();
            match command {
                PlaybackCommand::Play(id) | PlaybackCommand::PlayAlbum(id) => {
                    session.queue = PlaybackQueue::new(vec![id]);
                    session.current_position.total = Some(PROBE_TRACK_DURATION);
                    session.playback_state = PlaybackState::Playing;
                }
                PlaybackCommand::PlayRandom => {
                    let pick = library_ids
                        .first()
                        .cloned()
                        .expect("the probe library is never empty");
                    session.queue = PlaybackQueue::new(vec![pick]);
                    session.current_position.total = Some(PROBE_TRACK_DURATION);
                    session.playback_state = PlaybackState::Playing;
                }
                PlaybackCommand::Pause => session.playback_state = PlaybackState::Paused,
                PlaybackCommand::Resume => session.playback_state = PlaybackState::Playing,
                PlaybackCommand::Stop => session.playback_state = PlaybackState::Stopped,
                PlaybackCommand::Seek(target) => session.current_position.current = target,
                PlaybackCommand::SetVolume(volume) => session.current_volume = volume,
                PlaybackCommand::Next => {
                    session.queue.current_index = session
                        .queue
                        .current_index
                        .and_then(|index| index.checked_add(1))
                        .filter(|index| *index < session.queue.tracks.len());
                }
                PlaybackCommand::Previous => {
                    session.queue.current_index = session
                        .queue
                        .current_index
                        .and_then(|index| index.checked_sub(1));
                }
                PlaybackCommand::PlayNext(id) => {
                    let at = session.queue.current_index.map_or(0, |index| index + 1);
                    session.queue.tracks.insert(at, id);
                }
                PlaybackCommand::AddToQueue(id) => {
                    session.queue.tracks.push(id);
                }
                PlaybackCommand::AddMany(ids) => session.queue.tracks.extend(ids),
                PlaybackCommand::PlayPause => match session.playback_state {
                    PlaybackState::Playing => session.playback_state = PlaybackState::Paused,
                    _ => session.playback_state = PlaybackState::Playing,
                },
            }
        }
    });
}

/// The test component: the mock runtime, the pump, and the readout-plus-
/// triggers UI. Everything lives on the engine thread (nsh signals are
/// `!Send`); the test thread asserts only through the screen.
#[component]
pub(crate) fn BridgeProbeApp() -> impl IntoElement {
    let (one, two) = probe_tracks();
    let library_ids = vec![one.id.clone(), two.id.clone()];

    // The playback session the pump reads; starts at half volume so a
    // volume step moves the scalar row the settings commit diffs.
    let playback = Arc::new(Mutex::new(PlaybackSession {
        current_volume: 0.5,
        ..PlaybackSession::default()
    }));

    // The command channel: the UI transport and the applier share it, the
    // same shape the Composition Root wires. The session-coupled transport
    // methods (`seek`, `set_volume`) run against the shared session, so the
    // trigger buttons hold their own `Arc` clones of it.
    let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
    spawn_command_applier(cmd_rx, Arc::clone(&playback), library_ids.clone());
    let session_for_playpause = Arc::clone(&playback);
    let session_for_seek = Arc::clone(&playback);
    let session_for_volume = Arc::clone(&playback);

    // The event inbox, with the two channels the store-side effects ride:
    // the store's Library generations and the coordinator's notices. The
    // trigger buttons below send on them the way the real store and
    // coordinator do.
    let backend_events = Arc::new(Mutex::new(BackendEvents::default()));
    let (change_tx, change_rx) = unbounded::<StoreChanged>();
    backend_events
        .lock_or_recover()
        .subscribe_to_backend_changes(change_rx);
    let (notice_tx, notice_rx) = unbounded::<String>();
    backend_events
        .lock_or_recover()
        .subscribe_playback_notices(notice_rx);

    // The mock stores. The playlist generation handle is shared with the
    // component so its trigger can move it; the settings log is shared so
    // the readout can show what the pump's diff-commit landed.
    let query = MockLibraryQueryStore {
        library: [(one.id.clone(), one.clone()), (two.id.clone(), two.clone())]
            .into_iter()
            .collect(),
        flat: vec![one.clone(), two.clone()],
        library_counts: LibraryCounts {
            tracks: 2,
            artists: 1,
            albums: 1,
            genres: 1,
        },
        ..MockLibraryQueryStore::default()
    };
    let playlist = MockPlaylistStore::default();
    let playlist_gen = playlist.generation.clone();
    let settings_log: Arc<Mutex<Vec<SettingsCall>>> = Arc::new(Mutex::new(Vec::new()));
    let settings_log_line = Arc::clone(&settings_log);

    let views = SessionViews::new(Box::new(query), Box::new(playlist.clone()));

    let runtime = AppRuntime {
        playback,
        library: Arc::new(Mutex::new(LibrarySession::default())),
        backend_events,
        ui_transport: Box::new(ChannelTransport::new(cmd_tx.clone())),
        tray_transport: ChannelTransport::new(unbounded().0),
        preferences: Preferences::default(),
        scans: Box::new(MockScans::default()),
        watcher_manager: Arc::new(Mutex::new(None)),
        quit_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        settings: Box::new(MockSettingsStore::with_shared_calls(settings_log)),
        playlists: Box::new(playlist),
        library_mutations: Box::new(MockLibraryMutationStore::new()),
        session_views: views,
        tag_edits: Box::new(FakeTagEdits::new()),
        passes: Box::new(RecordingPasses::default()),
        covers: Box::new(RecordingCovers::new()),
        thumbnail_cache: Arc::new(StubThumbnailCache),
    };

    // The bridge, exactly as the app builds it: model, pump, signal handles.
    let model = stored(UiModel::new(runtime, None));
    install(model);
    let state = model.with(|m| m.signals());
    let playback_sig = state.playback;
    let library_sig = state.library_revision;
    let playlist_sig = state.playlist_revision;
    let covers_sig = state.covers;
    let notices_sig = state.notices;

    let transport = ChannelTransport::new(cmd_tx.clone());
    let transport_playpause = transport.clone();
    let transport_next = transport.clone();
    let transport_seek = transport.clone();
    let transport_add = transport.clone();
    let transport_volume = transport.clone();
    let add_target = library_ids[1].clone();
    let change_tx_lib = change_tx.clone();
    let notice_tx_line = notice_tx.clone();

    view()
        .flex_1()
        .flex_col()
        .gap_2()
        .p(px(16.0))
        .child(
            text(move || {
                let v = playback_sig.get();
                let duration = v
                    .duration_secs
                    .map_or("--".to_owned(), |d| format!("{d:.1}"));
                format!(
                    "{} pos {:.1} dur {duration} vol {:.2} muted {}",
                    state_word(v.state),
                    v.position_secs,
                    v.volume,
                    v.muted,
                )
            })
            .test_id("playback"),
        )
        .child(
            text(move || {
                let v = playback_sig.get();
                format!(
                    "queue {} of {} now {}",
                    v.queue_index,
                    v.queue_len,
                    v.current.as_ref().map_or_else(
                        || "nothing".to_owned(),
                        |track| track.metadata.title.clone().unwrap_or_default()
                    ),
                )
            })
            .test_id("queue"),
        )
        .child(
            text(move || {
                format!(
                    "lib {} pl {} covers {}",
                    library_sig.get(),
                    playlist_sig.get(),
                    covers_sig.get(),
                )
            })
            .test_id("revisions"),
        )
        .child(
            // The settings-commit count rides the volume read: a binding
            // re-runs only when a signal it reads changes (P0 correction
            // 21), and the volume is what a settings write moves.
            text(move || {
                let commits = settings_log_line
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .filter(|call| matches!(call, SettingsCall::Scalars))
                    .count();
                format!("vol {:.2} store {commits}", playback_sig.get().volume)
            })
            .test_id("store"),
        )
        .child(
            text(move || {
                notices_sig.get().last().map_or_else(
                    || "no notices".to_owned(),
                    |notice| format!("notice {}", notice.message),
                )
            })
            .test_id("notice"),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    button()
                        .test_id("play")
                        .on_press(move || transport.play_random())
                        .child(text("Play")),
                )
                .child(
                    button()
                        .test_id("playpause")
                        .on_press(move || {
                            transport_playpause
                                .play_pause(&session_for_playpause.lock_or_recover());
                        })
                        .child(text("PlayPause")),
                )
                .child(
                    button()
                        .test_id("next")
                        .on_press(move || transport_next.next())
                        .child(text("Next")),
                )
                .child(
                    button()
                        .test_id("seek")
                        .on_press(move || {
                            transport_seek.seek(&session_for_seek.lock_or_recover(), 1.0);
                        })
                        .child(text("Seek")),
                )
                .child(
                    button()
                        .test_id("add")
                        .on_press(move || transport_add.add_to_queue(add_target.clone()))
                        .child(text("Add")),
                )
                .child(
                    button()
                        .test_id("vol-up")
                        .on_press(move || {
                            transport_volume
                                .set_volume(&mut session_for_volume.lock_or_recover(), 0.6);
                        })
                        .child(text("VolUp")),
                )
                .child(
                    button()
                        .test_id("lib-change")
                        .on_press(move || {
                            let _ = change_tx_lib.send(StoreChanged::Library(3));
                        })
                        .child(text("LibChange")),
                )
                .child(
                    button()
                        .test_id("playlist-change")
                        .on_press(move || {
                            playlist_gen.bump();
                        })
                        .child(text("PlaylistChange")),
                )
                .child(
                    button()
                        .test_id("notice")
                        .on_press(move || {
                            let _ = notice_tx_line.send("the engine said something".to_owned());
                        })
                        .child(text("Notice")),
                ),
        )
}

fn state_word(state: PlaybackState) -> &'static str {
    match state {
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Stopped => "stopped",
    }
}

// --- The tests -------------------------------------------------------------

/// Start the probe headless and wait for its first stopped readout.
fn start_probe() -> Headless {
    let mut app = Headless::start(EngineConfig::new(Root::of::<BridgeProbeApp>()))
        .expect("headless probe starts");
    app.wait_for("text:/^stopped/").expect("initial readout");
    app
}

#[test]
fn a_play_command_changes_the_playback_view_within_one_pump_tick() {
    let mut app = start_probe();

    app.press("id:play").expect("play presses");
    // The readout renders the signal, and the pump is its only writer: the
    // playing state can appear here only through a pump tick that read the
    // session the applier moved.
    app.wait_for("text:/^playing/").expect("playing in a tick");
    app.wait_for("text:/queue 0 of 1/").expect("queue filled");
}

#[test]
fn pause_resume_seek_round_trip_through_the_pump() {
    let mut app = start_probe();

    app.press("id:play").expect("play presses");
    app.wait_for("text:/^playing/").expect("playing");
    app.press("id:playpause").expect("pause presses");
    app.wait_for("text:/^paused/").expect("paused");
    app.press("id:playpause").expect("resume presses");
    app.wait_for("text:/^playing/").expect("playing again");

    app.press("id:seek").expect("seek presses");
    app.wait_for("text:/pos 1\\./")
        .expect("position at the seek target");
}

#[test]
fn a_queue_change_from_the_engine_lands_in_the_signal() {
    let mut app = start_probe();

    app.press("id:play").expect("play presses");
    app.wait_for("text:/queue 0 of 1/")
        .expect("one-track queue");

    app.press("id:add").expect("add presses");
    app.wait_for("text:/queue 0 of 2/")
        .expect("queue grew to two");

    app.press("id:next").expect("next presses");
    app.wait_for("text:/queue 1 of 2/").expect("index advanced");
}

#[test]
fn library_and_playlist_revisions_move_with_their_generations() {
    let mut app = start_probe();

    // The priming tick lands the first projections, so the readout starts at
    // lib 0 (no event yet) and pl 1 (first playlist load).
    app.wait_for("text:/lib 0/")
        .expect("library revision starts at 0");
    app.wait_for("text:/pl 1/")
        .expect("playlist revision primed");

    app.press("id:lib-change").expect("lib trigger presses");
    app.wait_for("text:/lib 3/")
        .expect("library generation landed");

    app.press("id:playlist-change")
        .expect("playlist trigger presses");
    app.wait_for("text:/pl 2/")
        .expect("playlist generation landed");
}

#[test]
fn notices_reach_the_signal() {
    let mut app = start_probe();
    app.wait_for("text:/no notices/")
        .expect("empty notice slot");

    app.press("id:notice").expect("notice trigger presses");
    app.wait_for("text:/notice the engine said something/")
        .expect("notice landed in the signal");
}

#[test]
fn a_volume_write_round_trips_to_the_store_through_commit_if_changed() {
    let mut app = start_probe();

    // The priming commit: the mock's `Preferences` was never hydrated, so
    // the first tick's diff against the default row saves once.
    app.wait_for("text:/vol 0.50 store 1/")
        .expect("priming commit counted");

    app.press("id:vol-up").expect("volume presses");
    app.wait_for("text:/vol 0.60 store 2/")
        .expect("the settings write committed");
}
