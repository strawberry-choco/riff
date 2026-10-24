//! The ghost icon button: an invisible click target with a tinted glyph — the
//! frameless affordance the sidebar's playlist rows and the shell's search
//! clear control share. It wraps the button machinery
//! ([`super::button::begin_icon_button`] / [`super::button::finish_icon_button`]) and moved
//! down here beside it (ADR 0018), so a primitive never reaches into the
//! composite that hosts it.

use egui;

use riff_gui_theme::icons::{Icon, IconCache};
use riff_gui_theme::theme::{self, Palette};

/// A ghost icon button: an invisible click target with a tinted glyph. With
/// `hover_reveal` the glyph appears only while hovered (the mockup's
/// playlist-row affordances); otherwise it is always painted (the top bar's
/// search clear button). The hit area and accessibility label are registered every
/// frame regardless, so assistive tech and the kittest harness can reach it
/// whether or not it is currently painted. The label also shows as a tooltip
/// on hover — icon-only buttons explain themselves (Issue 12).
#[allow(clippy::too_many_arguments)]
pub fn ghost_icon_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    id: egui::Id,
    icon: Icon,
    label: &str,
    hover_reveal: bool,
) -> bool {
    ghost_icon_button_response(ui, cache, palette, rect, id, icon, label, hover_reveal).clicked()
}

/// The same button reporting its [`egui::Response`] rather than a bool, for
/// the hosts that anchor something to the click — a popup menu keyed by the
/// button's own response identity. The response is taken after the button's
/// paint and a11y are finished, so the two entry points are one button, not
/// two that could drift.
#[allow(clippy::too_many_arguments)]
pub fn ghost_icon_button_response(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    id: egui::Id,
    icon: Icon,
    label: &str,
    hover_reveal: bool,
) -> egui::Response {
    let button = super::button::begin_icon_button(ui, rect, id, false);
    // A ghost button has no hover *fill* — hovering only reveals the glyph at
    // full ink — so the press is the first thing it ever paints behind that
    // glyph: a held ghost button wears the framework's active fill, which is
    // what a stock egui button of the same size would wear, and the glyph goes
    // to full ink with it. Press replaces hover here rather than adding to it
    // for the same reason it does everywhere else (rule 4 of the motion rule in
    // `theme`): the two are states, not strengths, and this one is instant.
    let lit = button.hovered || button.pressed;
    if button.pressed {
        ui.painter_at(rect)
            .rect_filled(rect, theme::RADIUS_SM, super::button::active_fill(ui));
    }
    if !hover_reveal || lit {
        let tint = if lit { palette.ink } else { palette.ink_3 };
        let tex_id = cache.texture(ui.ctx(), icon, 16.0, tint);
        ui.painter_at(rect)
            .image(tex_id, rect.shrink(4.0), crate::UV_FULL, tint);
    }
    super::button::finish_icon_button(ui, palette, &button, label);
    button.response
}
