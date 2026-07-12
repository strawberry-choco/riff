mod domain;
mod app;
mod infra;
mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use crossbeam_channel::unbounded;

use crate::app::commands::{LibraryCommand, LibraryUpdate};
use crate::app::state::AppState;
use crate::app::traits::{AudioDecoder, AudioOutput};
use crate::app::watcher_manager::WatcherManager;
use crate::infra::{SymphoniaDecoder, CpalAudioOutput, LoftyMetadataReader, AudioFileScanner, FilesystemWatcher};
use crate::ui::RiffApp;
use crate::domain::{PlaybackCommand, PlaybackUpdate, PlaybackState};

fn main() {
    tracing_subscriber::fmt::init();

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
    let _audio_thread = thread::spawn(move || {
        run_audio_engine(cmd_rx, update_tx, app_state, engine_cmd_tx);
    });

    // Spawn update processor thread
    let update_state = state.clone();
    let update_cmd_tx = cmd_tx.clone();
    let _update_thread = thread::spawn(move || {
        while let Ok(update) = update_rx.recv() {
            let mut state = update_state.lock().unwrap();
            match update {
                PlaybackUpdate::StateChanged(new_state) => {
                    state.playback_state = new_state;
                }
                PlaybackUpdate::PositionChanged(pos) => {
                    state.current_position = pos;
                }
                PlaybackUpdate::TrackChanged(track_id) => {
                    state.queue.current_index = state.queue.tracks.iter().position(|id| id == &track_id);
                }
                PlaybackUpdate::TrackEnded => {
                    // Auto-advance to next track
                    drop(state);
                    let next_track = {
                        let mut state = update_state.lock().unwrap();
                        state.queue.next().cloned()
                    };
                    if let Some(track_id) = next_track {
                        let _ = update_cmd_tx.send(PlaybackCommand::Play(track_id));
                    } else {
                        let mut state = update_state.lock().unwrap();
                        state.playback_state = PlaybackState::Stopped;
                    }
                }
                PlaybackUpdate::Error(msg) => {
                    tracing::error!("Playback error: {}", msg);
                    state.playback_state = PlaybackState::Stopped;
                    state.scan_status = Some(format!("Playback error: {}", msg));
                }
            }
        }
    });

    // Spawn library scan thread
    let scan_state = state.clone();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let _library_scan_thread = thread::spawn(move || {
        let reader = LoftyMetadataReader::new();
        while let Ok(cmd) = library_cmd_rx.recv() {
            match cmd {
                LibraryCommand::ScanDirectory(path) => {
                    cancel_flag.store(false, Ordering::Relaxed);
                    let scanner = AudioFileScanner::new(cancel_flag.clone());

                    match scanner.scan(&path) {
                        Ok(files) => {
                            let total = files.len();
                            let chunk_size = 10;

                            for (i, chunk) in files.chunks(chunk_size).enumerate() {
                                if cancel_flag.load(Ordering::Relaxed) {
                                    break;
                                }

                                let chunk_paths: Vec<_> = chunk.to_vec();
                                let processed = i * chunk_size + chunk.len();

                                {
                                    let mut state = scan_state.lock().unwrap();
                                    if let Err(e) = state.library.scan_and_add_tracks(
                                        chunk_paths,
                                        &reader,
                                    ) {
                                        let _ = library_update_tx.send(LibraryUpdate::ScanError {
                                            path: path.clone(),
                                            message: e.to_string(),
                                        });
                                    }
                                }

                                let _ = library_update_tx.send(LibraryUpdate::ScanProgress {
                                    path: path.clone(),
                                    files_found: processed.min(total),
                                    current_dir: path.to_string_lossy().to_string(),
                                });
                            }

                            let _ = library_update_tx.send(LibraryUpdate::ScanComplete {
                                path: path.clone(),
                                total_files: total,
                            });
                        }
                        Err(e) => {
                            let _ = library_update_tx.send(LibraryUpdate::ScanError {
                                path: path.clone(),
                                message: e.to_string(),
                            });
                        }
                    }
                }
                LibraryCommand::CancelScan => {
                    cancel_flag.store(true, Ordering::Relaxed);
                }
            }
        }
    });

    let (fs_event_tx, fs_event_rx) = unbounded::<std::path::PathBuf>();
    let watcher = match FilesystemWatcher::new(fs_event_tx) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!("Failed to create filesystem watcher: {}", e);
            None
        }
    };

    let watcher_manager = Arc::new(Mutex::new(Some(WatcherManager::new(
        watcher,
        library_cmd_tx.clone(),
    ))));

    let fs_watcher_manager = watcher_manager.clone();
    let _fs_event_thread = thread::spawn(move || {
        while let Ok(changed_path) = fs_event_rx.recv() {
            if let Some(ref mut mgr) = *fs_watcher_manager.lock().unwrap() {
                mgr.on_fs_event(&changed_path);
            }
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    #[cfg(not(target_os = "linux"))]
    let tray_icon = Arc::new(Mutex::new(
        match crate::ui::tray::create_tray(cmd_tx.clone()) {
            Ok(tray) => {
                tracing::info!("Tray icon created");
                Some(tray)
            }
            Err(e) => {
                tracing::warn!("Failed to create tray icon: {}", e);
                None
            }
        }
    ));

    #[cfg(not(target_os = "linux"))]
    let app = RiffApp::new(state.clone(), ui_cmd_tx, ui_library_cmd_tx, library_update_rx, watcher_manager, tray_icon);

    #[cfg(target_os = "linux")]
    let app = RiffApp::new(state.clone(), ui_cmd_tx, ui_library_cmd_tx, library_update_rx, watcher_manager);

    eframe::run_native("riff", options, Box::new(|_cc| Ok(Box::new(app))))
        .expect("Failed to run eframe");
}

fn run_audio_engine(
    cmd_rx: crossbeam_channel::Receiver<PlaybackCommand>,
    update_tx: crossbeam_channel::Sender<PlaybackUpdate>,
    state: Arc<Mutex<AppState>>,
    cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
) {
    let mut codec_registry = symphonia::core::codecs::CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut codec_registry);
    codec_registry.register_all::<symphonia_adapter_libopus::OpusDecoder>();
    let mut decoder = SymphoniaDecoder::new(codec_registry);
    let mut audio_output = CpalAudioOutput::new();
    let mut current_track_id: Option<crate::domain::TrackId> = None;
    let mut paused_position: Option<std::time::Duration> = None;

    fn handle_engine_cmd(
        cmd: PlaybackCommand,
        is_playing: &mut bool,
        should_stop_audio: &mut bool,
        audio_output: &mut CpalAudioOutput,
        decoder: &mut SymphoniaDecoder,
        update_tx: &crossbeam_channel::Sender<PlaybackUpdate>,
        cmd_tx: &crossbeam_channel::Sender<PlaybackCommand>,
        state: &Arc<Mutex<AppState>>,
    ) -> bool {
        match cmd {
            PlaybackCommand::Pause => {
                *is_playing = false;
                *should_stop_audio = false;
                let _ = update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
                true
            }
            PlaybackCommand::Stop => {
                audio_output.clear_buffer();
                let _ = audio_output.stop();
                let _ = update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
                *should_stop_audio = false;
                *is_playing = false;
                true
            }
            PlaybackCommand::Seek(pos) => {
                let _ = decoder.seek(pos);
                audio_output.clear_buffer();
                false
            }
            PlaybackCommand::SetVolume(vol) => {
                audio_output.set_volume(vol);
                false
            }
            PlaybackCommand::Play(_)
            | PlaybackCommand::Next
            | PlaybackCommand::Previous => {
                let _ = cmd_tx.send(cmd);
                *should_stop_audio = true;
                *is_playing = false;
                true
            }
            PlaybackCommand::PlayNext(track_id) => {
                if let Ok(mut s) = state.lock() {
                    s.queue.insert_next(track_id);
                }
                false
            }
            PlaybackCommand::AddToQueue(track_id) => {
                if let Ok(mut s) = state.lock() {
                    s.queue.append(track_id);
                }
                false
            }
            _ => false,
        }
    }

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            PlaybackCommand::Play(track_id) => {
                audio_output.clear_buffer();
                let _ = audio_output.stop();
                let is_resuming = current_track_id.as_ref() == Some(&track_id) && paused_position.is_some();
                if !is_resuming {
                    paused_position = None;
                }
                current_track_id = Some(track_id.clone());
                
                let path = {
                    let mut state = state.lock().unwrap();
                    // When playing a track from the library with an empty queue,
                    // populate the queue so that Next/Previous/auto-advance work.
                    if state.queue.tracks.is_empty() {
                        let all_ids: Vec<crate::domain::TrackId> = state
                            .library
                            .all_tracks()
                            .iter()
                            .map(|t| t.id.clone())
                            .collect();
                        if !all_ids.is_empty() {
                            state.queue.tracks = all_ids;
                            state.queue.current_index = state
                                .queue
                                .tracks
                                .iter()
                                .position(|id| id == &track_id);
                            // Reset shuffle state since the queue has been replaced
                            state.queue.shuffle = false;
                            state.queue.shuffled_indices.clear();
                            state.queue.shuffle_history.clear();
                        }
                    }
                    state.library.get_track(&track_id).map(|t| t.file_path.clone())
                };

                if let Some(path) = path {
                    match decoder.open(&path) {
                        Ok(info) => {
                            if is_resuming {
                                if let Some(pos) = paused_position.take() {
                                    let _ = decoder.seek(pos);
                                }
                            }
                            let _ = update_tx.send(PlaybackUpdate::TrackChanged(track_id));
                            
                            if let Err(e) = audio_output.initialize(info.sample_rate, info.channels) {
                                let _ = update_tx.send(PlaybackUpdate::Error(e.to_string()));
                                continue;
                            }
                            if let Err(e) = audio_output.start() {
                                let _ = update_tx.send(PlaybackUpdate::Error(e.to_string()));
                                continue;
                            }

                            audio_output.clear_buffer();

                            let mut is_playing = true;
                            let mut should_stop_audio = true;
                            let _ = update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
                            let start_time = std::time::Instant::now();
                            let max_buffer_samples = (info.sample_rate as usize) * (info.channels as usize) * 2;

                            loop {
                                if !is_playing {
                                    break;
                                }

                                // Backpressure: don't decode when the buffer is already full
                                while audio_output.buffer_len() >= max_buffer_samples {
                                    if let Ok(cmd) = cmd_rx.try_recv() {
                                        if matches!(cmd, PlaybackCommand::Pause) {
                                            paused_position = Some(start_time.elapsed());
                                        }
                                        if handle_engine_cmd(cmd, &mut is_playing, &mut should_stop_audio, &mut audio_output, &mut decoder, &update_tx, &cmd_tx, &state) {
                                            break;
                                        }
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }

                                if !is_playing {
                                    break;
                                }

                                match decoder.next_frames(4096) {
                                    Ok(Some(samples)) => {
                                        if let Err(e) = audio_output.write_samples(&samples) {
                                            let _ = update_tx.send(PlaybackUpdate::Error(e.to_string()));
                                            break;
                                        }

                                        let elapsed = start_time.elapsed();
                                        let _ = update_tx.send(PlaybackUpdate::PositionChanged(
                                            crate::domain::PlaybackPosition {
                                                current: elapsed,
                                                total: info.duration,
                                            }
                                        ));
                                    }
                                    Ok(None) => {
                                        let _ = update_tx.send(PlaybackUpdate::TrackEnded);
                                        // Wait for the remaining buffer to drain before stopping
                                        while audio_output.buffer_len() > 0 {
                                            if let Ok(cmd) = cmd_rx.try_recv() {
                                                match cmd {
                                                    PlaybackCommand::Stop => {
                                                        audio_output.clear_buffer();
                                                        break;
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            std::thread::sleep(std::time::Duration::from_millis(50));
                                        }
                                        break;
                                    }
                                    Err(e) => {
                                        let _ = update_tx.send(PlaybackUpdate::Error(e.to_string()));
                                        break;
                                    }
                                }

                                if let Ok(cmd) = cmd_rx.try_recv() {
                                    if matches!(cmd, PlaybackCommand::Pause) {
                                        paused_position = Some(start_time.elapsed());
                                    }
                                    if handle_engine_cmd(cmd, &mut is_playing, &mut should_stop_audio, &mut audio_output, &mut decoder, &update_tx, &cmd_tx, &state) {
                                        break;
                                    }
                                }
                            }

                            if should_stop_audio {
                                let _ = audio_output.stop();
                            }
                        }
                        Err(e) => {
                            let _ = update_tx.send(PlaybackUpdate::Error(e.to_string()));
                        }
                    }
                }
            }
            PlaybackCommand::Pause => {
                let _ = update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
            }
            PlaybackCommand::Resume => {
                if let Some(track_id) = current_track_id.clone() {
                    let _ = cmd_rx.try_recv(); // clear any pending
                    let _ = cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::Stop => {
                paused_position = None;
                let _ = audio_output.stop();
                let _ = update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
            }
            PlaybackCommand::Seek(pos) => {
                let _ = decoder.seek(pos);
            }
            PlaybackCommand::SetVolume(vol) => {
                audio_output.set_volume(vol);
            }
            PlaybackCommand::Next => {
                let next_track = {
                    let mut state = state.lock().unwrap();
                    state.queue.next().cloned()
                };
                if let Some(track_id) = next_track {
                    current_track_id = Some(track_id.clone());
                    let _ = audio_output.stop();
                    let _ = cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::Previous => {
                let prev_track = {
                    let mut state = state.lock().unwrap();
                    state.queue.previous().cloned()
                };
                if let Some(track_id) = prev_track {
                    current_track_id = Some(track_id.clone());
                    let _ = audio_output.stop();
                    let _ = cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::ToggleVisibility => {}
            PlaybackCommand::PlayPause => {
                let current_state = {
                    let state = state.lock().unwrap();
                    state.playback_state
                };
                match current_state {
                    PlaybackState::Playing => {
                        let _ = cmd_tx.send(PlaybackCommand::Pause);
                    }
                    _ => {
                        let _ = cmd_tx.send(PlaybackCommand::Resume);
                    }
                }
            }
            PlaybackCommand::PlayNext(track_id) => {
                {
                    let mut state = state.lock().unwrap();
                    state.queue.insert_next(track_id.clone());
                }
                if current_track_id.is_none() {
                    let _ = cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::AddToQueue(track_id) => {
                {
                    let mut state = state.lock().unwrap();
                    state.queue.append(track_id.clone());
                }
                if current_track_id.is_none() {
                    let _ = cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
        }
    }
}
