use eframe::egui;
use std::path::PathBuf;
use crossbeam_channel::Sender;
use crate::app::commands::LibraryCommand;
use crate::app::state::{AppState, LibraryStatus, ViewMode};

const LIBRARY_PATHS_KEY: &str = "library_paths";

pub fn load_library_paths(storage: Option<&dyn eframe::Storage>) -> Vec<PathBuf> {
    let Some(storage) = storage else { return Vec::new(); };
    storage.get_string(LIBRARY_PATHS_KEY)
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| v.into_iter().map(PathBuf::from).collect())
        .unwrap_or_default()
}

pub fn save_library_paths(storage: &mut dyn eframe::Storage, paths: &[PathBuf]) {
    let strings: Vec<String> = paths.iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let json = serde_json::to_string(&strings).unwrap_or_else(|_| "[]".to_string());
    storage.set_string(LIBRARY_PATHS_KEY, json);
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
) {
    if *show_input {
        ui.horizontal(|ui| {
            ui.label("Path:");
            ui.text_edit_singleline(text_input);
            if ui.button("Confirm").clicked() {
                let path = PathBuf::from(text_input.clone());
                if path.exists() && path.is_dir() {
                    add_library_path(path, state, storage);
                }
                *text_input = String::new();
                *show_input = false;
            }
            if ui.button("Cancel").clicked() {
                *text_input = String::new();
                *show_input = false;
            }
        });
    } else if ui.button("Add Library").clicked() {
        *show_input = true;
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
