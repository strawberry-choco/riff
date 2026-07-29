use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use crossbeam_channel::{Sender, Receiver, unbounded};
use crate::app::commands::{LibraryCommand, LibraryUpdate};
use crate::app::cover_resolver::CoverResolver;
use crate::app::MutexExt;
use elegance::Theme as EleganceTheme;
use crate::app::state::{AppState, ViewMode, LibraryStatus, BrowseMode};
use crate::app::watcher_manager::WatcherManager;
use crate::domain::{PlaybackCommand, PlaybackState, RepeatMode, TrackId, Track};
use crate::infra::{LoftyMetadataReader, ImageCoverLoader};

pub struct RiffApp {
    pub state: Arc<Mutex<AppState>>,
    command_sender: Option<Sender<PlaybackCommand>>,
    library_command_sender: Option<Sender<LibraryCommand>>,
    library_update_rx: Option<Receiver<LibraryUpdate>>,
    cover_resolver: Arc<Mutex<CoverResolver>>,
    cover_textures: std::collections::HashMap<String, egui::TextureHandle>,
    cover_lru_keys: Vec<String>,
    cover_request_tx: Option<Sender<(TrackId, PathBuf)>>,
    cover_response_rx: Receiver<(String, Option<crate::app::traits::CoverImage>)>,
    search_focus: bool,
    pub(crate) settings_text_input: String,
    pub(crate) settings_show_input: bool,
    pub(crate) settings_path_error: Option<String>,
    first_frame: bool,
    pub(crate) watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
    elegance_theme: bool, // true = dark (slate), false = light (frost)
    #[cfg(not(target_os = "linux"))]
    tray_icon: Arc<Mutex<Option<tray_icon::TrayIcon>>>,
    quit_flag: Arc<AtomicBool>,
}

impl RiffApp {
    pub fn new(
        state: Arc<Mutex<AppState>>,
        command_sender: Sender<PlaybackCommand>,
        library_command_sender: Sender<LibraryCommand>,
        library_update_rx: Receiver<LibraryUpdate>,
        watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
        #[cfg(not(target_os = "linux"))]
        tray_icon: Arc<Mutex<Option<tray_icon::TrayIcon>>>,
        quit_flag: Arc<AtomicBool>,
    ) -> Self {
        let reader = Box::new(LoftyMetadataReader::new());
        let loader = Box::new(ImageCoverLoader::new());
        let resolver = CoverResolver::new(reader, loader);

        let (cover_tx, cover_rx_inner): (Sender<(TrackId, PathBuf)>, _) = unbounded();
        let (response_tx, response_rx): (Sender<(String, Option<crate::app::traits::CoverImage>)>, _) = unbounded();

        let resolver_arc = Arc::new(Mutex::new(resolver));
        let resolver_clone = resolver_arc.clone();

        std::thread::spawn(move || {
            while let Ok((track_id, path)) = cover_rx_inner.recv() {
                let result = match resolver_clone.lock_or_recover().resolve(&path) {
                    Ok(val) => val,
                    Err(e) => {
                        tracing::warn!("Cover resolution failed for {:?}: {}", path, e);
                        None
                    }
                };
                let _ = response_tx.send((track_id.0.clone(), result));
            }
        });

        Self {
            state,
            command_sender: Some(command_sender),
            library_command_sender: Some(library_command_sender),
            library_update_rx: Some(library_update_rx),
            cover_resolver: resolver_arc,
            cover_textures: std::collections::HashMap::new(),
            cover_lru_keys: Vec::new(),
            cover_request_tx: Some(cover_tx),
            cover_response_rx: response_rx,
            search_focus: false,
            settings_text_input: String::new(),
            settings_show_input: false,
            settings_path_error: None,
            first_frame: true,
            watcher_manager,
            elegance_theme: true, // dark (slate) by default
            #[cfg(not(target_os = "linux"))]
            tray_icon,
            quit_flag,
        }
    }

