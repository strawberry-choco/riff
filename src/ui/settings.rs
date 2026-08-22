use crate::app::commands::LibraryCommand;
use crate::app::state::{AppState, LibraryStatus, ViewMode, WatchState};
use crate::app::store::SettingsStore;
use crate::app::MutexExt;
use crossbeam_channel::Sender;
use eframe::egui;
use std::path::PathBuf;

/// Expand a leading `~/` (or a bare `~`) in `input` against the `HOME`
/// environment variable. Returns the path unchanged when there is no leading
/// `~` or when `HOME` is unset.
pub fn expand_tilde(input: &str) -> PathBuf {
    if input == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = input.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(input)
}

/// Return up to `max` existing subdirectories matching the typed partial
/// path, for directory autocomplete in the Linux folder picker.
///
/// Resolves the parent directory of `input` plus its partial last segment and
/// lists children of the parent whose names start with that segment
/// (case-insensitive). When `input` is empty, `~`, or ends with a separator,
/// the children of that directory itself are listed (empty input completes
/// against the current directory). Results are sorted, deduped, and capped.
/// Non-existent or unreadable parents yield an empty list (never a panic).
pub fn suggest_directories(input: &str, max: usize) -> Vec<PathBuf> {
    let expanded = expand_tilde(input);

    // Which directory to list, and which prefix its children must match.
    let (parent, partial) = if input.is_empty() {
        (PathBuf::from("."), String::new())
    } else if input == "~" || input.ends_with(['/', '\\']) {
        (expanded.clone(), String::new())
    } else {
        let partial = expanded
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        match expanded.parent() {
            // A relative single segment (e.g. "Mus") has an empty parent;
            // complete it against the current directory instead.
            Some(parent) if !parent.as_os_str().is_empty() => (parent.to_path_buf(), partial),
            _ => (PathBuf::from("."), partial),
        }
    };

    if !parent.is_dir() {
        return Vec::new();
    }

    let partial_lower = partial.to_lowercase();
    let mut matches: Vec<PathBuf> = match std::fs::read_dir(&parent) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .to_lowercase()
                        .starts_with(&partial_lower)
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    matches.sort();
    matches.dedup();
    matches.truncate(max);
    matches
}

/// Register one library root: update `AppState`, then persist the path list
/// to the Application Store. A store write failure is logged (the in-memory
/// change stands so the session still works).
fn add_library_path(path: PathBuf, state: &mut AppState, store: &mut dyn SettingsStore) {
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    if state.library_paths.contains(&canonical) {
        return;
    }
    state.library_paths.push(canonical.clone());
    state.library_statuses.entry(canonical.clone()).or_default();
    if let Err(e) = store.save_library_paths(&state.library_paths) {
        tracing::warn!("Failed to save library paths: {e}");
    }
}

#[cfg(not(target_os = "linux"))]
fn pick_folder_ui(ui: &mut egui::Ui, state: &mut AppState, store: &mut dyn SettingsStore) {
    if ui.button("Add Library").clicked() {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Add Music Library")
            .pick_folder()
        {
            add_library_path(path, state, store);
        }
    }
}

