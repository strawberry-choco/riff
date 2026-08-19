use crate::app::commands::LibraryCommand;
use crate::app::library_manager::LibraryManager;
use crate::app::state::{AppState, LibraryStatus, ViewMode, WatchState};
use crate::app::MutexExt;
use crossbeam_channel::Sender;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;

const LIBRARY_PATHS_KEY: &str = "library_paths";
const LIBRARY_PATHS_BACKUP_KEY: &str = "library_paths_backup";
const VOLUME_KEY: &str = "volume";
const VOLUME_BACKUP_KEY: &str = "volume_backup";
const WATCH_STATES_KEY: &str = "watch_states";
const WATCH_STATES_BACKUP_KEY: &str = "watch_states_backup";
const ADVANCED_MODE_KEY: &str = "advanced_mode";
const ADVANCED_MODE_BACKUP_KEY: &str = "advanced_mode_backup";
const HIGH_CONTRAST_KEY: &str = "high_contrast";
const HIGH_CONTRAST_BACKUP_KEY: &str = "high_contrast_backup";
const REPLAYGAIN_KEY: &str = "replaygain";
const REPLAYGAIN_BACKUP_KEY: &str = "replaygain_backup";

pub fn load_library_paths(storage: Option<&mut dyn eframe::Storage>) -> Vec<PathBuf> {
    let Some(storage) = storage else {
        return Vec::new();
    };

    // Try to load from primary storage first
    if let Some(paths) = storage.get_string(LIBRARY_PATHS_KEY).and_then(|s| {
        match serde_json::from_str::<Vec<String>>(&s) {
            Ok(v) => Some(v.into_iter().map(PathBuf::from).collect()),
            Err(e) => {
                tracing::warn!(
                    "Failed to deserialize library paths from primary storage: {}",
                    e
                );
                None
            }
        }
    }) {
        return paths;
    }

    // If primary storage fails, try to load from backup
    if let Some(paths) = storage.get_string(LIBRARY_PATHS_BACKUP_KEY).and_then(|s| {
        match serde_json::from_str::<Vec<String>>(&s) {
            Ok(v) => {
                tracing::info!("Restored library paths from backup");
                // Restore to primary storage
                let json = serde_json::to_string(&v)
                    .map_err(|e| tracing::error!("Failed to serialize restored paths: {}", e))
                    .unwrap_or_default();
                storage.set_string(LIBRARY_PATHS_KEY, json);
                Some(v.into_iter().map(PathBuf::from).collect())
            }
            Err(e) => {
                tracing::warn!("Failed to deserialize library paths from backup: {}", e);
                None
            }
        }
    }) {
        return paths;
    }

    // If both fail, return empty vector and create a backup of the empty state
    let empty_json = serde_json::to_string(&Vec::<String>::new())
        .map_err(|e| tracing::error!("Failed to serialize empty paths: {}", e))
        .unwrap_or_default();
    storage.set_string(LIBRARY_PATHS_KEY, empty_json.clone());
    storage.set_string(LIBRARY_PATHS_BACKUP_KEY, empty_json);

    Vec::new()
}

/// Load library paths from immutable storage (read-only)
pub fn load_library_paths_immutable(storage: Option<&dyn eframe::Storage>) -> Vec<PathBuf> {
    let Some(storage) = storage else {
        return Vec::new();
    };

    // Try to load from primary storage first
    if let Some(paths) = storage.get_string(LIBRARY_PATHS_KEY).and_then(|s| {
        match serde_json::from_str::<Vec<String>>(&s) {
            Ok(v) => Some(v.into_iter().map(PathBuf::from).collect()),
            Err(e) => {
                tracing::warn!(
                    "Failed to deserialize library paths from primary storage: {}",
                    e
                );
                None
            }
        }
    }) {
        return paths;
    }

    // If primary storage fails, try to load from backup
    if let Some(paths) = storage.get_string(LIBRARY_PATHS_BACKUP_KEY).and_then(|s| {
        match serde_json::from_str::<Vec<String>>(&s) {
            Ok(v) => Some(v.into_iter().map(PathBuf::from).collect()),
            Err(e) => {
                tracing::warn!("Failed to deserialize library paths from backup: {}", e);
                None
            }
        }
    }) {
        return paths;
    }

    Vec::new()
}

