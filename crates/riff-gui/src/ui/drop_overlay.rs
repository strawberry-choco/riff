//! The drop-hover veil: what the whole window wears while a platform file
//! drag hovers it, so the drop target reads as the window, not one panel.
//!
//! Draw-half paint, deliberately nothing more. The one fact it reads is
//! egui's own `hovered_files`, which egui-winit fills while a drag hovers the
//! window and clears the moment the drag is dropped, cancelled, or leaves it
//! (`egui-winit`'s `HoveredFile` / `HoveredFileCancelled` / `DroppedFile`
//! handling) — so showing the veil while that list is non-empty IS the whole
//! lifecycle, and the vanish on leave or cancel is next-frame free. The
//! overlay names no payload decision — no per-frame is-directory probing, no
//! frame-decision half — and stores nothing between frames: it appears for
//! any hovered payload and can leave no stale target behind.
//!
//! Every value it paints with is a token: the border's geometry in
//! [`theme::geometry::drop_overlay`], the veil's and dash's colours in
//! [`theme::drop_hover_veil`] and [`theme::drop_hover_dash`], the label in
//! [`theme::TEXT_XL`] at the installed [`egui::TextStyle::Heading`] family
//! over [`Palette::ink`]. The paint lands on a top [`egui::Order::Tooltip`]
//! layer over the whole content rect — above every panel, which egui paints
//! at [`egui::Order::Middle`] — straight onto the layer with no `Ui`, so it
//! can neither take a hit nor disturb the layout beneath it. Verified
//! manually, per the drop-folder spec's testing decision: a golden would need
//! headless hover-input injection, which the pin on egui 0.35 does not offer
//! today.

use eframe::egui;
use riff_gui_theme::theme::{self, Palette};

/// The overlay's copy, centered in the window.
const LABEL: &str = "Drop folder to add to library";

/// The layer the veil paints on, and the one [`egui::Id`] it is known by: a
/// fixed name, because the veil is a singleton — one window, one drag.
const VEIL_LAYER: &str = "drop_hover_veil";

/// Paint the drop-hover veil, if a platform file drag is hovering the window.
///
/// Called once per frame at the very end of
/// [`crate::ui::app::RiffApp`]'s [`eframe::App::ui`], after every panel has
/// drawn and the frame's palette is resolved: painting last is what lets a
/// single top-layer painter sit above all of them.
pub fn paint(ctx: &egui::Context, palette: &Palette) {
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if !hovering {
        return;
    }

    let window = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new(VEIL_LAYER),
    ));

    // The veil covers the whole window; the dashed border rides the inset
    // inside it, one edge at a time so every corner is met by a dash that
    // starts on it — a single closed path would cut a corner whenever a dash
    // straddles it, because the dasher walks straight through polyline
    // vertices.
    painter.rect_filled(
        window,
        egui::CornerRadius::ZERO,
        theme::drop_hover_veil(palette),
    );

    let border = window.shrink(theme::geometry::drop_overlay::INSET);
    let stroke = egui::Stroke::new(
        theme::geometry::drop_overlay::BORDER_W,
        theme::drop_hover_dash(palette),
    );
    let (dash, gap) = (
        theme::geometry::drop_overlay::DASH,
        theme::geometry::drop_overlay::GAP,
    );
    let mut dashes = Vec::new();
    for [from, to] in [
        [border.left_top(), border.right_top()],
        [border.right_top(), border.right_bottom()],
        [border.right_bottom(), border.left_bottom()],
        [border.left_bottom(), border.left_top()],
    ] {
        dashes.extend(egui::Shape::dashed_line(&[from, to], stroke, dash, gap));
    }
    painter.extend(dashes);

    let label_font = theme::styled_font_in(
        &ctx.global_style(),
        egui::TextStyle::Heading,
        theme::TEXT_XL,
    );
    painter.text(
        window.center(),
        egui::Align2::CENTER_CENTER,
        LABEL,
        label_font,
        palette.ink,
    );
}