    fn request_cover(&self, track_id: &TrackId, file_path: &PathBuf) {
        if !self.cover_textures.contains_key(&track_id.0) {
            if let Some(ref tx) = self.cover_request_tx {
                let _ = tx.send((track_id.clone(), file_path.clone()));
            }
        }
    }

    fn poll_library_updates(&self, state: &mut AppState) {
        if let Some(ref rx) = self.library_update_rx {
            while let Ok(update) = rx.try_recv() {
                match update {
                    LibraryUpdate::ScanProgress { path, files_found, current_dir } => {
                        state.library_statuses.insert(path, LibraryStatus::Scanning { files_found });
                        state.scan_status = Some(format!("{} files, {}", files_found, current_dir));
                    }
                    LibraryUpdate::ScanComplete { path, total_files } => {
                        state.library_statuses.insert(path.clone(), LibraryStatus::Scanned(total_files));
                        state.scan_status = Some(format!("Scan complete: {} tracks", total_files));
                        state.library.save_cache();
                        if let Some(ref mut mgr) = *self.watcher_manager.lock_or_recover() {
                            mgr.mark_scan_complete(&path);
                        }
                    }
                    LibraryUpdate::ScanError { path, message } => {
                        state.library_statuses.insert(path, LibraryStatus::Idle);
                        state.scan_status = Some(format!("Error: {}", message));
                    }
                }
            }
        }
    }

    fn poll_watchers(&self) {
        if let Some(ref mut mgr) = *self.watcher_manager.lock_or_recover() {
            mgr.poll();
        }
    }

