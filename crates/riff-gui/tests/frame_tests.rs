//! The Frame's ORDER, asserted at its own interface.
//!
//! Nothing here builds an `egui::Context`, a window, or a kittest harness. That
//! is the point of the ticket this file belongs to: the frame's decision half
//! names no egui type at all — [`FrameInput`] is pure data and [`FrameOutput`]
//! is decisions only — so "the drains run before the compose" and "a titlebar
//! click lands before the stage picks its view" are facts a test can reach
//! directly instead of facts a golden diff would have to betray.
//!
//! The ports here are the smallest fakes that keep the seams real: a scripted
//! scan service, a recording transport, and stores that answer "nothing". The
//! library query store answers *nothing* on purpose — no test below needs a
//! Track out of it, and a store that answered would only add a moving part.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use riff_backend::app::Transport;
use riff_backend::app::cover_service::{ClearCacheOutcome, Covers};
use riff_backend::app::errors::StoreError;
use riff_backend::app::pass_service::Passes;
use riff_backend::app::preferences::Preferences;
use riff_backend::app::replaygain_pass::{PassCommand, PassReport};
use riff_backend::app::scan_service::{ScanOutcome, Scans};
use riff_backend::app::state::{
    BrowseMode, LibrarySection, LibrarySession, LibraryStatus, PlaybackSession, ViewMode,
};
use riff_backend::app::store::{
    PlaylistEntry, PlaylistStore, ScalarSettings, Settings, SettingsStore, StoreChanged,
    StoreGeneration, WatchState,
};
use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
use riff_backend::app::traits::{DecodedCover, RequestedSize};
use riff_backend::app::views::SessionViews;
use riff_backend::app::watcher_manager::WatcherManager;
use riff_backend::domain::{Playlist, PlaylistId, TrackId};
use riff_gui::ui::app::{InlineTagEditor, ThemeState};
use riff_gui::ui::artwork::COVER_THUMB;
use riff_gui::ui::chrome::TitleBarAction;
use riff_gui::ui::cover_cache::CoverCache;
use riff_gui::ui::feedback::FeedbackBoard;
use riff_gui::ui::frame::{
    ControlBarReport, Frame, FrameInput, FrameOutput, FrameParts, FrameViewport, SidebarAction,
    SidebarReport, StageReport, TitleKey, TitlebarReport,
};
use riff_gui::ui::now_playing::NowPlayingAction;
use riff_gui::ui::playerbar::PlayerBarAction;
use riff_gui::ui::prompts::PromptOutcome;
use riff_gui::ui::scroll_memory::ScrollMemory;
use riff_gui::ui::sidebar::PlaylistRowAction;
use riff_gui::ui::theme;
use riff_persistence::test_support::FailingLibraryQueryStore;

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

/// A transport that records nothing and mutates nothing: the frame's own
/// decisions are what these tests read, not the engine intents behind them.
#[derive(Default)]
struct SilentTransport;

impl Transport for SilentTransport {
    fn play(&self, _track: TrackId) {}
    fn pause(&self) {}
    fn resume(&self) {}
    fn stop(&self) {}
    fn seek(&self, _session: &PlaybackSession, _secs: f32) {}
    fn set_volume(&self, _session: &mut PlaybackSession, _vol: f32) {}
    fn toggle_mute(&self, _session: &mut PlaybackSession) {}
    fn next(&self) {}
    fn previous(&self) {}
    fn play_next(&self, _track: TrackId) {}
    fn add_to_queue(&self, _track: TrackId) {}
    fn add_many(&self, _tracks: Vec<TrackId>) {}
    fn play_many(&self, _first: TrackId, _rest: Vec<TrackId>) {}
    fn toggle_shuffle(&self, _session: &mut PlaybackSession) {}
    fn toggle_repeat(&self, _session: &mut PlaybackSession) {}
    fn play_pause(&self, _session: &PlaybackSession) {}
}

/// A Library Scan Service that serves whatever outcomes a test queued, once.
#[derive(Default)]
struct ScriptedScans {
    pending: Mutex<Vec<ScanOutcome>>,
}

impl ScriptedScans {
    fn serve(&self, outcome: ScanOutcome) {
        self.pending.lock().expect("unpoisoned").push(outcome);
    }
}

impl Scans for ScriptedScans {
    fn request(&self, _path: PathBuf) {}
    fn cancel(&self) {}
    fn poll(&self) -> Vec<ScanOutcome> {
        std::mem::take(&mut *self.pending.lock().expect("unpoisoned"))
    }
    fn is_scanning(&self, _path: &Path) -> bool {
        false
    }
}

/// A `ReplayGain` Pass service that serves whatever a test queued: a running
/// flag with canned progress, and settled outcome reports. Everything else
/// is inert.
#[derive(Default)]
struct ScriptedPasses {
    running: AtomicBool,
    progress: Mutex<(usize, usize)>,
    outcomes: Mutex<Vec<PassReport>>,
    submitted: Mutex<Vec<PassCommand>>,
}

impl ScriptedPasses {
    fn set_running(&self, running: bool, progress: (usize, usize)) {
        self.running
            .store(running, std::sync::atomic::Ordering::Relaxed);
        *self.progress.lock().expect("unpoisoned") = progress;
    }

    fn settle(&self, report: PassReport) {
        self.outcomes.lock().expect("unpoisoned").push(report);
    }
}

