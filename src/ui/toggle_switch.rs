//! The reusable `ToggleSwitch` widget (Issue 11).
//!
//! The mockup's preference rows end in a 36×20 pill (`w-9 h-5`) with a 16px
//! round knob (`w-4 h-4`) inset 2px (`top-0.5 left-0.5`) that slides 16px
//! (`peer-checked:translate-x-4`) when on; the pill fills with the input-well
//! token when off and brand primary when on, and the knob is painted in the
//! on-brand ink.
//!
//! Pure widget seam, exactly like `sidebar.rs` / `playerbar.rs`: paints from
//! [`Palette`] tokens (ADR 0004), mutates nothing, and renders headlessly in
//! `tests/ui_tests.rs` / `tests/golden_tests.rs`. One widget drives every
//! boolean preference — Advanced mode, High contrast, `ReplayGain`.

use eframe::egui;

use super::theme::{self, Palette};

// --- Mockup dimensions ---------------------------------------------------------

/// Pill width (`w-9`): exactly 36px.
pub const TOGGLE_W: f32 = 36.0;

/// Pill height (`h-5`): exactly 20px.
pub const TOGGLE_H: f32 = 20.0;

/// Knob diameter (`w-4 h-4`): exactly 16px.
pub const KNOB_SIZE: f32 = 16.0;

/// Knob inset from the pill edge (`top-0.5 left-0.5`): 2px.
pub const KNOB_INSET: f32 = 2.0;

/// Horizontal knob travel when checked (`peer-checked:translate-x-4`): 16px.
pub const KNOB_TRAVEL: f32 = 16.0;

// --- Token-derived colors --------------------------------------------------------

/// The pill fill: the input-well token (aliases surface-2) when off, brand
/// primary when on — the mockup's `bg-input peer-checked:bg-primary`.
#[must_use]
pub fn pill_color(palette: &Palette, checked: bool) -> egui::Color32 {
    if checked {
        palette.brand_primary
    } else {
        palette.surface_2
    }
}

/// The knob fill: text painted on brand fills, so it reads on both pill
/// states (`bg-primary-foreground`).
#[must_use]
pub fn knob_color(palette: &Palette) -> egui::Color32 {
    palette.on_brand
}

/// The knob center for `checked`: rides the left inset when off, plus the
/// 16px travel when on.
#[must_use]
fn knob_center(pill: egui::Rect, checked: bool) -> egui::Pos2 {
    let x = pill.left() + KNOB_INSET + KNOB_SIZE / 2.0 + if checked { KNOB_TRAVEL } else { 0.0 };
    egui::pos2(x, pill.center().y)
}

// --- Widget -----------------------------------------------------------------------

/// Draw one toggle switch occupying exactly [`TOGGLE_W`] × [`TOGGLE_H`] at
/// the cursor. Returns `true` on click; the caller owns the state change.
///
/// `label` feeds the accessibility tree so assistive tech (and the kittest
/// harness) can find the switch by name despite having no visible text.
pub fn toggle_switch(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: egui::Id,
    label: &str,
    checked: bool,
) -> bool {
    let (pill, _) = ui.allocate_exact_size(egui::vec2(TOGGLE_W, TOGGLE_H), egui::Sense::click());
    toggle_switch_at(ui, palette, id, label, pill, checked)
}

/// Paint one toggle switch into an explicit `pill` rect — the variant
/// hand-laid rows use. Same contract as [`toggle_switch`].
pub fn toggle_switch_at(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: egui::Id,
    label: &str,
    pill: egui::Rect,
    checked: bool,
) -> bool {
    let response = ui.interact(pill, id, egui::Sense::click());
    let painter = ui.painter_at(pill);

    painter.rect_filled(pill, theme::RADIUS_FULL, pill_color(palette, checked));

    // Hover/focus feedback follows the repo's control treatment: the border
    // token idle, the focus ring while hovered.
    let stroke = if response.hovered() {
        egui::Stroke::new(1.0_f32, palette.focus_ring)
    } else {
        egui::Stroke::new(1.0_f32, palette.border)
    };
    painter.rect_stroke(pill, theme::RADIUS_FULL, stroke, egui::StrokeKind::Inside);

    painter.circle_filled(
        knob_center(pill, checked),
        KNOB_SIZE / 2.0,
        knob_color(palette),
    );

    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, checked, label)
    });
    response.clicked()
}
