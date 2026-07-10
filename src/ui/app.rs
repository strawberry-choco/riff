use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use crossbeam_channel::{Sender, Receiver, unbounded};
use crate::app::commands::{LibraryCommand, LibraryUpdate};
use crate::app::cover_resolver::CoverResolver;
use crate::app::state::{AppState, ViewMode, Theme as AppTheme};
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
    pending_scan_path: String,
    first_frame: bool,
}

impl RiffApp {
    pub fn new(
        state: Arc<Mutex<AppState>>,
        command_sender: Sender<PlaybackCommand>,
        library_command_sender: Sender<LibraryCommand>,
        library_update_rx: Receiver<LibraryUpdate>,
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
                let result = resolver_clone.lock().unwrap().resolve(&path).ok().flatten();
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
            pending_scan_path: String::new(),
            first_frame: true,
        }
    }

    fn request_cover(&self, track_id: &TrackId, file_path: &PathBuf) {
        if !self.cover_textures.contains_key(&track_id.0) {
            if let Some(ref tx) = self.cover_request_tx {
                let _ = tx.send((track_id.clone(), file_path.clone()));
            }
        }
    }

    fn apply_theme(&self, ctx: &egui::Context, theme: AppTheme) {
        match theme {
            AppTheme::Light => ctx.set_visuals(egui::Visuals::light()),
            AppTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
        }
    }

    fn poll_library_updates(&self, state: &mut AppState) {
        if let Some(ref rx) = self.library_update_rx {
            while let Ok(update) = rx.try_recv() {
                match update {
                    LibraryUpdate::ScanProgress { files_found, current_dir } => {
                        state.is_scanning = true;
                        state.scan_status = Some(format!("{} files, {}", files_found, current_dir));
                    }
                    LibraryUpdate::ScanComplete { total_files } => {
                        state.is_scanning = false;
                        state.scan_status = Some(format!("Scan complete: {} tracks", total_files));
                    }
                    LibraryUpdate::ScanError(msg) => {
                        state.is_scanning = false;
                        state.scan_status = Some(format!("Error: {}", msg));
                    }
                }
            }
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
                self.cover_lru_keys.push(key);
                while self.cover_lru_keys.len() > 50 {
                    if let Some(oldest) = self.cover_lru_keys.first().cloned() {
                        self.cover_lru_keys.remove(0);
                        self.cover_textures.remove(&oldest);
                    }
                }
            }
        }
    }
}