pub fn save_library_paths(storage: &mut dyn eframe::Storage, paths: &[PathBuf]) {
    let strings: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    // Save to primary storage
    let primary_json = match serde_json::to_string(&strings) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("Failed to serialize library paths: {}", e);
            return;
        }
    };
    storage.set_string(LIBRARY_PATHS_KEY, primary_json.clone());

    // Create/update backup
    storage.set_string(LIBRARY_PATHS_BACKUP_KEY, primary_json);
}

pub fn load_volume(storage: Option<&mut dyn eframe::Storage>) -> Option<f32> {
    let storage = storage?;

    // Try to load from primary storage first
    if let Some(volume) = storage
        .get_string(VOLUME_KEY)
        .and_then(|s| s.parse::<f32>().ok())
    {
        return Some(volume);
    }

    // If primary storage fails, try to load from backup
    if let Some(volume) = storage
        .get_string(VOLUME_BACKUP_KEY)
        .and_then(|s| s.parse::<f32>().ok())
    {
        tracing::info!("Restored volume from backup");
        // Restore to primary storage
        storage.set_string(VOLUME_KEY, volume.to_string());
        return Some(volume);
    }

    // If both fail, return default volume and create backup
    let default_volume = 1.0;
    storage.set_string(VOLUME_KEY, default_volume.to_string());
    storage.set_string(VOLUME_BACKUP_KEY, default_volume.to_string());

    Some(default_volume)
}

/// Load volume from immutable storage (read-only)
pub fn load_volume_immutable(storage: Option<&dyn eframe::Storage>) -> Option<f32> {
    let storage = storage?;

    // Try to load from primary storage first
    if let Some(volume) = storage
        .get_string(VOLUME_KEY)
        .and_then(|s| s.parse::<f32>().ok())
    {
        return Some(volume);
    }

    // If primary storage fails, try to load from backup
    if let Some(volume) = storage
        .get_string(VOLUME_BACKUP_KEY)
        .and_then(|s| s.parse::<f32>().ok())
    {
        return Some(volume);
    }

    None
}

pub fn save_volume(storage: &mut dyn eframe::Storage, volume: f32) {
    // Save to primary storage
    storage.set_string(VOLUME_KEY, volume.to_string());

    // Create/update backup
    storage.set_string(VOLUME_BACKUP_KEY, volume.to_string());
}