    fn update_cover_cache(&mut self, ctx: &egui::Context) {
        while let Ok((key, cover)) = self.cover_response_rx.try_recv() {
            if let Some(cover_image) = cover {
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [cover_image.width as usize, cover_image.height as usize],
                    &cover_image.rgba,
                );
                let texture = ctx.load_texture(
                    &key,
                    color_image,
                    egui::TextureOptions::default(),
                );
                self.cover_textures.insert(key.clone(), texture);
                // Dedupe: remove any existing instance to avoid duplicates.
                self.cover_lru_keys.retain(|k| k != &key);
                self.cover_lru_keys.push(key.clone());
                while self.cover_lru_keys.len() > 50 {
                    if let Some(oldest) = self.cover_lru_keys.first().cloned() {
                        self.cover_lru_keys.remove(0);
                        self.cover_textures.remove(&oldest);
                    }
                }
            }
        }
    }

    /// Get a cover texture, touching the LRU to mark it as recently used.
    fn get_cover_texture(&mut self, key: &str) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.cover_textures.get(key) {
            // Move to end (most recently used) and return a clone.
            self.cover_lru_keys.retain(|k| k != key);
            self.cover_lru_keys.push(key.to_string());
            Some(tex.clone())
        } else {
            None
        }
    }

}
impl eframe::App for RiffApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.elegance_theme {
            EleganceTheme::slate().install(ui.ctx());
        } else {
            EleganceTheme::frost().install(ui.ctx());
        }

        let cmd = self.command_sender.clone();
        let lib_cmd = self.library_command_sender.clone();
        let state_arc = self.state.clone();

        let mut state = state_arc.lock_or_recover();

        if self.quit_flag.load(Ordering::Relaxed) {
            state.library.save_cache();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if self.first_frame {
            state.library = crate::app::library_manager::LibraryManager::load_cache();
            let persisted_paths = crate::ui::settings::load_library_paths(frame.storage());
            if !persisted_paths.is_empty() {
                state.library_paths = persisted_paths.clone();
                for path in &persisted_paths {
                    let status = if path.exists() {
                        LibraryStatus::Idle
                    } else {
                        LibraryStatus::Unavailable
                    };
                    state.library_statuses.insert(path.clone(), status);
                }
            }

            if let Some(vol) = crate::ui::settings::load_volume(frame.storage()) {
                state.current_volume = vol;
                if let Some(ref s) = cmd {
                    let _ = s.send(PlaybackCommand::SetVolume(vol));
                }
            }

            state.watch_states = crate::ui::settings::load_watch_states(frame.storage());

            self.first_frame = false;
        }

        self.poll_library_updates(&mut state);
        self.update_cover_cache(ui.ctx());
        self.poll_watchers();

        // Keyboard shortcuts
        if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.search_focus = true;
        }
        if !ui.ctx().egui_wants_keyboard_input() {
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                let playing = state.playback_state == PlaybackState::Playing;
                if playing {
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Pause); }
                } else {
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Resume); }
                }
            }
        }

        // Update window title and tray tooltip
        if let Some(track_id) = state.queue.current_track() {
            if let Some(track) = state.library.get_track(track_id) {
                let title = format!(
                    "{} - {} \u{2014} riff",
                    track.metadata.display_artist(),
                    track.metadata.display_title(&track.file_path)
                );
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
                #[cfg(not(target_os = "linux"))]
                if let Ok(guard) = self.tray_icon.lock() {
                    if let Some(ref tray) = *guard {
                        crate::ui::tray::update_tooltip(tray, &title);
                    }
                }
            }
        } else {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title("riff".to_owned()));
            #[cfg(not(target_os = "linux"))]
            if let Ok(guard) = self.tray_icon.lock() {
                if let Some(ref tray) = *guard {
                    crate::ui::tray::update_tooltip(tray, "riff");
                }
            }
        }

        // --- TOP BAR ---
        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("riff");

                if let Some(ref status) = state.scan_status {
                    ui.label(status);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let theme_icon = if self.elegance_theme { "\u{2600}" } else { "\u{1F319}" };
                    if ui.button(theme_icon).clicked() {
                        self.elegance_theme = !self.elegance_theme;
                    }
                    if ui.button("\u{2699}").clicked() {
                        state.view_mode = ViewMode::Settings;
                    }
                    if ui.button("\u{1F3B5}").clicked() {
                        state.view_mode = match state.view_mode {
                            ViewMode::Library => ViewMode::NowPlaying,
                            ViewMode::NowPlaying => ViewMode::Library,
                            ViewMode::Settings => ViewMode::Library,
                        };
                    }
                });
            });
        });

        // --- CONTROL BAR ---
        egui::Panel::bottom("control_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let playing = state.playback_state == PlaybackState::Playing;
                let paused = state.playback_state == PlaybackState::Paused;

                if ui.button("\u{23EE}").clicked() {
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Previous); }
                }
                if ui.button("\u{23F9}").clicked() {
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Stop); }
                }
                if playing {
                    if ui.button("\u{23F8}").clicked() {
                        if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Pause); }
                    }
                } else if paused {
                    if ui.button("\u{25B6}").clicked() {
                        if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Resume); }
                    }
                } else if ui.button("\u{25B6}").clicked() {
                    if let Some(ref selected) = state.selected_track {
                        if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Play(selected.clone())); }
                    }
                }
                if ui.button("\u{23ED}").clicked() {
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Next); }
                }

                ui.separator();

                let progress = state.current_position.total
                    .map(|t| if t.as_secs() > 0 { state.current_position.current.as_secs_f32() / t.as_secs_f32() } else { 0.0 })
                    .unwrap_or(0.0);
                let current_str = format_duration(state.current_position.current);
                let total_str = state.current_position.total
                    .map(format_duration)
                    .unwrap_or_else(|| "--:--".to_string());
                ui.label(format!("{} / {}", current_str, total_str));

                let pr = ui.add(egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                    .show_percentage().desired_width(200.0));
                if pr.clicked() {
                    if let Some(total) = state.current_position.total {
                        if let Some(pos) = pr.interact_pointer_pos() {
                            let frac = ((pos.x - pr.rect.min.x) / pr.rect.width()).clamp(0.0, 1.0);
                            if let Some(ref s) = cmd {
                                let _ = s.send(PlaybackCommand::Seek(
                                    std::time::Duration::from_secs_f32(frac * total.as_secs_f32())
                                ));
                            }
                        }
                    }
                }

                ui.separator();
                ui.label("\u{1F50A}");
                let mut vol = state.current_volume;
                if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0)).changed() {
                    state.current_volume = vol;
                    if let Some(storage) = frame.storage_mut() {
                        crate::ui::settings::save_volume(storage, vol);
                    }
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::SetVolume(vol)); }
                }
                ui.separator();

                let cidx = state.queue.current_index.map(|i| i + 1).unwrap_or(0);
                ui.label(format!("{}/{}", cidx, state.queue.tracks.len()));

                let shuff = state.queue.shuffle;
                if ui.button(if shuff { "\u{1F500}" } else { "\u{27A1}\u{FE0F}" }).clicked() {
                    state.queue.set_shuffle(!shuff);
                }
                let rep = match state.queue.repeat {
                    RepeatMode::None => "\u{23F9}",
                    RepeatMode::All => "\u{1F501}",
                    RepeatMode::One => "\u{1F502}",
                };
                if ui.button(rep).clicked() {
                    state.queue.toggle_repeat();
                }
            });
        });

        // --- MAIN CONTENT ---
        match state.view_mode {
            ViewMode::Library => self.show_library_view(ui, &mut state, &cmd),
            ViewMode::NowPlaying => self.show_now_playing_view(ui, &mut state, &cmd),
            ViewMode::Settings => self.show_settings_view(ui, &mut state, &lib_cmd, frame),
        }

        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

