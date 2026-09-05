//! The browser column (design-handoff issue 08): the first pane of the
//! three-pane explorer. A generic list column that renders every section's
//! rows — artists with cover thumbnails, plus All Tracks / Albums / Genres /
//! Folders / smart-list / playlist rows — with an A–Z sort control and genre
//! filter chips on the artist variant, honoring the top bar's list/grid
//! toggle.
//!
//! Pure widget seam, same discipline as [`crate::ui::sidebar`] and
//! [`crate::ui::topbar`]: widgets paint from [`Palette`] tokens and report
//! [`BrowserAction`]s instead of mutating app state; `app.rs` applies them.
//! Rendered headlessly in `tests/ui_tests.rs`.

use eframe::egui;
use riff_backend::app::state::BrowserLayout;

use super::icons::IconCache;
use super::theme::Palette;

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

/// Row height of the browser column's list mode: room for a 36px cover
/// thumbnail (the artist variant's "small cover thumbnail") with breathing
/// room.
pub const BROWSER_ROW_H: f32 = 48.0;

/// Edge size of a row's cover thumbnail.
pub const THUMB_SIZE: f32 = 36.0;

/// Height of the header strip above the rows (sort control, genre chips).
pub const HEADER_H: f32 = 28.0;

/// What the user did to the browser column this frame; `app.rs` applies
/// these to the library session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserAction {
    /// A row or tile was selected, by its [`BrowserItem::key`].
    Select(String),
    /// The A–Z sort control was clicked; the caller flips the session's
    /// sort direction.
    ToggleSort,
    /// A genre chip was clicked: `Some(genre)` narrows the listing to that
    /// genre, `None` clears the filter (the All chip).
    SetGenreFilter(Option<String>),
}

/// One frame of the browser column: what to render and how.
pub struct BrowserColumn<'a> {
    /// List rows or grid tiles (the top bar's persisted toggle, issue 06).
    pub layout: BrowserLayout,
    /// `true` when the A–Z sort is flipped to Z–A (drives the sort button).
    pub sort_desc: bool,
    /// Whether this variant shows the A–Z sort control at all — only the
    /// variants the sort can actually order (artists, albums, genres) do;
    /// paged track listings keep their canonical store order.
    pub show_sort: bool,
    /// Genre chips (artist variant); empty renders no chip row.
    pub genres: &'a [riff_backend::domain::GenreCount],
    /// The currently selected genre chip, if any.
    pub genre_filter: Option<&'a str>,
    /// Total row count; `item` is consulted only for the visible window.
    pub total: usize,
    /// Row provider: map an index in `0..total` to its item. `FnMut`
    /// because providers page through the Session Views projections, whose
    /// caches are per-generation mutable state.
    pub item: &'a mut dyn FnMut(usize) -> Option<BrowserItem>,
    /// Friendly empty-state title when `total == 0`.
    pub empty_title: &'a str,
    /// Friendly empty-state hint when `total == 0`.
    pub empty_hint: &'a str,
}

/// Render the browser column and append observed [`BrowserAction`]s.
pub fn show_browser_column(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    mut column: BrowserColumn<'_>,
    actions: &mut Vec<BrowserAction>,
) {
    // Header first, even for empty sections: a filtered-to-empty list must
    // keep its chips visible so the filter can be cleared.
    if column.show_sort && sort_button(ui, palette, column.sort_desc) {
        actions.push(BrowserAction::ToggleSort);
    }
    if !column.genres.is_empty()
        && let Some(click) = genre_chips(ui, palette, column.genres, column.genre_filter)
    {
        let filter = match click {
            GenreChipClick::All => None,
            GenreChipClick::Genre(genre) => Some(genre),
        };
        actions.push(BrowserAction::SetGenreFilter(filter));
    }
    if column.total == 0 {
        // Friendly empty state, never a raw error: what the section is and
        // the one hint that moves the listener forward.
        empty_state(ui, palette, column.empty_title, column.empty_hint);
        return;
    }
    match column.layout {
        BrowserLayout::List => show_browser_list(ui, cache, palette, &mut column, actions),
        BrowserLayout::Grid => show_browser_grid(ui, cache, palette, &mut column, actions),
    }
}

/// Tile edge size in grid mode: two tiles per column width with a gutter.
pub const TILE_SIZE: f32 = 132.0;

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
            ui.spacing_mut().item_spacing.x = 6.0;
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

/// What a genre-chip click chose this frame: `All` clears the filter, a
/// genre name narrows the listing to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenreChipClick {
    /// The All chip: clear the filter.
    All,
    /// A genre chip: narrow to this genre.
    Genre(String),
}

