// Composition root for the `riff` binary (lives in the frontend crate per
// the spec: "the binary entry point, which becomes a thin composition over
// the backend facade"). The whole wiring pattern is unchanged from the
// pre-split code — the only delta is that the UI's command channel is now
// wrapped in a `FacadeTransport` so every dispatch is recorded onto the
// shared `BackendFacade` event inbox the tray also writes to. The audio
// engine, scan service, watcher, and tag/cover workers still own their
// threads exactly as before.
use crossbeam_channel::unbounded;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;

use riff_backend::app::MutexExt;
use riff_backend::app::audio_engine::AudioEngine;
use riff_backend::app::facade::BackendFacade;
use riff_backend::app::playback_coordinator::PlaybackCoordinator;
use riff_backend::app::state::{LibrarySession, PlaybackSession};
use riff_backend::app::traits::DecoderFactory;
use riff_backend::app::transport::FacadeTransport;
use riff_backend::app::watcher_manager::WatcherManager;
use riff_backend::domain::{PlaybackCommand, PlaybackUpdate};
use riff_backend::infra::{
    AudioFileScanner, CpalAudioOutput, FilesystemWatcher, ImageCoverLoader, LoftyMetadataReader,
    LoftyMetadataWriter, SymphoniaDecoder,
};
use riff_gui::ui::RiffApp;
use riff_gui::ui::window_visibility::spawn_visibility_listener;
use std::path::PathBuf;
#[allow(clippy::too_many_lines)]
fn main() {
    tracing_subscriber::fmt::init();
    // connection (fatal on open/migration failure — never silent fallbacks).
    let (
        settings_store,
        playlist_store,
        library_mutation_store,
        library_query_store,
        generation,
        playlist_generation,
        changes_rx,
    ) = open_application_store();
    // The single shared `BackendFacade` (Issues 02/04): every `FacadeTransport`
    // — the UI's and the tray's — wraps the same `Arc<Mutex<BackendFacade>>`,
    // so dispatched commands are recorded onto one observable event inbox.
    // The store's `StoreChanged` stream is the facade's second input.
    let facade = Arc::new(Mutex::new(BackendFacade::default()));
    // Playback errors surface as typed notices (issue 01 seam fix): the
    // coordinator sends pre-formatted messages over this channel and the
    // facade stamps them with playback source + error severity.
    let (notice_tx, notice_rx) = unbounded::<String>();
    {
        let mut f = facade.lock_or_recover();
        f.subscribe_to_backend_changes(changes_rx);
        f.subscribe_playback_notices(notice_rx);
    }
    // The UI's single read seam over the Application Store (ADR 0002).
    let session_views = wire_session_views(
        &library_query_store,
        &playlist_store,
        generation,
        playlist_generation,
    );

    let playback = Arc::new(Mutex::new(PlaybackSession::new()));
    let library = Arc::new(Mutex::new(LibrarySession::new()));
    let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
    let (update_tx, update_rx) = unbounded::<PlaybackUpdate>();

    // Clone senders for different consumers before cmd_tx is moved
    let ui_cmd_tx = cmd_tx.clone();
    let engine_cmd_tx = cmd_tx.clone();
    let tray_cmd_tx = cmd_tx.clone();
    let app_state = playback.clone();
    let engine_queries = library_query_store.clone();
    let _audio_thread = thread::spawn(move || {
        run_engine_thread(cmd_rx, engine_cmd_tx, update_tx, app_state, engine_queries);
    });

    // Playback Coordinator (CONTEXT.md): applies Playback Updates to session
    // state and owns playback continuation, on its dedicated thread.
    let _update_processor = PlaybackCoordinator::spawn(
        playback.clone(),
        update_rx,
        cmd_tx.clone(),
        Box::new(library_mutation_store.clone()),
        notice_tx,
    );

    // Library Scan Service (ADR 0006 pattern): the whole Library Scan flow —
    // walk, freshness filter, durable ~10-track batch commits, cancellation,
    // and per-path scan state — lives behind the `Scans` seam on one serial
    // worker thread, spawned here exactly like the Audio Engine. The walk
    // closure binds the real infra scanner to the SAME cancel flag the
    // service cancels through, so the app layer never names infra types.
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let scanner = AudioFileScanner::new(cancel_flag.clone());
    let (scans, scan_worker) = riff_backend::app::scan_service::ScanService::new(
        Box::new(LoftyMetadataReader::new()),
        Box::new(library_query_store.clone()),
        Box::new(library_mutation_store.clone()),
        cancel_flag,
        move |path| scanner.scan(path),
    );
    let _scan_thread = thread::spawn(move || scan_worker.run());

    let watcher_manager = spawn_fs_watcher(scans.clone());

    // Background services (ADR 0006): real adapters, dedicated worker
    // threads — spawned here exactly like the Audio Engine, nowhere else.
    let (tag_edits, covers) =
        spawn_background_services(library_query_store.clone(), library_mutation_store.clone());

    let options = eframe::NativeOptions {
        // Frameless launch (Issue 04, ADR 0005): OS decorations are replaced
        // by riff's custom titlebar with a drag region and window controls.
        viewport: riff_gui::ui::chrome::viewport_builder(),
        ..Default::default()
    };

    let quit_flag = Arc::new(AtomicBool::new(false));

    // Frontend-local visibility channel (Issue 03): the tray pushes
    // `Show Window` requests here, the UI thread drains them between frames.
    // The tray never constructs backend commands on this path.
    let (visibility_tx, visibility_listener) = spawn_visibility_listener();

    // The UI's `Box<dyn Transport>` is a `FacadeTransport` wrapping the
    // shared `Arc<Mutex<BackendFacade>>`, so every UI intent is recorded
    let ui_transport: Box<dyn riff_backend::app::transport::Transport> =
        Box::new(FacadeTransport::new(ui_cmd_tx, facade.clone()));
    // no Arc<Mutex<..>> wrapper is needed around the !Send handle.
    #[cfg(not(target_os = "linux"))]
    let tray_icon = match riff_gui::ui::tray::create_tray(
        FacadeTransport::new(tray_cmd_tx, facade.clone()),
        quit_flag.clone(),
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
        playback.clone(),
        library.clone(),
        ui_transport,
        Box::new(scans.clone()),
        watcher_manager,
        tray_icon,
        quit_flag.clone(),
        Box::new(settings_store),
        Box::new(playlist_store),
        Box::new(library_mutation_store),
        session_views,
        Box::new(tag_edits),
        Box::new(covers),
        facade,
        visibility_listener,
    );

    #[cfg(target_os = "linux")]
    let app = RiffApp::new(
        playback.clone(),
        library.clone(),
        ui_transport,
        Box::new(scans),
        watcher_manager,
        quit_flag.clone(),
        Box::new(settings_store),
        Box::new(playlist_store),
        Box::new(library_mutation_store),
        session_views,
        Box::new(tag_edits),
        Box::new(covers),
        facade,
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

/// Create the filesystem watcher and its manager, and spawn the thread that
/// forwards watch events. Returns the shared manager handle.
fn spawn_fs_watcher(
    scans: riff_backend::app::scan_service::ScanService,
) -> Arc<Mutex<Option<WatcherManager>>> {
    let (fs_event_tx, fs_event_rx) = unbounded::<Vec<PathBuf>>();
    let watcher: Option<Box<dyn riff_backend::app::traits::FilesystemWatch>> =
        match FilesystemWatcher::new(fs_event_tx) {
            Ok(w) => Some(Box::new(w)),
            Err(e) => {
                tracing::warn!("Failed to create filesystem watcher: {}", e);
                None
            }
        };

    let watcher_manager = Arc::new(Mutex::new(Some(WatcherManager::new(watcher, scans))));

    let thread_manager = watcher_manager.clone();
    let _handle = thread::spawn(move || {
        while let Ok(changed_paths) = fs_event_rx.recv() {
            if let Some(ref mut mgr) = *thread_manager.lock_or_recover() {
                mgr.on_fs_events(&changed_paths);
            }
        }
    });

    watcher_manager
}

/// Open the Application Store before anything else and wire every store port
/// over its one shared connection. Open or migration failures are fatal
/// startup errors with a clear message — never silent fallbacks to empty
/// state. Returns one clone of the shared store handle per port in their
/// UI/thread wiring order plus both session generations the store bumps on
/// committed mutations (ADR 0002): the Library generation and the dedicated
/// playlist generation.
#[allow(clippy::type_complexity)]
fn open_application_store() -> (
    riff_backend::infra::store::SqliteStore,
    riff_backend::infra::store::SqliteStore,
    riff_backend::infra::store::SqliteStore,
    riff_backend::infra::store::SqliteStore,
    riff_backend::app::store::StoreGeneration,
    riff_backend::app::store::StoreGeneration,
    crossbeam_channel::Receiver<riff_backend::app::store::StoreChanged>,
) {
    let store_path = riff_backend::infra::store::default_store_path().expect(
        "fatal: could not resolve the Application Store location \
         (no data-local directory available)",
    );
    // One shared connection behind an internal mutex serves every store
    // port: every clone of the handle shares it and both session
    // generations, whose bumps live inside the store's mutation impls —
    // callers cannot forget them. The receiver is handed to the
    // `BackendFacade` (issue 04: emit-beside-the-bump), so the facade
    // sees every committed mutation as it lands.
    let (changes_tx, changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store = riff_backend::infra::store::SqliteStore::open_and_migrate(&store_path, changes_tx)
        .unwrap_or_else(|e| panic!("fatal: {e}"));
    (
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        store.library_generation(),
        store.playlist_generation(),
        changes_rx,
    )
}

/// Composition-root wiring for the UI's read seam (ADR 0002): box the query
/// ports and hand over both session generations. Committed mutations bump
/// those same handles inside the store itself.
fn wire_session_views(
    library_query_store: &riff_backend::infra::store::SqliteStore,
    playlist_store: &riff_backend::infra::store::SqliteStore,
    generation: riff_backend::app::store::StoreGeneration,
    playlist_generation: riff_backend::app::store::StoreGeneration,
) -> riff_backend::app::views::SessionViews {
    riff_backend::app::views::SessionViews::new(
        Box::new(library_query_store.clone()),
        Box::new(playlist_store.clone()),
        generation,
        playlist_generation,
    )
}

/// Composition-root wiring for the background services (ADR 0006): construct
/// the real Tag Edit and Cover service pairs over real adapters and run each
/// blocking worker on its dedicated thread — exactly like the Audio Engine.
/// Returns the front-end handles the UI holds boxed (`Box<dyn TagEdits>`,
/// `Box<dyn Covers>`).
fn spawn_background_services(
    library_queries: riff_backend::infra::store::SqliteStore,
    library_mutations: riff_backend::infra::store::SqliteStore,
) -> (
    riff_backend::app::tag_edit_service::TagEditService,
    riff_backend::app::cover_service::CoverService,
) {
    let (tag_edits, tag_worker) = riff_backend::app::tag_edit_service::TagEditService::new(
        Box::new(LoftyMetadataWriter::new()),
        Box::new(library_queries),
        Box::new(library_mutations),
    );
    let _handle = thread::spawn(move || tag_worker.run());

    let (covers, cover_worker) = riff_backend::app::cover_service::CoverService::new(
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
    state: Arc<Mutex<PlaybackSession>>,
    library_queries: riff_backend::infra::store::SqliteStore,
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