#[cfg(target_os = "linux")]
fn pick_folder_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    store: &mut dyn SettingsStore,
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
                let path = expand_tilde(text_input);
                if !path.exists() {
                    *path_error = Some(format!("Path does not exist: {}", path.display()));
                } else if !path.is_dir() {
                    *path_error = Some(format!("Not a directory: {}", path.display()));
                } else {
                    add_library_path(path, state, store);
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

        // Directory autocomplete: clickable suggestions that fill the input so
        // the user can drill down without a native dialog. Clicking appends a
        // separator, which lists that directory's children on the next frame.
        for suggestion in suggest_directories(text_input.as_str(), 8) {
            let label = suggestion.to_string_lossy().to_string();
            if ui
                .selectable_label(false, format!("\u{1F4C1} {label}"))
                .clicked()
            {
                *text_input = format!("{label}/");
                *path_error = None;
            }
        }
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
    /// Render the settings view inside `CentralPanel`.
    pub fn show_settings_view(
        &mut self,
        parent_ui: &mut egui::Ui,
        state: &mut AppState,
        lib_cmd: Option<&Sender<LibraryCommand>>,
        frame: &mut eframe::Frame,
    ) {
        egui::CentralPanel::default().show_inside(parent_ui, |ui| {
            ui.vertical(|ui| {
                // --- TOP BAR ---
                ui.horizontal(|ui| {
                    if ui.button("\u{2190} Back").clicked() {
                        state.view_mode = ViewMode::Library;
                    }
                    ui.heading("Settings");
                });

                ui.add_space(16.0);

                // --- GENERAL (most common settings first) ---
                ui.heading("General");
                ui.separator();

                ui.strong("Music Libraries");

                self.render_library_list(ui, state, lib_cmd, frame);
                #[cfg(not(target_os = "linux"))]
                self.render_library_actions(ui, state, lib_cmd);
                #[cfg(target_os = "linux")]
                self.render_library_actions(
                    ui,
                    state,
                    lib_cmd,
                    &mut self.settings_text_input,
                    &mut self.settings_show_input,
                    &mut self.settings_path_error,
                );

                ui.add_space(8.0);
                self.render_clear_library(ui, state);

                ui.add_space(16.0);

                // --- PREFERENCES ---
                ui.heading("Preferences");
                ui.separator();

                self.render_preferences(ui, state);

                ui.add_space(16.0);

                // --- ADVANCED & PLATFORM INFO ---
                ui.heading("Advanced & platform info");
                ui.separator();

                render_platform_info(ui, state.ui_flags.advanced_mode);
            });
        });
    }

    /// --- LIBRARY LIST --- one row per library path plus its watch toggle.
    fn render_library_list(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        lib_cmd: Option<&Sender<LibraryCommand>>,
        _frame: &mut eframe::Frame,
    ) {
        if state.library_paths.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label("No music libraries configured. Add one to get started.");
            });
            return;
        }
        let paths = state.library_paths.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for path in paths {
                self.render_library_row(ui, state, lib_cmd, &path);
                self.render_watch_toggle_row(ui, state, &path);
                if state.library_statuses.get(&path) == Some(&LibraryStatus::Unavailable) {
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

    /// Watch toggle row below a library path.
    fn render_watch_toggle_row(&mut self, ui: &mut egui::Ui, state: &mut AppState, path: &PathBuf) {
        let status = state
            .library_statuses
            .get(path)
            .cloned()
            .unwrap_or_default();
        let is_unavailable = matches!(status, LibraryStatus::Unavailable);

        ui.horizontal(|ui| {
            ui.add_space(20.0);
            let watch_state = state.watch_states.get(path).cloned().unwrap_or_default();
            let is_warning = matches!(watch_state, WatchState::Warning(_));
            let can_watch = !is_warning && !is_unavailable;

            let mut watching = watch_state == WatchState::Enabled;
            let toggle = ui.add_enabled(can_watch, egui::Checkbox::new(&mut watching, "Watch"));

            if toggle.changed() {
                let wm = self.watcher_manager.clone();
                let path_c = path.clone();
                if watching {
                    let result = {
                        let mut guard = wm.lock_or_recover();
                        guard.as_mut().map_or_else(
                            || Err("Watcher not initialized".to_string()),
                            |mgr| mgr.start_watching(&path_c),
                        )
                    };
                    match result {
                        Ok(()) => {
                            state.watch_states.insert(path.clone(), WatchState::Enabled);
                        }
                        Err(reason) => {
                            state
                                .watch_states
                                .insert(path.clone(), WatchState::Warning(reason));
                        }
                    }
                } else {
                    if let Some(ref mut mgr) = *self.watcher_manager.lock_or_recover() {
                        mgr.stop_watching(path);
                    }
                    state
                        .watch_states
                        .insert(path.clone(), WatchState::Disabled);
                }
                if let Err(e) = self.settings_store.save_watch_states(&state.watch_states) {
                    tracing::warn!("Failed to save watch states: {e}");
                }
            }

            if let WatchState::Warning(ref reason) = watch_state {
                ui.label("\u{26A0}").on_hover_text(reason);
            }
        });
    }
}

impl super::app::RiffApp {
    /// One library-path row: icon, (possibly unavailable) path, remove + scan
    /// buttons, and status label.
    fn render_library_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        lib_cmd: Option<&Sender<LibraryCommand>>,
        path: &PathBuf,
    ) {
        let status = state
            .library_statuses
            .get(path)
            .cloned()
            .unwrap_or_default();
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
                if ui
                    .button("\u{1F5D1}")
                    .on_hover_text("Remove library")
                    .clicked()
                {
                    // One durable store transaction removes the root's tracks,
                    // orphaned parents, and the path record; playlist entries
                    // survive dangling so they recover when files return.
                    match self.library_mutations.remove_library_path(path) {
                        Ok(_) => {
                            self.store_generation.bump();
                        }
                        Err(e) => tracing::error!("Failed to remove {path:?} from store: {e}"),
                    }
                    state.library.remove_tracks_by_root(path);
                    state.library_paths.retain(|p| p != path);
                    state.library_statuses.remove(path);
                    if let Err(e) = self.settings_store.save_library_paths(&state.library_paths) {
                        tracing::warn!("Failed to save library paths: {e}");
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
                    if let Some(s) = lib_cmd {
                        let _ = s.send(LibraryCommand::ScanDirectory(path.clone()));
                    }
                }

                // Status label
                let (status_text, status_color) = match status {
                    LibraryStatus::Idle => {
                        ("\u{2705} Ready".to_string(), ui.visuals().selection.bg_fill)
                    }
                    LibraryStatus::Scanning { files_found } => {
                        let text = format!("\u{23F3} Scanning... {files_found} files");
                        (text, ui.visuals().hyperlink_color)
                    }
                    LibraryStatus::Scanned(n) => {
                        (format!("{n} tracks"), ui.visuals().weak_text_color())
                    }
                    LibraryStatus::Unavailable => (
                        "\u{26A0} Unavailable".to_string(),
                        ui.visuals().error_fg_color,
                    ),
                };
                ui.colored_label(status_color, status_text);
            });
        });
    }
}

