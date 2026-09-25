//! The elastic column stage's neutral geometry.
//!
//! One owner for the stage's width allocation, hairline-separator accounting,
//! zero-gap horizontal composition, stable per-column child identities, and
//! optional inspector placement. Both the production stage renderer
//! ([`app::browser_pane`](super::app)) and the golden harness call
//! [`show_elastic_stage`], so the two can never silently diverge on where a
//! column edge lands or which child id a column owns.
//!
//! This module owns geometry only. Column kinds, drilling, Session
//! Projections, and the content each slot paints stay at the call site — the
//! [`StageSlot`] they are handed just says *which* slot to fill.

use eframe::egui;

use super::theme;

/// Which slot of the elastic stage a content closure is being asked to fill:
/// the `i`th list column (left to right), or the rightmost inspector column.
pub enum StageSlot {
    Column(usize),
    Inspector,
}

/// Lay out the elastic stage inside `ui`: `columns` list columns sized by
/// [`column_widths`] with a hairline separator between each, and — when
/// `inspector` is set — a final separator and the fixed-width inspector column
/// on the right. `add` paints each slot's content.
///
/// No horizontal scrolling: each separator consumes the style's separator
/// spacing from the width handed to the sizing policy, so the last column (or
/// the inspector) ends exactly at the stage's right edge rather than being
/// clipped by the panel. Narrow windows shrink columns toward their floors.
#[expect(
    clippy::cast_precision_loss,
    reason = "a separator count is a small non-negative number"
)]
pub fn show_elastic_stage(
    ui: &mut egui::Ui,
    columns: usize,
    inspector: bool,
    mut add: impl FnMut(&mut egui::Ui, StageSlot),
) {
    let available = ui.available_width();
    // One separator per gap between columns, plus one before the inspector.
    let gaps = columns.saturating_sub(1) + usize::from(inspector);
    let separator_w = ui
        .style()
        .separator_style(
            &egui::widget_style::Classes::default(),
            egui::widget_style::WidgetState::default(),
        )
        .spacing;
    let list_widths = column_widths(
        (available - separator_w * gaps as f32).max(0.0),
        columns,
        inspector,
    );

    // `horizontal_top` (not `horizontal`): the plain variant sizes its row to
    // `interact_size.y` and only grows with content, which would collapse every
    // stage column to one short row.
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, &width) in list_widths.iter().enumerate() {
            if i > 0 {
                ui.separator();
            }
            column_scope(ui, width, ("stage-column", i), |ui| {
                add(ui, StageSlot::Column(i));
            });
        }
        if inspector {
            ui.separator();
            column_scope(ui, theme::INSPECTOR_WIDTH, "inspector", |ui| {
                add(ui, StageSlot::Inspector);
            });
        }
    });
}

/// Draw one stage slot inside a `width`-constrained child ui.
///
/// The distinct `salt` is what keeps the columns independent: sibling child uis
/// made through [`egui::Ui::allocate_ui_with_layout`] all share the parent's
/// `"child"` salt, so persistent-id widgets inside them — each column's
/// `ScrollArea`, which ids itself via [`egui::Ui::make_persistent_id`] — would
/// otherwise collide, sharing scroll state between columns and drawing egui's
/// id-collision overlays.
fn column_scope(
    ui: &mut egui::Ui,
    width: f32,
    salt: impl egui::AsIdSalt,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, ui.available_height()),
        egui::Sense::hover(),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min))
            .id_salt(salt),
        add_contents,
    );
}

/// The elastic stage's column sizing policy (elastic-column spec): non-last
/// list columns keep their preferred [`theme::COLUMN_WIDTH`], the last list
/// column absorbs the remaining width, and the inspector (when visible)
/// takes [`theme::INSPECTOR_WIDTH`] off the top. When the width left for
/// the list columns cannot satisfy the minimum floors ([`theme::COLUMN_MIN_W`]
/// per entity column, [`theme::LAST_COLUMN_MIN_W`] for the last), every
/// column shrinks proportionally to its floor — the stage never introduces
/// horizontal scrolling, accepting below-floor widths only in extreme narrow
/// windows. Returns one width per list column (the inspector is separate).
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a column count is a small non-negative number"
)]
pub fn column_widths(available: f32, list_columns: usize, inspector: bool) -> Vec<f32> {
    let inspector_w = if inspector {
        theme::INSPECTOR_WIDTH
    } else {
        0.0
    };
    let available_lists = (available - inspector_w).max(0.0);
    if list_columns == 0 {
        return Vec::new();
    }
    // Preferred widths: non-last columns at COLUMN_WIDTH, the last column
    // absorbing the remainder.
    let mut widths = vec![theme::COLUMN_WIDTH; list_columns - 1];
    widths.push(available_lists - theme::COLUMN_WIDTH * (list_columns - 1) as f32);
    if list_columns == 1 {
        // A single column fills the stage; floors do not apply.
        return widths;
    }
    let floors = theme::COLUMN_MIN_W * (list_columns - 1) as f32 + theme::LAST_COLUMN_MIN_W;
    if available_lists < floors {
        // Narrow window: shrink every column proportionally to its floor.
        let scale = available_lists / floors;
        for (i, width) in widths.iter_mut().enumerate() {
            let floor = if i + 1 == list_columns {
                theme::LAST_COLUMN_MIN_W
            } else {
                theme::COLUMN_MIN_W
            };
            *width = floor * scale;
        }
    } else if let Some(last) = widths.last_mut()
        && *last < theme::LAST_COLUMN_MIN_W
    {
        // The remainder is enough for the floors, but the last column's
        // remainder would land below its floor: the non-last columns (which
        // have headroom above their own floors) yield width toward the last
        // column's floor first, then the last column absorbs the rest.
        let non_last_total = theme::COLUMN_WIDTH * (list_columns - 1) as f32;
        let headroom = non_last_total - theme::COLUMN_MIN_W * (list_columns - 1) as f32;
        let give = (theme::LAST_COLUMN_MIN_W - *last).min(headroom);
        *last += give;
        let scale = (available_lists - *last) / non_last_total;
        for width in widths.iter_mut().take(list_columns - 1) {
            *width *= scale;
        }
    }
    widths
}
