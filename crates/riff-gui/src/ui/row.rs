//! The neutral row frame shared by every entity and Track row in a Library
//! Section or Drill Column.
//!
//! One owner for the row's interaction treatment — the selected/hover band,
//! the focus ring, and the accessibility registration — so those states cannot
//! drift between the sidebar tree rows ([`crate::ui::sidebar::tree_row`]) and
//! the browser rows ([`crate::ui::browser`]). Row density, leading media (the
//! favorite cell, cover tile, thumbnail), the trailing clusters, and the
//! truncate-vs-wrap strategy stay with each variant; only the shared band and
//! identity live here.

use eframe::egui;

use super::theme::{self, Palette};

/// Paint one row's background band: the selected fill, else the hover wash,
/// and the row's own focus ring.
///
/// `band` is the full row shape the fills cover — the favorite cell included on
/// track rows, so the row reads as one continuous band. `selected` wins over
/// `hovered`; `focused` is whether the row's interactive response currently
/// holds keyboard focus.
pub fn paint_row_band(
    painter: &egui::Painter,
    palette: &Palette,
    band: egui::Rect,
    selected: bool,
    hovered: bool,
    focused: bool,
) {
    if selected {
        painter.rect_filled(band, theme::RADIUS_MD, palette.surface_3);
    } else if hovered {
        painter.rect_filled(band, theme::RADIUS_MD, palette.row_hover);
    }
    if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
        painter.rect_stroke(band, theme::RADIUS_MD, ring, egui::StrokeKind::Inside);
    }
}

/// Register one row in the accessibility tree as a selectable label carrying
/// its identity and selection state.
///
/// `label` folds in the row's count or detail line so assistive tech reads it
/// rather than only painting it — the same "Name (detail)" shape every list
/// row speaks.
pub fn register_row_a11y(response: &egui::Response, selected: bool, label: impl Into<String>) {
    let label = label.into();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, selected, &label)
    });
}
