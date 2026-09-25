//! The Composition Root — the one place that knows both ports and concrete
//! adapters. [`AppRuntime::spawn`] opens the Application Store, constructs
//! every real adapter from `riff-infra`, wires them into the slice-defined
//! ports, and spawns the worker threads (audio engine, playback coordinator,
//! scan worker, watcher forwarder, tag-edit and cover workers) exactly as
//! the frontend's binary entry point did before backend-crate-split
//! issue 08.
//!
//! It returns two halves. [`AppRuntime`] is what the frontend renders with;
//! [`RuntimeLifecycle`] owns the worker threads and the levers that end
//! them, so the process that spawns the workers is also the process that
//! joins them. The frontend becomes a thin composition over both.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

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
use riff_playback::app::transport::{ChannelTransport, Transport};
use riff_playback::infra::audio_engine::AudioEngine;
use riff_playback::infra::ports::DecoderFactory;

use crate::app::MutexExt;
use crate::app::events::BackendEvents;
use crate::app::state::{LibrarySession, PlaybackSession};
use crate::app::tag_edit_service::TagEditService;
use crate::app::views::SessionViews;
use crate::app::watcher_manager::WatcherManager;

pub use riff_infra::store::default_store_path;

/// The composed application: every handle the frontend renders with — one
/// half of what [`AppRuntime::spawn`] returns. The worker threads that drive
/// this state belong to the other half, [`RuntimeLifecycle`].
pub struct AppRuntime {
    /// The shared playback session (engine, coordinator, transport, tray).
    pub playback: Arc<Mutex<PlaybackSession>>,
    /// The shared library session (library use cases and UI state).
    pub library: Arc<Mutex<LibrarySession>>,
    /// The backend's event inbox: both transports record dispatched
    /// commands onto it, and the frontend drains it each frame.
    pub backend_events: Arc<Mutex<BackendEvents>>,
    /// The UI's command transport: every dispatch is recorded onto the
    /// event inbox before being forwarded to the audio engine.
    pub ui_transport: Box<dyn Transport>,
    /// The tray's command transport — same event inbox, same command channel.
    pub tray_transport: ChannelTransport,
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

/// The other half of [`AppRuntime::spawn`]: the worker threads the runtime
/// started, plus every lever that ends them.
///
/// It deliberately holds nothing the UI renders with, and it has no `Drop`
/// impl: shutdown is an explicit call, so a runtime is never torn down as a
/// side effect of a value going out of scope. That also keeps `shutdown`
/// idempotent — each cell's join handle is `take()`n before it is joined, so
/// a second call finds nothing left to do and returns immediately.
///
/// There is exactly one method, [`Self::shutdown`].
pub struct RuntimeLifecycle {
    /// The audio engine thread cell. Joined first: its exit drops the update
    /// sender, which is what ends the coordinator.
    audio: WorkerCell,
    /// The Playback Coordinator thread cell. No stop lever: it exits when
    /// the audio engine's exit drops its update channel.
    coordinator: WorkerCell,
    /// The Library Scan worker thread cell.
    scan: WorkerCell,
    /// The Tag Edit worker thread cell.
    tag_edit: WorkerCell,
    /// The Cover worker thread cell.
    cover: WorkerCell,
    /// The filesystem-event forwarder thread cell. No stop lever: it exits
    /// when clearing the watcher drops the event sender it is parked on.
    fs_forwarder: WorkerCell,
    /// The scan worker's cancel flag. Held as its own clone rather than
    /// reached through `Scans::cancel`, so shutdown does not depend on a
    /// front-end handle the UI may already have moved or dropped.
    scan_cancel: Arc<AtomicBool>,
    /// The watcher-manager cell. Clearing it drops the filesystem watcher,
    /// which drops the event sender the forwarder is parked on.
    watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
}

/// One worker thread's lifecycle cell: the optional cooperative stop lever
/// shutdown raises to end it, plus its join handle.
///
/// Workers that stop by lever hold a cell with a lever — the audio engine,
/// the library scan worker, the tag-edit worker, and the cover worker — each
/// returns at its next poll once the lever is raised. Workers that exit when
/// their upstream channel closes hold a cell without one — the playback
/// coordinator (its update channel disconnects when the audio engine's exit
/// drops the sender) and the filesystem-event forwarder (clearing the
/// watcher drops the event sender it is parked on). Those exceptions are a
/// property of each cell, not of the shutdown sequence.
struct WorkerCell {
    /// The cooperative stop lever, `None` for workers that end when their
    /// upstream channel closes.
    stop: Option<Arc<AtomicBool>>,
    /// The worker thread's join handle; taken by [`Self::join`], so a second
    /// shutdown finds no handle left to wait on.
    join: Option<JoinHandle<()>>,
}

impl WorkerCell {
    /// One worker thread: `stop` is the lever shutdown raises to end it, or
    /// `None` when the worker exits on its upstream channel closing.
    fn new(stop: Option<Arc<AtomicBool>>, join: JoinHandle<()>) -> Self {
        Self {
            stop,
            join: Some(join),
        }
    }

