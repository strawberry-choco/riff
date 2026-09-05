//! The Composition Root — the one place that knows both ports and concrete
//! adapters. [`AppRuntime::spawn`] opens the Application Store, constructs
//! every real adapter from `riff-infra`, wires them into the slice-defined
//! ports, and spawns the worker threads (audio engine, playback coordinator,
//! scan worker, watcher forwarder, tag-edit and cover workers) exactly as
//! the frontend's binary entry point did before backend-crate-split
//! issue 08. The frontend then becomes a thin composition over the returned
//! [`AppRuntime`] handles.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam_channel::unbounded;

use riff_infra::audio::decoder::default_codec_registry;
use riff_infra::audio::{CpalAudioOutput, SymphoniaDecoder};
use riff_infra::filesystem::{AudioFileScanner, FilesystemWatcher};
use riff_infra::media::{ImageCoverLoader, LoftyMetadataReader, LoftyMetadataWriter};
use riff_infra::store::SqliteStore;
use riff_persistence::errors::StoreError;
use riff_persistence::store::{LibraryMutationStore, PlaylistStore, ScanOptions, SettingsStore};

use riff_library::app::cover_service::{CoverPolicy, CoverService};
use riff_library::app::scan_service::ScanService;

use riff_playback::app::playback_coordinator::PlaybackCoordinator;
use riff_playback::app::transport::{ChannelTransport, FacadeTransport, Transport};
use riff_playback::infra::audio_engine::AudioEngine;
use riff_playback::infra::ports::DecoderFactory;

use crate::app::MutexExt;
use crate::app::facade::BackendFacade;
use crate::app::state::{LibrarySession, PlaybackSession};
use crate::app::tag_edit_service::TagEditService;
use crate::app::views::SessionViews;
use crate::app::watcher_manager::WatcherManager;

pub use riff_infra::store::default_store_path;

/// The composed application: every handle the frontend renders with, plus
/// the worker threads it drives — all produced by one [`AppRuntime::spawn`]
/// call over the Composition Root.
pub struct AppRuntime {
    /// The shared playback session (engine, coordinator, transport, tray).
    pub playback: Arc<Mutex<PlaybackSession>>,
    /// The shared library session (library use cases and UI state).
    pub library: Arc<Mutex<LibrarySession>>,
    /// The single shared backend facade: both transports record dispatched
    /// commands onto its event inbox, and the frontend drains its events.
    pub facade: Arc<Mutex<BackendFacade>>,
    /// The UI's command transport: every dispatch is recorded onto the
    /// facade's event inbox before being forwarded to the audio engine.
    pub ui_transport: Box<dyn Transport>,
    /// The tray's command transport — same facade, same command channel.
    pub tray_transport: FacadeTransport,
    /// The Library Scan front-end handle (boxed into the UI; cloned by the
    /// watcher manager, which is already wired inside `spawn`).
    pub scans: ScanService,
    /// The filesystem-watcher manager handle the UI reconfigures.
    pub watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
    /// Set by the UI/tray to request application shutdown.
    pub quit_flag: Arc<AtomicBool>,
    /// Settings section of the Application Store.
    pub settings: Box<dyn SettingsStore>,
    /// Playlists section of the Application Store (mutations commit here).
    pub playlists: Box<dyn PlaylistStore>,
    /// Library mutation section of the Application Store.
    pub library_mutations: Box<dyn LibraryMutationStore>,
    /// The UI's single read seam over the Application Store (ADR 0002).
    pub session_views: SessionViews,
    /// The Tag Edit service front-end handle.
    pub tag_edits: Box<dyn crate::app::tag_edit_service::TagEdits>,
    /// The Cover service front-end handle.
    pub covers: Box<dyn riff_library::app::cover_service::Covers>,
}

