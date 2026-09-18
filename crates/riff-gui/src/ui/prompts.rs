//! The hand-built modals and inline prompts (golden-image gap audit P1-7).
//!
//! The Clear Library confirmation and the playlist create/rename prompts used
//! to be composed inline in [`crate::ui::app`] and [`crate::ui::settings`],
//! where no test could reach their pixels. They live here now as pure widget
//! seams with the same discipline as [`crate::ui::sidebar`] /
//! [`crate::ui::browser`]: every widget paints from [`Palette`] tokens,
//! mutates nothing, and reports a [`PromptOutcome`] the caller applies — so
//! the golden harness can render each of them headlessly.
//!
//! The copy, the widgets, and their geometry are unchanged from the inline
//! originals; only the state plumbing moved out. (The "Edit Tags" modal was
//! retired: tag editing lives in the Detail Panel's inline editor.)

use eframe::egui;

use super::theme::Palette;

/// What the listener did to a prompt, modal, or confirmation this frame. The
/// caller owns the state change (ADR 0002): the widget never writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    /// The confirming action was clicked (Save / Create / Confirm).
    Confirm,
    /// The prompt was dismissed — Cancel, Escape, or the window's close
    /// control.
    Cancel,
}

// --- Inline text prompts -------------------------------------------------------

/// One inline text prompt: the name field over a confirm/cancel pair.
/// `confirm_label` is the confirming button's copy (`Create` / `Save`); the
/// visible text doubles as the accessibility label.
fn text_prompt(
    ui: &mut egui::Ui,
    draft: &mut String,
    confirm_label: &str,
) -> Option<PromptOutcome> {
    let mut confirmed = false;
    let mut cancelled = false;
    ui.horizontal(|ui| {
        ui.text_edit_singleline(draft);
        if ui.button(confirm_label).clicked() {
            confirmed = true;
        }
        if ui.button("Cancel").clicked() {
            cancelled = true;
        }
    });
    if confirmed {
        Some(PromptOutcome::Confirm)
    } else if cancelled {
        Some(PromptOutcome::Cancel)
    } else {
        None
    }
}

/// The inline "New Playlist" name prompt: a name field over Create / Cancel.
/// `draft` is the caller-owned name buffer.
pub fn playlist_create_prompt(ui: &mut egui::Ui, draft: &mut String) -> Option<PromptOutcome> {
    text_prompt(ui, draft, "Create")
}

/// The inline playlist rename prompt: the same shape as
/// [`playlist_create_prompt`] with the rename's `Save` confirmation.
pub fn playlist_rename_prompt(ui: &mut egui::Ui, draft: &mut String) -> Option<PromptOutcome> {
    text_prompt(ui, draft, "Save")
}

// --- Destructive confirmation -----------------------------------------------------

/// The confirmation copy beside the destructive Clear Library action.
pub const CLEAR_LIBRARY_CONFIRM_COPY: &str =
    "Remove every indexed track? Playlists and settings are kept.";

/// The inline confirmation for the destructive Clear Library action, rendered
/// beneath the stage until confirmed or cancelled. The warning line carries
/// the palette's warning token — the destructive copy's
/// `--riff-state-warning`.
pub fn clear_library_confirm(ui: &mut egui::Ui, palette: &Palette) -> Option<PromptOutcome> {
    let mut confirmed = false;
    let mut cancelled = false;
    ui.add_space(8.0);
    ui.label(egui::RichText::new(CLEAR_LIBRARY_CONFIRM_COPY).color(palette.warning));
    ui.horizontal(|ui| {
        if ui.button("Confirm").clicked() {
            confirmed = true;
        }
        if ui.button("Cancel").clicked() {
            cancelled = true;
        }
    });
    if confirmed {
        Some(PromptOutcome::Confirm)
    } else if cancelled {
        Some(PromptOutcome::Cancel)
    } else {
        None
    }
}
