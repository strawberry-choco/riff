//! The browser column (design-handoff issue 08): the first pane of the
//! three-pane explorer. A generic list column that renders every section's
//! rows — artists with cover thumbnails, plus All Tracks / Albums / Genres /
//! Folders / smart-list / playlist rows — with an A–Z sort control. The
//! browser is permanently list-only: the grid render path was retired
//! end-to-end.
//!
//! Pure widget seam, same discipline as [`crate::ui::sidebar`]: widgets paint
//! from [`Palette`] tokens and report [`BrowserAction`]s instead of mutating
//! app state; `app.rs` applies them. Rendered headlessly in
//! `tests/ui_tests.rs`.

use eframe::egui;

use super::icons::IconCache;
use super::theme::geometry::browser::{
    HEADER_H, MIN_TEXT_W, ROW_H, TEXT_GAP, TEXT_INSET_Y, TEXT_RIGHT_PAD, THUMB_SIZE, THUMB_TEXT_GAP,
};
use super::theme::{self, Palette};

/// One row/tile of the browser column.
#[derive(Clone)]
pub struct BrowserItem {
    /// Selection identity the detail column (issue 09) resolves — a variant
    /// key such as an artist name or a `TrackId`.
    pub key: String,
    /// Primary row text.
    pub label: String,
    /// Secondary text under the label (e.g. an album's year).
    pub detail: Option<String>,
    /// Cover thumbnail for the variants that show one (artists, grid tiles).
    /// `None` paints the placeholder slot.
    pub thumbnail: Option<egui::TextureHandle>,
    /// Whether this row is the current selection.
    pub selected: bool,
    /// Whether this row IS the track currently loaded in the player.
    pub now_playing: bool,
}

/// A row's label (and optional muted detail line) laid out wrapped at the
/// text column's width — the measurement the row's height and the list
/// walker share.
struct RowText {
    label: std::sync::Arc<egui::Galley>,
    detail: Option<std::sync::Arc<egui::Galley>>,
}

/// What the user did to the browser column this frame; `app.rs` applies
/// these to the library session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserAction {
    /// A row or tile was selected, by its [`BrowserItem::key`].
    Select(String),
    /// The A–Z sort control was clicked; the caller flips the session's
    /// sort direction.
    ToggleSort,
}

/// One frame of the browser column: what to render and how.
pub struct BrowserColumn<'a> {
    /// `true` when the A–Z sort is flipped to Z–A (drives the sort button).
    pub sort_desc: bool,
    /// Whether this variant shows the A–Z sort control at all — only the
    /// variants the sort can actually order (artists, albums, genres) do;
    /// paged track listings keep their canonical store order.
    pub show_sort: bool,
    /// Total row count; `item` is consulted only for the visible window.
    pub total: usize,
    /// Row provider: map an index in `0..total` to its item. `FnMut`
    /// because providers page through the Session Views projections, whose
    /// caches are per-generation mutable state.
    pub item: &'a mut dyn FnMut(usize) -> Option<BrowserItem>,
    /// When `true`, the list walker reserves default-height slots for rows
    /// entirely above the viewport without consulting `item`, so the
    /// provider only serves the on-screen window (plus the row crossing the
    /// bottom edge). Per-row provider work — paged store reads, cover
    /// intents — stays bounded to what is visible instead of running once
    /// per walked row every frame (the artists-root idle-CPU fix). Heights
    /// of rows above the viewport are approximated at the default; they are
    /// never rendered, so nothing visible changes.
    pub virtualize: bool,
    /// Friendly empty-state title when `total == 0`.
    pub empty_title: &'a str,
    /// Friendly empty-state hint when `total == 0`.
    pub empty_hint: &'a str,
}

/// Render the browser column and append observed [`BrowserAction`]s. No
/// scroll memory: the column keys egui's state by the shared positional salt
/// (the seam's plain rendering path — goldens and widget tests).
pub fn show_browser_column(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    column: BrowserColumn<'_>,
    actions: &mut Vec<BrowserAction>,
) {
    show_browser_column_scrolled(ui, cache, palette, column, None, actions);
}