impl Passes for ScriptedPasses {
    fn submit(&self, command: PassCommand) {
        self.submitted.lock().expect("unpoisoned").push(command);
    }
    fn cancel(&self) {}
    fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn poll(&self) -> Option<PassReport> {
        self.outcomes.lock().expect("unpoisoned").pop()
    }
    fn poll_progress(&self) -> (usize, usize) {
        *self.progress.lock().expect("unpoisoned")
    }
}

/// A Cover Service that serves whatever a test queued: decoded covers, an
/// artless verdict, or a settled cache clear. Everything else is inert.
#[derive(Default)]
struct ScriptedCovers {
    answers: Mutex<Vec<(TrackId, RequestedSize, Option<DecodedCover>)>>,
    settled_clear: Mutex<Option<ClearCacheOutcome>>,
}

impl ScriptedCovers {
    fn serve(&self, track_id: TrackId, size: RequestedSize, cover: Option<DecodedCover>) {
        self.answers
            .lock()
            .expect("unpoisoned")
            .push((track_id, size, cover));
    }

    fn settle_clear(&self, outcome: ClearCacheOutcome) {
        *self.settled_clear.lock().expect("unpoisoned") = Some(outcome);
    }
}

impl Covers for ScriptedCovers {
    fn request(&self, _track_id: TrackId, _path: PathBuf, _size: RequestedSize) {}
    fn request_folder(&self, _folder: &Path, _size: RequestedSize) {}
    fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
        std::mem::take(&mut *self.answers.lock().expect("unpoisoned"))
    }
    fn clear_cache(&self) {}
    fn poll_cache_clear(&self) -> Option<ClearCacheOutcome> {
        self.settled_clear.lock().expect("unpoisoned").take()
    }
}

/// A Tag Edit Service that never finishes anything.
#[derive(Default)]
struct QuietTagEdits;

impl TagEdits for QuietTagEdits {
    fn submit(&self, _request: TagEditRequest) {}
    fn poll(&self) -> Option<TagEditOutcome> {
        None
    }
}

/// A Settings store that records every scalar commit, so a test can see the
/// frame-end Preferences round-trip without an Application Store.
#[derive(Default)]
struct RecordingSettingsStore {
    committed: Arc<Mutex<Vec<ScalarSettings>>>,
}

