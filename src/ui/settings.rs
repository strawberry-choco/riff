use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use crossbeam_channel::Sender;
use crate::app::commands::LibraryCommand;
use crate::app::state::{AppState, LibraryStatus, ViewMode, WatchState};

const LIBRARY_PATHS_KEY: &str = "library_paths";
const VOLUME_KEY: &str = "volume";
const WATCH_STATES_KEY: &str = "watch_states";

pub fn load_library_paths(storage: Option<&dyn eframe::Storage>) -> Vec<PathBuf> {
    let Some(storage) = storage else { return Vec::new(); };
    storage.get_string(LIBRARY_PATHS_KEY)
        .and_then(|s| match serde_json::from_str::<Vec<String>>(&s) {
            Ok(v) => Some(v.into_iter().map(PathBuf::from).collect()),
            Err(e) => {
                tracing::warn!("Failed to deserialize library paths: {}", e);
                None
            }
        })
        .unwrap_or_default()
}

pub fn save_library_paths(storage: &mut dyn eframe::Storage, paths: &[PathBuf]) {
    let strings: Vec<String> = paths.iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let json = match serde_json::to_string(&strings) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("Failed to serialize library paths: {}", e);
            "[]".to_string()
        }
    };
    storage.set_string(LIBRARY_PATHS_KEY, json);
}

pub fn load_volume(storage: Option<&dyn eframe::Storage>) -> Option<f32> {
    let Some(storage) = storage else { return None; };
    storage.get_string(VOLUME_KEY)
        .and_then(|s| s.parse::<f32>().ok())
}

pub fn save_volume(storage: &mut dyn eframe::Storage, volume: f32) {
    storage.set_string(VOLUME_KEY, volume.to_string());
}

pub fn load_watch_states(storage: Option<&dyn eframe::Storage>) -> HashMap<PathBuf, WatchState> {
    let Some(storage) = storage else { return HashMap::new(); };
    storage
        .get_string(WATCH_STATES_KEY)
        .and_then(|s| match serde_json::from_str(&s) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("Failed to deserialize watch states: {}", e);
                None
            }
        })
        .unwrap_or_default()
}

pub fn save_watch_states(
    storage: &mut dyn eframe::Storage,
    states: &HashMap<PathBuf, WatchState>,
) {
    let json = match serde_json::to_string(states) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("Failed to serialize watch states: {}", e);
            "{}".to_string()
        }
    };
    storage.set_string(WATCH_STATES_KEY, json);
}

fn add_library_path(path: PathBuf, state: &mut AppState, storage: &mut dyn eframe::Storage) {
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    if state.library_paths.contains(&canonical) {
        return;
    }
    state.library_paths.push(canonical.clone());
    state.library_statuses.entry(canonical.clone()).or_default();
    save_library_paths(storage, &state.library_paths);
}

#[cfg(not(target_os = "linux"))]
fn pick_folder_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    storage: &mut dyn eframe::Storage,
) {
    if ui.button("Add Library").clicked() {
        if let Some(path) = rfd::FileDialog::new().set_title("Add Music Library").pick_folder() {
            add_library_path(path, state, storage);
        }
    }
}

#[cfg(target_os = "linux")]
fn pick_folder_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    storage: &mut dyn eframe::Storage,
    text_input: &mut String,
    show_input: &mut bool,
    path_error: &mut Option<String>,
) {
    if *show_input {
        if let Some(ref err) = *path_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.horizontal(|ui| {
            ui.label("Path:");
            ui.text_edit_singleline(text_input);
            if ui.button("Confirm").clicked() {
                let expanded = if text_input.starts_with("~/") {
                    if let Ok(home) = std::env::var("HOME") {
                        format!("{}/{}" , home, &text_input[2..])
                    } else {
                        text_input.clone()
                    }
                } else {
                    text_input.clone()
                };
                let path = PathBuf::from(expanded);
                if !path.exists() {
                    *path_error = Some(format!("Path does not exist: {}", path.display()));
                } else if !path.is_dir() {
                    *path_error = Some(format!("Not a directory: {}", path.display()));
                } else {
                    add_library_path(path, state, storage);
                    *text_input = String::new();
                    *show_input = false;
                    *path_error = None;
                }
            }
            if ui.button("Cancel").clicked() {
                *text_input = String::new();
                *show_input = false;
                *path_error = None;
            }
        });
    } else if ui.button("Add Library").clicked() {
        *show_input = true;
        *path_error = None;
    }
}

fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        format!("...{}", &path[path.len().saturating_sub(max_len - 3)..])
    }
}