/// The app's per-Section render path: like [`show_browser_column`], but the
/// list's `ScrollArea` takes a [`ScrollControl`] — the stable per-slot salt
/// plus the offset to start at. Returns the actual vertical scroll offset
/// after the frame (0 when nothing scrolled), so the Scroll Memory can
/// record it back and stay the single source of truth between frames.
pub fn show_browser_column_scrolled(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    mut column: BrowserColumn<'_>,
    scroll: Option<super::scroll_memory::ScrollControl>,
    actions: &mut Vec<BrowserAction>,
) -> f32 {
    // Header first, even for empty sections: the sort control stays
    // visible so the listing can always be re-ordered.
    if column.show_sort && sort_button(ui, palette, column.sort_desc) {
        actions.push(BrowserAction::ToggleSort);
    }
    if column.total == 0 {
        // Friendly empty state, never a raw error: what the section is and
        // the one hint that moves the listener forward.
        empty_state(ui, palette, column.empty_title, column.empty_hint);
        return 0.0;
    }
    show_browser_list(ui, cache, palette, &mut column, scroll, actions)
}

/// Map a flat listing index into `(bucket, offset)` over a prefix-sum
/// table `counts` (`counts[0] == 0`, monotone, `counts[n]` the total). The
/// Albums variant derives its flat listing from the per-artist album
/// tables this way: `bucket` is the artist, `offset` the album slot within
/// that artist's albums.
///
/// With `desc`, the listing is traversed back to front — the Z–A flip
/// reverses the whole listing, which reverses both the bucket order and
/// each bucket's contents at once. Indexes past the total yield `None`.
#[must_use]
pub fn flat_slot(counts: &[usize], index: usize, desc: bool) -> Option<(usize, usize)> {
    let total = *counts.last()?;
    if index >= total {
        return None;
    }
    let flat = if desc { total - 1 - index } else { index };
    let bucket = counts.partition_point(|&c| c <= flat).saturating_sub(1);
    Some((bucket, flat - counts[bucket]))
}

/// The friendly empty state: what the section is plus the one hint that
/// moves the listener forward — never a raw error. Public so the app-level
/// variants that render outside [`show_browser_column`] (the paged flat
/// list, the folder tree, search misses) reuse the same shape.
pub fn empty_state(ui: &mut egui::Ui, palette: &Palette, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(
            egui::RichText::new(title)
                .text_style(egui::TextStyle::Heading)
                .color(palette.ink_2),
        );
        ui.label(
            egui::RichText::new(hint)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
    });
}

/// The A–Z sort control: a small ghost button at the column's top-right.
/// Ascending offers the Z–A flip and vice versa — the label names what the
/// click does, and doubles as the accessibility label. Returns whether it
/// was clicked.
fn sort_button(ui: &mut egui::Ui, palette: &Palette, sort_desc: bool) -> bool {
    let (text, label) = if sort_desc {
        ("Z\u{2013}A", "Sort A to Z")
    } else {
        ("A\u{2013}Z", "Sort Z to A")
    };
    ui.allocate_ui(egui::vec2(ui.available_width(), HEADER_H), |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
            let button = egui::Button::new(
                egui::RichText::new(text)
                    .text_style(egui::TextStyle::Small)
                    .color(palette.ink_2),
            )
            .fill(palette.surface_2)
            .corner_radius(super::theme::RADIUS_SM);
            let response = ui.add(button).on_hover_text(label);
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            response.clicked()
        })
        .inner
    })
    .inner
}

/// The list layout: rows flowing top-down, each at its natural height —
/// the classic 48px slot, taller when a wrapped label (or detail line)
/// needs more room. Rows are culled to the visible viewport, so only the
/// window in hand is materialized; the walk still measures every row above
/// the window so positions stay exact.
///
/// With a [`ScrollControl`] the list keys egui's state by the per-slot salt
/// and starts the frame at the control's offset (saved, or 0 on a reset);
/// without one it keeps the shared positional salt and egui's own state.
/// Returns the actual vertical offset after the frame.
fn show_browser_list(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    column: &mut BrowserColumn<'_>,
    scroll: Option<super::scroll_memory::ScrollControl>,
    actions: &mut Vec<BrowserAction>,
) -> f32 {
    let mut scroll_area = egui::ScrollArea::vertical()
        .auto_shrink(false)
        .animated(false);
    match scroll {
        Some(control) => {
            scroll_area = scroll_area.id_salt(control.salt);
            if let Some(offset) = control.start {
                scroll_area = scroll_area.vertical_scroll_offset(offset);
            }
        }
        None => scroll_area = scroll_area.id_salt("browser_list_rows"),
    }
    let output = scroll_area.show_viewport(ui, |ui, viewport| {
        let total = column.total;
        let mut y = 0.0_f32;
        let mut start = 0;
        if column.virtualize {
            // Virtualization: rows whose default slot ends above the
            // viewport are reserved in one jump at the default height —
            // the provider is consulted only for the on-screen window,
            // so per-row work (paged store reads, cover intents) stays
            // bounded to what is visible. Exact heights of rows above
            // the viewport are approximated at the default; they are
            // never rendered, so nothing visible changes (the same
            // uniform-height trade-off egui's `show_rows` makes).
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the floored quotient is a non-negative row index"
            )]
            let first = (viewport.min.y / ROW_H).floor() as usize;
            start = first.min(total);
            #[expect(
                clippy::cast_precision_loss,
                reason = "f32 keeps 48px row offsets exact for any real library"
            )]
            let jump_y = start as f32 * ROW_H;
            y = jump_y;
            if start > 0 {
                ui.advance_cursor_after_rect(egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), y),
                ));
            }
        }
        for i in start..total {
            let Some(item) = (column.item)(i) else {
                // The provider declined this slot; reserve a default row
                // so the walk stays in step with the provider.
                y += ROW_H;
                ui.advance_cursor_after_rect(egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), ROW_H),
                ));
                continue;
            };
            let h = browser_row_height(ui, palette, &item);
            if y + h <= viewport.min.y || y >= viewport.max.y {
                // Above or below the visible window: reserve the slot
                // without materializing the row.
                ui.advance_cursor_after_rect(egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), h),
                ));
            } else {
                let response = browser_row(ui, cache, palette, &item);
                if response.clicked() {
                    actions.push(BrowserAction::Select(item.key));
                }
            }
            y += h;
            if y >= viewport.max.y {
                break;
            }
        }
    });
    output.state.offset.y
}

