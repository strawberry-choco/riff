//! The one prompt and confirmation surface (golden-image gap audit P1-7;
//! component-layer issue 14).
//!
//! The Clear Library confirmation and the playlist create/rename prompts used
//! to be composed inline in [`crate::ui::app`] and [`crate::ui::settings`],
//! where no test could reach their pixels. They live here now as pure widget
//! seams with the same discipline as [`crate::ui::sidebar`] /
//! [`crate::ui::browser`]: every widget paints from [`Palette`] tokens,
//! mutates nothing, and reports a [`PromptOutcome`] the caller applies — so
//! the golden harness can render each of them headlessly.
//!
//! What the surface owns is the *interaction contract*, identical on every
//! prompt: the name field takes keyboard focus the frame it opens, so a
//! listener can type at once; Enter means the same thing as the confirming
//! button; and Cancel, Escape, or a click outside the prompt dismisses it.
//! Dismissal is always reported as [`PromptOutcome::Cancel`], never applied
//! here — nothing a prompt can do reaches the durable store, because the state
//! it edits is the caller's draft buffer and the write belongs to whoever holds
//! the ports ([ADR 0002](docs/adr/0002)).
//!
//! (The "Edit Tags" modal was retired: tag editing lives in the Detail Panel's
//! inline editor.)

use eframe::egui;

use super::button;
use super::icons::IconCache;
use super::theme::Palette;

/// What the listener did to a prompt, modal, or confirmation this frame. The
/// caller owns the state change (ADR 0002): the widget never writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    /// The confirming action was clicked (Save / Create / Confirm), or Enter
    /// was pressed in the field.
    Confirm,
    /// The prompt was dismissed — Cancel, Escape, or a click outside it.
    Cancel,
}

// --- Inline text prompts -------------------------------------------------------

/// A prompt's identity and copy. `id` is the stable widget id the interaction
/// plumbing keys its one-shot focus and Escape tracking on; it must be derived
/// from the surface's own id stack so two prompts never share it.
pub struct TextPrompt<'a> {
    pub id: egui::Id,
    /// Placeholder while the draft is empty.
    pub hint: &'a str,
    /// The confirming button's copy (`Create` / `Save`).
    pub confirm_label: &'a str,
}

/// The shared prompt mechanics over a caller-owned draft buffer: initial
/// focus, Enter-to-confirm, Escape/outside-click dismissal, and the
/// confirm/cancel pair, reported as one [`PromptOutcome`].
fn text_prompt(
    ui: &mut egui::Ui,
    draft: &mut String,
    prompt: &TextPrompt<'_>,
) -> Option<PromptOutcome> {
    // The prompt's first frame is the one that claims focus. The marker is
    // dropped with each reported outcome, so the next opening is fresh again.
    let open = prompt.id.with("prompt_open");
    let fresh = ui.memory(|m| m.data.get_temp::<bool>(open).is_none());
    ui.memory_mut(|m| m.data.insert_temp(open, true));

    let top = ui.cursor().min;
    let mut outcome = None;
    let field = ui
        .horizontal(|ui| {
            let field = ui.text_edit_singleline(draft);
            if ui.button(prompt.confirm_label).clicked() {
                outcome = Some(PromptOutcome::Confirm);
            }
            if ui.button("Cancel").clicked() {
                outcome = Some(PromptOutcome::Cancel);
            }
            field
        })
        .inner;
    if fresh {
        field.request_focus();
    }

    // Enter in the field is the confirming button. A single-line edit releases
    // focus as it takes the Enter, so the pair of facts identifies the frame.
    if outcome.is_none() && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(PromptOutcome::Confirm);
    }

    // Escape dismisses while the field holds — or has just released — focus:
    // egui drops focus itself during pass-begin on the Escape frame, so last
    // frame's focus counts too (the shared text field's precedent).
    let focused = prompt.id.with("field_focus");
    let had_focus = field.has_focus()
        || ui
            .memory(|m| m.data.get_temp::<bool>(focused))
            .unwrap_or(false);
    let field_focused = field.has_focus();
    ui.memory_mut(|m| m.data.insert_temp(focused, field_focused));
    if outcome.is_none() && had_focus && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Some(PromptOutcome::Cancel);
    }

    // A click anywhere outside the prompt's own band dismisses it. The band is
    // the row the prompt just laid out, so the rows above and below it keep
    // their clicks; the opening frame is exempt, because the pointer click
    // that opened the prompt is still this frame's click.
    let band =
        egui::Rect::from_min_max(top, egui::pos2(ui.max_rect().right(), ui.min_rect().max.y));
    let area = ui.interact(band, prompt.id.with("prompt_area"), egui::Sense::click());
    if outcome.is_none() && !fresh && !area.hovered() && ui.input(|i| i.pointer.any_click()) {
        outcome = Some(PromptOutcome::Cancel);
    }

    if outcome.is_some() {
        ui.memory_mut(|m| {
            m.data.remove::<bool>(open);
            m.data.remove::<bool>(focused);
        });
    }
    outcome
}