/// The genre filter chip row: an All chip first, then one chip per genre
/// with its track count. The active chip carries the brand tint. Returns
/// the chip the listener clicked this frame, if any.
fn genre_chips(
    ui: &mut egui::Ui,
    palette: &Palette,
    genres: &[riff_backend::domain::GenreCount],
    selected: Option<&str>,
) -> Option<GenreChipClick> {
    let mut chosen = None;
    ui.allocate_ui(egui::vec2(ui.available_width(), HEADER_H), |ui| {
        ui.with_layout(
            egui::Layout::left_to_right(egui::Align::Center)
                .with_main_wrap(true)
                .with_cross_align(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.spacing_mut().item_spacing.y = 4.0;

                if chip(ui, palette, "All", "All genres", selected.is_none()).clicked() {
                    chosen = Some(GenreChipClick::All);
                }
                for genre in genres {
                    let active = selected == Some(genre.genre.as_str());
                    if chip(
                        ui,
                        palette,
                        &genre.genre,
                        &format!("Genre: {}", genre.genre),
                        active,
                    )
                    .clicked()
                    {
                        chosen = Some(GenreChipClick::Genre(genre.genre.clone()));
                    }
                }
            },
        );
    });
    chosen
}

/// One filter chip: a small rounded pill; the active one carries the brand
/// tint, idle ones the surface fill.
fn chip(
    ui: &mut egui::Ui,
    palette: &Palette,
    text: &str,
    label: &str,
    active: bool,
) -> egui::Response {
    let (fill, ink) = if active {
        (palette.brand_primary, palette.background)
    } else {
        (palette.surface_2, palette.ink_2)
    };
    let button = egui::Button::new(
        egui::RichText::new(text)
            .text_style(egui::TextStyle::Small)
            .color(ink),
    )
    .fill(fill)
    .corner_radius(super::theme::RADIUS_FULL);
    let response = ui.add(button);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_text(label)
}

/// The list layout: virtualized 48px rows, one per visible window index.
fn show_browser_list(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    column: &mut BrowserColumn<'_>,
    actions: &mut Vec<BrowserAction>,
) {
    egui::ScrollArea::vertical().show_rows(ui, BROWSER_ROW_H, column.total, |ui, row_range| {
        for i in row_range {
            let Some(item) = (column.item)(i) else {
                continue;
            };
            let response = browser_row(ui, cache, palette, &item);
            if response.clicked() {
                actions.push(BrowserAction::Select(item.key));
            }
        }
    });
}

/// The grid layout: square tiles (thumbnail + label) flowing two per row —
/// the same items the list shows, just denser visually. The provider is
/// consulted only for the tiles currently on screen.
fn show_browser_grid(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    column: &mut BrowserColumn<'_>,
    actions: &mut Vec<BrowserAction>,
) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the floored quotient is positive and small"
    )]
    let columns = ((ui.available_width() / (TILE_SIZE + 8.0)).floor() as usize).max(1);
    let rows = column.total.div_ceil(columns);
    egui::ScrollArea::vertical().show_rows(ui, TILE_SIZE + 24.0, rows, |ui, row_range| {
        for grid_row in row_range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for c in 0..columns {
                    let i = grid_row * columns + c;
                    let Some(item) = (column.item)(i) else {
                        continue;
                    };
                    let response = grid_tile(ui, cache, palette, &item);
                    if response.clicked() {
                        actions.push(BrowserAction::Select(item.key));
                    }
                }
            });
        }
    });
}

/// The row/tile's accessibility label: the primary text with the muted
/// detail line folded in `"Label (detail)"`-style when present (handoff
/// issue 16) — counts must be read, not just painted.
fn accessible_label(item: &BrowserItem) -> String {
    match &item.detail {
        Some(detail) => format!("{} ({detail})", item.label),
        None => item.label.clone(),
    }
}

/// One square grid tile: the thumbnail slot (placeholder well or cover
/// texture) over the label.
fn grid_tile(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    item: &BrowserItem,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(TILE_SIZE, TILE_SIZE + 20.0),
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

    let art = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 6.0, rect.top() + 6.0),
        egui::vec2(TILE_SIZE - 12.0, TILE_SIZE - 12.0),
    );
    if let Some(texture) = &item.thumbnail {
        painter.image(
            texture.id(),
            art,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            super::theme::TEXTURE_TINT,
        );
    } else {
        painter.rect_filled(art, super::theme::RADIUS_SM, palette.surface_2);
        let tex_id = cache.texture(ui.ctx(), super::icons::Icon::Music, 16.0, palette.ink_3);
        painter.image(
            tex_id,
            egui::Rect::from_center_size(art.center(), egui::vec2(24.0, 24.0)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            palette.ink_3,
        );
    }

    painter.text(
        egui::pos2(rect.center().x, art.bottom() + 12.0),
        egui::Align2::CENTER_CENTER,
        &item.label,
        egui::FontId::new(super::theme::TEXT_XS, egui::FontFamily::Proportional),
        if item.now_playing {
            palette.brand_primary
        } else {
            palette.ink
        },
    );

    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::SelectableLabel,
            item.selected,
            accessible_label(item),
        )
    });
    response
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

/// One 48px browser row: an optional rounded cover thumbnail (or its
/// placeholder slot), the label with its muted detail line, and selection
/// fill. Clicks stay with the caller, exactly like [`super::sidebar`]'s
/// row widgets.
///
/// The muted detail line (a count, an artist · year, the artist under a
/// track) folds into the accessibility label — `"Label (detail)"`, the same
/// shape the sidebar rows speak — so it is read, not just painted
/// (handoff issue 16).
fn browser_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    item: &BrowserItem,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), BROWSER_ROW_H),
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

    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::SelectableLabel,
            item.selected,
            accessible_label(item),
        )
    });
    response
}
