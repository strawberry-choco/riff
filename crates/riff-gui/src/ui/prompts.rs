//! The hand-built modals and inline prompts (golden-image gap audit P1-7).
//!
//! The Edit Tags modal, the Clear Library confirmation, and the playlist
//! create/rename prompts used to be composed inline in [`crate::ui::app`] and
//! [`crate::ui::settings`], where no test could reach their pixels. They live
//! here now as pure widget seams with the same discipline as
//! [`crate::ui::sidebar`] / [`crate::ui::browser`]: every widget paints from
//! [`Palette`] tokens, mutates nothing, and reports a [`PromptOutcome`] the
//! caller applies — so the golden harness can render each of them headlessly.
//!
//! The copy, the widgets, and their geometry are unchanged from the inline
//! originals; only the state plumbing moved out.

use eframe::egui;

use super::app::TagEditState;
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

// --- Edit Tags modal ---------------------------------------------------------------

/// The Edit Tags modal's field column width (a comfortable 280px field).
const TAG_FIELD_W: f32 = 280.0;
/// The two numeric fields (year, track number) are much narrower.
const TAG_NUMERIC_W: f32 = 80.0;

/// The Edit Tags modal: the track's path, one labeled field per editable tag,
/// an error line while one is set, and Save / Cancel (with a spinner while a
/// write is in flight). Writing only ever happens on an explicit Save — the
/// caller owns the commit; Escape and the window's close control report
/// [`PromptOutcome::Cancel`], matching the Cancel button (REQ-UI-007 keyboard
/// navigation).
pub fn tag_edit_modal(
    ctx: &egui::Context,
    palette: &Palette,
    state: &mut TagEditState,
) -> Option<PromptOutcome> {
    let mut open = true;
    let mut save_clicked = false;
    let mut cancel_clicked = false;
    // Escape closes the modal, matching the window close button and Cancel.
    let escape_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));

    egui::Window::new("Edit Tags")
        .id(egui::Id::new("tag_edit_modal"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(state.path.to_string_lossy()).weak());
            ui.separator();
            egui::Grid::new("tag_edit_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Title");
                    ui.add(egui::TextEdit::singleline(&mut state.title).desired_width(TAG_FIELD_W));
                    ui.end_row();
                    ui.label("Artist");
                    ui.add(
                        egui::TextEdit::singleline(&mut state.artist).desired_width(TAG_FIELD_W),
                    );
                    ui.end_row();
                    ui.label("Album");
                    ui.add(egui::TextEdit::singleline(&mut state.album).desired_width(TAG_FIELD_W));
                    ui.end_row();
                    ui.label("Album Artist");
                    ui.add(
                        egui::TextEdit::singleline(&mut state.album_artist)
                            .desired_width(TAG_FIELD_W),
                    );
                    ui.end_row();
                    ui.label("Genre");
                    ui.add(egui::TextEdit::singleline(&mut state.genre).desired_width(TAG_FIELD_W));
                    ui.end_row();
                    ui.label("Year");
                    ui.add(
                        egui::TextEdit::singleline(&mut state.year).desired_width(TAG_NUMERIC_W),
                    );
                    ui.end_row();
                    ui.label("Track Number");
                    ui.add(
                        egui::TextEdit::singleline(&mut state.track_number)
                            .desired_width(TAG_NUMERIC_W),
                    );
                    ui.end_row();
                });

            if let Some(error) = &state.error {
                ui.colored_label(palette.error, error);
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!state.saving, egui::Button::new("Save"))
                    .clicked()
                {
                    save_clicked = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel_clicked = true;
                }
                if state.saving {
                    ui.spinner();
                }
            });
        });

    if !open || cancel_clicked || escape_pressed {
        Some(PromptOutcome::Cancel)
    } else if save_clicked {
        Some(PromptOutcome::Confirm)
    } else {
        None
    }
}