    /// Raise this cell's stop lever, if it has one. Workers without a lever
    /// (the playback coordinator, the filesystem-event forwarder) exit when
    /// their upstream channel closes instead.
    fn raise_stop(&self) {
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::Relaxed);
        }
    }

    /// Join this cell's worker thread, naming the stage. A worker that
    /// panicked is reported here rather than taking the shutting-down process
    /// with it, and the log line names the stage, which is the only way to
    /// tell a wedged join from a slow one. A second call finds no handle and
    /// returns immediately.
    fn join(&mut self, stage: &str) {
        let Some(handle) = self.join.take() else {
            // Already joined: a repeated `shutdown` finds no handle to wait on.
            return;
        };
        tracing::debug!("Joining the {stage} thread");
        if let Err(panic) = handle.join() {
            tracing::error!("The {stage} thread panicked: {panic:?}");
        }
    }
}

impl RuntimeLifecycle {
    /// Stop every worker thread and wait for it to return.
    ///
    /// The sequence matters:
    ///
    /// 1. Raise the stop levers, so each request-channel worker returns at
    ///    its next poll. The coordinator and the forwarder hold no lever:
    ///    they exit when their upstream channel closes.
    /// 2. Cancel the scan in flight. It aborts at its next batch boundary,
    ///    and the batches already committed stay — durability is per batch,
    ///    so an interrupted scan never rolls work back (spec user story 3).
    /// 3. Clear the watcher, which drops the event sender the forwarder is
    ///    blocked on, so that loop exits without needing a flag of its own.
    /// 4. Join, in dependency order: the audio engine first, because its exit
    ///    is what disconnects the coordinator's update channel.
    ///
    /// A second call is a no-op: every handle has already been taken.
    pub fn shutdown(&mut self) {
        self.audio.raise_stop();
        self.scan.raise_stop();
        self.tag_edit.raise_stop();
        self.cover.raise_stop();

        self.scan_cancel.store(true, Ordering::Relaxed);

        *self.watcher_manager.lock_or_recover() = None;

        self.audio.join("audio engine");
        self.coordinator.join("playback coordinator");
        self.scan.join("library scan worker");
        self.tag_edit.join("tag-edit worker");
        self.cover.join("cover worker");
        self.fs_forwarder.join("filesystem-event forwarder");
    }
}

