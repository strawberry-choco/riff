//! The host's Library Path adapter (component-layer issue 16).
//!
//! The Settings modal and the sidebar's Add Library control are two entry
//! points on one flow: choose a directory, make it a Library Path fact, and let
//! the listener decide when to index it. Everything that touches the platform —
//! the native folder dialog, the filesystem probe, the autocomplete listing —
//! and the application (the session's registered roots, the durable settings
//! row) lives here. [`crate::ui::settings::path_input`] only draws what this
//! resolves and reports what the listener chose, which is what lets the Linux
//! text flow and the native dialog share one set of rules.

use std::path::PathBuf;

#[cfg(target_os = "linux")]
use eframe::egui;

use super::super::settings;
#[cfg(target_os = "linux")]
use super::super::settings::PathInputAction;
use riff_backend::app::state::LibrarySession;
use riff_backend::app::store::SettingsStore;

/// Accept one typed candidate as a Library Path.
///
/// `~` expands by the same rule the autocomplete uses, so a path the input
/// offered is a path the input can register. A candidate that is not there, or
/// is not a directory, is refused with the sentence the input then shows —
/// nothing registers and nothing writes. A registered root starts no scan and
/// no watcher (indexing waits for the listener), and a root the session already
/// carries registers once, so picking the same folder twice writes once.
pub fn register_library_path(
    candidate: &str,
    library: &mut LibrarySession,
    settings: &mut dyn SettingsStore,
) -> Result<PathBuf, String> {
    let path = settings::expand_tilde(candidate);
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", path.display()));
    }
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    library.library_paths.register(canonical.clone(), settings);
    Ok(canonical)
}

impl super::RiffApp {
    /// Register a new library root through the platform picker: the native
    /// folder dialog everywhere except Linux, which opens the text-input row
    /// rendered beneath the stage. Also called from the sidebar footer
    /// (design-handoff issue 07) and the Settings modal's Add Library action, so
    /// both entry points share one flow.
    pub(crate) fn add_library_via_platform_picker(&mut self, library: &mut LibrarySession) {
        #[cfg(not(target_os = "linux"))]
        {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Add Music Library")
                .pick_folder()
            {
                library
                    .library_paths
                    .register(path, self.settings_store.as_mut());
            }
        }
        #[cfg(target_os = "linux")]
        {
            let _ = library;
            self.settings_show_input = true;
            self.settings_path_error = None;
        }
    }

    /// The Linux text-path flow: draw what the input reports, then do the
    /// filesystem and application work the drawing layer may not. The candidate
    /// goes through [`register_library_path`], the same acceptance the native
    /// dialog's result is.
    #[cfg(target_os = "linux")]
    pub(crate) fn render_library_path_input(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
    ) {
        if !self.settings_show_input {
            return;
        }
        let suggestions = settings::suggest_directories(&self.settings_text_input, 8);
        let palette = self.theme.active;
        let actions = {
            let mut input = settings::PathInput {
                text: &mut self.settings_text_input,
                error: self.settings_path_error.as_deref(),
                suggestions: &suggestions,
            };
            settings::path_input(ui, &palette, &mut input)
        };
        for action in actions {
            match action {
                PathInputAction::Confirm => {
                    let candidate = self.settings_text_input.clone();
                    match register_library_path(&candidate, library, self.settings_store.as_mut()) {
                        Ok(_) => self.close_library_path_input(),
                        Err(error) => self.settings_path_error = Some(error),
                    }
                }
                PathInputAction::Cancel => self.close_library_path_input(),
                // Taking a suggestion fills the field with the directory itself,
                // so the next frame lists its children.
                PathInputAction::Complete(path) => {
                    self.settings_text_input = format!("{}/", path.to_string_lossy());
                    self.settings_path_error = None;
                }
            }
        }
    }

    /// Abandon or complete the Linux text flow: the draft and any rejection go
    /// with it, and no Library Path fact changes.
    #[cfg(target_os = "linux")]
    fn close_library_path_input(&mut self) {
        self.settings_text_input.clear();
        self.settings_show_input = false;
        self.settings_path_error = None;
    }
}
