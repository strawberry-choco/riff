// The module tree lives in the library crate (`src/lib.rs`). Import the
// modules into the binary crate root so the existing `crate::app::...`,
// `crate::domain::...`, `crate::infra::...` and `crate::ui::...` paths below
// keep resolving unchanged.
use riff::app;
use riff::domain;
use riff::infra;
use riff::ui;

use crossbeam_channel::unbounded;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::app::MutexExt;
use crate::app::audio_engine::AudioEngine;
use crate::app::commands::{LibraryCommand, LibraryUpdate};
use crate::app::scan::build_tracks;
use crate::app::state::AppState;
use crate::app::store::{LibraryMutationStore, LibraryQueryStore};
use crate::app::traits::DecoderFactory;
use crate::app::watcher_manager::WatcherManager;
use crate::domain::{PlaybackCommand, PlaybackState, PlaybackUpdate, RepeatMode, TrackId};
use crate::infra::{
    AudioFileScanner, CpalAudioOutput, FilesystemWatcher, ImageCoverLoader, LoftyMetadataReader,
    LoftyMetadataWriter, SymphoniaDecoder,
};
use crate::ui::RiffApp;
use std::path::PathBuf;

fn main() {
    tracing_subscriber::fmt::init();

    // Open the Application Store and wire every port over its shared
    // connection (fatal on open/migration failure — never silent fallbacks).
    let (settings_store, playlist_store, library_mutation_store, library_query_store, generation) =
        open_application_store();
    // The UI's single read seam over the Library collection (ADR 0002): owns
    // the five Session Projections, the query port, and the generation.
    let session_views =
        riff::app::views::SessionViews::new(Box::new(library_query_store.clone()), generation);

    let state = Arc::new(Mutex::new(AppState::new()));
    let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
    let (update_tx, update_rx) = unbounded::<PlaybackUpdate>();
    let (library_cmd_tx, library_cmd_rx) = unbounded::<LibraryCommand>();
    let (library_update_tx, library_update_rx) = unbounded::<LibraryUpdate>();

    // Clone senders for different consumers before cmd_tx is moved
    let ui_cmd_tx = cmd_tx.clone();
    let engine_cmd_tx = cmd_tx.clone();
    let ui_library_cmd_tx = library_cmd_tx.clone();

    let app_state = state.clone();
    let engine_queries = library_query_store.clone();
    let _audio_thread = thread::spawn(move || {
        run_engine_thread(cmd_rx, engine_cmd_tx, update_tx, app_state, engine_queries);
    });

    spawn_update_processor(
        state.clone(),
        update_rx,
        cmd_tx.clone(),
        library_mutation_store.clone(),
    );

    // Library scan thread
    let cancel_flag = Arc::new(AtomicBool::new(false));
    spawn_library_scanner(
        library_cmd_rx,
        library_update_tx,
        cancel_flag.clone(),
        library_mutation_store.clone(),
        library_query_store.clone(),
    );

    let watcher_manager = spawn_fs_watcher(library_cmd_tx);

    // Background services (ADR 0006): real adapters, dedicated worker
    // threads — spawned here exactly like the Audio Engine, nowhere else.
    let (tag_edits, covers) =
        spawn_background_services(library_query_store.clone(), library_mutation_store.clone());

    let options = eframe::NativeOptions {
        // Frameless launch (Issue 04, ADR 0005): OS decorations are replaced
        // by riff's custom titlebar with a drag region and window controls.
        viewport: crate::ui::chrome::viewport_builder(),
        ..Default::default()
    };

    let quit_flag = Arc::new(AtomicBool::new(false));

    // The tray icon is owned directly by the UI (single owner, main thread);
    // no Arc<Mutex<..>> wrapper is needed around the !Send handle.
    #[cfg(not(target_os = "linux"))]
    let tray_icon = match crate::ui::tray::create_tray(cmd_tx.clone(), quit_flag.clone()) {
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
        state.clone(),
        ui_cmd_tx,
        ui_library_cmd_tx,
        library_update_rx,
        watcher_manager,
        tray_icon,
        quit_flag.clone(),
        Box::new(settings_store),
        Box::new(playlist_store),
        Box::new(library_mutation_store),
        session_views,
        Box::new(tag_edits),
        Box::new(covers),
    );

    #[cfg(target_os = "linux")]
    let app = RiffApp::new(
        state.clone(),
        ui_cmd_tx,
        ui_library_cmd_tx,
        library_update_rx,
        watcher_manager,
        quit_flag.clone(),
        Box::new(settings_store),
        Box::new(playlist_store),
        Box::new(library_mutation_store),
        session_views,
        Box::new(tag_edits),
        Box::new(covers),
    );

    eframe::run_native(
        "riff",
        options,
        Box::new(|cc| {
            crate::ui::fonts::configure_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .expect("Failed to run eframe");
}

/// Thread that applies [`PlaybackUpdate`]s to the shared state and drives
/// auto-advance when a track ends.
fn spawn_update_processor(
    state: Arc<Mutex<AppState>>,
    update_rx: crossbeam_channel::Receiver<PlaybackUpdate>,
    cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
    mut mutation_store: riff::infra::store::MutexLibraryMutationStore,
) {
    let _handle = thread::spawn(move || {
        while let Ok(update) = update_rx.recv() {
            let mut locked = state.lock_or_recover();
            match update {
                PlaybackUpdate::StateChanged(new_state) => {
                    locked.playback_state = new_state;
                }
                PlaybackUpdate::PositionChanged(pos) => {
                    locked.current_position = pos;
                }
                PlaybackUpdate::TrackChanged(track_id) => {
                    locked.queue.current_index =
                        locked.queue.tracks.iter().position(|id| id == &track_id);
                }
                PlaybackUpdate::TrackEnded => {
                    drop(locked);
                    handle_track_ended(&state, &cmd_tx, &mut mutation_store);
                }
                PlaybackUpdate::Error(msg) => {
                    tracing::error!("Playback error: {}", msg);
                    locked.playback_state = PlaybackState::Stopped;
                    locked.scan_status = Some(format!("Playback error: {msg}"));
                }
            }
        }
    });
}

/// Record play history for the track that just finished — the queue's current
/// track at this moment, before the auto-advance below moves the index — and
/// advance the queue (or stop when nothing follows).
///
/// The play commits to the Application Store FIRST as its own single durable
/// transaction (ticket 06), so a crash right after the track ends cannot lose
/// it; the mutation adapter bumps the session generation so Session
/// Projections refetch.
fn handle_track_ended(
    state: &Arc<Mutex<AppState>>,
    cmd_tx: &crossbeam_channel::Sender<PlaybackCommand>,
    mutation_store: &mut riff::infra::store::MutexLibraryMutationStore,
) {
    let finished_id = {
        let locked = state.lock_or_recover();
        locked.queue.current_track().cloned()
    };
    if let Some(finished_id) = finished_id {
        // The mutation adapter bumps the session generation when the play
        // commits; the mirror no longer tracks play history.
        match mutation_store.record_track_played(&finished_id, std::time::SystemTime::now()) {
            Ok(true) => {}
            Ok(false) => tracing::debug!(?finished_id, "finished track is not in the store"),
            Err(e) => tracing::error!("Failed to persist play history for {finished_id:?}: {e}"),
        }
    }
    let next_track = {
        let mut locked = state.lock_or_recover();
        if locked.queue.repeat == RepeatMode::One {
            // Repeat-one loops the SAME track (Task 4.1): the queue
            // deliberately doesn't model it (`advance()` would move on), so
            // re-play the current track. If the engine already handed off
            // gaplessly, its Play(current) dedup guard swallows this no-op.
            locked.queue.current_track().cloned()
        } else {
            locked.queue.advance().cloned()
        }
    };
    if let Some(track_id) = next_track {
        let _ = cmd_tx.send(PlaybackCommand::Play(track_id));
    } else {
        let mut locked = state.lock_or_recover();
        locked.playback_state = PlaybackState::Stopped;
    }
}

/// Thread that receives [`LibraryCommand`]s and runs directory scans. The
/// scan thread never touches `AppState`: it reads the store through the
/// query port and commits through the mutation port.
fn spawn_library_scanner(
    library_cmd_rx: crossbeam_channel::Receiver<LibraryCommand>,
    library_update_tx: crossbeam_channel::Sender<LibraryUpdate>,
    cancel_flag: Arc<AtomicBool>,
    mut mutation_store: riff::infra::store::MutexLibraryMutationStore,
    query_store: riff::infra::store::MutexLibraryQueryStore,
) {
    let _handle = thread::spawn(move || {
        let reader = LoftyMetadataReader::new();
        while let Ok(cmd) = library_cmd_rx.recv() {
            match cmd {
                LibraryCommand::ScanDirectory(path) => {
                    cancel_flag.store(false, Ordering::Relaxed);
                    let scanner = AudioFileScanner::new(cancel_flag.clone());
                    scan_directory(
                        &reader,
                        &scanner,
                        &path,
                        &library_update_tx,
                        &cancel_flag,
                        &mut mutation_store,
                        &query_store,
                    );
                }
                LibraryCommand::CancelScan => {
                    cancel_flag.store(true, Ordering::Relaxed);
                }
            }
        }
    });
}

/// Scan one directory in ~10-track batches. Every batch commits to the
/// Application Store as ONE durable transaction first — an interrupted scan
/// keeps all committed batches — and the mutation adapter bumps the session
/// generation so Session Projections refetch.
#[allow(clippy::too_many_arguments)]
fn scan_directory(
    reader: &LoftyMetadataReader,
    scanner: &AudioFileScanner,
    path: &std::path::Path,
    library_update_tx: &crossbeam_channel::Sender<LibraryUpdate>,
    cancel_flag: &Arc<AtomicBool>,
    mutation_store: &mut riff::infra::store::MutexLibraryMutationStore,
    query_store: &riff::infra::store::MutexLibraryQueryStore,
) {
    let files = scanner.scan(path);
    let total = files.len();
    let chunk_size = 10;

    for (i, chunk) in files.chunks(chunk_size).enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }

        let processed = i * chunk_size + chunk.len();

        // Skip paths the store already knows so rescans don't re-read
        // unchanged metadata. One indexed primary-key lookup per path —
        // cheap next to the tag I/O it saves, and the scan thread stays
        // off `AppState` entirely.
        let mut fresh_paths: Vec<PathBuf> = Vec::with_capacity(chunk.len());
        for p in chunk {
            match query_store.get_track(&TrackId::from_path(p)) {
                Ok(None) => fresh_paths.push(p.clone()),
                Ok(Some(_)) => {}
                Err(e) => {
                    // When the check fails, scan the path anyway: the store
                    // upsert is idempotent and preserves play history.
                    tracing::warn!("Freshness check failed for {p:?}: {e}");
                    fresh_paths.push(p.clone());
                }
            }
        }

        if !fresh_paths.is_empty() {
            // Per-file read failures are skipped inside `build_tracks`, so a
            // scan never aborts on one bad file.
            let tracks = build_tracks(fresh_paths, reader);
            if !tracks.is_empty() {
                // The mutation adapter bumps the session generation on each
                // committed batch.
                if let Err(e) = mutation_store.apply_scan_batch(&tracks) {
                    tracing::error!("Scan batch failed to commit: {e}");
                }
            }
        }

        let _ = library_update_tx.send(LibraryUpdate::Progress {
            path: path.to_path_buf(),
            files_found: processed.min(total),
            current_dir: path.to_string_lossy().to_string(),
        });
    }

    let _ = library_update_tx.send(LibraryUpdate::Complete {
        path: path.to_path_buf(),
        total_files: total,
    });
}

/// Create the filesystem watcher and its manager, and spawn the thread that
/// forwards watch events. Returns the shared manager handle.
fn spawn_fs_watcher(
    library_cmd_tx: crossbeam_channel::Sender<LibraryCommand>,
) -> Arc<Mutex<Option<WatcherManager>>> {
    let (fs_event_tx, fs_event_rx) = unbounded::<PathBuf>();
    let watcher = match FilesystemWatcher::new(fs_event_tx) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!("Failed to create filesystem watcher: {}", e);
            None
        }
    };

    let watcher_manager = Arc::new(Mutex::new(Some(WatcherManager::new(
        watcher,
        library_cmd_tx,
    ))));

    let thread_manager = watcher_manager.clone();
    let _handle = thread::spawn(move || {
        while let Ok(changed_path) = fs_event_rx.recv() {
            if let Some(ref mut mgr) = *thread_manager.lock_or_recover() {
                mgr.on_fs_event(&changed_path);
            }
        }
    });

    watcher_manager
}