impl AppRuntime {
    /// Open the Application Store at `store_path`, wire every real adapter
    /// into its port, and spawn the worker threads. Store open/migration
    /// failures are returned to the caller — never silent fallbacks.
    ///
    /// Returns the runtime and its lifecycle as two halves: the frontend
    /// builds the app from the first and keeps the second to stop the worker
    /// threads with. See [`RuntimeLifecycle::shutdown`].
    // One linear wiring sequence, and the order is load-bearing: every
    // handle's clones — including each worker's stop flag — must be taken
    // before its consumer moves it. Splitting it into helpers would hide
    // that order behind call boundaries.
    #[allow(clippy::too_many_lines)]
    pub fn spawn(store_path: &Path) -> Result<(Self, RuntimeLifecycle), StoreError> {
        // The store: one shared connection behind an internal mutex serves
        // every store port; both session generations bump inside the store's
        // mutation impls, and the change channel feeds the event inbox.
        let (
            settings_store,
            playlist_store,
            library_mutation_store,
            library_query_store,
            generation,
            playlist_generation,
            changes_rx,
        ) = open_application_store(store_path)?;

        // The single shared event inbox: both transports — the UI's and the
        // tray's — are recording `ChannelTransport`s over the same command
        // channel and the same recorder closure, so dispatched commands land
        // on one observable surface. The store's `StoreChanged` stream is the
        // inbox's second input. Playback errors surface as typed notices
        // (issue 01 seam fix): the coordinator sends pre-formatted messages
        // over this channel and `BackendEvents` stamps them with playback
        // source + error severity.
        let backend_events = Arc::new(Mutex::new(BackendEvents::default()));
        let (notice_tx, notice_rx) = unbounded::<String>();
        {
            let mut f = backend_events.lock_or_recover();
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
        // Stop flags for the request-channel workers. Each one is raised by
        // `RuntimeLifecycle::shutdown`; the lifecycle keeps the original and
        // the worker gets the clone.
        let engine_stop = Arc::new(AtomicBool::new(false));
        let engine_stop_worker = Arc::clone(&engine_stop);
        let audio_thread = thread::spawn(move || {
            run_engine_thread(
                cmd_rx,
                engine_cmd_tx,
                update_tx,
                app_state,
                engine_queries,
                engine_stop_worker,
            );
        });

        // Playback Coordinator: applies Playback Updates to session state —
        // play history first, then **Continuation**'s answer — on its
        // dedicated thread. It needs
        // no stop flag — the engine thread's exit drops `update_tx` and
        // disconnects this loop.
        let coordinator = PlaybackCoordinator::spawn(
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
        // The lifecycle keeps its own clone: shutdown cancels the scan in
        // flight through this flag rather than calling back through the
        // `Scans` front end, which the UI holds by then.
        let scan_cancel = Arc::clone(&cancel_flag);
        let scanner = AudioFileScanner::new(cancel_flag.clone());
        // A settings clone bound into the walk so each scan reads the
        // Library Scan options fresh — the Settings pane's toggles and
        // format chips take effect on the next scan without a restart
        // (design-handoff issue 12). A read failure falls back to the
        // historical defaults rather than failing the scan.
        let scan_settings = settings_store.clone();
        let scan_stop = Arc::new(AtomicBool::new(false));
        let scan_stop_worker = Arc::clone(&scan_stop);
        let (scans, scan_worker) = ScanService::new(
            Box::new(LoftyMetadataReader::new()),
            Box::new(library_query_store.clone()),
            Box::new(library_mutation_store.clone()),
            cancel_flag,
            scan_stop_worker,
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
        let scan = thread::spawn(move || scan_worker.run());

        let (watcher_manager, fs_forwarder) = spawn_fs_watcher(scans.clone());

        // Background services (ADR 0006): real adapters, dedicated worker
        // threads — spawned here exactly like the Audio Engine.
        let tag_edit_stop = Arc::new(AtomicBool::new(false));
        let cover_stop = Arc::new(AtomicBool::new(false));
        let (tag_edits, covers, tag_edit, cover) = spawn_background_services(
            library_query_store.clone(),
            library_mutation_store.clone(),
            Arc::clone(&tag_edit_stop),
            Arc::clone(&cover_stop),
        );

        // The UI's `Box<dyn Transport>` and the tray's transport are
        // `ChannelTransport`s wired with the same shared recorder, so every
        // UI and tray intent is recorded synchronously onto the shared
        // event inbox before it is forwarded to the engine's command
        // channel.
        let recorder = {
            let backend_events = backend_events.clone();
            move |cmd: &riff_playback::domain::PlaybackCommand| {
                backend_events.lock_or_recover().record_command(cmd.clone());
            }
        };
        let ui_transport: Box<dyn Transport> = Box::new(ChannelTransport::new_recording(
            ui_cmd_tx,
            Box::new(recorder.clone()),
        ));
        let tray_transport = ChannelTransport::new_recording(tray_cmd_tx, Box::new(recorder));

        let quit_flag = Arc::new(AtomicBool::new(false));

        let runtime = Self {
            playback,
            library,
            backend_events,
            ui_transport,
            tray_transport,
            scans,
            watcher_manager: Arc::clone(&watcher_manager),
            quit_flag,
            settings: Box::new(settings_store),
            playlists: Box::new(playlist_store),
            library_mutations: Box::new(library_mutation_store),
            session_views,
            tag_edits: Box::new(tag_edits),
            covers: Box::new(covers),
        };

        let lifecycle = RuntimeLifecycle {
            audio: WorkerCell::new(Some(engine_stop), audio_thread),
            coordinator: WorkerCell::new(None, coordinator),
            scan: WorkerCell::new(Some(scan_stop), scan),
            tag_edit: WorkerCell::new(Some(tag_edit_stop), tag_edit),
            cover: WorkerCell::new(Some(cover_stop), cover),
            fs_forwarder: WorkerCell::new(None, fs_forwarder),
            scan_cancel,
            watcher_manager,
        };

        Ok((runtime, lifecycle))
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
/// forwards watch events. Returns the shared manager handle plus the
/// forwarder's join handle.
///
/// The forwarder has no stop flag by design: clearing the manager cell in
/// [`RuntimeLifecycle::shutdown`] drops the watcher, which drops
/// `fs_event_tx`, which makes the `recv()` below error out and end the loop.
fn spawn_fs_watcher(scans: ScanService) -> (Arc<Mutex<Option<WatcherManager>>>, JoinHandle<()>) {
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
    let handle = thread::spawn(move || {
        while let Ok(changed_paths) = fs_event_rx.recv() {
            if let Some(ref mut mgr) = *thread_manager.lock_or_recover() {
                mgr.on_fs_events(&changed_paths);
            }
        }
    });

    (watcher_manager, handle)
}

/// Composition-root wiring for the background services (ADR 0006): construct
/// the real Tag Edit and Cover service pairs over real adapters and run each
/// blocking worker on its dedicated thread — exactly like the Audio Engine.
/// Returns the front-end handles the UI holds boxed (`Box<dyn TagEdits>`,
/// `Box<dyn Covers>`) plus each worker's join handle, in that order.
fn spawn_background_services(
    library_queries: SqliteStore,
    library_mutations: SqliteStore,
    tag_edit_stop: Arc<AtomicBool>,
    cover_stop: Arc<AtomicBool>,
) -> (TagEditService, CoverService, JoinHandle<()>, JoinHandle<()>) {
    // The cover worker reads the artwork policy fresh per resolution so
    // the Settings pane's "Read embedded artwork" toggle applies
    // immediately (design-handoff issue 12); a read failure falls back to
    // the historical read-embedded default.
    let cover_settings = library_queries.clone();
    let (tag_edits, tag_worker) = TagEditService::new(
        Box::new(LoftyMetadataWriter::new()),
        Box::new(library_queries),
        Box::new(library_mutations),
        tag_edit_stop,
    );
    let tag_edit = thread::spawn(move || tag_worker.run());

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
        cover_stop,
    );
    let cover = thread::spawn(move || cover_worker.run());

    (tag_edits, covers, tag_edit, cover)
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
    stop: Arc<AtomicBool>,
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
        stop,
    );
    engine.run();
}