impl super::app::RiffApp {
    /// "Add Library" (native dialog or text picker) + "Scan All" actions.
    #[cfg(not(target_os = "linux"))]
    fn render_library_actions(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        lib_cmd: Option<&Sender<LibraryCommand>>,
    ) {
        ui.horizontal(|ui| {
            pick_folder_ui(ui, state, self.settings_store.as_mut());
            render_scan_all(ui, state, lib_cmd);
        });
    }

    /// "Add Library" (native dialog or text picker) + "Scan All" actions.
    #[cfg(target_os = "linux")]
    fn render_library_actions(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        lib_cmd: Option<&Sender<LibraryCommand>>,
        text_input: &mut String,
        show_input: &mut bool,
        path_error: &mut Option<String>,
    ) {
        ui.horizontal(|ui| {
            pick_folder_ui(
                ui,
                state,
                self.settings_store.as_mut(),
                text_input,
                show_input,
                path_error,
            );
            render_scan_all(ui, state, lib_cmd);
        });
    }

    /// The "Clear Library" maintenance action: wipes the indexed collection
    /// (tracks with their play history, albums, artists) as one durable
    /// store transaction while playlists and settings are untouched. Behind
    /// an inline confirmation; on success the session generation bumps so
    /// every projection refreshes immediately without a restart.
    fn render_clear_library(&mut self, ui: &mut egui::Ui, state: &mut AppState) {
        ui.separator();
        ui.strong("Maintenance");
        if !self.clear_library_confirm {
            if ui
                .button("Clear Library")
                .on_hover_text("Wipe all indexed tracks and rebuild from scratch by rescanning.")
                .clicked()
            {
                self.clear_library_confirm = true;
            }
            return;
        }

        ui.label(
            egui::RichText::new("Remove every indexed track? Playlists and settings are kept.")
                .color(ui.visuals().warn_fg_color),
        );
        ui.horizontal(|ui| {
            if ui.button("Confirm").clicked() {
                self.clear_library_confirm = false;
                match self.library_mutations.clear_library() {
                    Ok(removed) => {
                        self.store_generation.bump();
                        // The transitional mirror drops too, so any view not
                        // yet migrated shows the cleared state immediately.
                        state.library.clear();
                        state.scan_status = Some(format!(
                            "Library cleared ({removed} tracks removed). Rescan to rebuild."
                        ));
                    }
                    Err(e) => {
                        tracing::error!("Failed to clear the library: {e}");
                        state.scan_status = Some(
                            "Failed to clear the library \u{2014} nothing was changed.".to_string(),
                        );
                    }
                }
            }
            if ui.button("Cancel").clicked() {
                self.clear_library_confirm = false;
            }
        });
    }
}