impl eframe::App for RiffApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let cmd = self.command_sender.clone();
        let lib_cmd = self.library_command_sender.clone();
        // Clone Arc before locking so guard borrows local arc, not self.state
        let state_arc = self.state.clone();

        let mut state = state_arc.lock().unwrap();

        if self.first_frame {
            self.apply_theme(ctx, state.theme);
            self.first_frame = false;
        }

        self.poll_library_updates(&mut state);
        self.update_cover_cache(ctx);

        // Keyboard shortcuts
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.search_focus = true;
        }
        if !ctx.wants_keyboard_input() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                let playing = state.playback_state == PlaybackState::Playing;
                if playing {
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Pause); }
                } else {
                    if let Some(ref s) = cmd { let _ = s.send(PlaybackCommand::Resume); }
                }
            }
        }

        // Update window title
        if let Some(track_id) = state.queue.current_track() {
            if let Some(track) = state.library.get_track(track_id) {
                let title = format!(
                    "{} - {} \u{2014} riff",
                    track.metadata.display_artist(),
                    track.metadata.display_title(&track.file_path)
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            }
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("riff".to_owned()));
        }

        // --- TOP BAR ---
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("riff");

                ui.add(
                    egui::TextEdit::singleline(&mut self.pending_scan_path)
                        .desired_width(200.0)
                        .hint_text("/path/to/music"),
                );
                let scanning = state.is_scanning;
                let scan_btn = if scanning {
                    ui.add_enabled(false, egui::Button::new("\u{23F3} Scan"))
                } else {
                    ui.button("\u{1F4C2} Scan")
                };
                if scan_btn.clicked() && !self.pending_scan_path.is_empty() {
                    let path = PathBuf::from(self.pending_scan_path.clone());
                    if let Some(ref s) = lib_cmd {
                        let _ = s.send(LibraryCommand::ScanDirectory(path));
                    }
                }
                if let Some(ref status) = state.scan_status {
                    ui.label(status);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let theme_icon = match state.theme {
                        AppTheme::Dark => "\u{2600}",
                        AppTheme::Light => "\u{1F319}",
                    };
                    if ui.button(theme_icon).clicked() {
                        state.theme = match state.theme {
                            AppTheme::Dark => AppTheme::Light,
                            AppTheme::Light => AppTheme::Dark,
                        };
                        self.apply_theme(ctx, state.theme);
                    }
                    if ui.button("\u{2699}").clicked() {}
                    if ui.button("\u{1F3B5}").clicked() {
                        state.view_mode = match state.view_mode {
                            ViewMode::Library => ViewMode::NowPlaying,
                            ViewMode::NowPlaying => ViewMode::Library,
                        };
                    }
                });
            });
        });

        // --- CONTROL BAR ---
        egui::TopBottomPanel::bottom("control_bar").show(ctx, |ui| {
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
            ViewMode::Library => self.show_library_view(ctx, &mut state, &cmd),
            ViewMode::NowPlaying => self.show_now_playing_view(ctx, &mut state, &cmd),
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

// --- Helper methods factored out to avoid borrow conflicts ---
impl RiffApp {
    fn show_library_view(
        &mut self,
        ctx: &egui::Context,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
    ) {
        egui::SidePanel::left("library_panel").show(ctx, |ui| {
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

            // View mode toggle
            let mut show_artists = false;
            ui.horizontal(|ui| {
                if ui.selectable_label(!show_artists, "All Tracks").clicked() {
                    show_artists = false;
                }
                if ui.selectable_label(show_artists, "Artists").clicked() {
                    show_artists = true;
                }
            });
            ui.separator();

            let query = state.search_query.clone();
            let has_results = query.is_empty() || !state.library.search(&query).is_empty();

            if !has_results && !query.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.label(format!("No tracks found matching '{}'", query));
                });
            } else if show_artists {
                self.render_artist_view(ui, state, cmd, &query);
            } else {
                self.render_flat_view(ui, state, cmd, &query);
            }
        });

        // Right side: track details + cover
        egui::CentralPanel::default().show(ctx, |ui| {
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
                        });
                        ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                            if let Some(texture) = self.cover_textures.get(&track.id.0) {
                                let sized = egui::load::SizedTexture::new(texture.id(), egui::vec2(200.0, 200.0));
                                ui.add(egui::Image::from_texture(sized));
                            } else {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 200.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(40));
                                ui.allocate_ui_at_rect(rect, |ui| {
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
                                                    ui.close_menu();
                                                }
                                                if ui.button("Play Next").clicked() {
                                                    if let Some(ref s) = cmd_ct {
                                                        let _ = s.send(PlaybackCommand::PlayNext(track.id.clone()));
                                                    }
                                                    ui.close_menu();
                                                }
                                                if ui.button("Add to Queue").clicked() {
                                                    if let Some(ref s) = cmd_ct {
                                                        let _ = s.send(PlaybackCommand::AddToQueue(track.id.clone()));
                                                    }
                                                    ui.close_menu();
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
                                ui.close_menu();
                            }
                            if ui.button("Play Next").clicked() {
                                if let Some(ref s) = cmd_ct {
                                    let _ = s.send(PlaybackCommand::PlayNext(tid_ct.clone()));
                                }
                                ui.close_menu();
                            }
                            if ui.button("Add to Queue").clicked() {
                                if let Some(ref s) = cmd_ct {
                                    let _ = s.send(PlaybackCommand::AddToQueue(tid_ct.clone()));
                                }
                                ui.close_menu();
                            }
                        });
                    });
                }
            }
        });
    }

    fn show_now_playing_view(
        &mut self,
        ctx: &egui::Context,
        state: &mut AppState,
        cmd: &Option<Sender<PlaybackCommand>>,
    ) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                if let Some(track_id) = state.queue.current_track().cloned() {
                    if let Some(track) = state.library.get_track(&track_id) {
                        self.request_cover(&track.id, &track.file_path);

                        if let Some(texture) = self.cover_textures.get(&track.id.0) {
                            let sized = egui::load::SizedTexture::new(texture.id(), egui::vec2(300.0, 300.0));
                            ui.add(egui::Image::from_texture(sized));
                        } else {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 300.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(40));
                            ui.allocate_ui_at_rect(rect, |ui| {
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
}

fn format_duration(duration: std::time::Duration) -> String {
    let mins = duration.as_secs() / 60;
    let secs = duration.as_secs() % 60;
    format!("{:02}:{:02}", mins, secs)
}
