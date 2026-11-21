//! The N16 end-to-end flows, driven headless through the real shell over a
//! mutating mock runtime: a playlist's create, rename and delete; a tag
//! edit's submission; the settings toggle's session write; the queue's
//! promote and clear. Selectors address the production `testID`s, so the
//! tests fail when the real controls disappear.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::unbounded;
use nsh_base::button;
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use riff_backend::app::MutexExt;
use riff_backend::app::StubThumbnailCache;
use riff_backend::app::events::BackendEvents;
use riff_backend::app::preferences::Preferences;
use riff_backend::app::state::{LibrarySession, PlaybackSession};
use riff_backend::app::store::{PlaylistEntry, PlaylistStore, StoreChanged, StoreGeneration};
use riff_backend::app::transport::{ChannelTransport, Transport};
use riff_backend::app::views::SessionViews;
use riff_backend::composition::AppRuntime;
use riff_backend::domain::Playlist;
use riff_backend::domain::{PlaybackCommand, PlaylistId, Track, TrackId};
use riff_gui_nsh::bridge::{UiModel, install};
use riff_persistence::errors::StoreError;

use crate::mocks::{
    FakeTagEdits, MockLibraryMutationStore, MockLibraryQueryStore, MockScans, MockSettingsStore,
    RecordingCovers, RecordingPasses,
};
use crate::nsh_bridge_tests::{probe_tracks, spawn_command_applier};

/// A mutating playlist store: creation, rename, delete and reorder land in
/// shared rows and bump the playlist generation, so the projection's cache
/// refetches and the sidebar and stage follow.
#[derive(Clone, Default)]
pub(crate) struct FlowPlaylists {
    rows: Arc<Mutex<Vec<Playlist>>>,
    known: Arc<Mutex<std::collections::HashMap<TrackId, Track>>>,
    generation: StoreGeneration,
}

impl FlowPlaylists {
    fn new(known: Vec<Track>) -> Self {
        let map = known.into_iter().map(|t| (t.id.clone(), t)).collect();
        Self {
            rows: Arc::new(Mutex::new(Vec::new())),
            known: Arc::new(Mutex::new(map)),
            generation: StoreGeneration::new(),
        }
    }
}

impl PlaylistStore for FlowPlaylists {
    fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError> {
        Ok(self.rows.lock_or_recover().clone())
    }