pub fn load_advanced_mode(storage: Option<&mut dyn eframe::Storage>) -> bool {
    let Some(storage) = storage else {
        return false;
    };

    // Try to load from primary storage first
    if let Some(enabled) = storage
        .get_string(ADVANCED_MODE_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    // If primary storage fails, try to load from backup
    if let Some(enabled) = storage
        .get_string(ADVANCED_MODE_BACKUP_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        tracing::info!("Restored advanced mode from backup");
        // Restore to primary storage
        storage.set_string(ADVANCED_MODE_KEY, enabled.to_string());
        return enabled;
    }

    // If both fail, return the default (off) and create a backup
    let default_enabled = false;
    storage.set_string(ADVANCED_MODE_KEY, default_enabled.to_string());
    storage.set_string(ADVANCED_MODE_BACKUP_KEY, default_enabled.to_string());

    default_enabled
}

/// Load advanced mode from immutable storage (read-only)
pub fn load_advanced_mode_immutable(storage: Option<&dyn eframe::Storage>) -> bool {
    let Some(storage) = storage else {
        return false;
    };

    // Try to load from primary storage first
    if let Some(enabled) = storage
        .get_string(ADVANCED_MODE_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    // If primary storage fails, try to load from backup
    if let Some(enabled) = storage
        .get_string(ADVANCED_MODE_BACKUP_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    false
}

pub fn save_advanced_mode(storage: &mut dyn eframe::Storage, enabled: bool) {
    // Save to primary storage
    storage.set_string(ADVANCED_MODE_KEY, enabled.to_string());

    // Create/update backup
    storage.set_string(ADVANCED_MODE_BACKUP_KEY, enabled.to_string());
}

pub fn load_high_contrast(storage: Option<&mut dyn eframe::Storage>) -> bool {
    let Some(storage) = storage else {
        return false;
    };

    // Try to load from primary storage first
    if let Some(enabled) = storage
        .get_string(HIGH_CONTRAST_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    // If primary storage fails, try to load from backup
    if let Some(enabled) = storage
        .get_string(HIGH_CONTRAST_BACKUP_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        tracing::info!("Restored high contrast from backup");
        // Restore to primary storage
        storage.set_string(HIGH_CONTRAST_KEY, enabled.to_string());
        return enabled;
    }

    // If both fail, return the default (off) and create a backup
    let default_enabled = false;
    storage.set_string(HIGH_CONTRAST_KEY, default_enabled.to_string());
    storage.set_string(HIGH_CONTRAST_BACKUP_KEY, default_enabled.to_string());

    default_enabled
}

/// Load high contrast from immutable storage (read-only)
pub fn load_high_contrast_immutable(storage: Option<&dyn eframe::Storage>) -> bool {
    let Some(storage) = storage else {
        return false;
    };

    // Try to load from primary storage first
    if let Some(enabled) = storage
        .get_string(HIGH_CONTRAST_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    // If primary storage fails, try to load from backup
    if let Some(enabled) = storage
        .get_string(HIGH_CONTRAST_BACKUP_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    false
}

pub fn save_high_contrast(storage: &mut dyn eframe::Storage, enabled: bool) {
    // Save to primary storage
    storage.set_string(HIGH_CONTRAST_KEY, enabled.to_string());

    // Create/update backup
    storage.set_string(HIGH_CONTRAST_BACKUP_KEY, enabled.to_string());
}

pub fn load_replaygain(storage: Option<&mut dyn eframe::Storage>) -> bool {
    let Some(storage) = storage else {
        return false;
    };

    // Try to load from primary storage first
    if let Some(enabled) = storage
        .get_string(REPLAYGAIN_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    // If primary storage fails, try to load from backup
    if let Some(enabled) = storage
        .get_string(REPLAYGAIN_BACKUP_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        tracing::info!("Restored replaygain from backup");
        // Restore to primary storage
        storage.set_string(REPLAYGAIN_KEY, enabled.to_string());
        return enabled;
    }

    // If both fail, return the default (off) and create a backup
    let default_enabled = false;
    storage.set_string(REPLAYGAIN_KEY, default_enabled.to_string());
    storage.set_string(REPLAYGAIN_BACKUP_KEY, default_enabled.to_string());

    default_enabled
}

/// Load replaygain from immutable storage (read-only)
pub fn load_replaygain_immutable(storage: Option<&dyn eframe::Storage>) -> bool {
    let Some(storage) = storage else {
        return false;
    };

    // Try to load from primary storage first
    if let Some(enabled) = storage
        .get_string(REPLAYGAIN_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    // If primary storage fails, try to load from backup
    if let Some(enabled) = storage
        .get_string(REPLAYGAIN_BACKUP_KEY)
        .and_then(|s| s.parse::<bool>().ok())
    {
        return enabled;
    }

    false
}

pub fn save_replaygain(storage: &mut dyn eframe::Storage, enabled: bool) {
    // Save to primary storage
    storage.set_string(REPLAYGAIN_KEY, enabled.to_string());

    // Create/update backup
    storage.set_string(REPLAYGAIN_BACKUP_KEY, enabled.to_string());
}

pub fn load_watch_states(
    storage: Option<&mut dyn eframe::Storage>,
) -> HashMap<PathBuf, WatchState> {
    let Some(storage) = storage else {
        return HashMap::new();
    };

    // Try to load from primary storage first
    if let Some(states) =
        storage
            .get_string(WATCH_STATES_KEY)
            .and_then(|s| match serde_json::from_str(&s) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(
                        "Failed to deserialize watch states from primary storage: {}",
                        e
                    );
                    None
                }
            })
    {
        return states;
    }

    // If primary storage fails, try to load from backup
    if let Some(states) =
        storage
            .get_string(WATCH_STATES_BACKUP_KEY)
            .and_then(|s| match serde_json::from_str(&s) {
                Ok(v) => {
                    tracing::info!("Restored watch states from backup");
                    // Restore to primary storage
                    let json = serde_json::to_string(&v)
                        .map_err(|e| {
                            tracing::error!("Failed to serialize restored watch states: {}", e);
                        })
                        .unwrap_or_default();
                    storage.set_string(WATCH_STATES_KEY, json);
                    Some(v)
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize watch states from backup: {}", e);
                    None
                }
            })
    {
        return states;
    }

    // If both fail, return empty HashMap and create backup of empty state
    let empty_json = serde_json::to_string(&HashMap::<PathBuf, WatchState>::new())
        .map_err(|e| tracing::error!("Failed to serialize empty watch states: {}", e))
        .unwrap_or_default();
    storage.set_string(WATCH_STATES_KEY, empty_json.clone());
    storage.set_string(WATCH_STATES_BACKUP_KEY, empty_json);

    HashMap::new()
}

/// Load watch states from immutable storage (read-only)
pub fn load_watch_states_immutable(
    storage: Option<&dyn eframe::Storage>,
) -> HashMap<PathBuf, WatchState> {
    let Some(storage) = storage else {
        return HashMap::new();
    };

    // Try to load from primary storage first
    if let Some(states) =
        storage
            .get_string(WATCH_STATES_KEY)
            .and_then(|s| match serde_json::from_str(&s) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(
                        "Failed to deserialize watch states from primary storage: {}",
                        e
                    );
                    None
                }
            })
    {
        return states;
    }

    // If primary storage fails, try to load from backup
    if let Some(states) =
        storage
            .get_string(WATCH_STATES_BACKUP_KEY)
            .and_then(|s| match serde_json::from_str(&s) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!("Failed to deserialize watch states from backup: {}", e);
                    None
                }
            })
    {
        return states;
    }

    HashMap::new()
}

pub fn save_watch_states<S: std::hash::BuildHasher>(
    storage: &mut dyn eframe::Storage,
    states: &HashMap<PathBuf, WatchState, S>,
) {
    // Save to primary storage
    let json = match serde_json::to_string(states) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("Failed to serialize watch states: {}", e);
            return;
        }
    };
    storage.set_string(WATCH_STATES_KEY, json.clone());

    // Create/update backup
    storage.set_string(WATCH_STATES_BACKUP_KEY, json);
}