/// The row's accessibility label: the primary text with the muted
/// detail line folded in `"Label (detail)"`-style when present (handoff
/// issue 16) — counts must be read, not just painted.
fn accessible_label(item: &BrowserItem) -> String {
    match &item.detail {
        Some(detail) => format!("{} ({detail})", item.label),
        None => item.label.clone(),
    }
}

/// The detail column (issue 09) reuses the browser row for its entity
/// listings (an artist's albums, a genre's artists) — same 48px row shape,
/// one pane over. Clicks stay with the caller.
pub fn detail_entity_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    item: &BrowserItem,
) -> egui::Response {
    browser_row(ui, cache, palette, item)
}

/// The text column's wrap width: the row width minus the thumbnail slot and
/// edge padding.
fn text_w(row_w: f32) -> f32 {
    (row_w - THUMB_TEXT_GAP - TEXT_RIGHT_PAD).max(MIN_TEXT_W)
}

/// Lay out the row's label (and muted detail line) wrapped at the text
/// column's width. Memoized by egui's galley cache, so the measure passes
/// and the paint passes share one layout.
fn layout_row_text(ui: &egui::Ui, palette: &Palette, item: &BrowserItem) -> RowText {
    let ink = if item.now_playing {
        palette.brand_primary
    } else {
        palette.ink
    };
    let label_font = egui::FontId::new(super::theme::TEXT_SM, egui::FontFamily::Proportional);
    let detail_font = egui::FontId::new(super::theme::TEXT_XS, egui::FontFamily::Proportional);
    let wrap = text_w(ui.available_width());
    ui.fonts_mut(|f| RowText {
        label: f.layout(item.label.clone(), label_font, ink, wrap),
        detail: item
            .detail
            .as_ref()
            .map(|d| f.layout(d.clone(), detail_font, palette.ink_3, wrap)),
    })
}

/// The label/detail block's laid-out height: label, the gap, then the
/// detail line when present.
fn text_block_h(text: &RowText) -> f32 {
    text.label.size().y + text.detail.as_ref().map_or(0.0, |g| TEXT_GAP + g.size().y)
}

/// Whether the row's text wraps past its single-line slot: the label or the
/// detail line broke onto a second line at the text column's width. The
/// classic 48px row fits one label line over one detail line; anything that
/// wraps must grow.
fn row_wraps(text: &RowText) -> bool {
    text.label.rows.len() > 1 || text.detail.as_ref().is_some_and(|g| g.rows.len() > 1)
}

/// The natural height of one browser row: the classic 48px slot, grown when
/// the wrapped label (or detail) needs more room. Pure measurement — the
/// list walker uses it to place rows of varying heights, so it must agree
/// exactly with [`browser_row`]'s own allocation.
fn browser_row_height(ui: &egui::Ui, palette: &Palette, item: &BrowserItem) -> f32 {
    let text = layout_row_text(ui, palette, item);
    if row_wraps(&text) {
        text_block_h(&text) + 2.0 * TEXT_INSET_Y
    } else {
        ROW_H
    }
}