// --- Helper methods factored out to avoid borrow conflicts ---
impl RiffApp {
    fn show_library_view(
        &mut self,
        parent_ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
    ) {
        egui::Panel::left("library_panel").show_inside(parent_ui, |ui| {
            ui.label("Library");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("\u{1F50D}");
                let sr = ui.text_edit_singleline(&mut state.search_query);
                if self.search_focus {
                    sr.request_focus();
                    self.search_focus = false;
                }
                if ui.button("\u{2715}").clicked() {
                    state.search_query.clear();
                }
            });
            ui.separator();

            // Library / Folders view toggle
            ui.horizontal(|ui| {
                if ui.selectable_label(state.browse_mode == BrowseMode::Library, "Library").clicked() {
                    state.browse_mode = BrowseMode::Library;
                }
                if ui.selectable_label(state.browse_mode == BrowseMode::Folders, "Folders").clicked() {
                    state.browse_mode = BrowseMode::Folders;
                }
            });
            ui.separator();

            let query = state.search_query.clone();

            match state.browse_mode {
                BrowseMode::Library => {
                    // Existing sub-toggle: All Tracks / Artists
                    ui.horizontal(|ui| {
                        if ui.selectable_label(!state.show_artists_view, "All Tracks").clicked() {
                            state.show_artists_view = false;
                        }
                        if ui.selectable_label(state.show_artists_view, "Artists").clicked() {
                            state.show_artists_view = true;
                        }
                    });
                    ui.separator();

                    let has_results = query.is_empty() || !state.library.search(&query).is_empty();

                    if !has_results && !query.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.label(format!("No tracks found matching '{}'", query));
                        });
                    } else if state.show_artists_view {
                        self.render_artist_view(ui, state, cmd, &query);
                    } else {
                        self.render_flat_view(ui, state, cmd, &query);
                    }
                }
                BrowseMode::Folders => {
                    self.render_folder_tree(ui, state, cmd, &query);
                }
            }
        });

        // Right side: track details + cover
        egui::CentralPanel::default().show_inside(parent_ui, |ui| {
            if let Some(track_id) = state.selected_track.clone() {
                if let Some(track) = state.library.get_track(&track_id) {
                    self.request_cover(&track.id, &track.file_path);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.heading(track.metadata.display_title(&track.file_path));
                            ui.label(format!("Artist: {}", track.metadata.display_artist()));
                            ui.label(format!("Album: {}", track.metadata.display_album()));
                            if let Some(ref aa) = track.metadata.album_artist {
                                if *aa != track.metadata.display_artist() {
                                    ui.label(format!("Album Artist: {}", aa));
                                }
                            }
                            if let Some(y) = track.metadata.year {
                                ui.label(format!("Year: {}", y));
                            }
                            if let Some(g) = &track.metadata.genre {
                                ui.label(format!("Genre: {}", g));
                            }
                            if let Some(tn) = track.metadata.track_number {
                                ui.label(format!("Track: {}", tn));
                            }
                            ui.separator();
                            let path_display = track.file_path.to_string_lossy().to_string();
                            ui.label(format!("File: {}", path_display));
                        });
                        ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                            if let Some(texture) = self.get_cover_texture(&track.id.0) {
                                let sized = egui::load::SizedTexture::new(texture.id(), egui::vec2(200.0, 200.0));
                                ui.add(egui::Image::from_texture(sized));
                            } else {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 200.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(40));
                                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(80.0);
                                        ui.label("\u{1F3B5}");
                                    });
                                });
                            }
                        });
                    });
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a track to view details");
                });
            }
        });
    }

    fn render_artist_view(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
        query: &str,
    ) {
        // Clone data to avoid holding borrows across closures
        let artists: Vec<_> = {
            let mut all: Vec<_> = state.library.all_artists().into_iter().cloned().collect();
            all.sort_by(|a, b| a.name.cmp(&b.name));
            if query.is_empty() {
                all
            } else {
                let q = query.to_lowercase();
                all.into_iter().filter(|a| a.name.to_lowercase().contains(&q)).collect()
            }
        };
        let current_track = state.queue.current_track().cloned();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for artist in &artists {
                let artist_has_current = artist.albums.iter().any(|key| {
                    state.library.albums.get(key).map_or(false, |album| {
                        album.tracks.iter().any(|tid| Some(tid) == current_track.as_ref())
                    })
                });
                egui::CollapsingHeader::new(&artist.name)
                    .default_open(artist_has_current)
                    .show(ui, |ui| {
                        let mut albums: Vec<_> = artist.albums.iter()
                            .filter_map(|key| state.library.albums.get(key))
                            .collect();
                        albums.sort_by(|a, b| {
                            b.year.unwrap_or(0).cmp(&a.year.unwrap_or(0))
                                .then_with(|| a.title.cmp(&b.title))
                        });

                        for album in albums {
                            let album_has_current = album.tracks.iter()
                                .any(|tid| Some(tid) == current_track.as_ref());
                            let year_str = album.year.map_or(String::new(), |y| format!(" ({})", y));
                            egui::CollapsingHeader::new(format!("{}{}", album.title, year_str))
                                .default_open(album_has_current)
                                .show(ui, |ui| {
                                    let mut track_ids = album.tracks.clone();
                                    track_ids.sort_by_key(|tid| {
                                        state.library.get_track(tid)
                                            .and_then(|t| t.metadata.track_number)
                                            .unwrap_or(0)
                                    });

                                    for tid in &track_ids {
                                        let is_selected = state.selected_track.as_ref() == Some(tid);
                                        let is_current = current_track.as_ref() == Some(tid);
                                        let track = match state.library.get_track(tid) {
                                            Some(t) => t.clone(),
                                            None => continue,
                                        };

                                        self.request_cover(&track.id, &track.file_path);

                                        ui.horizontal(|ui| {
                                            ui.set_min_height(20.0);
                                            if is_current {
                                                ui.label("\u{25B6}");
                                            }
                                            let display = format!(
                                                "{}. {}",
                                                track.metadata.track_number.unwrap_or(0),
                                                track.metadata.display_title(&track.file_path)
                                            );
                                            let resp = ui.selectable_label(is_selected, display);
                                            if resp.clicked() {
                                                state.selected_track = Some(track.id.clone());
                                            }
                                            if resp.double_clicked() {
                                                state.selected_track = Some(track.id.clone());
                                                if let Some(ref s) = cmd {
                                                    let _ = s.send(PlaybackCommand::Play(track.id.clone()));
                                                }
                                            }
                                            let cmd_ct = cmd.clone();
                                            resp.context_menu(move |ui| {
                                                if ui.button("Play").clicked() {
                                                    if let Some(ref s) = cmd_ct {
                                                        let _ = s.send(PlaybackCommand::Play(track.id.clone()));
                                                    }
                                                    ui.close();
                                                }
                                                if ui.button("Play Next").clicked() {
                                                    if let Some(ref s) = cmd_ct {
                                                        let _ = s.send(PlaybackCommand::PlayNext(track.id.clone()));
                                                    }
                                                    ui.close();
                                                }
                                                if ui.button("Add to Queue").clicked() {
                                                    if let Some(ref s) = cmd_ct {
                                                        let _ = s.send(PlaybackCommand::AddToQueue(track.id.clone()));
                                                    }
                                                    ui.close();
                                                }
                                            });
                                        });
                                    }
                                });
                        }
                    });
            }
        });
    }

    fn render_flat_view(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
        query: &str,
    ) {
        let tracks: Vec<Track> = if query.is_empty() {
            state.library.all_tracks().into_iter().cloned().collect()
        } else {
            state.library.search(query).into_iter().cloned().collect()
        };

        let current_track = state.queue.current_track().cloned();

        egui::ScrollArea::vertical().show_rows(ui, 22.0, tracks.len(), |ui, row_range| {
            for i in row_range {
                if let Some(track) = tracks.get(i) {
                    let is_selected = state.selected_track.as_ref() == Some(&track.id);
                    let is_playing = current_track.as_ref() == Some(&track.id);

                    self.request_cover(&track.id, &track.file_path);

                    ui.horizontal(|ui| {
                        ui.set_min_height(20.0);
                        if is_playing {
                            ui.label("\u{25B6}");
                        }
                        let label = format!(
                            "{} - {}",
                            track.metadata.display_artist(),
                            track.metadata.display_title(&track.file_path)
                        );
                        let response = ui.selectable_label(is_selected, label);
                        if response.clicked() {
                            state.selected_track = Some(track.id.clone());
                        }
                        if response.double_clicked() {
                            state.selected_track = Some(track.id.clone());
                            if let Some(ref s) = cmd {
                                let _ = s.send(PlaybackCommand::Play(track.id.clone()));
                            }
                        }
                        let cmd_ct = cmd.clone();
                        let tid_ct = track.id.clone();
                        response.context_menu(move |ui| {
                            if ui.button("Play").clicked() {
                                if let Some(ref s) = cmd_ct {
                                    let _ = s.send(PlaybackCommand::Play(tid_ct.clone()));
                                }
                                ui.close();
                            }
                            if ui.button("Play Next").clicked() {
                                if let Some(ref s) = cmd_ct {
                                    let _ = s.send(PlaybackCommand::PlayNext(tid_ct.clone()));
                                }
                                ui.close();
                            }
                            if ui.button("Add to Queue").clicked() {
                                if let Some(ref s) = cmd_ct {
                                    let _ = s.send(PlaybackCommand::AddToQueue(tid_ct.clone()));
                                }
                                ui.close();
                            }
                        });
                    });
                }
            }
        });
    }

    fn show_now_playing_view(
        &mut self,
        parent_ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
    ) {
        egui::CentralPanel::default().show_inside(parent_ui, |ui| {
            ui.vertical_centered(|ui| {
                if let Some(track_id) = state.queue.current_track().cloned() {
                    if let Some(track) = state.library.get_track(&track_id) {
                        self.request_cover(&track.id, &track.file_path);

                        if let Some(texture) = self.get_cover_texture(&track.id.0) {
                            let sized = egui::load::SizedTexture::new(texture.id(), egui::vec2(300.0, 300.0));
                            ui.add(egui::Image::from_texture(sized));
                        } else {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 300.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(40));
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(130.0);
                                    ui.label("\u{1F3B5}");
                                });
                            });
                        }

                        ui.add_space(10.0);
                        ui.heading(track.metadata.display_title(&track.file_path));
                        ui.label(format!("{} - {}",
                            track.metadata.display_artist(),
                            track.metadata.display_album()
                        ));
                        if let Some(ref aa) = track.metadata.album_artist {
                            if *aa != track.metadata.display_artist() {
                                ui.label(format!("Album Artist: {}", aa));
                            }
                        }
                        if let Some(y) = track.metadata.year {
                            ui.label(format!("Year: {}", y));
                        }
                        if let Some(g) = &track.metadata.genre {
                            ui.label(format!("Genre: {}", g));
                        }
                        if let Some(tn) = track.metadata.track_number {
                            ui.label(format!("Track: {} / Disc: {}",
                                tn, track.metadata.disc_number.unwrap_or(1)));
                        }

                        ui.separator();
                        let path_display = track.file_path.to_string_lossy().to_string();
                        ui.label(format!("File: {}", path_display));

                        ui.separator();
                        ui.label("Up Next:");
                        let upcoming = state.queue.upcoming(5);
                        if upcoming.is_empty() {
                            ui.label("Queue is empty");
                        } else {
                            let cmd = cmd.clone();
                            for upcoming_tid in upcoming {
                                if let Some(t) = state.library.get_track(upcoming_tid) {
                                    let label = format!("\u{2022} {}", t.metadata.display_title(&t.file_path));
                                    let tid = upcoming_tid.clone();
                                    if ui.link(label).clicked() {
                                        if let Some(ref s) = cmd {
                                            let _ = s.send(PlaybackCommand::Play(tid));
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    ui.heading("Nothing Playing");
                    ui.label("Select a track to start playback");
                }
            });
        });
    }

    fn render_folder_tree(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
        query: &str,
    ) {
        let current_track = state.queue.current_track().cloned();

        if state.library_paths.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label("No library paths configured.");
            });
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for lib_path in state.library_paths.clone() {
                if !state.library.folder_has_audio(&lib_path) {
                    continue;
                }
                let folder_name = lib_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| lib_path.to_string_lossy().to_string());
                self.render_folder_node(
                    ui, state, cmd, &lib_path, &folder_name, 0, &current_track, query,
                );
            }
        });
    }

    fn render_folder_node(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
        path: &PathBuf,
        label: &str,
        depth: usize,
        current_track_id: &Option<TrackId>,
        query: &str,
    ) {
        if !state.library.folder_has_audio(path) {
            return;
        }

        if !query.is_empty() {
            let q = query.to_lowercase();
            let has_match = state.library.all_tracks().iter().any(|t| {
                t.file_path.starts_with(path) && t.metadata.search_text().contains(&q)
            });
            if !has_match {
                return;
            }
        }

        let contains_current = current_track_id.as_ref().map_or(false, |tid| {
            state
                .library
                .get_track(tid)
                .map_or(false, |t| t.file_path.starts_with(path))
        });

        let is_selected = state.selected_folder.as_ref() == Some(path);

        let header_text = if depth == 0 {
            format!("\u{1F4C1} {}", label)
        } else {
            label.to_string()
        };

        let folder_track_ids: Vec<TrackId> = state.library.track_ids_in_folder_tree(path);

        let header = egui::CollapsingHeader::new(header_text)
            .default_open(contains_current || is_selected);

        let header_response = header.show(ui, |ui| {
            let children = state.library.subdirs_with_audio(path);
            for child_path in &children {
                let child_name = child_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.render_folder_node(
                    ui, state, cmd, child_path, &child_name, depth + 1, current_track_id, query,
                );
            }

            let tracks: Vec<crate::domain::Track> = if query.is_empty() {
                state
                    .library
                    .tracks_in_folder(path)
                    .into_iter()
                    .cloned()
                    .collect()
            } else {
                let q = query.to_lowercase();
                state
                    .library
                    .tracks_in_folder(path)
                    .into_iter()
                    .filter(|t| t.metadata.search_text().contains(&q))
                    .cloned()
                    .collect()
            };

            for track in &tracks {
                let is_track_selected = state.selected_track.as_ref() == Some(&track.id);
                let is_current = current_track_id.as_ref() == Some(&track.id);

                self.request_cover(&track.id, &track.file_path);

                ui.horizontal(|ui| {
                    ui.set_min_height(20.0);
                    if depth > 0 {
                        ui.add_space(depth as f32 * 16.0);
                    }
                    if is_current {
                        ui.label("\u{25B6}");
                    }
                    let display = format!(
                        "{}. {}",
                        track.metadata.track_number.unwrap_or(0),
                        track.metadata.display_title(&track.file_path)
                    );
                    let resp = ui.selectable_label(is_track_selected, display);
                    if resp.clicked() {
                        state.selected_track = Some(track.id.clone());
                    }
                    if resp.double_clicked() {
                        state.selected_track = Some(track.id.clone());
                        if let Some(ref s) = cmd {
                            let _ = s.send(PlaybackCommand::Play(track.id.clone()));
                        }
                    }
                    let cmd_ct = cmd.clone();
                    let tid_ct = track.id.clone();
                    resp.context_menu(move |ui| {
                        if ui.button("Play").clicked() {
                            if let Some(ref s) = cmd_ct {
                                let _ = s.send(PlaybackCommand::Play(tid_ct.clone()));
                            }
                            ui.close();
                        }
                        if ui.button("Play Next").clicked() {
                            if let Some(ref s) = cmd_ct {
                                let _ = s.send(PlaybackCommand::PlayNext(tid_ct.clone()));
                            }
                            ui.close();
                        }
                        if ui.button("Add to Queue").clicked() {
                            if let Some(ref s) = cmd_ct {
                                let _ = s.send(PlaybackCommand::AddToQueue(tid_ct.clone()));
                            }
                            ui.close();
                        }
                    });
                });
            }
        });

        if header_response.header_response.clicked() {
            state.selected_folder = Some(path.clone());
        }

        if header_response.header_response.double_clicked() {
            self.play_folder(state, path, cmd);
        }

        if !folder_track_ids.is_empty() {
            let cmd_ct = cmd.clone();
            let tids_ct = folder_track_ids;
            header_response.header_response.context_menu(move |ui| {
                if ui.button("Play").clicked() {
                    if let Some(ref s) = cmd_ct {
                        if let Some(first) = tids_ct.first() {
                            let _ = s.send(PlaybackCommand::Play(first.clone()));
                            for tid in &tids_ct[1..] {
                                let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
                            }
                        }
                    }
                    ui.close();
                }
                if ui.button("Play Next").clicked() {
                    if let Some(ref s) = cmd_ct {
                        for tid in tids_ct.iter().rev() {
                            let _ = s.send(PlaybackCommand::PlayNext(tid.clone()));
                        }
                    }
                    ui.close();
                }
                if ui.button("Append to Queue").clicked() {
                    if let Some(ref s) = cmd_ct {
                        for tid in &tids_ct {
                            let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
                        }
                    }
                    ui.close();
                }
            });
        }
    }

    fn play_folder(
        &self,
        state: &AppState,
        path: &PathBuf,
        cmd: &Option<Sender<PlaybackCommand>>,
    ) {
        let track_ids = state.library.track_ids_in_folder_tree(path);
        let Some(ref s) = cmd else { return };
        if let Some(first) = track_ids.first() {
            let _ = s.send(PlaybackCommand::Play(first.clone()));
            for tid in &track_ids[1..] {
                let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
            }
        }
    }
}

fn format_duration(duration: std::time::Duration) -> String {
    let mins = duration.as_secs() / 60;
    let secs = duration.as_secs() % 60;
    format!("{:02}:{:02}", mins, secs)
}