/// Open the Application Store before anything else and wire every store port
/// over its one shared connection. Open or migration failures are fatal
/// startup errors with a clear message — never silent fallbacks to empty
/// state. Returns the ports in their UI/thread wiring order plus the session
/// generation the mutation adapter bumps (ADR 0002).
#[allow(clippy::type_complexity)]
fn open_application_store() -> (
    riff::infra::store::MutexSettingsStore,
    riff::infra::store::MutexPlaylistStore,
    riff::infra::store::MutexLibraryMutationStore,
    riff::infra::store::MutexLibraryQueryStore,
    riff::app::store::StoreGeneration,
) {
    let store_path = riff::infra::store::default_store_path().expect(
        "fatal: could not resolve the Application Store location \
         (no data-local directory available)",
    );
    // One shared connection behind a mutex serves every store port.
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        riff::infra::store::SqliteStore::open_and_migrate(&store_path)
            .unwrap_or_else(|e| panic!("fatal: {e}")),
    ));
    let settings_store = riff::infra::store::MutexSettingsStore::new(store.clone());
    let playlist_store = riff::infra::store::MutexPlaylistStore::new(store.clone());
    // Library collection ports over the same shared connection: scans write
    // through the mutation port, playback resolves through the query port,
    // and every committed mutation bumps this session-local generation so
    // Session Projections know to refetch (ADR 0002). The mutation adapter
    // owns the bump — callers cannot forget it.
    let generation = riff::app::store::StoreGeneration::new();
    let library_mutation_store =
        riff::infra::store::MutexLibraryMutationStore::new(store.clone(), generation.clone());
    let library_query_store = riff::infra::store::MutexLibraryQueryStore::new(store.clone());
    (
        settings_store,
        playlist_store,
        library_mutation_store,
        library_query_store,
        generation,
    )
}

