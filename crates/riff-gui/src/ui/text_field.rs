//! The shared single-line text field.
//!
//! One owner for what every typed field in the shell does the same way: paint
//! the rounded input well (hairline when idle, the search focus ring when the
//! field holds keyboard focus), lay out a leading glyph beside a frameless
//! [`egui::TextEdit`] riding a hint string, offer a clear affordance while the
//! value is non-empty, and dismiss on Escape (clear the value and surrender
//! focus, so a keyboard user can operate the field end to end).
//!
//! The field edits a caller-owned `value` buffer in place and returns the text
//! edit's response so the host keeps the shortcuts it owns — the Titlebar
//! drives its Ctrl+K request-focus from that response. This is the widget seam
//! the Titlebar "Search or jump to…" field proves end to end (see
//! [`crate::ui::chrome`]); the later prompt, Settings, and Inline Tag Editor
//! surfaces reuse the same contract.
//!
//! Pure widget seam: it paints from [`Palette`] tokens, mutates only the
//! `value` buffer it is handed, and reads/writes egui focus memory — never
//! application state.

use eframe::egui;

use super::icons::{Icon, IconCache};
use super::sidebar::{ghost_icon_button, search_ring_stroke};
use super::theme::{self, Palette};

/// The geometry and copy of one shared text field. The caller allocates `rect`
/// (the well) so it controls placement and height; the field reserves a fixed
/// inner inset for its glyph, text, and clear affordance.
///
/// The glyph and the clear affordance are optional because the same well serves
/// both search boxes (a magnifier, and clearing is the point) and plain data
/// fields — the Inline Tag Editor's rows, which edit a draft buffer the host can
/// discard wholesale, so Escape belongs to the host rather than the field.
pub struct TextField<'a> {
    /// Stable widget id — the focus target and the clear button's parent id.
    pub id: egui::Id,
    /// The well's rect.
    pub rect: egui::Rect,
    /// Placeholder shown while the value is empty.
    pub hint: &'a str,
    /// The leading 16px glyph painted at the well's left.
    pub leading_icon: Option<Icon>,
    /// The clear affordance's accessible name and tooltip; `None` offers no way
    /// to empty the field from inside it.
    pub clear_label: Option<&'a str>,
    /// Whether Escape clears the value and surrenders focus (`true`, a search
    /// box) or is left to whoever owns the surrounding flow (`false`).
    pub dismiss_on_escape: bool,
}

/// Render one shared text field over `value`, editing it in place, and return
/// the [`egui::TextEdit`] response for the host's own shortcuts. A focused
/// field paints the search ring; Escape while focused clears `value` and gives
/// up focus; a non-empty `value` shows the clear affordance.
pub fn text_field(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    value: &mut String,
    field: &TextField,
) -> egui::Response {
    // Read focus BEFORE painting so the ring lands on the same frame the field
    // gains focus (sidebar precedent).
    let focused = ui.memory(|m| m.has_focus(field.id));
    paint_well(ui, palette, field.rect, focused);

    let inner = field.rect.shrink2(egui::vec2(10.0_f32, 4.0_f32));
    let response = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_MD;

                if let Some(icon) = field.leading_icon {
                    let tex_id = cache.texture(ui.ctx(), icon, 16.0, palette.ink_3);
                    let sized = egui::load::SizedTexture::new(tex_id, egui::vec2(16.0, 16.0));
                    ui.add(egui::Image::from_texture(sized));
                }

                let response = ui.add(
                    egui::TextEdit::singleline(value)
                        .id(field.id)
                        .frame(egui::Frame::NONE)
                        .hint_text(field.hint)
                        .desired_width(ui.available_width() - 20.0),
                );

                if let Some(clear_label) = field.clear_label
                    && !value.is_empty()
                {
                    let clear_rect = egui::Rect::from_center_size(
                        egui::pos2(inner.right() - 10.0, field.rect.center().y),
                        egui::vec2(20.0, field.rect.height() - 8.0),
                    );
                    if ghost_icon_button(
                        ui,
                        cache,
                        palette,
                        clear_rect,
                        field.id.with("clear"),
                        Icon::Close,
                        clear_label,
                        false,
                    ) {
                        value.clear();
                    }
                }

                response
            },
        )
        .inner;

    if field.dismiss_on_escape {
        dismiss_on_escape(ui, field.id, focused, value);
    }
    response
}

/// The rounded input well behind the field: surface-2 fill with the sidebar
/// search's ring border — hairline when idle, focus ring when focused.
fn paint_well(ui: &egui::Ui, palette: &Palette, rect: egui::Rect, focused: bool) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_2);
    painter.rect_stroke(
        rect,
        theme::RADIUS_MD,
        search_ring_stroke(palette, focused),
        egui::StrokeKind::Inside,
    );
}

/// Keyboard dismissal (REQ-UI-007 parity): while the field has focus, Escape
/// clears the value and gives the focus back, so a keyboard user can operate —
/// and dismiss — the field entirely from the keyboard.
///
/// The gate is *last frame's* focus, not this frame's: egui itself clears
/// keyboard focus during pass begin when Escape is pressed, so by the time
/// widget code runs on the Escape frame the field no longer reports focus.
fn dismiss_on_escape(ui: &egui::Ui, id: egui::Id, focused: bool, value: &mut String) {
    let focus_key = id.with("had_focus");
    let had_focus = focused || ui.memory(|m| m.data.get_temp::<bool>(focus_key).unwrap_or(false));
    ui.memory_mut(|m| m.data.insert_temp(focus_key, focused));
    if had_focus && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        value.clear();
        ui.memory_mut(|m| m.surrender_focus(id));
    }
}
