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

use crate::app::commands::{LibraryCommand, LibraryUpdate};
use crate::app::errors::AppError;
use crate::app::gapless::{
    elapsed_from_samples, formats_gapless_compatible, is_gapless_eligible, pre_buffer_cap,
    repeat_one_handoff_eligible, samples_from_duration, GaplessConditions, QueueConditions,
};
use crate::app::scan::build_tracks;
use crate::app::state::AppState;
use crate::app::store::{LibraryMutationStore, LibraryQueryStore};
use crate::app::traits::{AudioDecoder, AudioFormatInfo, AudioOutput};
use crate::app::watcher_manager::WatcherManager;
use crate::app::MutexExt;
use crate::domain::{PlaybackCommand, PlaybackState, PlaybackUpdate, RepeatMode, TrackId};
use crate::infra::{
    AudioFileScanner, CpalAudioOutput, FilesystemWatcher, LoftyMetadataReader, SymphoniaDecoder,
};
use crate::ui::RiffApp;
use std::path::PathBuf;
use std::time::Duration;

/// Gapless (Task 4.1): how many seconds before EOF the engine starts
/// pre-decoding the successor track.
const PRE_ENCODE_SECONDS: f32 = 2.0;
/// Gapless (Task 4.1): max seconds of successor audio held in the pre-buffer
/// (~1.5 MB at 48 kHz stereo). Exactly one successor is buffered, so total
/// extra memory is bounded independently of queue length.
const PRE_BUFFER_SECONDS: f32 = 4.0;

