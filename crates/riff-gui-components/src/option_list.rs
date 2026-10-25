//! The inline option-list selector — the component library's one generic
//! single-choice control (Smart Playlists ticket 06, ADR 0018).
//!
//! Fed `(label, key)` pairs, the current key and an identity, it draws every
//! option inline, in whatever Ui it is handed, and reports the clicked key to
//! the caller. A host pane draws it in its own layout flow; the rule editor
//! draws it inside an `egui::Popup::menu`, which is how the row's field,
//! comparison, yes/no and Library-value choices float beside the chips that
//! open them — the very pattern the track-sort control in [`crate::menu`]
//! ships, and one this project's test suites click through with real
//! pointers (`tests/ui_tests.rs`: the sort control, the date picker, and the
//! rule row's own lists), so a popup-hosted list is held to the same
//! reachability as an inline one.
//!
//! The contract is the crate's standing props-in / typed-response-out one:
//! [`option_list`] mutates nothing — not the palette, not the options, not
//! the current key — and returns the chosen key (if any) for the caller to
//! apply. Every design value is read from the design authority (ADR 0004):
//! geometry from `theme::geometry::option_list`, colour from the [`Palette`]
//! prop.
//!
//! Overflow is defined here rather than discovered in the field: a row is
//! exactly the width its host gives it and a too-long label is truncated to
//! the row's text budget, so nothing escapes the host horizontally; a deep
//! list's vertical growth belongs to the host — a pane's own scroll, or the
//! popup's — and a deep tail stays reachable through it (pinned by the
//! crate's probe suite, `tests/option_list_tests.rs`).

use egui;

use riff_gui_theme::theme::geometry::option_list::{
    BAND_SHRINK_X, MARK_INSET_Y, MARK_W, ROW_GAP, ROW_H, TEXT_END_PAD, TEXT_X,
};
use riff_gui_theme::theme::{self, Palette};

/// One `(label, key)` option offered to [`option_list`].
///
/// The selector never interprets the key — it compares it against `current`
/// to decide which row to mark, and clones it out of the report when its row
/// is clicked. That is the whole extent of what this crate knows about the
/// caller's vocabulary: a Matchable Field id, an operator id and a genre
/// string are all the same shape from here.
#[derive(Debug, Clone)]
pub struct OptionChoice<'a, K> {
    /// The value reported when this choice is clicked.
    pub key: K,
    /// The label painted on the row, and the accessibility name the harness
    /// and the a11y tree address the option by.
    pub label: &'a str,
}

/// The props of one [`option_list`] instance.
pub struct OptionList<'a, K> {
    /// This instance's identity, supplied by the caller — the same
    /// explicit-`egui::Id` contract `toggle_switch` and [`TextField`]
    /// keep. Each option's widget id derives from it
    /// (`id.with(("option", index))`), so two selectors can share one pane
    /// as long as the caller gives them two different ids: their rows
    /// cannot collide even when the same key appears in both lists.
    ///
    /// [`TextField`]: crate::text_field::TextField
    pub id: egui::Id,
    /// The options, drawn inline in this order.
    pub options: &'a [OptionChoice<'a, K>],
    /// The current choice. Its row carries the visible mark and the a11y
    /// selected state. `None` marks nothing — the draft case, where nothing
    /// has been chosen yet.
    pub current: Option<&'a K>,
}