/// One browser row: an optional rounded cover thumbnail (or its placeholder
/// slot), the label with its muted detail line, and selection fill. Clicks
/// stay with the caller, exactly like [`super::sidebar`]'s row widgets.
///
/// The text wraps within the text column, so a label (or detail line) too
/// long for one line breaks onto the next and the row grows to fit it —
/// the classic 48px slot renders exactly as before. The muted detail line
/// (a count, an artist · year, the artist under a track) folds into the
/// accessibility label — `"Label (detail)"`, the same shape the sidebar
/// rows speak — so it is read, not just painted (handoff issue 16).
fn browser_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    item: &BrowserItem,
) -> egui::Response {
    let text = layout_row_text(ui, palette, item);
    let wraps = row_wraps(&text);
    let row_h = if wraps {
        text_block_h(&text) + 2.0 * TEXT_INSET_Y
    } else {
        ROW_H
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );
    let painter = ui.painter_at(rect);

    if item.selected {
        painter.rect_filled(rect, super::theme::RADIUS_MD, palette.surface_3);
    } else if response.hovered() {
        painter.rect_filled(rect, super::theme::RADIUS_MD, palette.row_hover);
    }
    if let Some(ring) =
        super::theme::focus_ring_stroke(palette, ui.memory(|m| m.has_focus(response.id)))
    {
        painter.rect_stroke(
            rect,
            super::theme::RADIUS_MD,
            ring,
            egui::StrokeKind::Inside,
        );
    }

    // Thumbnail slot: the cover texture when one resolved, otherwise the
    // muted placeholder well (the generated-colour block, issue 14,
    // replaces the placeholder look without touching this seam).
    let thumb_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 6.0 + THUMB_SIZE / 2.0, rect.center().y),
        egui::vec2(THUMB_SIZE, THUMB_SIZE),
    );
    if let Some(texture) = &item.thumbnail {
        let sized = egui::load::SizedTexture::new(texture.id(), thumb_rect.size());
        painter.image(
            sized.id,
            thumb_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            super::theme::TEXTURE_TINT,
        );
    } else {
        painter.rect_filled(thumb_rect, super::theme::RADIUS_SM, palette.surface_2);
        let tex_id = cache.texture(ui.ctx(), super::icons::Icon::Music, 16.0, palette.ink_3);
        painter.image(
            tex_id,
            thumb_rect.shrink(10.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            palette.ink_3,
        );
    }

    // Label (and its muted detail line) to the right of the thumbnail.
    let text_x = thumb_rect.right() + 10.0;
    let ink = if item.now_playing {
        palette.brand_primary
    } else {
        palette.ink
    };
    paint_row_text(painter, palette, item, &text, rect, text_x, ink, wraps);

    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::SelectableLabel,
            item.selected,
            accessible_label(item),
        )
    });
    response
}

/// Paint the row's label (and muted detail line) right of the thumbnail:
/// the classic single-line anchors when the text fits the 48px slot, the
/// wrapped block under the row's fixed inset when it wraps.
#[allow(clippy::too_many_arguments, reason = "one painter call per row")]
fn paint_row_text(
    painter: egui::Painter,
    palette: &Palette,
    item: &BrowserItem,
    text: &RowText,
    rect: egui::Rect,
    text_x: f32,
    ink: egui::Color32,
    wraps: bool,
) {
    if wraps {
        // The wrapped block sits under a fixed inset, the detail line
        // directly under the label's last line.
        let top = rect.top() + TEXT_INSET_Y;
        painter.galley(egui::pos2(text_x, top), text.label.clone(), ink);
        if let Some(detail) = &text.detail {
            painter.galley(
                egui::pos2(text_x, top + text.label.size().y + TEXT_GAP),
                detail.clone(),
                palette.ink_3,
            );
        }
    } else {
        match &item.detail {
            Some(detail) => {
                painter.text(
                    egui::pos2(text_x, rect.center().y - 8.0),
                    egui::Align2::LEFT_CENTER,
                    &item.label,
                    egui::FontId::new(super::theme::TEXT_SM, egui::FontFamily::Proportional),
                    ink,
                );
                painter.text(
                    egui::pos2(text_x, rect.center().y + 9.0),
                    egui::Align2::LEFT_CENTER,
                    detail,
                    egui::FontId::new(super::theme::TEXT_XS, egui::FontFamily::Proportional),
                    palette.ink_3,
                );
            }
            None => {
                painter.text(
                    egui::pos2(text_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &item.label,
                    egui::FontId::new(super::theme::TEXT_SM, egui::FontFamily::Proportional),
                    ink,
                );
            }
        }
    }
}