    fn load_playlist_entries(&self, id: &PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError> {
        let rows = self.rows.lock_or_recover();
        let Some(playlist) = rows.iter().find(|p| &p.id == id) else {
            return Ok(Vec::new());
        };
        let known = self.known.lock_or_recover();
        Ok(playlist
            .tracks
            .iter()
            .map(|id| PlaylistEntry {
                id: id.clone(),
                track: known.get(id).cloned(),
                valid: known.contains_key(id),
            })
            .collect())
    }

    fn create_playlist(
        &mut self,
        name: &str,
        _initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, StoreError> {
        let name = name.trim().to_owned();
        let id = PlaylistId::new(&name);
        self.rows
            .lock_or_recover()
            .push(Playlist::new(id.clone(), name));
        self.generation.bump();
        Ok(id)
    }

    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, StoreError> {
        let mut rows = self.rows.lock_or_recover();
        let Some(row) = rows.iter_mut().find(|p| &p.id == id) else {
            return Ok(false);
        };
        new_name.trim().clone_into(&mut row.name);
        drop(rows);
        self.generation.bump();
        Ok(true)
    }

    fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, StoreError> {
        let before = self.rows.lock_or_recover().len();
        self.rows.lock_or_recover().retain(|p| &p.id != id);
        let after = self.rows.lock_or_recover().len();
        self.generation.bump();
        Ok(before != after)
    }

    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Result<bool, StoreError> {
        let mut rows = self.rows.lock_or_recover();
        let Some(row) = rows.iter_mut().find(|p| &p.id == id) else {
            return Ok(false);
        };
        if row.tracks.contains(track) {
            return Ok(false);
        }
        row.tracks.push(track.clone());
        drop(rows);
        self.generation.bump();
        Ok(true)
    }

    fn remove_playlist_entries(
        &mut self,
        id: &PlaylistId,
        track: &TrackId,
    ) -> Result<bool, StoreError> {
        let mut rows = self.rows.lock_or_recover();
        let Some(row) = rows.iter_mut().find(|p| &p.id == id) else {
            return Ok(false);
        };
        let before = row.tracks.len();
        row.tracks.retain(|t| t != track);
        let removed = before != row.tracks.len();
        drop(rows);
        if removed {
            self.generation.bump();
        }
        Ok(removed)
    }

    fn reorder_playlist_entries(
        &mut self,
        id: &PlaylistId,
        ordered: &[TrackId],
    ) -> Result<bool, StoreError> {
        let mut rows = self.rows.lock_or_recover();
        let Some(row) = rows.iter_mut().find(|p| &p.id == id) else {
            return Ok(false);
        };
        row.tracks.clone_from(&ordered.to_vec());
        drop(rows);
        self.generation.bump();
        Ok(true)
    }

    fn playlist_generation(&self) -> StoreGeneration {
        self.generation.clone()
    }
}

/// The flow probe: the mock runtime with mutating playlists, under the real
/// shell, plus readout buttons for facts only the engine thread holds.
#[component]
pub(crate) fn FlowProbeApp() -> impl IntoElement {
    let (one, two) = probe_tracks();
    let library_ids = vec![one.id.clone(), two.id.clone()];
    let playback = Arc::new(Mutex::new(PlaybackSession::default()));
    let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
    spawn_command_applier(cmd_rx, Arc::clone(&playback), library_ids.clone());
    let seed_transport = ChannelTransport::new(cmd_tx.clone());

    let backend_events = Arc::new(Mutex::new(BackendEvents::default()));
    let (_change_tx, change_rx) = unbounded::<StoreChanged>();
    backend_events
        .lock_or_recover()
        .subscribe_to_backend_changes(change_rx);
    let (_notice_tx, notice_rx) = unbounded::<String>();
    backend_events
        .lock_or_recover()
        .subscribe_playback_notices(notice_rx);

    let library = Arc::new(Mutex::new(LibrarySession::default()));
    let settings_store = MockSettingsStore::default();
    let preferences = Preferences::hydrate(
        &playback,
        &library,
        &settings_store,
        &ChannelTransport::new(unbounded().0),
        &Arc::new(Mutex::new(None)),
    );

    let tracks = vec![one.clone(), two.clone()];
    let library_map: std::collections::HashMap<TrackId, Track> =
        tracks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    let playlists = FlowPlaylists::new(tracks.clone());
    let views = SessionViews::new(
        Box::new(MockLibraryQueryStore {
            library: library_map,
            flat: tracks,
            library_counts: riff_persistence::store::LibraryCounts {
                tracks: 2,
                artists: 1,
                albums: 1,
                genres: 1,
            },
            ..MockLibraryQueryStore::default()
        }),
        Box::new(playlists.clone()),
    );

    let tag_edits = FakeTagEdits::new();
    let tags_handle = tag_edits.clone();

    let runtime = AppRuntime {
        playback,
        library: Arc::clone(&library),
        backend_events,
        ui_transport: Box::new(ChannelTransport::new(cmd_tx)),
        tray_transport: ChannelTransport::new(unbounded().0),
        preferences,
        scans: Box::new(MockScans::default()),
        watcher_manager: Arc::new(Mutex::new(None)),
        quit_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        settings: Box::new(settings_store),
        playlists: Box::new(playlists),
        library_mutations: Box::new(MockLibraryMutationStore::new()),
        session_views: views,
        tag_edits: Box::new(tag_edits),
        passes: Box::new(RecordingPasses::default()),
        covers: Box::new(RecordingCovers::new()),
        thumbnail_cache: Arc::new(StubThumbnailCache),
    };

    let model = stored(UiModel::new(runtime, None));
    install(model);
    let state = model.with(|m| m.signals());
    let session_line = signal(String::new());
    let session_out = session_line;
    let tags_line = signal(String::new());
    let tags_out = tags_line;
    let queue_len = move || state.playback.get().queue_len;
    let seed_state = state;

    view()
        .flex_1()
        .flex_col()
        .child(riff_gui_nsh::shell::install(model))
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .child(
                    button()
                        .test_id("flow-read-session")
                        .on_press({
                            let library = Arc::clone(&library);
                            move || {
                                let library = library.lock_or_recover();
                                session_out.set(format!(
                                    "flow session advanced {}",
                                    library.ui_flags.advanced_mode
                                ));
                            }
                        })
                        .child(text("read session")),
                )
                .child(
                    button()
                        .test_id("flow-read-tags")
                        .on_press(move || {
                            tags_out.set(format!("flow tags {}", tags_handle.requests().len()));
                        })
                        .child(text("read tags")),
                )
                .child(
                    button()
                        .test_id("flow-seed-queue")
                        .on_press(move || {
                            let _ = seed_state;
                            seed_transport
                                .add_many(vec![TrackId("/music/probe/two.wav".to_owned())]);
                        })
                        .child(text("seed queue")),
                ),
        )
        .child(text(move || format!("flow queue {}", queue_len())).test_id("flow-queue"))
        .child(text(move || session_out.get()).test_id("flow-session"))
        .child(text(move || tags_out.get()).test_id("flow-tags"))
}

fn flow_desktop() -> EngineConfig {
    let mut config = EngineConfig::new(Root::of::<FlowProbeApp>());
    config.environment.os = nsh_core::platform::Os::Windows;
    config.viewport = size(px(1280.0), px(800.0));
    config
}

#[test]
fn a_playlist_is_created_renamed_and_deleted() {
    let mut app = Headless::start(flow_desktop()).unwrap();
    app.wait_for("id:player-bar").unwrap();

    app.press("id:playlist-create").unwrap();
    app.wait_for("id:playlist-name").unwrap();
    app.type_text("id:playlist-name", "Night Drive").unwrap();
    app.wait_for("id:playlist-create-commit").unwrap();
    app.press("id:playlist-create-commit").unwrap();
    app.wait_for("text:Night Drive (0)").unwrap();
    app.wait_for("id:playlist-stage").unwrap();

    app.press("id:playlist-rename").unwrap();
    app.wait_for("id:playlist-rename-input").unwrap();
    app.clear("id:playlist-rename-input").unwrap();
    app.type_text("id:playlist-rename-input", "Dawn Drive")
        .unwrap();
    app.press("id:playlist-rename-save").unwrap();
    app.wait_for("text:Dawn Drive (0)").unwrap();

    app.press("id:playlist-delete").unwrap();
    app.wait_for("id:playlist-delete-band").unwrap();
    app.press("id:playlist-delete-confirm").unwrap();
    app.wait_for_gone("text:Dawn Drive (0)", Duration::from_secs(3))
        .expect("the deleted row leaves the sidebar");
}

#[test]
fn a_tag_edit_submits_one_request() {
    let mut app = Headless::start(flow_desktop()).unwrap();
    app.wait_for("id:track-row").unwrap();

    app.press("id:track-row >> nth=0").unwrap();
    app.wait_for("id:tag-edit-open").unwrap();
    app.press("id:tag-edit-open").unwrap();
    app.wait_for("id:tag-editor").unwrap();
    app.wait_for("id:tag-field-0").unwrap();
    app.clear("id:tag-field-0").unwrap();
    app.type_text("id:tag-field-0", "Renamed").unwrap();
    app.press("id:tag-save-track").unwrap();
    app.press("id:flow-read-tags").unwrap();
    app.wait_for("text:flow tags 1")
        .expect("one tag-edit request reached the service");
}

#[test]
fn the_settings_toggle_writes_the_session() {
    let mut app = Headless::start(flow_desktop()).unwrap();
    app.wait_for("id:player-bar").unwrap();

    app.press("id:settings-open").unwrap();
    app.wait_for("id:settings").unwrap();
    app.press("id:settings-page-advanced").unwrap();
    app.wait_for("id:settings-advanced-mode").unwrap();
    app.press("id:settings-advanced-mode-toggle").unwrap();
    app.press("id:flow-read-session").unwrap();
    app.wait_for("text:flow session advanced true")
        .expect("Advanced mode reached the session");
}

#[test]
fn the_queue_promotes_and_clears() {
    let mut app = Headless::start(flow_desktop()).unwrap();
    app.wait_for("id:track-row").unwrap();

    app.press("id:track-row >> nth=0").unwrap();
    app.press("id:track-row >> nth=0").unwrap();
    app.wait_for("text:flow queue 1").unwrap();

    app.press("id:flow-seed-queue").unwrap();
    app.wait_for("text:flow queue 2").unwrap();
    app.press("id:queue-toggle").unwrap();
    app.wait_for("id:queue-panel").unwrap();
    app.press("id:queue-row >> nth=0").unwrap();
    app.press("id:queue-clear").unwrap();
    app.wait_for("text:Queue is empty").unwrap();
    app.wait_for("text:flow queue 0").unwrap();
}