/// Check if storage is corrupted and attempt to restore from backup
pub fn restore_from_backup_if_corrupted(storage: &mut dyn eframe::Storage) {
    // Check library paths
    if let Some(primary) = storage.get_string(LIBRARY_PATHS_KEY) {
        if serde_json::from_str::<Vec<String>>(&primary).is_err() {
            tracing::warn!("Library paths storage is corrupted, attempting to restore from backup");
            if let Some(backup) = storage.get_string(LIBRARY_PATHS_BACKUP_KEY) {
                if serde_json::from_str::<Vec<String>>(&backup).is_ok() {
                    storage.set_string(LIBRARY_PATHS_KEY, backup);
                    tracing::info!("Library paths restored from backup");
                }
            }
        }
    }

    // Check volume
    if let Some(primary) = storage.get_string(VOLUME_KEY) {
        if primary.parse::<f32>().is_err() {
            tracing::warn!("Volume storage is corrupted, attempting to restore from backup");
            if let Some(backup) = storage.get_string(VOLUME_BACKUP_KEY) {
                if backup.parse::<f32>().is_ok() {
                    storage.set_string(VOLUME_KEY, backup);
                    tracing::info!("Volume restored from backup");
                }
            }
        }
    }

    // Check watch states
    if let Some(primary) = storage.get_string(WATCH_STATES_KEY) {
        if serde_json::from_str::<HashMap<PathBuf, WatchState>>(&primary).is_err() {
            tracing::warn!("Watch states storage is corrupted, attempting to restore from backup");
            if let Some(backup) = storage.get_string(WATCH_STATES_BACKUP_KEY) {
                if serde_json::from_str::<HashMap<PathBuf, WatchState>>(&backup).is_ok() {
                    storage.set_string(WATCH_STATES_KEY, backup);
                    tracing::info!("Watch states restored from backup");
                }
            }
        }
    }
}

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
fn pick_folder_ui(ui: &mut egui::Ui, state: &mut AppState, storage: &mut dyn eframe::Storage) {
    if ui.button("Add Library").clicked() {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Add Music Library")
            .pick_folder()
        {
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
                let path = expand_tilde(text_input);
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
                render_library_actions(ui, state, lib_cmd, frame);
                #[cfg(target_os = "linux")]
                render_library_actions(
                    ui,
                    state,
                    lib_cmd,
                    frame,
                    &mut self.settings_text_input,
                    &mut self.settings_show_input,
                    &mut self.settings_path_error,
                );
                render_cache_controls(ui, state);

                ui.add_space(16.0);

                // --- PREFERENCES ---
                ui.heading("Preferences");
                ui.separator();

                render_preferences(ui, state, frame);

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
        frame: &mut eframe::Frame,
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
                render_library_row(ui, state, lib_cmd, frame, &path);
                self.render_watch_toggle_row(ui, state, frame, &path);
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
    fn render_watch_toggle_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        frame: &mut eframe::Frame,
        path: &PathBuf,
    ) {
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
                if let Some(storage) = frame.storage_mut() {
                    save_watch_states(storage, &state.watch_states);
                }
            }

            if let WatchState::Warning(ref reason) = watch_state {
                ui.label("\u{26A0}").on_hover_text(reason);
            }
        });
    }
}