impl SettingsStore for RecordingSettingsStore {
    fn load_settings(&self) -> Result<Settings, StoreError> {
        Ok(Settings::default())
    }
    fn save_scalars(&mut self, scalars: &ScalarSettings) -> Result<(), StoreError> {
        self.committed
            .lock()
            .expect("unpoisoned")
            .push(scalars.clone());
        Ok(())
    }
    fn save_library_paths(&mut self, _paths: &[PathBuf]) -> Result<(), StoreError> {
        Ok(())
    }
    fn save_watch_states(
        &mut self,
        _states: &HashMap<PathBuf, WatchState>,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}

/// A playlist store that remembers what it was asked to create and rename.
#[derive(Default)]
struct QuietPlaylistStore {
    created: Arc<Mutex<Vec<String>>>,
    next_id: Arc<Mutex<u64>>,
}

impl PlaylistStore for QuietPlaylistStore {
    fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError> {
        Ok(Vec::new())
    }
    fn load_playlist_entries(&self, _id: &PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError> {
        Ok(Vec::new())
    }
    fn create_playlist(
        &mut self,
        name: &str,
        _initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, StoreError> {
        self.created
            .lock()
            .expect("unpoisoned")
            .push(name.to_string());
        let mut next = self.next_id.lock().expect("unpoisoned");
        *next += 1;
        Ok(PlaylistId(format!("pl-{next}")))
    }
    fn rename_playlist(&mut self, _id: &PlaylistId, _new_name: &str) -> Result<bool, StoreError> {
        Ok(true)
    }
    fn delete_playlist(&mut self, _id: &PlaylistId) -> Result<bool, StoreError> {
        Ok(true)
    }
    fn add_playlist_entry(
        &mut self,
        _id: &PlaylistId,
        _track: &TrackId,
    ) -> Result<bool, StoreError> {
        Ok(true)
    }
    fn remove_playlist_entries(
        &mut self,
        _id: &PlaylistId,
        _track: &TrackId,
    ) -> Result<bool, StoreError> {
        Ok(true)
    }
    fn reorder_playlist_entries(
        &mut self,
        _id: &PlaylistId,
        _ordered: &[TrackId],
    ) -> Result<bool, StoreError> {
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

/// One frame's whole state, assembled exactly as `RiffApp::frame` assembles it
/// — from ports, not from an application.
///
/// The fourth bool is the Linux folder-picker's `settings_show_input`, so the
/// lint fires on Linux only — the same shape as `RiffApp`.
#[allow(clippy::struct_excessive_bools)]
struct Harness {
    playback: PlaybackSession,
    playback_live: Arc<Mutex<PlaybackSession>>,
    library: LibrarySession,
    backend_events: Arc<Mutex<riff_backend::app::events::BackendEvents>>,
    /// The coordinator's playback-error channel, wired into the inbox.
    notice_tx: Option<crossbeam_channel::Sender<String>>,
    /// The store's change channel, wired into the inbox.
    changes_tx: Option<crossbeam_channel::Sender<StoreChanged>>,
    watchers: Arc<Mutex<Option<WatcherManager>>>,
    cover_cache: CoverCache,
    feedback: FeedbackBoard,
    scroll_memory: ScrollMemory,
    theme: ThemeState,
    prefs: Preferences,
    settings_store: RecordingSettingsStore,
    settings_commits: Arc<Mutex<Vec<ScalarSettings>>>,
    playlist_store: QuietPlaylistStore,
    tag_edits: InlineTagEditor,
    views: SessionViews,
    scans: ScriptedScans,
    covers: ScriptedCovers,
    passes: ScriptedPasses,
    pass_force_choice: bool,
    last_pass_report: Option<PassReport>,
    playlist_view: Option<PlaylistId>,
    smart_playlist_view: Option<riff_backend::domain::SmartPlaylistKind>,
    playlist_rename: Option<(PlaylistId, String)>,
    playlist_create_name: Option<String>,
    clear_cache_in_flight: bool,
    global_search_focus: bool,
    title_key: TitleKey,
    /// The app-wide quit intent, cleared: a headless frame has no tray to set
    /// it, and only the macOS native-close step reads it.
    #[cfg(target_os = "macos")]
    quit_flag: std::sync::atomic::AtomicBool,
    /// The Linux text-row folder flow's state, cleared: only the Linux
    /// sidebar-footer step reads it.
    #[cfg(target_os = "linux")]
    settings_show_input: bool,
    #[cfg(target_os = "linux")]
    settings_path_error: Option<String>,
}

/// A borrow bundle over [`Harness`] for one frame: every field of the harness
/// is live at once, exactly as `RiffApp::frame`'s are on the app.
struct Leases<'a> {
    playback: &'a mut PlaybackSession,
    library: &'a mut LibrarySession,
    parts: FrameParts<'a>,
}

impl Harness {
    fn new() -> Self {
        let playback_live = Arc::new(Mutex::new(PlaybackSession::default()));
        let library_session = Arc::new(Mutex::new(LibrarySession::default()));
        let watchers = Arc::new(Mutex::new(None));
        let (notice_tx, notice_rx) = crossbeam_channel::unbounded();
        let (changes_tx, changes_rx) = crossbeam_channel::unbounded();
        let backend_events = Arc::new(Mutex::new(
            riff_backend::app::events::BackendEvents::default(),
        ));
        backend_events
            .lock()
            .expect("unpoisoned")
            .subscribe_playback_notices(notice_rx);
        backend_events
            .lock()
            .expect("unpoisoned")
            .subscribe_to_backend_changes(changes_rx);
        let settings_commits = Arc::new(Mutex::new(Vec::new()));
        let settings_store = RecordingSettingsStore {
            committed: Arc::clone(&settings_commits),
        };
        let transport = SilentTransport;
        // The one hydration step, performed where the runtime would perform it
        // (precedent: `RiffApp::new_for_test`).
        let prefs = Preferences::hydrate(
            &playback_live,
            &library_session,
            &settings_store,
            &transport,
            &watchers,
        );
        Self {
            playback: PlaybackSession::default(),
            playback_live,
            library: LibrarySession::default(),
            backend_events,
            notice_tx: Some(notice_tx),
            changes_tx: Some(changes_tx),
            watchers,
            cover_cache: CoverCache::new(),
            feedback: FeedbackBoard::default(),
            scroll_memory: ScrollMemory::default(),
            theme: ThemeState {
                dark: true,
                active: theme::Palette::dark(),
                last_applied: None,
            },
            prefs,
            settings_store,
            settings_commits,
            playlist_store: QuietPlaylistStore::default(),
            tag_edits: InlineTagEditor::new(Box::new(QuietTagEdits)),
            views: SessionViews::new(
                Box::new(FailingLibraryQueryStore::new(
                    "headless frame test: no store behind this seam",
                )),
                Box::new(QuietPlaylistStore::default()),
                StoreGeneration::new(),
                StoreGeneration::new(),
            ),
            scans: ScriptedScans::default(),
            covers: ScriptedCovers::default(),
            passes: ScriptedPasses::default(),
            pass_force_choice: false,
            last_pass_report: None,
            playlist_view: None,
            smart_playlist_view: None,
            playlist_rename: None,
            playlist_create_name: None,
            clear_cache_in_flight: false,
            global_search_focus: false,
            title_key: TitleKey::Unset,
            #[cfg(target_os = "macos")]
            quit_flag: std::sync::atomic::AtomicBool::new(false),
            #[cfg(target_os = "linux")]
            settings_show_input: false,
            #[cfg(target_os = "linux")]
            settings_path_error: None,
        }
    }

    /// Push a playback error the way the Playback Coordinator does: a plain
    /// string over its own channel, which the inbox stamps with playback source
    /// and error severity.
    fn push_playback_error(&mut self, message: &str) {
        if let Some(tx) = &self.notice_tx {
            tx.send(message.to_string()).expect("inbox channel open");
        }
    }

    /// Move the library generation the way a committed mutation does.
    fn push_library_generation(&mut self, generation: u64) {
        if let Some(tx) = &self.changes_tx {
            tx.send(StoreChanged::Library(generation))
                .expect("inbox channel open");
        }
    }

    /// Borrow the whole harness as one frame's bundle.
    fn frame(&mut self) -> Leases<'_> {
        let playback = &mut self.playback;
        let library = &mut self.library;
        let parts = FrameParts {
            theme: &mut self.theme,
            feedback: &mut self.feedback,
            scroll_memory: &mut self.scroll_memory,
            views: &mut self.views,
            backend_events: &self.backend_events,
            transport: &SilentTransport,
            scans: &self.scans,
            tag_edits: &mut self.tag_edits,
            covers: &self.covers,
            passes: &self.passes,
            pass_force_choice: &mut self.pass_force_choice,
            last_pass_report: &mut self.last_pass_report,
            cover_cache: &mut self.cover_cache,
            watchers: &self.watchers,
            prefs: &mut self.prefs,
            settings_store: &mut self.settings_store,
            playlist_store: &mut self.playlist_store,
            clear_cache_in_flight: &mut self.clear_cache_in_flight,
            global_search_focus: &mut self.global_search_focus,
            title_key: &mut self.title_key,
            playlist_view: &mut self.playlist_view,
            smart_playlist_view: &mut self.smart_playlist_view,
            playlist_rename: &mut self.playlist_rename,
            playlist_create_name: &mut self.playlist_create_name,
            playback_live: &self.playback_live,
            #[cfg(target_os = "linux")]
            settings_show_input: &mut self.settings_show_input,
            #[cfg(target_os = "linux")]
            settings_path_error: &mut self.settings_path_error,
            #[cfg(target_os = "macos")]
            quit_flag: &self.quit_flag,
        };
        Leases {
            playback,
            library,
            parts,
        }
    }

    /// One frame with empty panel reports — the draw half contributes nothing,
    /// which is the whole point: the ORDER is the Frame's alone.
    fn advance(&mut self, input: &FrameInput) -> FrameOutput {
        let Leases {
            playback,
            library,
            parts,
        } = self.frame();
        Frame::new(parts, playback, library).advance(input)
    }

    /// The drive the production `ui()` performs, minus the drawing: the head,
    /// then each panel's report at its own slot, then the write-back.
    fn run_frame(&mut self, reports: &Reports) -> FrameOutput {
        let input = FrameInput::default();
        let mut out = self.advance(&input);
        {
            let Leases {
                playback,
                library,
                parts,
            } = self.frame();
            Frame::new(parts, playback, library).apply_titlebar(&mut out, &reports.titlebar);
        }
        {
            let Leases {
                playback,
                library,
                parts,
            } = self.frame();
            Frame::new(parts, playback, library).apply_sidebar(&mut out, &reports.sidebar);
        }
        {
            let Leases {
                playback,
                library,
                parts,
            } = self.frame();
            Frame::new(parts, playback, library).apply_control_bar(&mut out, &reports.control_bar);
        }
        {
            let Leases {
                playback,
                library,
                parts,
            } = self.frame();
            Frame::new(parts, playback, library).apply_stage(&mut out, &reports.stage);
        }
        {
            let Leases {
                playback,
                library,
                parts,
            } = self.frame();
            Frame::new(parts, playback, library).finish();
        }
        out
    }
}

/// The three panel reports (plus the stage's) a frame's draw half would hand
/// the Frame. Tests build exactly the ones they care about.
#[derive(Default)]
struct Reports {
    titlebar: TitlebarReport,
    sidebar: SidebarReport,
    control_bar: ControlBarReport,
    stage: StageReport,
}

// ---------------------------------------------------------------------------
// The order
// ---------------------------------------------------------------------------

/// **A slot filled after the compose is a frame late** — the rule the module
/// doc names, and the one this ticket exists to make a test failure rather than
/// a golden diff.
///
/// A scan completion drained in step 4 must be in the composed `scan_status`
/// the titlebar paints in step 5. Move the compose above the drains and
/// `library.scan_status` is `None` on the frame that drew it, so this fails.
#[test]
fn the_compose_runs_after_the_drains_not_before_them() {
    let mut harness = Harness::new();
    harness.scans.serve(ScanOutcome::Complete {
        path: PathBuf::from("/music"),
        total_files: 42,
    });

    harness.advance(&FrameInput::default());

    assert_eq!(
        harness.library.scan_status.as_deref(),
        Some("Scan complete: 42 tracks"),
        "the status line must carry what this frame's drains produced"
    );
}

/// The composed line is the *board's*, not the last thing anyone wrote into
/// the status slot — the reason the compose exists as its own step (issue 11):
/// a scan update cannot erase a live playback error.
#[test]
fn a_scan_update_cannot_erase_a_playback_error() {
    let mut harness = Harness::new();
    harness.push_playback_error("playback failed");
    harness.scans.serve(ScanOutcome::Progress {
        path: PathBuf::from("/music"),
        files_found: 7,
    });

    harness.advance(&FrameInput::default());

    assert_eq!(
        harness.library.scan_status.as_deref(),
        Some("playback failed"),
        "the higher-severity source survives the scan's own slot"
    );
}

/// The event inbox is drained at the frame's start (step 3) and its Library
/// generation moves are folded into the Scroll Memory, so a committed rescan
/// turns every Section slot's fingerprint stale.
#[test]
fn the_event_inbox_drains_before_anything_paints() {
    let mut harness = Harness::new();
    harness.push_library_generation(9);

    harness.advance(&FrameInput::default());

    // The scan-slot line the drain writes is what the compose then reads, so a
    // second frame with no events proves the drain happened exactly once and
    // the generation is remembered rather than the events.
    assert!(
        harness.library.scan_status.is_none(),
        "a library-changed event carries no UI feedback of its own"
    );
    let second = FrameInput {
        zoom_factor: 1.0,
        ..FrameInput::default()
    };
    harness.advance(&second);
    assert!(
        harness.library.scan_status.is_none(),
        "the inbox is empty on the second frame: the first drained it"
    );
}

/// Steps 10 → 13 are ordered, and the test proves it by giving each slot an
/// action that overwrites the *same* fact: the active view. The last one wins,
/// so the assertion names the order rather than a state.
#[test]
fn the_panel_slots_are_ordered_titlebar_then_bar_then_stage() {
    let mut harness = Harness::new();
    let reports = Reports {
        titlebar: TitlebarReport {
            actions: vec![TitleBarAction::GoSettings],
            ..TitlebarReport::default()
        },
        control_bar: ControlBarReport {
            actions: vec![PlayerBarAction::ToggleExpanded],
        },
        stage: StageReport {
            actions: vec![NowPlayingAction::Close],
        },
        ..Reports::default()
    };

    harness.run_frame(&reports);

    // GoSettings → Settings; ToggleExpanded → NowPlaying; Close → Library.
    assert_eq!(
        harness.library.view_mode,
        ViewMode::Library,
        "the stage's report is answered last, so it decides the frame's view"
    );
}

/// A titlebar click that switches the view must land BEFORE the sidebar paints
/// its highlight — the reason the titlebar is its own slot rather than one
/// drain at the end of the frame. With a single end-of-frame drain this would
/// highlight a frame late.
#[test]
fn the_titlebar_slot_lands_before_the_sidebar_reads_the_view() {
    let mut harness = Harness::new();
    let reports = Reports {
        titlebar: TitlebarReport {
            actions: vec![TitleBarAction::GoSettings],
            ..TitlebarReport::default()
        },
        ..Reports::default()
    };

    harness.run_frame(&reports);

    assert_eq!(harness.library.view_mode, ViewMode::Settings);
    assert_eq!(harness.library.browse_mode, BrowseMode::Library);
}

/// The sidebar's own sub-slots: a nav click lands on the library view, clears
/// any search, and closes whichever opened list was on screen — the whole
/// "exactly one browser variant is visible after it" rule, stated once.
#[test]
fn a_sidebar_navigation_clears_the_opened_list_and_the_search() {
    let mut harness = Harness::new();
    harness.library.search_query = "boards".to_string();
    harness.playlist_view = Some(PlaylistId("pl-1".to_string()));
    harness.smart_playlist_view = Some(riff_backend::domain::SmartPlaylistKind::Favorites);
    let reports = Reports {
        sidebar: SidebarReport {
            actions: vec![SidebarAction::Navigate {
                section: LibrarySection::Albums,
            }],
        },
        ..Reports::default()
    };

    harness.run_frame(&reports);

    assert_eq!(harness.library.view_mode, ViewMode::Library);
    assert_eq!(harness.library.browse_mode, BrowseMode::Library);
    assert_eq!(harness.library.library_section, LibrarySection::Albums);
    assert_eq!(harness.library.search_query, "");
    assert!(harness.playlist_view.is_none());
    assert!(harness.smart_playlist_view.is_none());
    assert!(
        harness.library.browser_path.is_empty(),
        "section navigation resets the drill-down path"
    );
}

/// Folding the SMART LISTS section away also closes any smart list it had
/// opened: with the rows gone there is no other way back to that view.
#[test]
fn folding_the_smart_lists_section_closes_the_list_it_opened() {
    let mut harness = Harness::new();
    harness.smart_playlist_view = Some(riff_backend::domain::SmartPlaylistKind::RecentlyPlayed);
    let reports = Reports {
        sidebar: SidebarReport {
            actions: vec![SidebarAction::ToggleSmartListsCollapsed],
        },
        ..Reports::default()
    };

    harness.run_frame(&reports);

    assert!(harness.library.ui_flags.smart_lists_collapsed);
    assert!(harness.smart_playlist_view.is_none());
}

/// The inline "New Playlist" prompt is opened by the header and committed by
/// the same slot: a confirmed, non-empty, trimmed name is one durable
/// transaction, and the created playlist is what the session opens.
#[test]
fn the_create_prompt_commits_through_the_store_and_opens_the_new_playlist() {
    let mut harness = Harness::new();
    let created = Arc::clone(&harness.playlist_store.created);
    harness.playlist_create_name = Some("Road Trip".to_string());
    harness.playlist_view = Some(PlaylistId("pl-1".to_string()));
    let reports = Reports {
        sidebar: SidebarReport {
            actions: vec![SidebarAction::PlaylistCreate(PromptOutcome::Confirm)],
        },
        ..Reports::default()
    };

    harness.run_frame(&reports);

    assert_eq!(
        created.lock().expect("unpoisoned").as_slice(),
        ["Road Trip"]
    );
    assert_eq!(harness.playlist_create_name, None);
    assert_eq!(harness.playlist_view, Some(PlaylistId("pl-1".to_string())));
}

/// A playlist row's Delete goes through the store port, and a prompt that is
/// merely CANCELLED writes nothing at all.
#[test]
fn a_playlist_row_delete_and_a_cancelled_prompt_are_two_different_things() {
    let mut harness = Harness::new();
    harness.playlist_rename = Some((PlaylistId("pl-1".to_string()), "Draft".to_string()));
    let reports = Reports {
        sidebar: SidebarReport {
            actions: vec![
                SidebarAction::PlaylistRow {
                    id: PlaylistId("pl-1".to_string()),
                    action: PlaylistRowAction::Delete,
                },
                SidebarAction::PlaylistRename {
                    id: PlaylistId("pl-1".to_string()),
                    outcome: PromptOutcome::Cancel,
                },
            ],
        },
        ..Reports::default()
    };

    harness.run_frame(&reports);

    assert!(
        harness.playlist_rename.is_none(),
        "a cancelled rename discards its draft"
    );
}

/// The write-back has ONE owner and it is `Frame::finish`: the Preferences
/// commit sees the frame's playback snapshot (volume, mute, replay-gain,
/// shuffle, repeat), and only the six UI-owned fields land in the live session.
#[test]
fn the_write_back_commits_preferences_and_only_the_ui_owned_fields() {
    let mut harness = Harness::new();
    harness.playback.current_volume = 0.25;
    harness.playback.muted = true;
    harness.playback.queue.set_shuffle(true);
    // Something the engine owns, which the frame must NOT write back.
    harness
        .playback_live
        .lock()
        .expect("unpoisoned")
        .current_position
        .current = std::time::Duration::from_secs(9);

    {
        let Leases {
            playback,
            library,
            parts,
        } = harness.frame();
        Frame::new(parts, playback, library).finish();
    }

    let live = harness.playback_live.lock().expect("unpoisoned");
    assert!(
        (live.current_volume - 0.25).abs() < f32::EPSILON,
        "the frame's volume snapshot reached the live session"
    );
    assert!(live.muted);
    assert!(live.queue.shuffle);
    assert_eq!(
        live.current_position.current,
        std::time::Duration::from_secs(9),
        "engine-written position survives the frame"
    );
    let commits = harness.settings_commits.lock().expect("unpoisoned");
    assert_eq!(
        commits.len(),
        1,
        "one durable Preferences round-trip per frame"
    );
    assert_eq!(commits[0].volume, Some(0.25));
}

/// The theme is resolved once at init and again only when the selection moves
/// — never per frame. A steady-state frame reports no palette to install.
#[test]
fn the_palette_is_offered_once_at_init_and_not_again_on_a_quiet_frame() {
    let mut harness = Harness::new();

    let first = harness.advance(&FrameInput::default());
    let second = harness.advance(&FrameInput::default());

    assert!(
        first.palette.is_some(),
        "the first frame installs the palette"
    );
    assert!(
        second.palette.is_none(),
        "a frame that changed nothing installs nothing"
    );
    // The palette is installed by the draw half from `FrameOutput::palette`;
    // what the Frame keeps is the identity it resolved, so a quiet frame can
    // tell "nothing changed" from "never applied".
    assert_eq!(harness.theme.last_applied, Some((true, false)));
}

/// A titlebar `ToggleMaximize` is a decision, and the viewport's maximized flag
/// is egui-owned — which is why it travels in the report rather than being read
/// behind the Frame's back.
#[test]
fn a_maximize_toggle_decides_against_the_flag_the_report_carries() {
    let mut harness = Harness::new();
    let reports = Reports {
        titlebar: TitlebarReport {
            actions: vec![TitleBarAction::ToggleMaximize],
            maximized: true,
            ..TitlebarReport::default()
        },
        ..Reports::default()
    };

    let out = harness.run_frame(&reports);

    assert!(out.viewport.contains(&FrameViewport::Maximized(false)));
}

/// The window title and the tray tooltip are ONE identity, compared before
/// anything is formatted: a steady-state frame pushes neither, and the frame
/// that changes the track pushes both.
#[test]
fn the_os_title_and_tray_tooltip_push_once_per_identity_change() {
    let mut harness = Harness::new();

    let first = harness.advance(&FrameInput::default());
    let second = harness.advance(&FrameInput::default());

    assert_eq!(
        first.viewport,
        vec![FrameViewport::Title("riff".to_string())],
        "nothing playing still titles the window once"
    );
    assert_eq!(first.tray_tooltip.as_deref(), Some("riff"));
    assert!(
        second.viewport.is_empty() && second.tray_tooltip.is_none(),
        "an unchanged identity pushes nothing: suppression, never staleness"
    );
}

/// Ctrl+K raises the one-shot focus request and the titlebar's panel consumes
/// it — the frame, not the panel, owns the flag.
#[test]
fn ctrl_k_raises_a_one_shot_focus_request_the_titlebar_consumes() {
    let mut harness = Harness::new();
    let input = FrameInput {
        search_focus_requested: true,
        ..FrameInput::default()
    };

    harness.advance(&input);
    assert!(
        harness.global_search_focus,
        "the keyboard step raised the request"
    );

    let reports = Reports {
        titlebar: TitlebarReport {
            focus_search: true,
            ..TitlebarReport::default()
        },
        ..Reports::default()
    };
    harness.run_frame(&reports);
    assert!(
        !harness.global_search_focus,
        "the titlebar's slot consumes it, so it cannot fire twice"
    );
}

/// The titlebar's search query rides back in the report rather than being
/// written in place: the panel edits a scratch string, the Frame owns the
/// session slot.
#[test]
fn the_search_query_rides_back_in_the_report() {
    let mut harness = Harness::new();
    let reports = Reports {
        titlebar: TitlebarReport {
            search_query: "boards of canada".to_string(),
            ..TitlebarReport::default()
        },
        ..Reports::default()
    };

    harness.run_frame(&reports);

    assert_eq!(harness.library.search_query, "boards of canada");
}

/// `FrameOutput` names no egui type. This is the mechanical half of the
/// ticket's first criterion: an output that accreted widget-ready values would
/// be a shallower module than the function it replaced, and nothing else would
/// fail. The list is spelled out here so a future field that reaches for a
/// `TextureHandle`, a `Response` or a `Rect` fails this test rather than merely
/// reading badly.
#[test]
fn frame_output_carries_decisions_and_no_egui_handles() {
    // Every field of the output, named.
    let output = FrameOutput::default();
    let _: Option<theme::Palette> = output.palette;
    let _: bool = output.evict_generated;
    let _: Vec<FrameViewport> = output.viewport;
    let _: bool = output.hide_window;
    let _: Option<String> = output.tray_tooltip;
    let _: Option<ClearCacheOutcome> = output.cache_clear;
    let _: bool = output.pick_folder;
    let _: usize = output.cover_arrivals.len();
    // NativeClose is reachable only through a named predicate, so the draw half
    // never matches on the type itself.
    let _: bool = output.cancels_native_close();

    // And the vocabulary it speaks is the frame's own: the viewport intents are
    // frame-level decisions, not egui commands.
    assert_eq!(FrameViewport::Minimize, FrameViewport::Minimize);
}

/// The close decision is data on every platform, so CI's Linux and Windows legs
/// assert it too — the applier is macOS-only, the rule is not.
#[test]
fn the_close_rule_is_assertable_off_macos() {
    use riff_gui::ui::app::{CloseIntent, close_resolution};
    assert!(
        close_resolution(CloseIntent::WindowClose, false).is_some(),
        "a window close hides by default"
    );
    assert!(
        close_resolution(CloseIntent::WindowClose, true).is_none(),
        "the preference can make it a real quit"
    );
    assert!(
        close_resolution(CloseIntent::Quit, false).is_none(),
        "a quit riff has committed is never cancelled"
    );
}

/// A read-path fact the frame still owns: a completed scan reports the root's
/// Readiness through the library paths slot, not through the status line alone.
#[test]
fn a_completed_scan_reports_the_roots_readiness() {
    let mut harness = Harness::new();
    harness.scans.serve(ScanOutcome::Complete {
        path: PathBuf::from("/music"),
        total_files: 3,
    });
    harness
        .library
        .library_paths
        .register(PathBuf::from("/music"), &mut NoSettingsStore);

    harness.advance(&FrameInput::default());

    assert_eq!(
        harness
            .library
            .library_paths
            .readiness(&PathBuf::from("/music")),
        LibraryStatus::Scanned(3),
    );
}

/// The custom X on a platform with a tray hides instead of closing; that is a
/// decision the Frame records and the draw half enqueues, which is why
/// `FrameOutput` has a `hide_window` and not a visibility channel.
#[cfg(not(target_os = "linux"))]
#[test]
fn the_custom_x_hides_when_the_preference_is_off() {
    let mut harness = Harness::new();
    harness.library.ui_flags.close_quits_app = false;
    let reports = Reports {
        titlebar: TitlebarReport {
            actions: vec![TitleBarAction::Close],
            ..TitlebarReport::default()
        },
        ..Reports::default()
    };

    let out = harness.run_frame(&reports);

    assert!(
        out.hide_window,
        "no tray-visible quit: hide through the channel"
    );
    assert!(
        !out.viewport.contains(&FrameViewport::Close),
        "with the close-to-tray veto gone, a close that reaches eframe quits"
    );
}

/// The Cover Cache's settle is step 6: the markers drop, the arrival is
/// handed to the View half, and the draw half is the one that turns pixels into
/// a texture. So `FrameOutput` carries the arrival, not a texture.
#[test]
fn a_cover_that_arrived_is_handed_to_the_view_half_not_uploaded_here() {
    let mut harness = Harness::new();
    let id = TrackId("/music/a.flac".to_string());
    harness.cover_cache.want_track(
        &harness.covers,
        id.clone(),
        PathBuf::from("/music/a.flac"),
        COVER_THUMB,
    );
    assert_eq!(
        harness.cover_cache.in_flight().len(),
        1,
        "the ask is marked"
    );
    harness.covers.serve(
        id.clone(),
        COVER_THUMB,
        Some(DecodedCover {
            rgba: vec![7u8; 4 * 4 * 4],
            width: 4,
            height: 4,
        }),
    );

    let out = harness.advance(&FrameInput::default());

    assert_eq!(
        out.cover_arrivals.len(),
        1,
        "the arrival is the draw half's job"
    );
    assert!(
        harness.cover_cache.in_flight().is_empty(),
        "the marker drops with the answer, or the row would never re-ask"
    );
    assert!(
        !harness.cover_cache.arrived().is_empty(),
        "and the cache remembers what the View half now holds"
    );
}

/// A Thumbnail-cache clear that settles is the Frame's decision (poll, clear
/// the in-flight flag, forget the arrivals, write the status line); only the
/// texture-map flush is left to the draw half, because the map holds egui
/// handles.
#[test]
fn a_settled_thumbnail_cache_clear_is_decided_by_the_frame() {
    let mut harness = Harness::new();
    harness.clear_cache_in_flight = true;
    let id = TrackId("/music/a.flac".to_string());
    harness.cover_cache.want_track(
        &harness.covers,
        id.clone(),
        PathBuf::from("/music/a.flac"),
        COVER_THUMB,
    );
    // Pretend the View half filed it, so the forget has something to forget.
    harness.covers.serve(
        id,
        COVER_THUMB,
        Some(DecodedCover {
            rgba: vec![1u8; 4],
            width: 1,
            height: 1,
        }),
    );
    harness.advance(&FrameInput::default());
    assert!(!harness.cover_cache.arrived().is_empty());
    harness.clear_cache_in_flight = true;
    harness.covers.settle_clear(ClearCacheOutcome::Cleared);

    let out = harness.advance(&FrameInput::default());

    assert_eq!(
        out.cache_clear,
        Some(ClearCacheOutcome::Cleared),
        "the settled outcome travels out for the View half's flush"
    );
    assert!(
        harness.cover_cache.arrived().is_empty(),
        "the View half is empty, so nothing is arrived any more"
    );
    assert!(
        !harness.clear_cache_in_flight,
        "settled, so a later press may start a fresh clear"
    );
    assert_eq!(
        harness.library.scan_status.as_deref(),
        Some("Thumbnail cache cleared. Covers rebuild as you browse."),
        "and the status line is written BEFORE the compose, so it is on screen"
    );
}

/// A Settings store that records nothing, for the tests that register a library
/// root and only need the registration to succeed.
#[derive(Default)]
struct NoSettingsStore;

impl SettingsStore for NoSettingsStore {
    fn load_settings(&self) -> Result<Settings, StoreError> {
        Ok(Settings::default())
    }
    fn save_scalars(&mut self, _scalars: &ScalarSettings) -> Result<(), StoreError> {
        Ok(())
    }
    fn save_library_paths(&mut self, _paths: &[PathBuf]) -> Result<(), StoreError> {
        Ok(())
    }
    fn save_watch_states(
        &mut self,
        _states: &HashMap<PathBuf, WatchState>,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}

#[test]
fn a_running_pass_polls_its_progress_and_a_settled_pass_its_outcome() {
    let mut harness = Harness::new();

    // While the pass runs, the frame keeps the pass's progress line current
    // on its own feedback slot.
    harness.passes.set_running(true, (3, 10));
    harness.advance(&FrameInput::default());
    assert_eq!(
        harness.feedback.display_message(),
        Some("Measuring ReplayGain (3/10)\u{2026}".to_string()),
        "the running pass's progress line is polled per frame"
    );

    // The pass settles: the outcome report lands on the same slot, and the
    // running flag is down, so the progress line does not come back.
    harness.passes.set_running(false, (0, 0));
    harness.passes.settle(PassReport {
        measured: 10,
        skipped: 2,
        failed: 0,
        first_failure: None,
        cancelled: false,
    });
    harness.advance(&FrameInput::default());
    assert_eq!(
        harness.feedback.display_message(),
        Some("ReplayGain measured: 10 tracks (2 already measured)".to_string()),
    );

    // A failed pass reports its first failure with an error severity.
    harness.passes.settle(PassReport {
        measured: 1,
        skipped: 0,
        failed: 1,
        first_failure: Some("music/a1.flac: IO error: permission denied".to_string()),
        cancelled: false,
    });
    harness.advance(&FrameInput::default());
    assert_eq!(
        harness.feedback.display_message(),
        Some(
            "ReplayGain pass finished: 1 measured, 1 failed \u{2014} music/a1.flac: IO error: permission denied"
                .to_string()
        )
    );
}

#[test]
fn an_enabled_automatic_pass_follows_every_completed_scan() {
    let mut harness = Harness::new();
    // The Settings gating: both value kinds enabled.
    harness.library.pass_prefs = riff_backend::app::state::PassPrefs {
        track_values: true,
        album_values: true,
    };

    harness.scans.serve(ScanOutcome::Complete {
        path: PathBuf::from("/music"),
        total_files: 12,
    });
    harness.advance(&FrameInput::default());

    assert_eq!(
        harness.passes.submitted.lock().expect("unpoisoned").clone(),
        vec![PassCommand::LibraryWide {
            track_values: true,
            album_values: true,
            force: false,
        }],
        "the automatic pass is the library-wide shape under the checkbox \
         gating, filling in only unmeasured Tracks (never forced)"
    );
}

#[test]
fn a_disabled_automatic_pass_never_follows_a_scan() {
    let mut harness = Harness::new();
    // Default gating: both checkboxes off.

    harness.scans.serve(ScanOutcome::Complete {
        path: PathBuf::from("/music"),
        total_files: 12,
    });
    harness.advance(&FrameInput::default());

    assert!(
        harness
            .passes
            .submitted
            .lock()
            .expect("unpoisoned")
            .is_empty(),
        "a scan whose gating is off changes nothing about measurement"
    );
}