impl AppRuntime {
    /// Open the Application Store at `store_path`, wire every real adapter
    /// into its port, and spawn the worker threads. Store open/migration
    /// failures are returned to the caller — never silent fallbacks.
    pub fn spawn(store_path: &Path) -> Result<Self, StoreError> {
        // The store: one shared connection behind an internal mutex serves
        // every store port; both session generations bump inside the store's
        // mutation impls, and the change channel feeds the facade.
        let (
            settings_store,
            playlist_store,
            library_mutation_store,
            library_query_store,
            generation,
            playlist_generation,
            changes_rx,
        ) = open_application_store(store_path)?;

        // The single shared `BackendFacade`: every `FacadeTransport` — the
        // UI's and the tray's — wraps the same `Arc<Mutex<BackendFacade>>`,
        // so dispatched commands are recorded onto one observable event
        // inbox. The store's `StoreChanged` stream is the facade's second
        // input. Playback errors surface as typed notices (issue 01 seam
        // fix): the coordinator sends pre-formatted messages over this
        // channel and the facade stamps them with playback source + error
        // severity.
        let facade = Arc::new(Mutex::new(BackendFacade::default()));
        let (notice_tx, notice_rx) = unbounded::<String>();
        {
            let mut f = facade.lock_or_recover();
            f.subscribe_to_backend_changes(changes_rx);
            f.subscribe_playback_notices(notice_rx);
        }

        // The UI's single read seam over the Application Store (ADR 0002).
        let session_views = SessionViews::new(
            Box::new(library_query_store.clone()),
            Box::new(playlist_store.clone()),
            generation,
            playlist_generation,
        );

        let playback = Arc::new(Mutex::new(PlaybackSession::default()));
        let library = Arc::new(Mutex::new(LibrarySession::default()));
        let (cmd_tx, cmd_rx) = unbounded::<riff_playback::domain::PlaybackCommand>();
        let (update_tx, update_rx) = unbounded::<riff_playback::domain::PlaybackUpdate>();

        // Clone senders for different consumers before cmd_tx is moved.
        let ui_cmd_tx = cmd_tx.clone();
        let tray_cmd_tx = cmd_tx.clone();
        let engine_cmd_tx = cmd_tx.clone();
        let app_state = playback.clone();
        let engine_queries = library_query_store.clone();
        let _audio_thread = thread::spawn(move || {
            run_engine_thread(cmd_rx, engine_cmd_tx, update_tx, app_state, engine_queries);
        });

        // Playback Coordinator: applies Playback Updates to session state
        // and owns playback continuation, on its dedicated thread.
        let _update_processor = PlaybackCoordinator::spawn(
            playback.clone(),
            update_rx,
            cmd_tx.clone(),
            Box::new(library_mutation_store.clone()),
            notice_tx,
        );

        // Library Scan Service (ADR 0006 pattern): the whole Library Scan
        // flow — walk, freshness filter, durable ~10-track batch commits,
        // cancellation, and per-path scan state — lives behind the `Scans`
        // seam on one serial worker thread. The walk closure binds the real
        // infra scanner to the SAME cancel flag the service cancels through,
        // so the app layer never names infra types.
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let scanner = AudioFileScanner::new(cancel_flag.clone());
        // A settings clone bound into the walk so each scan reads the
        // Library Scan options fresh — the Settings pane's toggles and
        // format chips take effect on the next scan without a restart
        // (design-handoff issue 12). A read failure falls back to the
        // historical defaults rather than failing the scan.
        let scan_settings = settings_store.clone();
        let (scans, scan_worker) = ScanService::new(
            Box::new(LoftyMetadataReader::new()),
            Box::new(library_query_store.clone()),
            Box::new(library_mutation_store.clone()),
            cancel_flag,
            move |path| {
                let options = scan_settings.load_settings().map_or_else(
                    |e| {
                        tracing::warn!("Failed to read the scan options from the store: {e}");
                        ScanOptions::default()
                    },
                    |settings| ScanOptions::from(&settings.scalars),
                );
                scanner.scan(path, &options)
            },
        );
        let _scan_thread = thread::spawn(move || scan_worker.run());

        let watcher_manager = spawn_fs_watcher(scans.clone());

        // Background services (ADR 0006): real adapters, dedicated worker
        // threads — spawned here exactly like the Audio Engine.
        let (tag_edits, covers) =
            spawn_background_services(library_query_store.clone(), library_mutation_store.clone());

        // The UI's `Box<dyn Transport>` is a `FacadeTransport` wrapping the
        // shared facade, so every UI intent is recorded synchronously onto
        // the facade's event inbox before it is forwarded. The tray's
        // transport shares both the facade and the command channel.
        let recorder = {
            let facade = facade.clone();
            move |cmd: riff_playback::domain::PlaybackCommand| {
                facade.lock_or_recover().record_command(cmd);
            }
        };
        let ui_transport: Box<dyn Transport> = Box::new(FacadeTransport::new(
            ChannelTransport::new(ui_cmd_tx),
            Box::new(recorder.clone()),
        ));
        let tray_transport =
            FacadeTransport::new(ChannelTransport::new(tray_cmd_tx), Box::new(recorder));

        let quit_flag = Arc::new(AtomicBool::new(false));

        Ok(Self {
            playback,
            library,
            facade,
            ui_transport,
            tray_transport,
            scans,
            watcher_manager,
            quit_flag,
            settings: Box::new(settings_store),
            playlists: Box::new(playlist_store),
            library_mutations: Box::new(library_mutation_store),
            session_views,
            tag_edits: Box::new(tag_edits),
            covers: Box::new(covers),
        })
    }
}