/// One library-path row: icon, (possibly unavailable) path, remove + scan
/// buttons, and status label.
fn render_library_row(
    ui: &mut egui::Ui,
    state: &mut AppState,
    lib_cmd: Option<&Sender<LibraryCommand>>,
    frame: &mut eframe::Frame,
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
                state.library.remove_tracks_by_root(path);
                state.library_paths.retain(|p| p != path);
                state.library_statuses.remove(path);
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

/// "Add Library" (native dialog or text picker) + "Scan All" actions.
#[cfg(not(target_os = "linux"))]
fn render_library_actions(
    ui: &mut egui::Ui,
    state: &mut AppState,
    lib_cmd: Option<&Sender<LibraryCommand>>,
    frame: &mut eframe::Frame,
) {
    ui.horizontal(|ui| {
        if let Some(storage) = frame.storage_mut() {
            pick_folder_ui(ui, state, storage);
        }
        render_scan_all(ui, state, lib_cmd);
    });
}

/// "Add Library" (native dialog or text picker) + "Scan All" actions.
#[cfg(target_os = "linux")]
fn render_library_actions(
    ui: &mut egui::Ui,
    state: &mut AppState,
    lib_cmd: Option<&Sender<LibraryCommand>>,
    frame: &mut eframe::Frame,
    text_input: &mut String,
    show_input: &mut bool,
    path_error: &mut Option<String>,
) {
    ui.horizontal(|ui| {
        if let Some(storage) = frame.storage_mut() {
            pick_folder_ui(ui, state, storage, text_input, show_input, path_error);
        }
        render_scan_all(ui, state, lib_cmd);
    });
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

/// "Clear Library Cache" + cache file location. The cache is a derived,
/// rebuildable index of the music files, so clearing it needs no confirmation.
fn render_cache_controls(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        if ui
            .button("Clear Library Cache")
            .on_hover_text("Deletes the cached library index. It is rebuilt on the next scan.")
            .clicked()
        {
            LibraryManager::delete_cache_file();
            state.scan_status = Some(
                "Library cache cleared \u{2014} it will be rebuilt on the next scan.".to_string(),
            );
        }
        if let Some(cache_path) = LibraryManager::cache_path() {
            let path_str = cache_path.to_string_lossy().to_string();
            ui.label(
                egui::RichText::new(format!("Cache file: {}", truncate_path(&path_str, 60))).weak(),
            );
        }
    });
}

/// The three persisted preference toggles: advanced mode, high contrast,
/// `ReplayGain`.
fn render_preferences(ui: &mut egui::Ui, state: &mut AppState, frame: &mut eframe::Frame) {
    let mut advanced = state.ui_flags.advanced_mode;
    let advanced_toggle = ui.checkbox(&mut advanced, "Advanced mode").on_hover_text(
        "Reveals power features: tag editing, smart playlists, \
         and extra transport controls (stop, repeat).",
    );
    if advanced_toggle.changed() {
        state.ui_flags.advanced_mode = advanced;
        if let Some(storage) = frame.storage_mut() {
            save_advanced_mode(storage, advanced);
        }
    }
    ui.label(
        egui::RichText::new("Shows tag editing, smart playlists, and stop/repeat controls.").weak(),
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
        if let Some(storage) = frame.storage_mut() {
            save_high_contrast(storage, high_contrast);
        }
    }
    ui.label(
        egui::RichText::new("Near-black background, white text, and bright focus outlines.").weak(),
    );

    let mut replaygain = state.replaygain_enabled;
    let replaygain_toggle = ui
        .checkbox(&mut replaygain, "ReplayGain")
        .on_hover_text("Normalize playback loudness using ReplayGain tags from your files.");
    if replaygain_toggle.changed() {
        state.replaygain_enabled = replaygain;
        if let Some(storage) = frame.storage_mut() {
            save_replaygain(storage, replaygain);
        }
    }
    ui.label(
        egui::RichText::new(
            "Normalize loudness from REPLAYGAIN_TRACK tags; takes effect on the next track.",
        )
        .weak(),
    );
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