fn main() {
    tracing_subscriber::fmt::init();

    // Open the Application Store before anything else. Open or migration
    // failures are fatal startup errors with a clear message — never silent
    // fallbacks to empty state.
    let store_path = riff::infra::store::default_store_path().expect(
        "fatal: could not resolve the Application Store location \
         (no data-local directory available)",
    );
    // One shared connection behind a mutex serves every store port; settings
    // reads/writes go through the `SettingsStore` implementation below.
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
    let store_generation = riff::app::store::StoreGeneration::new();
    let library_mutation_store =
        riff::infra::store::MutexLibraryMutationStore::new(store.clone(), store_generation.clone());
    let library_query_store = riff::infra::store::MutexLibraryQueryStore::new(store.clone());

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
        run_audio_engine(cmd_rx, update_tx, app_state, engine_cmd_tx, engine_queries);
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
        Box::new(library_query_store),
        Box::new(library_mutation_store),
        store_generation,
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
        Box::new(library_query_store),
        Box::new(library_mutation_store),
        store_generation,
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

/// Build the shared codec registry (symphonia defaults + the Opus adapter).
/// Used for both the primary decoder and the gapless pre-decode decoder.
fn build_codec_registry() -> symphonia::core::codecs::registry::CodecRegistry {
    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

/// The audio engine thread: owns the decoders and audio output, processes
/// [`PlaybackCommand`]s, and drives decode/position/gapless logic.
struct AudioEngine {
    cmd_rx: crossbeam_channel::Receiver<PlaybackCommand>,
    update_tx: crossbeam_channel::Sender<PlaybackUpdate>,
    cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
    state: Arc<Mutex<AppState>>,
    /// Store query port for playback-time track resolution: playing a track
    /// resolves it by `TrackId` from the Application Store, never from an
    /// in-memory copy.
    library_queries: riff::infra::store::MutexLibraryQueryStore,
    decoder: SymphoniaDecoder,
    next_decoder: SymphoniaDecoder,
    audio_output: CpalAudioOutput,
    current_track_id: Option<TrackId>,
    paused_position: Option<Duration>,
    /// Set on a gapless handoff: the `TrackEnded` we emit makes the update
    /// processor re-send Play(current) for the track that is ALREADY playing;
    /// that duplicate must be swallowed or it would tear down the stream.
    gapless_dup_expected: bool,
    pre_decode: PreDecodeState,
}

/// Gapless pre-decode state (Task 4.1). Purely additive: when a valid,
/// format-compatible successor has been pre-buffered, EOF hands off without
/// stopping the cpal stream; in EVERY other case these fields are ignored and
/// the existing gapped path runs unchanged.
#[derive(Default)]
struct PreDecodeState {
    buffer: Vec<f32>,
    track_id: Option<TrackId>,
    track_path: Option<PathBuf>,
    format: Option<AudioFormatInfo>,
    /// Whether the (one-shot) pre-encode attempt has run for this track.
    attempted: bool,
}

impl PreDecodeState {
    fn reset(&mut self) {
        self.buffer.clear();
        self.track_id = None;
        self.track_path = None;
        self.format = None;
        self.attempted = false;
    }
}

/// Outcome of the end-of-track handling in the decode loop.
#[derive(PartialEq, Eq)]
enum EndOfTrack {
    /// Gapless handoff happened; continue the loop with the promoted track.
    HandedOff,
    /// Session ended (gapped path or mid-handoff write error).
    SessionEnded,
}

fn run_audio_engine(
    cmd_rx: crossbeam_channel::Receiver<PlaybackCommand>,
    update_tx: crossbeam_channel::Sender<PlaybackUpdate>,
    state: Arc<Mutex<AppState>>,
    cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
    library_queries: riff::infra::store::MutexLibraryQueryStore,
) {
    let engine = AudioEngine {
        cmd_rx,
        update_tx,
        cmd_tx,
        state,
        library_queries,
        decoder: SymphoniaDecoder::new(build_codec_registry()),
        next_decoder: SymphoniaDecoder::new(build_codec_registry()),
        audio_output: CpalAudioOutput::new(),
        current_track_id: None,
        paused_position: None,
        gapless_dup_expected: false,
        pre_decode: PreDecodeState::default(),
    };
    engine.run();
}

impl AudioEngine {
    fn run(mut self) {
        while let Ok(cmd) = self.cmd_rx.recv() {
            self.dispatch(cmd);
        }
    }

    fn dispatch(&mut self, cmd: PlaybackCommand) {
        match cmd {
            PlaybackCommand::Play(track_id) => self.handle_play(track_id),
            PlaybackCommand::Pause => {
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
            }
            PlaybackCommand::Resume => {
                if let Some(track_id) = self.current_track_id.clone() {
                    let _ = self.cmd_rx.try_recv(); // clear any pending
                    let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::Stop => {
                self.paused_position = None;
                self.gapless_dup_expected = false;
                let _ = self.audio_output.stop();
                self.decoder.close();
                // Discard any gapless pre-decode state (Task 4.1).
                self.pre_decode.reset();
                self.next_decoder.close();
                // Drop any stale per-track ReplayGain factor (Task 4.3).
                self.audio_output.set_replaygain(1.0);
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
            }
            PlaybackCommand::Seek(pos) => {
                let _ = self.decoder.seek(pos);
            }
            PlaybackCommand::SetVolume(vol) => {
                self.audio_output.set_volume(vol);
            }
            // User navigation invalidates any pending gapless Play dup.
            PlaybackCommand::Next => self.jump_queue(true),
            PlaybackCommand::Previous => self.jump_queue(false),
            PlaybackCommand::ToggleVisibility => {
                // Close-to-tray (REQ-SI-001): the tray thread cannot touch the
                // window, so flip the shared flag; the UI's per-frame `logic`
                // reconciles it with the real viewport visibility (Show Window
                // and the left-click toggle both route through here).
                let mut s = self.state.lock_or_recover();
                s.window_visible = !s.window_visible;
            }
            PlaybackCommand::PlayPause => {
                let current_state = {
                    let state = self.state.lock_or_recover();
                    state.playback_state
                };
                match current_state {
                    PlaybackState::Playing => {
                        let _ = self.cmd_tx.send(PlaybackCommand::Pause);
                    }
                    _ => {
                        let _ = self.cmd_tx.send(PlaybackCommand::Resume);
                    }
                }
            }
            PlaybackCommand::PlayNext(track_id) => {
                {
                    let mut state = self.state.lock_or_recover();
                    state.queue.insert_next(track_id.clone());
                }
                if self.current_track_id.is_none() {
                    let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::AddToQueue(track_id) => {
                {
                    let mut state = self.state.lock_or_recover();
                    state.queue.append(track_id.clone());
                }
                if self.current_track_id.is_none() {
                    let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
        }
    }

    /// Next/Previous navigation: move the queue and play the new current
    /// track.
    fn jump_queue(&mut self, advance: bool) {
        self.gapless_dup_expected = false;
        let target = {
            let mut state = self.state.lock_or_recover();
            if advance {
                state.queue.advance().cloned()
            } else {
                state.queue.previous().cloned()
            }
        };
        if let Some(track_id) = target {
            self.current_track_id = Some(track_id.clone());
            let _ = self.audio_output.stop();
            let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
        }
    }

    fn handle_play(&mut self, track_id: TrackId) {
        // Gapless dedup (Task 4.1): after a gapless handoff the update
        // processor's TrackEnded handling re-sends Play() for the track that
        // is ALREADY playing. Swallow exactly that duplicate so it cannot
        // tear down the live stream; any other Play clears the expectation
        // and runs normally.
        if self.gapless_dup_expected && self.current_track_id.as_ref() == Some(&track_id) {
            self.gapless_dup_expected = false;
            tracing::debug!("Gapless: ignored auto-Play for the already-playing track");
            return;
        }
        self.gapless_dup_expected = false;

        // Reset pre-decode state left over from a previous session.
        self.pre_decode.reset();

        self.audio_output.clear_buffer();
        let _ = self.audio_output.stop();
        let is_resuming =
            self.current_track_id.as_ref() == Some(&track_id) && self.paused_position.is_some();
        if !is_resuming {
            self.paused_position = None;
        }
        self.current_track_id = Some(track_id.clone());

        let Some((path, replaygain)) = self.resolve_track(&track_id) else {
            return;
        };

        // Apply the per-track ReplayGain factor before samples flow.
        self.audio_output.set_replaygain(replaygain);
        let mut info = match self.decoder.open(&path) {
            Ok(info) => info,
            Err(e) => {
                let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                return;
            }
        };
        if is_resuming {
            if let Some(pos) = self.paused_position.take() {
                let _ = self.decoder.seek(pos);
            }
        }
        let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(track_id));

        if let Err(e) = self.start_output(info.sample_rate, info.channels) {
            let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
            return;
        }

        self.audio_output.clear_buffer();
        let _ = self
            .update_tx
            .send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
        let should_stop_audio = self.run_decode_loop(&mut info);

        if should_stop_audio {
            let _ = self.audio_output.stop();
        }
    }

    /// Initialize and start the audio output for the given format.
    fn start_output(&mut self, sample_rate: u32, channels: u16) -> Result<(), AppError> {
        self.audio_output.initialize(sample_rate, channels)?;
        self.audio_output.start()?;
        Ok(())
    }

    /// Look up the file path and `ReplayGain` factor for a track. When
    /// playing from the library with an empty queue, populate the queue so
    /// that Next/Previous/auto-advance work.
    fn resolve_track(&mut self, track_id: &TrackId) -> Option<(PathBuf, f32)> {
        let mut state = self.state.lock_or_recover();
        if state.queue.tracks.is_empty() {
            // Queue Fill (glossary): the whole Library loads into the empty
            // Playback Queue in canonical flat ordering (path ascending,
            // ADR 0003 — deliberately replacing the mirror's HashMap-luck
            // order), the requested TrackId becomes current, and shuffle
            // resets. The data source is the Application Store; extraction
            // of this use case above the engine seam is Part 2 step 2.
            match self.library_queries.all_track_ids() {
                Ok(all_ids) if !all_ids.is_empty() => {
                    state.queue.tracks = all_ids;
                    state.queue.current_index =
                        state.queue.tracks.iter().position(|id| id == track_id);
                    // Reset shuffle state since the queue was replaced
                    state.queue.shuffle = false;
                    state.queue.shuffled_indices.clear();
                    state.queue.shuffle_history.clear();
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("Failed to fill the queue from the store: {e}");
                }
            }
        }
        // Playback resolves the track by TrackId from the Application Store.
        let stored = match self.library_queries.get_track(track_id) {
            Ok(track) => track,
            Err(e) => {
                tracing::error!("Failed to resolve track {track_id:?} from the store: {e}");
                None
            }
        };
        // ReplayGain (Task 4.3): compute the peak-capped factor from the
        // track's tags and the user preference. Untagged tracks (or the
        // disabled setting) yield 1.0 — no adjustment.
        let (gain_db, peak) = stored.as_ref().map_or((None, None), |t| {
            (
                t.metadata.replaygain_track_gain,
                t.metadata.replaygain_track_peak,
            )
        });
        let factor = crate::app::state::replaygain_factor(state.replaygain_enabled, gain_db, peak);
        stored.map(|t| (t.file_path.clone(), factor))
    }

    /// The decode loop for the currently open track: decode + write with
    /// backpressure, position updates, gapless handoff at EOF, pre-decode,
    /// and inline command polling. Returns `true` when the audio output
    /// should be stopped afterwards (every break except Pause/Stop, which
    /// manage the stream themselves).
    fn run_decode_loop(&mut self, info: &mut AudioFormatInfo) -> bool {
        // Sample-exact elapsed tracking (Task 4.1): every decoded chunk adds
        // its length; elapsed = samples / (rate * ch). Replaces the old
        // wall-clock timing and resets to 0 at each gapless handoff.
        let mut accumulated_samples: usize = 0;
        let mut is_playing = true;
        let mut should_stop_audio = true;
        let mut max_buffer_samples = (info.sample_rate as usize) * usize::from(info.channels) * 2;

        loop {
            if !is_playing {
                break;
            }

            // Backpressure: don't decode when the buffer is already full
            self.wait_for_buffer_space(
                max_buffer_samples,
                &mut is_playing,
                &mut should_stop_audio,
                &mut accumulated_samples,
                info,
            );

            if !is_playing {
                break;
            }

            match self.decoder.next_frames(4096) {
                Ok(Some(samples)) => {
                    if let Err(e) = self.audio_output.write_samples(&samples) {
                        let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                        break;
                    }
                    accumulated_samples += samples.len();
                    let elapsed =
                        elapsed_from_samples(accumulated_samples, info.sample_rate, info.channels);
                    let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(
                        crate::domain::PlaybackPosition {
                            current: elapsed,
                            total: info.duration,
                        },
                    ));
                }
                Ok(None) => {
                    // Final position at the track's true total.
                    if let Some(dur) = info.duration {
                        let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(
                            crate::domain::PlaybackPosition {
                                current: dur,
                                total: Some(dur),
                            },
                        ));
                    }

                    if self.end_of_track(info, &mut accumulated_samples, &mut max_buffer_samples)
                        == EndOfTrack::HandedOff
                    {
                        continue;
                    }
                    break;
                }
                Err(e) => {
                    let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                    break;
                }
            }

            self.maybe_pre_encode(info, accumulated_samples);

            if let Ok(cmd) = self.cmd_rx.try_recv() {
                if self.handle_loop_command(
                    cmd,
                    &mut is_playing,
                    &mut should_stop_audio,
                    &mut accumulated_samples,
                    info,
                ) {
                    break;
                }
            }
        }

        // The decode loop ended for a reason other than a gapless handoff
        // (Pause, Stop, error, or the gapped EOF path): discard any leftover
        // pre-decode state so nothing leaks into the next session. Pause and
        // the gapped path already clear it; this covers Stop and decode-error
        // breaks that bypass those sites.
        self.pre_decode.reset();
        self.next_decoder.close();

        should_stop_audio
    }

    /// Block until the output buffer has space, processing any commands that
    /// arrive while waiting.
    fn wait_for_buffer_space(
        &mut self,
        max_buffer_samples: usize,
        is_playing: &mut bool,
        should_stop_audio: &mut bool,
        accumulated_samples: &mut usize,
        info: &AudioFormatInfo,
    ) {
        while self.audio_output.buffer_len() >= max_buffer_samples {
            if let Ok(cmd) = self.cmd_rx.try_recv() {
                if self.handle_loop_command(
                    cmd,
                    is_playing,
                    should_stop_audio,
                    accumulated_samples,
                    info,
                ) {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Process one command received mid-session (during the backpressure wait
    /// or the per-chunk poll). Handles the gapless Play-dedup, records the
    /// pause position, and rebases elapsed tracking after seeks. Returns
    /// `true` when the decode loop should end.
    fn handle_loop_command(
        &mut self,
        cmd: PlaybackCommand,
        is_playing: &mut bool,
        should_stop_audio: &mut bool,
        accumulated_samples: &mut usize,
        info: &AudioFormatInfo,
    ) -> bool {
        // Gapless dedup (Task 4.1): swallow the post-handoff auto-Play
        // instead of tearing down the stream.
        if self.gapless_dup_expected
            && matches!(&cmd, PlaybackCommand::Play(id)
                if self.current_track_id.as_ref() == Some(id))
        {
            self.gapless_dup_expected = false;
            return false;
        }
        if matches!(cmd, PlaybackCommand::Pause) {
            self.paused_position = Some(elapsed_from_samples(
                *accumulated_samples,
                info.sample_rate,
                info.channels,
            ));
            // Discard the pre-buffer; resume re-does pre-encode
            // (safe/simple).
            self.pre_decode.reset();
        }
        if let PlaybackCommand::Seek(pos) = &cmd {
            // Elapsed continues from the seek target (exact integer math).
            *accumulated_samples = samples_from_duration(*pos, info.sample_rate, info.channels);
        }
        self.apply_command(cmd, is_playing, should_stop_audio)
    }

    /// Apply one command to the output/decoder/queue. Returns `true` when the
    /// command ends the current decode session.
    fn apply_command(
        &mut self,
        cmd: PlaybackCommand,
        is_playing: &mut bool,
        should_stop_audio: &mut bool,
    ) -> bool {
        match cmd {
            PlaybackCommand::Pause => {
                *is_playing = false;
                *should_stop_audio = false;
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
                true
            }
            PlaybackCommand::Stop => {
                self.audio_output.clear_buffer();
                let _ = self.audio_output.stop();
                // Drop any stale per-track ReplayGain factor (Task 4.3).
                self.audio_output.set_replaygain(1.0);
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
                *should_stop_audio = false;
                *is_playing = false;
                true
            }
            PlaybackCommand::Seek(pos) => {
                let _ = self.decoder.seek(pos);
                self.audio_output.clear_buffer();
                false
            }
            PlaybackCommand::SetVolume(vol) => {
                self.audio_output.set_volume(vol);
                false
            }
            PlaybackCommand::Play(_) | PlaybackCommand::Next | PlaybackCommand::Previous => {
                let _ = self.cmd_tx.send(cmd);
                *should_stop_audio = true;
                *is_playing = false;
                true
            }
            PlaybackCommand::PlayNext(track_id) => {
                let mut s = self.state.lock_or_recover();
                s.queue.insert_next(track_id);
                false
            }
            PlaybackCommand::AddToQueue(track_id) => {
                let mut s = self.state.lock_or_recover();
                s.queue.append(track_id);
                false
            }
            _ => false,
        }
    }

    /// Gapless pre-decode (Task 4.1): once playback nears EOF, opportunistically
    /// decode up to `PRE_BUFFER_SECONDS` of the natural successor on this same
    /// thread (symphonia decodes faster than real time, so the ring buffer
    /// cannot starve). Best-effort: any failure leaves `pre_decode.track_id`
    /// unset and the gapped path runs unchanged at EOF.
    fn maybe_pre_encode(&mut self, info: &AudioFormatInfo, accumulated_samples: usize) {
        let ready = !self.pre_decode.attempted
            && self.pre_decode.track_id.is_none()
            && info.duration.is_some_and(|d| {
                elapsed_from_samples(accumulated_samples, info.sample_rate, info.channels)
                    >= d.saturating_sub(Duration::from_secs_f32(PRE_ENCODE_SECONDS))
            });
        if !ready {
            return;
        }
        self.pre_decode.attempted = true;

        let successor = {
            let s = self.state.lock_or_recover();
            let next_id = if s.queue.repeat == RepeatMode::One {
                // Repeat-one loops the SAME track gaplessly.
                s.queue.current_track().cloned()
            } else {
                s.queue.upcoming(1).into_iter().next().cloned()
            };
            // The successor's path resolves from the Application Store.
            next_id.and_then(|id| match self.library_queries.get_track(&id) {
                Ok(Some(t)) => Some((id, t.file_path)),
                Ok(None) => None,
                Err(e) => {
                    tracing::error!("Failed to resolve successor {id:?}: {e}");
                    None
                }
            })
        };
        let Some((next_id, path)) = successor else {
            return;
        };

        match self.next_decoder.open(&path) {
            Ok(fmt) => {
                let cap = pre_buffer_cap(fmt.sample_rate, fmt.channels, PRE_BUFFER_SECONDS);
                let mut failed = false;
                while self.pre_decode.buffer.len() < cap {
                    match self.next_decoder.next_frames(4096) {
                        Ok(Some(samples)) => {
                            self.pre_decode.buffer.extend_from_slice(&samples);
                        }
                        // Short track: fully buffered already.
                        Ok(None) => break,
                        Err(e) => {
                            tracing::warn!("Gapless pre-decode error: {}", e);
                            failed = true;
                            break;
                        }
                    }
                }
                if failed || self.pre_decode.buffer.is_empty() {
                    self.pre_decode.buffer.clear();
                    self.next_decoder.close();
                } else {
                    tracing::info!(
                        "Gapless: pre-buffered successor {:?} ({} samples)",
                        path,
                        self.pre_decode.buffer.len()
                    );
                    self.pre_decode.format = Some(fmt);
                    self.pre_decode.track_id = Some(next_id);
                    self.pre_decode.track_path = Some(path);
                }
            }
            Err(e) => {
                tracing::warn!("Gapless pre-decode open failed: {}", e);
                self.next_decoder.close();
            }
        }
    }

    /// End-of-track handling (Task 4.1): attempt the gapless handoff when a
    /// valid, format-compatible, pre-buffered natural successor exists;
    /// otherwise run the gapped path (`TrackEnded` → drain buffer → stop).
    /// Purely additive: any ineligible case falls through unchanged.
    fn end_of_track(
        &mut self,
        info: &mut AudioFormatInfo,
        accumulated_samples: &mut usize,
        max_buffer_samples: &mut usize,
    ) -> EndOfTrack {
        let promoted = self
            .pre_decode
            .track_id
            .take()
            .zip(self.pre_decode.format.take())
            .map(|(id, fmt)| (id, fmt, self.pre_decode.track_path.take()));

        if let Some((promoted_id, promoted_format, promoted_path)) = promoted {
            let eligible = {
                let s = self.state.lock_or_recover();
                let repeat_one = s.queue.repeat == RepeatMode::One;
                let natural = if repeat_one {
                    s.queue.current_track().cloned()
                } else {
                    s.queue.upcoming(1).into_iter().next().cloned()
                };
                let has_successor =
                    natural.as_ref() == Some(&promoted_id) && !self.pre_decode.buffer.is_empty();
                let compatible = formats_gapless_compatible(
                    self.audio_output.effective_sample_rate(),
                    info.channels,
                    promoted_format.sample_rate,
                    promoted_format.channels,
                );
                is_gapless_eligible(GaplessConditions {
                    queue: QueueConditions {
                        shuffle: s.queue.shuffle,
                        repeat_one,
                    },
                    formats_compatible: compatible,
                    has_successor,
                }) || repeat_one_handoff_eligible(
                    s.queue.shuffle,
                    repeat_one,
                    compatible,
                    has_successor,
                )
            };

            if eligible {
                // Expect (and swallow) the update processor's automatic
                // Play() for the promoted track.
                self.gapless_dup_expected = true;
                // TrackEnded keeps play-count and auto-advance bookkeeping
                // intact; the resulting Play dup is neutralized by the dedup
                // guard.
                let _ = self.update_tx.send(PlaybackUpdate::TrackEnded);
                // 1. Flush the successor's pre-buffer. The cpal stream is
                //    NEVER stopped/re-initialized across this boundary; the
                //    ring buffer is NOT cleared.
                if let Err(e) = self.audio_output.write_samples(&self.pre_decode.buffer) {
                    let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                    return EndOfTrack::SessionEnded;
                }
                // 2. Promote the pre-decode decoder to primary and retire the
                //    old primary.
                std::mem::swap(&mut self.decoder, &mut self.next_decoder);
                self.next_decoder.close();
                // 3. Position tracking restarts for the new track.
                *accumulated_samples = 0;
                self.pre_decode.buffer.clear();
                self.pre_decode.attempted = false;
                // ReplayGain (Task 4.3): apply the promoted track's factor at
                // the boundary; its tags resolve from the Application Store.
                let rg = {
                    let s = self.state.lock_or_recover();
                    let stored = self.library_queries.get_track(&promoted_id).ok().flatten();
                    let (g, p) = stored.map_or((None, None), |t| {
                        (
                            t.metadata.replaygain_track_gain,
                            t.metadata.replaygain_track_peak,
                        )
                    });
                    crate::app::state::replaygain_factor(s.replaygain_enabled, g, p)
                };
                self.audio_output.set_replaygain(rg);
                // 4. Announce the new track at the sample boundary.
                self.current_track_id = Some(promoted_id.clone());
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::TrackChanged(promoted_id.clone()));
                tracing::info!("Gapless handoff to {:?} ({:?})", promoted_id, promoted_path);
                // 5. Continue the decode loop with the new track.
                *info = promoted_format;
                *max_buffer_samples = (info.sample_rate as usize) * usize::from(info.channels) * 2;
                return EndOfTrack::HandedOff;
            }

            // Successor was pre-buffered but the handoff is ineligible (format
            // mismatch, shuffle/jump, or pre-buffer issues). The gapped path
            // below runs unchanged and discards the pre-buffer.
            tracing::info!(
                "Gapless: skipping handoff to {:?} (ineligible); gapped path",
                promoted_id
            );
        }

        // --- Existing gapped path, unchanged ---
        let _ = self.update_tx.send(PlaybackUpdate::TrackEnded);
        // Wait for the remaining buffer to drain before stopping
        while self.audio_output.buffer_len() > 0 {
            if let Ok(PlaybackCommand::Stop) = self.cmd_rx.try_recv() {
                self.audio_output.clear_buffer();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // Discard any unused pre-buffer (format mismatch, stale successor, or
        // none); the next Play re-arms pre-decode.
        self.pre_decode.reset();
        self.next_decoder.close();
        EndOfTrack::SessionEnded
    }
}
