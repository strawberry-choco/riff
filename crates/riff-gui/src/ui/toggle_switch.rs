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

use super::theme::geometry::toggle::{KNOB_INSET, KNOB_SIZE, KNOB_TRAVEL, TOGGLE_H, TOGGLE_W};
use super::theme::{self, Palette};

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
    let focused = ui.memory(|m| m.has_focus(id));
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

    // The shared keyboard-focus ring, on top and only while the switch holds
    // focus — the same treatment the text buttons and rows draw.
    if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
        painter.rect_stroke(pill, theme::RADIUS_FULL, ring, egui::StrokeKind::Inside);
    }

    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), checked, label)
    });
    response.clicked()
}

/// Paint one shared checkbox box into `rect`: brand fill + a two-stroke
/// checkmark when `checked`, the input-well otherwise. The boolean-control
/// counterpart to the toggle pill — the Watch control and any standalone
/// checkbox render through here so their paint lives in one owner.
pub fn paint_checkbox_box(
    painter: &egui::Painter,
    palette: &Palette,
    rect: egui::Rect,
    checked: bool,
) {
    let side = rect.width();
    painter.rect_filled(
        rect,
        theme::RADIUS_SM,
        if checked {
            palette.brand_primary
        } else {
            palette.surface_2
        },
    );
    painter.rect_stroke(
        rect,
        theme::RADIUS_SM,
        egui::Stroke::new(1.0_f32, palette.border),
        egui::StrokeKind::Inside,
    );
    if checked {
        let a = egui::pos2(rect.left() + side * 0.25, rect.center().y + side * 0.05);
        let b = egui::pos2(rect.left() + side * 0.42, rect.bottom() - side * 0.25);
        let c = egui::pos2(rect.right() - side * 0.2, rect.top() + side * 0.25);
        let check = egui::Stroke::new(1.5_f32, palette.on_brand);
        painter.line_segment([a, b], check);
        painter.line_segment([b, c], check);
    }
}

/// Paint one shared checkbox box plus the keyboard-focus ring on top when
/// `focused` — the treatment the Watch control (and any standalone checkbox)
/// draws, so a focused box rings exactly like a focused toggle pill.
pub fn paint_checkbox_with_focus(
    painter: &egui::Painter,
    palette: &Palette,
    rect: egui::Rect,
    checked: bool,
    focused: bool,
) {
    paint_checkbox_box(painter, palette, rect, checked);
    if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
        painter.rect_stroke(rect, theme::RADIUS_SM, ring, egui::StrokeKind::Inside);
    }
}

/// Register a checkbox's accessible state on its interaction `response`: a
/// [`egui::WidgetType::Checkbox`] carrying `checked` and whether the control is
/// enabled, so assistive tech reads the same selected state the toggle does.
pub fn register_checkbox_a11y(
    response: &egui::Response,
    enabled: bool,
    checked: bool,
    label: &str,
) {
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, checked, label)
    });
}