/// Open the Application Store before anything else and wire every store port
/// over its one shared connection. Returns one clone of the shared store
/// handle per port in their UI/thread wiring order plus both session
/// generations the store bumps on committed mutations (ADR 0002): the
/// Library generation and the dedicated playlist generation.
#[allow(clippy::type_complexity)]
fn open_application_store(
    store_path: &Path,
) -> Result<
    (
        SqliteStore,
        SqliteStore,
        SqliteStore,
        SqliteStore,
        crate::app::store::StoreGeneration,
        crate::app::store::StoreGeneration,
        crossbeam_channel::Receiver<crate::app::store::StoreChanged>,
    ),
    StoreError,
> {
    let (changes_tx, changes_rx) =
        crossbeam_channel::unbounded::<crate::app::store::StoreChanged>();
    let store = SqliteStore::open_and_migrate(store_path, changes_tx)?;
    Ok((
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        store.library_generation(),
        store.playlist_generation(),
        changes_rx,
    ))
}

/// Create the filesystem watcher and its manager, and spawn the thread that
/// forwards watch events. Returns the shared manager handle.
fn spawn_fs_watcher(scans: ScanService) -> Arc<Mutex<Option<WatcherManager>>> {
    let (fs_event_tx, fs_event_rx) = unbounded::<Vec<std::path::PathBuf>>();
    let watcher: Option<Box<dyn crate::app::traits::FilesystemWatch>> =
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

/// Composition-root wiring for the background services (ADR 0006): construct
/// the real Tag Edit and Cover service pairs over real adapters and run each
/// blocking worker on its dedicated thread — exactly like the Audio Engine.
/// Returns the front-end handles the UI holds boxed (`Box<dyn TagEdits>`,
/// `Box<dyn Covers>`).
fn spawn_background_services(
    library_queries: SqliteStore,
    library_mutations: SqliteStore,
) -> (TagEditService, CoverService) {
    // The cover worker reads the artwork policy fresh per resolution so
    // the Settings pane's "Read embedded artwork" toggle applies
    // immediately (design-handoff issue 12); a read failure falls back to
    // the historical read-embedded default.
    let cover_settings = library_queries.clone();
    let (tag_edits, tag_worker) = TagEditService::new(
        Box::new(LoftyMetadataWriter::new()),
        Box::new(library_queries),
        Box::new(library_mutations),
    );
    let _handle = thread::spawn(move || tag_worker.run());

    let cover_policy: CoverPolicy = Box::new(move || {
        cover_settings.load_settings().map_or_else(
            |e| {
                tracing::warn!("Failed to read the artwork policy from the store: {e}");
                true
            },
            |settings| settings.scalars.read_embedded_artwork,
        )
    });
    let (covers, cover_worker) = CoverService::new(
        Box::new(LoftyMetadataReader::new()),
        Box::new(ImageCoverLoader::new()),
        cover_policy,
    );
    let _handle = thread::spawn(move || cover_worker.run());

    (tag_edits, covers)
}

/// Composition-root wiring for the audio engine thread: construct the real
/// adapters (symphonia decoder factory, cpal output, store query port) and
/// run the engine loop on the calling thread. `CodecRegistry` is not `Clone`,
/// so the factory builds a fresh registry for every decoder it mints.
fn run_engine_thread(
    cmd_rx: crossbeam_channel::Receiver<riff_playback::domain::PlaybackCommand>,
    cmd_tx: crossbeam_channel::Sender<riff_playback::domain::PlaybackCommand>,
    update_tx: crossbeam_channel::Sender<riff_playback::domain::PlaybackUpdate>,
    state: Arc<Mutex<PlaybackSession>>,
    library_queries: SqliteStore,
) {
    let decoder_factory: DecoderFactory =
        Box::new(|| Box::new(SymphoniaDecoder::new(default_codec_registry())));
    let engine = AudioEngine::new(
        cmd_rx,
        cmd_tx,
        update_tx,
        Box::new(library_queries),
        decoder_factory,
        Box::new(CpalAudioOutput::new()),
        state,
    );
    engine.run();
}