/// Draw a generic single-choice option list inline in the pane's layout
/// flow, and report the clicked option's key.
///
/// Returns the key of the option the user clicked this frame — including a
/// click on the row already current, which is the caller's no-op to decide
/// about; the widget never writes `current` itself (the caller owns the
/// state change, ADR 0002). Returns `None` when nothing was clicked, and
/// draws nothing for an empty list.
///
/// Each row is a real control of its own: the layout band is allocated
/// full-width with [`egui::Sense::hover`] and the hit area is a separate
/// `ui.interact` on a fresh derived id with [`egui::Sense::click`] — the
/// recipe the Settings nav rail proves clickable in production. A click
/// sense also puts the row in the keyboard focus order, and egui reports a
/// Space/Enter press on the focused row through the same `clicked()` — so
/// the list is Tab-reachable end to end, not mouse-only.
#[must_use]
pub fn option_list<K>(ui: &mut egui::Ui, palette: &Palette, list: &OptionList<'_, K>) -> Option<K>
where
    K: Clone + PartialEq,
{
    let mut chosen = None;
    ui.vertical(|ui| {
        // Zero egui's own gaps so the stack's rhythm is this module's
        // tokens alone; opting out of the shared layout is structure, not
        // a spacing choice.
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        let row_width = ui.available_width();
        let text_width = (row_width - TEXT_X - TEXT_END_PAD).max(0.0);
        let font_id = theme::styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
        for (index, choice) in list.options.iter().enumerate() {
            if index > 0 {
                ui.add_space(ROW_GAP);
            }
            draw_row(
                ui,
                palette,
                list,
                choice,
                index,
                &font_id,
                row_width,
                text_width,
                &mut chosen,
            );
        }
    });
    chosen
}

/// Draw one option row: layout band, hit area, mark, label, a11y state.
#[allow(
    clippy::too_many_arguments,
    reason = "the instance props threaded to one row; the row owns no state of its own"
)]
fn draw_row<K>(
    ui: &mut egui::Ui,
    palette: &Palette,
    list: &OptionList<'_, K>,
    choice: &OptionChoice<'_, K>,
    index: usize,
    font_id: &egui::FontId,
    row_width: f32,
    text_width: f32,
    chosen: &mut Option<K>,
) where
    K: Clone + PartialEq,
{
    let selected = list.current.is_some_and(|current| *current == choice.key);
    let color = if selected { palette.ink } else { palette.ink_2 };

    // Measure first: the row is one text line tall, so a label too long for
    // the pane is truncated with an ellipsis at the text budget instead of
    // widening or escaping the row (the horizontal-overflow contract).
    let mut job =
        egui::text::LayoutJob::simple_singleline(choice.label.to_owned(), font_id.clone(), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
    let galley = ui.painter().layout_job(job);

    let (rect, _) = ui.allocate_exact_size(egui::vec2(row_width, ROW_H), egui::Sense::hover());
    let row_id = list.id.with(("option", index));
    let response = ui.interact(rect, row_id, egui::Sense::click());
    let focused = ui.memory(|mem| mem.has_focus(row_id));
    let painter = ui.painter_at(rect);

    // The current choice wears the nav rail's mark: a surface-2 band with a
    // full-round brand bar at its edge; hover alone is the wash.
    let band = rect.shrink2(egui::vec2(BAND_SHRINK_X, 0.0));
    if selected {
        painter.rect_filled(band, theme::RADIUS_MD, palette.surface_2);
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(band.left(), rect.top() + MARK_INSET_Y),
                egui::pos2(band.left() + MARK_W, rect.bottom() - MARK_INSET_Y),
            ),
            theme::RADIUS_FULL,
            palette.brand_primary,
        );
    } else if response.hovered() {
        painter.rect_filled(band, theme::RADIUS_MD, palette.row_hover);
    }
    if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
        painter.rect_stroke(band, theme::RADIUS_MD, ring, egui::StrokeKind::Inside);
    }

    painter.galley(
        egui::pos2(
            rect.left() + TEXT_X,
            rect.top() + (ROW_H - galley.size().y) / 2.0,
        ),
        galley,
        color,
    );

    // A hand-painted rectangle is not a control until it says its own name:
    // the a11y registration is what lets the harness, the a11y tree and
    // assistive tech address this row by its label and read the mark. The
    // role is a radio because this is a single-choice group — egui's own
    // `RadioButton` reports its state through the very same `selected`
    // channel, which lands in AccessKit as `Toggled`.
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            ui.is_enabled(),
            selected,
            choice.label,
        )
    });

    if response.clicked() {
        *chosen = Some(choice.key.clone());
    }
}