/// Composition-root wiring for the background services (ADR 0006): construct
/// the real Tag Edit and Cover service pairs over real adapters and run each
/// blocking worker on its dedicated thread — exactly like the Audio Engine.
/// Returns the front-end handles the UI holds boxed (`Box<dyn TagEdits>`,
/// `Box<dyn Covers>`).
fn spawn_background_services(
    library_queries: riff::infra::store::MutexLibraryQueryStore,
    library_mutations: riff::infra::store::MutexLibraryMutationStore,
) -> (
    riff::app::tag_edit_service::TagEditService,
    riff::app::cover_service::CoverService,
) {
    let (tag_edits, tag_worker) = riff::app::tag_edit_service::TagEditService::new(
        Box::new(LoftyMetadataWriter::new()),
        Box::new(library_queries),
        Box::new(library_mutations),
    );
    let _handle = thread::spawn(move || tag_worker.run());

    let (covers, cover_worker) = riff::app::cover_service::CoverService::new(
        Box::new(LoftyMetadataReader::new()),
        Box::new(ImageCoverLoader::new()),
    );
    let _handle = thread::spawn(move || cover_worker.run());

    (tag_edits, covers)
}

/// Build the shared codec registry (symphonia defaults + the Opus adapter).
/// Used for both the primary decoder and the gapless pre-decode decoder.
fn build_codec_registry() -> symphonia::core::codecs::registry::CodecRegistry {
    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

/// Composition-root wiring for the audio engine thread: construct the real
/// adapters (symphonia decoder factory, cpal output, store query port) and
/// run the engine loop on the calling thread. `CodecRegistry` is not `Clone`,
/// so the factory builds a fresh registry for every decoder it mints.
fn run_engine_thread(
    cmd_rx: crossbeam_channel::Receiver<PlaybackCommand>,
    cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
    update_tx: crossbeam_channel::Sender<PlaybackUpdate>,
    state: Arc<Mutex<AppState>>,
    library_queries: riff::infra::store::MutexLibraryQueryStore,
) {
    let decoder_factory: DecoderFactory =
        Box::new(|| Box::new(SymphoniaDecoder::new(build_codec_registry())));
    let engine = AudioEngine::new(
        cmd_rx,
        cmd_tx,
        update_tx,
        state,
        Box::new(library_queries),
        decoder_factory,
        Box::new(CpalAudioOutput::new()),
    );
    engine.run();
}