/// The right-aligned "Scan All" button.
fn render_scan_all(
    ui: &mut egui::Ui,
    state: &mut AppState,
    lib_cmd: Option<&Sender<LibraryCommand>>,
) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let all_empty = state.library_paths.is_empty();
        let any_scanning = state
            .library_statuses
            .values()
            .any(|s| matches!(s, LibraryStatus::Scanning { .. }));
        let scan_all_enabled = !all_empty && !any_scanning;

        let scan_all_btn = if scan_all_enabled {
            ui.button("Scan All")
        } else {
            ui.add_enabled(false, egui::Button::new("Scan All"))
        };
        if scan_all_btn.clicked() {
            for path in &state.library_paths {
                if let Some(s) = lib_cmd {
                    let _ = s.send(LibraryCommand::ScanDirectory(path.clone()));
                }
            }
        }
    });
}

impl super::app::RiffApp {
    /// The three persisted preference toggles: advanced mode, high contrast,
    /// `ReplayGain`. Each change commits as its own small durable store
    /// transaction; failures are logged, the in-memory change stands.
    fn render_preferences(&mut self, ui: &mut egui::Ui, state: &mut AppState) {
        let mut advanced = state.ui_flags.advanced_mode;
        let advanced_toggle = ui.checkbox(&mut advanced, "Advanced mode").on_hover_text(
            "Reveals power features: tag editing, smart playlists, \
         and extra transport controls (stop, repeat).",
        );
        if advanced_toggle.changed() {
            state.ui_flags.advanced_mode = advanced;
            self.persist_scalars(state);
        }
        ui.label(
            egui::RichText::new("Shows tag editing, smart playlists, and stop/repeat controls.")
                .weak(),
        );

        let mut high_contrast = state.ui_flags.high_contrast;
        let high_contrast_toggle = ui
            .checkbox(&mut high_contrast, "High contrast")
            .on_hover_text(
                "Uses a high-contrast theme: near-black background, white text, \
             and bright focus outlines. Overrides the light/dark theme.",
            );
        if high_contrast_toggle.changed() {
            state.ui_flags.high_contrast = high_contrast;
            self.persist_scalars(state);
        }
        ui.label(
            egui::RichText::new("Near-black background, white text, and bright focus outlines.")
                .weak(),
        );

        let mut replaygain = state.replaygain_enabled;
        let replaygain_toggle = ui
            .checkbox(&mut replaygain, "ReplayGain")
            .on_hover_text("Normalize playback loudness using ReplayGain tags from your files.");
        if replaygain_toggle.changed() {
            state.replaygain_enabled = replaygain;
            self.persist_scalars(state);
        }
        ui.label(
            egui::RichText::new(
                "Normalize loudness from REPLAYGAIN_TRACK tags; takes effect on the next track.",
            )
            .weak(),
        );
    }

    /// Commit the current scalar preferences as one small durable
    /// transaction.
    fn persist_scalars(&mut self, state: &AppState) {
        let scalars = crate::app::state::ScalarSettings {
            volume: Some(state.current_volume),
            advanced_mode: state.ui_flags.advanced_mode,
            high_contrast: state.ui_flags.high_contrast,
            replaygain_enabled: state.replaygain_enabled,
        };
        if let Err(e) = self.settings_store.save_scalars(&scalars) {
            tracing::warn!("Failed to save settings: {e}");
        }
    }
}

/// Smart-playlist note plus per-platform capability notes.
fn render_platform_info(ui: &mut egui::Ui, advanced_mode: bool) {
    // Smart Playlists note: always one factual line, never an empty
    // block. The lists themselves only exist in advanced mode.
    if advanced_mode {
        ui.label(
            "\u{2022} Smart Playlists: auto-generated, read-only lists \
             (Recently Added, Most Played, Never Played, Lost Gems) in the library view.",
        );
    } else {
        ui.label("\u{2022} Smart Playlists are hidden. Enable Advanced mode above to reveal them.");
    }

    ui.add_space(8.0);
    ui.strong("Platform notes");
    #[cfg(target_os = "linux")]
    {
        ui.label("\u{2022} No system tray on Linux: closing the window quits the app.");
        ui.label(
            "\u{2022} Folders are added with a text-based picker with directory \
             autocomplete; there is no native file dialog.",
        );
    }
    #[cfg(not(target_os = "linux"))]
    {
        ui.label(
            "\u{2022} A system tray icon is available with playback controls, \
             Show Window, and Quit.",
        );
        ui.label("\u{2022} Folders are added with the native file dialog.");
    }
}