/// The inline "New Playlist" name prompt: a name field over Create / Cancel.
/// `draft` is the caller-owned name buffer.
pub fn playlist_create_prompt(ui: &mut egui::Ui, draft: &mut String) -> Option<PromptOutcome> {
    text_prompt(
        ui,
        draft,
        &TextPrompt {
            id: ui.id().with("playlist_create_prompt"),
            hint: "Playlist name",
            confirm_label: "Create",
        },
    )
}

/// The inline playlist rename prompt: the same shape as
/// [`playlist_create_prompt`] with the rename's `Save` confirmation.
pub fn playlist_rename_prompt(ui: &mut egui::Ui, draft: &mut String) -> Option<PromptOutcome> {
    text_prompt(
        ui,
        draft,
        &TextPrompt {
            id: ui.id().with("playlist_rename_prompt"),
            hint: "Playlist name",
            confirm_label: "Save",
        },
    )
}

// --- Destructive confirmation -----------------------------------------------------

/// The confirmation copy beside the destructive Clear Library action. It says
/// what the wipe does, what it keeps, and how the collection comes back, so the
/// listener is not left to infer whether Playlists and Settings survive or
/// whether the library is gone for good.
///
/// The recovery clause is the substance the footer used to carry in a note of
/// its own ("rebuild it on the next scan"). That note went when the in-pane
/// clear row became the page footer, whose single note is the generic
/// immediate-apply line — so the consequence explanation lives here, beside the
/// action it belongs to, rather than being lost.
pub const CLEAR_LIBRARY_CONFIRM_COPY: &str = "Remove every indexed track? Playlists and settings are kept, and the collection \
     rebuilds on the next scan.";

/// The confirming action's copy, distinct from the plain "Cancel" beside it.
pub const CLEAR_LIBRARY_CONFIRM_LABEL: &str = "Confirm";

/// The inline confirmation for the destructive Clear Library action, rendered
/// beneath the stage until confirmed or cancelled. The warning line carries
/// the palette's warning token — the destructive copy's `--riff-state-warning`
/// — and the affirmative action paints through the shared
/// [`button::Variant::Destructive`], so it is recognizably the dangerous one
/// and never a brand-filled primary.
pub fn clear_library_confirm(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
) -> Option<PromptOutcome> {
    let mut confirmed = false;
    let mut cancelled = false;
    ui.add_space(8.0);
    ui.label(egui::RichText::new(CLEAR_LIBRARY_CONFIRM_COPY).color(palette.warning));
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            button::text_button_size(ui, palette, CLEAR_LIBRARY_CONFIRM_LABEL, false),
            egui::Sense::hover(),
        );
        if button::text_button(
            ui,
            cache,
            palette,
            &button::TextButton {
                id: egui::Id::new("clear_library_confirm_action"),
                rect,
                label: CLEAR_LIBRARY_CONFIRM_LABEL,
                a11y: CLEAR_LIBRARY_CONFIRM_LABEL,
                tooltip: None,
                icon: None,
                small: false,
                variant: button::Variant::Destructive,
                enabled: true,
            },
        ) {
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

/// What the row says before the listener commits. The recovery clause is the
/// substance: it says what a clear *costs*, because the action has no automatic
/// counterpart — there is no eviction, so this is the only reclaim there is.
pub const CLEAR_THUMBNAIL_CACHE_CONFIRM_COPY: &str = "Delete every cached cover thumbnail? Each album's artwork is read and decoded again the next time it is shown.";

/// The action's own label, kept distinct from "Clear Library" — the two sit in the
/// same pane and wipe very different things.
pub const CLEAR_THUMBNAIL_CACHE_LABEL: &str = "Clear Thumbnail cache";

/// The inline confirmation for the destructive Clear Thumbnail cache action,
/// rendered beneath the stage until confirmed or cancelled. Same shape and same
/// tokens as [`clear_library_confirm`]: the warning line in the palette's warning
/// colour, the affirmative action in [`button::Variant::Destructive`].
///
/// Deleting the rungs is not a data-loss event — the sources are untouched and the
/// cache rebuilds as you browse — but it is irreversible in the moment and slow to
/// undo, so it asks first like every other destructive row here.
pub fn clear_thumbnail_cache_confirm(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
) -> Option<PromptOutcome> {
    let mut confirmed = false;
    let mut cancelled = false;
    ui.add_space(8.0);
    ui.label(egui::RichText::new(CLEAR_THUMBNAIL_CACHE_CONFIRM_COPY).color(palette.warning));
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            button::text_button_size(ui, palette, CLEAR_LIBRARY_CONFIRM_LABEL, false),
            egui::Sense::hover(),
        );
        if button::text_button(
            ui,
            cache,
            palette,
            &button::TextButton {
                id: egui::Id::new("clear_thumbnail_cache_confirm_action"),
                rect,
                label: CLEAR_LIBRARY_CONFIRM_LABEL,
                a11y: CLEAR_LIBRARY_CONFIRM_LABEL,
                tooltip: None,
                icon: None,
                small: false,
                variant: button::Variant::Destructive,
                enabled: true,
            },
        ) {
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