impl super::app::RiffApp {
    /// Render the settings view inside CentralPanel.
    pub fn show_settings_view(
        &mut self,
        ctx: &egui::Context,
        state: &mut AppState,
        lib_cmd: &Option<Sender<LibraryCommand>>,
        frame: &mut eframe::Frame,
    ) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical(|ui| {
                // --- TOP BAR ---
                ui.horizontal(|ui| {
                    if ui.button("\u{2190} Back").clicked() {
                        state.view_mode = ViewMode::Library;
                    }
                    ui.heading("Settings");
                });

                ui.add_space(16.0);
                ui.heading("Music Libraries");
                ui.separator();

                // --- LIBRARY LIST ---
                if state.library_paths.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label("No music libraries configured. Add one to get started.");
                    });
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for path in state.library_paths.clone() {
                            let status = state.library_statuses.get(&path).cloned().unwrap_or_default();
                            let is_unavailable = matches!(status, LibraryStatus::Unavailable);
                            let is_scanning = matches!(status, LibraryStatus::Scanning { .. });

                            ui.horizontal(|ui| {
                                ui.label("\u{1F4C1}");

                                let path_str = path.to_string_lossy().to_string();
                                let display_path = truncate_path(&path_str, 60);

                                if is_unavailable {
                                    ui.label(
                                        egui::RichText::new(display_path)
                                            .color(ui.visuals().warn_fg_color)
                                            .strikethrough(),
                                    );
                                } else {
                                    ui.label(display_path);
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    // Delete button
                                    if ui.button("\u{1F5D1}").clicked() {
                                        state.library.remove_tracks_by_root(&path);
                                        state.library_paths.retain(|p| p != &path);
                                        state.library_statuses.remove(&path);
                                        state.library.save_cache();
                                        if let Some(storage) = frame.storage_mut() {
                                            save_library_paths(storage, &state.library_paths);
                                        }
                                    }

                                    // Scan button
                                    let scan_enabled = !is_scanning && !is_unavailable;
                                    let scan_btn = if scan_enabled {
                                        ui.button("\u{1F50D} Scan")
                                    } else {
                                        ui.add_enabled(false, egui::Button::new("\u{1F50D} Scan"))
                                    };
                                    if scan_btn.clicked() {
                                        if let Some(ref s) = lib_cmd {
                                            let _ = s.send(LibraryCommand::ScanDirectory(path.clone()));
                                        }
                                    }

                                    // Status label
                                    let (status_text, status_color) = match status {
                                        LibraryStatus::Idle => {
                                            ("\u{2705} Ready".to_string(), ui.visuals().selection.bg_fill)
                                        }
                                        LibraryStatus::Scanning { files_found } => {
                                            let text = format!("\u{23F3} Scanning... {} files", files_found);
                                            (text, ui.visuals().hyperlink_color)
                                        }
                                        LibraryStatus::Scanned(n) => {
                                            (format!("{} tracks", n), ui.visuals().weak_text_color())
                                        }
                                        LibraryStatus::Unavailable => {
                                            ("\u{26A0} Unavailable".to_string(), ui.visuals().error_fg_color)
                                        }
                                    };
                                    ui.colored_label(status_color, status_text);
                                });
                            });

                            // Watch toggle row
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                let watch_state = state
                                    .watch_states
                                    .get(&path)
                                    .cloned()
                                    .unwrap_or_default();
                                let is_warning = matches!(watch_state, WatchState::Warning(_));
                                let can_watch = !is_warning && !is_unavailable;

                                let mut watching = watch_state == WatchState::Enabled;
                                let toggle = ui.add_enabled(
                                    can_watch,
                                    egui::Checkbox::new(&mut watching, "Watch"),
                                );

                                if toggle.changed() {
                                    let wm = self.watcher_manager.clone();
                                    let path_c = path.clone();
                                    if watching {
                                        let result = {
                                            let mut guard = wm.lock().unwrap();
                                            guard
                                                .as_mut()
                                                .map(|mgr| mgr.start_watching(&path_c))
                                                .unwrap_or(Err(
                                                    "Watcher not initialized".to_string(),
                                                ))
                                        };
                                        match result {
                                            Ok(()) => {
                                                state.watch_states.insert(
                                                    path.clone(),
                                                    WatchState::Enabled,
                                                );
                                            }
                                            Err(reason) => {
                                                state.watch_states.insert(
                                                    path.clone(),
                                                    WatchState::Warning(reason),
                                                );
                                            }
                                        }
                                    } else {
                                        if let Some(ref mut mgr) =
                                            *self.watcher_manager.lock().unwrap()
                                        {
                                            mgr.stop_watching(&path);
                                        }
                                        state.watch_states.insert(
                                            path.clone(),
                                            WatchState::Disabled,
                                        );
                                    }
                                    if let Some(storage) = frame.storage_mut() {
                                        save_watch_states(storage, &state.watch_states);
                                    }
                                }

                                if let WatchState::Warning(ref reason) = watch_state {
                                    ui.label("\u{26A0}")
                                        .on_hover_text(reason);
                                }
                            });

                            if is_unavailable {
                                ui.horizontal(|ui| {
                                    ui.add_space(20.0);
                                    ui.label(
                                        egui::RichText::new("Path not found")
                                            .small()
                                            .color(ui.visuals().error_fg_color),
                                    );
                                });
                            }
                            ui.separator();
                        }
                    });
                }

                ui.add_space(16.0);
                ui.separator();

                // --- BOTTOM ACTIONS ---
                ui.horizontal(|ui| {
                    if let Some(storage) = frame.storage_mut() {
                        #[cfg(not(target_os = "linux"))]
                        pick_folder_ui(ui, state, storage);

                        #[cfg(target_os = "linux")]
                        pick_folder_ui(
                            ui,
                            state,
                            storage,
                            &mut self.settings_text_input,
                            &mut self.settings_show_input,
                            &mut self.settings_path_error,
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let all_empty = state.library_paths.is_empty();
                        let any_scanning = state.library_statuses.values()
                            .any(|s| matches!(s, LibraryStatus::Scanning { .. }));
                        let scan_all_enabled = !all_empty && !any_scanning;

                        let scan_all_btn = if scan_all_enabled {
                            ui.button("Scan All")
                        } else {
                            ui.add_enabled(false, egui::Button::new("Scan All"))
                        };
                        if scan_all_btn.clicked() {
                            for path in &state.library_paths {
                                if let Some(ref s) = lib_cmd {
                                    let _ = s.send(LibraryCommand::ScanDirectory(path.clone()));
                                }
                            }
                        }
                    });
                });
            });
        });
    }
}
