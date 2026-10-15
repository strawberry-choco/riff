//! The host's Library Path adapter (component-layer issue 16).
//!
//! The Settings modal, the sidebar's Add Library control, and a folder dropped
//! onto the window are three entry points on one flow: hand over a directory,
//! make it a Library Path fact, and let the listener decide when to index it —
//! with the one deliberate divergence: a drop is an impatient gesture, so each
//! *newly registered* root's scan request leaves with the drop, while the
//! picker and the typed path still register without scanning. Everything that
//! touches the platform — the native folder dialog, the filesystem probe, the
//! autocomplete listing — and the application (the session's registered roots,
//! the durable settings row) lives here. [`crate::ui::settings::path_input`]
//! only draws what this resolves and reports what the listener chose, which is
//! what lets the Linux text flow, the native dialog, and the drop share one
//! set of rules.

use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use eframe::egui;

use super::super::feedback::FeedbackBoard;
use super::super::settings;
#[cfg(target_os = "linux")]
use super::super::settings::PathInputAction;
use riff_backend::app::MutexExt;
use riff_backend::app::scan_service::Scans;
use riff_backend::app::state::LibrarySession;
use riff_backend::app::store::SettingsStore;

/// The acceptance judgment every entry point shares: the candidate must exist
/// and be a directory, and the answer is its canonical form or the standard
/// refusal sentence. Registering is the caller's next step, because the drop
/// flow has one more question to ask first — whether the root is already here.
fn judge_candidate(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", path.display()));
    }
    Ok(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

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
    let canonical = judge_candidate(&settings::expand_tilde(candidate))?;
    library.library_paths.register(canonical.clone(), settings);
    Ok(canonical)
}

/// What one drop resolved to, in drop order: the canonical roots it newly
/// registered — each of which already got its scan request — and the refusal
/// sentence each rejected candidate was reported with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DropResolution {
    /// The canonical roots newly registered by this drop, in drop order.
    pub registered: Vec<PathBuf>,
    /// The sentence reported for each refused candidate, in drop order.
    pub rejections: Vec<String>,
}

/// Resolve one drop's candidates through the same acceptance every entry
/// point shares. The drop is the entry point that indexes immediately: each
/// newly registered root's scan request leaves here, at drop time, while an
/// already-registered root is a silent no-op — the session's own dedupe keeps
/// the write from repeating, and this judgment keeps a re-drop from ever
/// triggering a surprise rescan. A refused candidate stops nothing: the rest
/// of the drop still registers.
///
/// The report rides the structured board on the Library notice — the
/// titlebar's scan slot — rejections first, then the one success line, so an
/// all-rejected drop leaves the standard refusal standing and a drop that
/// registered anything leaves "Added library path, scanning…" for the live
/// scan progress to overwrite moments later. One slot, no new surface.
pub fn resolve_dropped_paths(
    candidates: &[PathBuf],
    library: &mut LibrarySession,
    settings: &mut dyn SettingsStore,
    scans: &dyn Scans,
    feedback: &mut FeedbackBoard,
) -> DropResolution {
    use riff_backend::app::events::{NoticeSeverity, NoticeSource};

    let mut resolution = DropResolution::default();
    for candidate in candidates {
        match judge_candidate(candidate) {
            Err(refusal) => {
                feedback.set(
                    NoticeSource::Library,
                    refusal.clone(),
                    NoticeSeverity::Warning,
                );
                resolution.rejections.push(refusal);
            }
            Ok(canonical) => {
                // The adapter's own dedupe judgment: `register` dedupes the
                // write, but the scan request must not ride a re-drop.
                if library.library_paths.paths().contains(&canonical) {
                    continue;
                }
                library.library_paths.register(canonical.clone(), settings);
                scans.request(canonical.clone());
                resolution.registered.push(canonical);
            }
        }
    }
    if !resolution.registered.is_empty() {
        feedback.set(
            NoticeSource::Library,
            "Added library path, scanning\u{2026}",
            NoticeSeverity::Info,
        );
    }
    resolution
}

impl super::RiffApp {
    /// Enact the frame's drop decision: lock the Library Session once — the
    /// frame's own guard is already dropped by the time the tail runs — and
    /// hand every candidate to [`resolve_dropped_paths`], the one operation
    /// the flow's application work lives in.
    pub(crate) fn enact_dropped_library_paths(&mut self, candidates: &[PathBuf]) {
        let library_arc = self.library.clone();
        let mut guard = library_arc.lock_or_recover();
        resolve_dropped_paths(
            candidates,
            &mut guard,
            self.settings_store.as_mut(),
            self.scans.as_ref(),
            &mut self.feedback,
        );
    }

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
