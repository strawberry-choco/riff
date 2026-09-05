//! The restyled sidebar widgets (Issue 07, restructured per design-handoff
//! issue 07).
//!
//! The design sidebar is: a search box with a focus-ring border, flat
//! sectioned nav (LIBRARY / SMART LISTS / PLAYLISTS) on 40px tree rows with
//! hover states and right-aligned live counts, an animated equalizer-bars
//! indicator on the now-playing row, playlist rows whose edit/delete
//! affordances reveal on hover, and an Add-folder / last-scan footer.
//!
//! Everything here is a pure widget seam: widgets paint from [`Palette`]
//! tokens (ADR 0004), report actions instead of mutating app state, and
//! render headlessly in `tests/ui_tests.rs` / `tests/golden_tests.rs`. The
//! Store flows behind the playlist actions stay in `app.rs`, which keeps this
//! module egui-only and behavior-free.

use eframe::egui;
use std::time::Duration;

use super::icons::{Icon, IconCache};
use super::theme::{self, Palette};

// --- Mockup dimensions --------------------------------------------------------

/// Tree-row height (`h-10`): every sidebar row is exactly 40px tall.
pub const ROW_H: f32 = 40.0;

/// Search-box height (`h-8`).
pub const SEARCH_H: f32 = 32.0;

/// First-level indent: content starts 12px into the row.
pub const INDENT_BASE: f32 = 12.0;

/// The mockup's three-level indent scale, verbatim: 12 / 44 / 80px.
const INDENT_SCALE: [f32; 3] = [12.0, 44.0, 80.0];

/// Indent step between tree levels past the mockup's third; deep levels keep
/// stepping so deep trees never fold into one edge.
pub const INDENT_STEP: f32 = 36.0;

/// Horizontal padding of the icon strip inside a row.
const ICON_GAP: f32 = 8.0;

/// The equalizer-bars indicator: four bars, like the mockup's now-playing
/// glyph.
pub const EQ_BAR_COUNT: usize = 4;

/// The three-level indent scale: 12/44/80px for levels 0/1/2. Levels beyond
/// the mockup's three keep stepping (see [`INDENT_STEP`]) so deep trees never
/// fold into one edge.
#[must_use]
#[expect(clippy::cast_precision_loss)]
pub fn indent_px(level: usize) -> f32 {
    match level.checked_sub(INDENT_SCALE.len() - 1) {
        Some(extra) => INDENT_SCALE[INDENT_SCALE.len() - 1] + INDENT_STEP * extra as f32,
        None => INDENT_SCALE[level],
    }
}

// --- Equalizer bars -------------------------------------------------------------

/// Normalized equalizer-bar heights for `phase` seconds of playback time:
/// each bar bounces on its own sine wave with a small phase offset, so the
/// group reads as dancing rather than pulsing. Pure and deterministic — the
/// golden harness renders it idle, and `ui_tests` pins the animation
/// properties.
#[must_use]
#[expect(clippy::cast_possible_truncation)]
pub fn equalizer_heights(phase: f64) -> [f32; EQ_BAR_COUNT] {
    let t = phase;
    [
        ((t * 5.1 + 0.0).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
        ((t * 6.3 + 1.3).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
        ((t * 4.7 + 2.6).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
        ((t * 5.9 + 3.9).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
    ]
}

/// Paint the equalizer-bars indicator into `rect`: `heights` are normalized
/// bar heights from [`equalizer_heights`].
#[expect(clippy::cast_precision_loss)]
fn paint_equalizer(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    heights: &[f32],
) {
    let n = heights.len().max(1) as f32;
    let bar_w = rect.width() / (n * 1.6);
    let gap = (rect.width() - bar_w * n) / (n - 1.0).max(1.0);
    for (i, h) in heights.iter().enumerate() {
        let x = rect.left() + i as f32 * (bar_w + gap);
        let bar_h = rect.height() * h;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - bar_h / 2.0),
                egui::vec2(bar_w, bar_h),
            ),
            bar_w / 2.0,
            color,
        );
    }
}

// --- Search box -----------------------------------------------------------------

/// The search box border: the hairline border token when idle, the palette's
/// focus ring (thicker) once the field has keyboard focus — the mockup's
/// "focus-ring border".
#[must_use]
pub fn search_ring_stroke(palette: &Palette, focused: bool) -> egui::Stroke {
    if focused {
        egui::Stroke::new(1.5_f32, palette.focus_ring)
    } else {
        egui::Stroke::new(1.0_f32, palette.border)
    }
}

/// Full-texture UV rect for [`egui::Painter::image`].
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

/// A ghost icon button: an invisible click target with a tinted glyph. With
/// `hover_reveal` the glyph appears only while hovered (the mockup's
/// playlist-row affordances); otherwise it is always painted (the search
/// clear button). The hit area and accessibility label are registered every
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
    let response = ui.interact(rect, id, egui::Sense::click());
    let visible = !hover_reveal || response.hovered();
    if visible {
        let tint = if response.hovered() {
            palette.ink
        } else {
            palette.ink_3
        };
        let tex_id = cache.texture(ui.ctx(), icon, 16.0, tint);
        ui.painter_at(rect)
            .image(tex_id, rect.shrink(4.0), UV_FULL, tint);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_text(label).clicked()
}

/// The restyled search box: a rounded input well with the search glyph at the
/// left edge, a frameless text field, and a clear affordance while the query
/// is non-empty. The border follows [`search_ring_stroke`] — hairline when
/// idle, focus ring when the field has focus.
///
/// Returns the text field's response so the caller can keep driving
/// request-focus shortcuts (Ctrl+F) exactly as before.
pub fn search_box(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    query: &mut String,
) -> egui::Response {
    let id = ui.id().with("sidebar_search");
    // Read focus BEFORE painting so the ring lands on the same frame the
    // field gains focus (egui re-renders immediately after focus changes).
    let focused = ui.memory(|m| m.has_focus(id));

    let size = egui::vec2(ui.available_width(), SEARCH_H);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_2);
    painter.rect_stroke(
        rect,
        theme::RADIUS_MD,
        search_ring_stroke(palette, focused),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink2(egui::vec2(10.0_f32, 4.0_f32));
    let inner_ui = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = ICON_GAP;

            let tex_id = cache.texture(ui.ctx(), Icon::Search, 16.0, palette.ink_3);
            let sized = egui::load::SizedTexture::new(tex_id, egui::vec2(16.0, 16.0));
            ui.add(egui::Image::from_texture(sized));

            let response = ui.add(
                egui::TextEdit::singleline(query)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width() - 20.0),
            );

            if !query.is_empty() {
                let clear_rect = egui::Rect::from_center_size(
                    egui::pos2(inner.right() - 10.0, rect.center().y),
                    egui::vec2(20.0, SEARCH_H - 8.0),
                );
                if ghost_icon_button(
                    ui,
                    cache,
                    palette,
                    clear_rect,
                    id.with("clear"),
                    Icon::Close,
                    "Clear search",
                    false,
                ) {
                    query.clear();
                }
            }

            response
        },
    );
    inner_ui.inner
}

// --- Tree rows --------------------------------------------------------------------

/// One 40px sidebar tree row: indent level, optional leading glyph, label,
/// selection state, and now-playing state. Rows paint their own hover fill
/// ([`Palette::row_hover`], the design's amber wash), selected fill
/// ([`Palette::surface_3`]), and — on the now-playing row — the animated
/// equalizer-bars indicator in the brand tint.
pub struct TreeRow<'a> {
    /// Nesting depth; 0 is a top-level row (see [`indent_px`]).
    pub indent_level: usize,
    /// Optional leading Lucide glyph (e.g. sparkles for smart playlists).
    pub icon: Option<Icon>,
    /// Row text.
    pub label: &'a str,
    /// Live count shown right-aligned in muted ink (the design's count on
    /// every sidebar row, from the counts read model). `None` paints no
    /// count; the accessibility label gains a `(count)` suffix when present.
    pub count: Option<usize>,
    /// Whether this row is the current selection.
    pub selected: bool,
    /// Whether this row IS the track currently loaded in the player; paints
    /// the equalizer indicator and brand-tints the label.
    pub now_playing: bool,
    /// Whether playback is actually running (bars animate only then).
    pub playing: bool,
    /// `Some(open)` paints a disclosure chevron before the label for
    /// collapsible tree nodes (folder tree, artist/album tree). The chevron
    /// is an affordance only — toggling stays with the caller's click
    /// handling.
    pub disclosure: Option<bool>,
}

/// Draw one tree row and return its full response — clicks, double-clicks,
/// hover, and context menus all stay with the caller so existing behaviors
/// (selection, play-on-double-click, context menus) are untouched.
pub fn tree_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    row: TreeRow<'_>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::click(),
    );
    let painter = ui.painter_at(rect);

    if row.selected {
        painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_3);
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS_MD, palette.row_hover);
    }
    if let Some(ring) = theme::focus_ring_stroke(palette, ui.memory(|m| m.has_focus(response.id))) {
        painter.rect_stroke(rect, theme::RADIUS_MD, ring, egui::StrokeKind::Inside);
    }

    let mut x = rect.left() + indent_px(row.indent_level);

    if let Some(open) = row.disclosure {
        let chevron = if open { "\u{25BE}" } else { "\u{25B8}" };
        painter.text(
            egui::pos2(x + 6.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            chevron,
            egui::FontId::new(theme::TEXT_SM, egui::FontFamily::Proportional),
            palette.ink_3,
        );
        x += 12.0 + 4.0;
    }

    if let Some(icon) = row.icon {
        let tint = if row.now_playing || row.selected {
            palette.brand_primary
        } else {
            palette.ink_2
        };
        let tex_id = cache.texture(ui.ctx(), icon, 16.0, tint);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(x + 8.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        );
        painter.image(tex_id, icon_rect, UV_FULL, tint);
        x += 16.0 + ICON_GAP;
    }

    if row.now_playing {
        // The animated equalizer-bars indicator replaces the old ▶ glyph.
        let phase = if row.playing {
            ui.ctx().input(|i| i.time)
        } else {
            0.0
        };
        let heights = equalizer_heights(phase);
        let eq_rect = egui::Rect::from_center_size(
            egui::pos2(x + 7.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        );
        paint_equalizer(&painter, eq_rect, palette.brand_primary, &heights);
        x += 14.0 + ICON_GAP;
        if row.playing {
            // Keep the bars dancing between repaints.
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    let ink = if row.now_playing {
        palette.brand_primary
    } else {
        palette.ink
    };
    let font = egui::FontId::new(theme::TEXT_SM, egui::FontFamily::Proportional);
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        row.label,
        font,
        ink,
    );

    if let Some(count) = row.count {
        painter.text(
            egui::pos2(rect.right() - INDENT_BASE, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::new(theme::TEXT_SM, egui::FontFamily::Proportional),
            palette.ink_3,
        );
    }

    // The accessibility label folds the live count in so assistive tech and
    // the kittest harness read the same "Name (count)" shape the playlist
    // rows paint.
    let labeled = match row.count {
        Some(count) => format!("{} ({count})", row.label),
        None => row.label.to_owned(),
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, row.selected, &labeled)
    });
    response
}

// --- Playlist rows ------------------------------------------------------------------

/// What the user did to one playlist row. The rename/delete actions drive the
/// EXISTING Store flows in `app.rs` — this widget only reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistRowAction {
    /// Open the playlist.
    Open,
    /// Open the inline rename prompt (the pencil affordance).
    Rename,
    /// Delete the playlist through the store (the trash affordance).
    Delete,
}

/// One playlist row: the name opens the playlist, and hovering reveals the
/// edit/delete affordances at the right edge. The affordances are always
/// interactive (and always present in the accessibility tree); only their
/// glyphs wait for the hover, matching the mockup's hover-reveal. `label`
/// is the painted text — the caller supplies it preformatted ("Name
/// (count)") from its label cache so steady-state frames allocate nothing.
pub fn playlist_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    name: &str,
    label: &str,
    selected: bool,
) -> Option<PlaylistRowAction> {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::click(),
    );
    let painter = ui.painter_at(rect);

    if selected {
        painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_3);
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS_MD, palette.row_hover);
    }
    if let Some(ring) = theme::focus_ring_stroke(palette, ui.memory(|m| m.has_focus(response.id))) {
        painter.rect_stroke(rect, theme::RADIUS_MD, ring, egui::StrokeKind::Inside);
    }

    let font = egui::FontId::new(theme::TEXT_SM, egui::FontFamily::Proportional);
    painter.text(
        egui::pos2(rect.left() + indent_px(0), rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        palette.ink,
    );

    // Hover-revealed affordances at the right edge. Registered every frame so
    // they stay clickable/reachable; their glyphs tint up only while hovered.
    let mut action = None;
    let btn_size = egui::vec2(24.0, ROW_H - 8.0);
    let delete_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 28.0 - btn_size.x, rect.top() + 4.0),
        btn_size,
    );
    let edit_rect = egui::Rect::from_min_size(
        egui::pos2(delete_rect.left() - btn_size.x, rect.top() + 4.0),
        btn_size,
    );

    if ghost_icon_button(
        ui,
        cache,
        palette,
        delete_rect,
        ui.id().with(("playlist_delete", name)),
        Icon::Trash,
        "Delete playlist",
        true,
    ) {
        action = Some(PlaylistRowAction::Delete);
    }
    if ghost_icon_button(
        ui,
        cache,
        palette,
        edit_rect,
        ui.id().with(("playlist_rename", name)),
        Icon::Pencil,
        "Rename playlist",
        true,
    ) {
        action = Some(PlaylistRowAction::Rename);
    }

    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, selected, name)
    });
    if response.clicked() {
        action = Some(PlaylistRowAction::Open);
    }
    action
}

// --- Footer ----------------------------------------------------------------------------

/// The sidebar footer (design-handoff issue 07): an "Add folder" row that
/// routes through the existing add-library-path flow, and the muted
/// "Last scan X ago" stamp. `last_scan` carries the caller-preformatted
/// stamp text (see [`format_last_scan_ago`]); `None` renders no stamp —
/// before any scan has completed the footer just shows the action. Returns
/// whether Add folder was clicked this frame; the add flow stays with the
/// caller.
pub fn sidebar_footer(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    last_scan: Option<&str>,
) -> bool {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter_at(rect).line_segment(
        [rect.left_center(), rect.right_center()],
        egui::Stroke::new(1.0, palette.border),
    );

    let clicked = tree_row(
        ui,
        cache,
        palette,
        TreeRow {
            indent_level: 0,
            icon: Some(Icon::Folder),
            label: "Add folder",
            count: None,
            selected: false,
            now_playing: false,
            playing: false,
            disclosure: None,
        },
    )
    .clicked();

    if let Some(stamp) = last_scan {
        ui.label(
            egui::RichText::new(stamp)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
    }
    clicked
}

// --- Section headers -----------------------------------------------------------------

/// The footer's "Last scan" age, bucketed coarsely for a status readout:
/// `just now` under a minute, then whole minutes, hours, and days.
#[must_use]
pub fn format_last_scan_ago(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        return "just now".to_owned();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", hours / 24)
}

/// A muted section header ("Smart Playlists", "Playlists") in the design's
/// xs size. Letter-spacing is unavailable in egui; the muted ink carries the
/// hierarchy instead. Returns the label's response so callers can attach
/// hover help.
pub fn section_header(ui: &mut egui::Ui, palette: &Palette, text: &str) -> egui::Response {
    ui.label(
        egui::RichText::new(text)
            .text_style(egui::TextStyle::Small)
            .color(palette.ink_3),
    )
}

// --- Drag-reorderable rows (Issue 12) -------------------------------------------------

/// Drag-and-drop payload for a list-row drag: the source row index. Private
/// to this module — callers only ever see decoded indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowDrag(usize);

/// What one drag-reorderable row reported this frame.
pub struct ReorderableRow {
    /// Union response of the wrapped row: clicks, double-clicks, hover, and
    /// context menus all stay with the caller exactly as with [`tree_row`].
    pub response: egui::Response,
    /// Source index of a drag released over THIS row this frame, if any.
    pub drop_from: Option<usize>,
}

/// One 40px tree row wrapped in built-in drag-and-drop support: press-drag
/// picks the row up (it follows the pointer as a floating layer), and
/// releasing over another row reports the move through
/// [`ReorderableRow::drop_from`] — combining `(source, this row's index)`
/// is the caller's job. Clicks, double-clicks, and context menus pass
/// through untouched, and a stale payload (a drag that ended off every row)
/// is cleared automatically so it can never fire a phantom reorder on a
/// later plain click.
///
/// The drag hit-area is registered BEFORE the row paints (the titlebar
/// drag-region precedent): egui's hit test swallows clicks that land on a
/// drag-only widget stacked above a click widget, so the row must sit on
/// top for selection and menus to keep working.
pub fn reorderable_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    index: usize,
    row: TreeRow<'_>,
) -> ReorderableRow {
    // A payload can only outlive its own gesture when the drag ended off
    // every row. Drop it before it can misfire: on a genuine drop frame
    // `any_released` is true, so the rows below still consume it first.
    if egui::DragAndDrop::has_payload_of_type::<RowDrag>(ui.ctx()) {
        let released_this_frame = ui.ctx().input(|i| i.pointer.any_released());
        let pointer_down = ui.ctx().input(|i| i.pointer.any_down());
        if !pointer_down && !released_this_frame {
            egui::DragAndDrop::clear_payload(ui.ctx());
        }
    }

    // Reserve this row's layout slot up front so the list never shifts,
    // whether the row paints in place or floats after the pointer.
    let (slot_rect, slot_response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::hover(),
    );

    let response = if ui.ctx().is_being_dragged(id) {
        // This row is in flight: carry the payload and paint it into a
        // floating tooltip layer that follows the pointer (what
        // `Ui::dnd_drag_source` does for its own wrappers).
        egui::DragAndDrop::set_payload(ui.ctx(), RowDrag(index));
        let layer_id = egui::LayerId::new(egui::Order::Tooltip, id);
        let floated = ui.scope_builder(
            egui::UiBuilder::new()
                .layer_id(layer_id)
                .max_rect(slot_rect),
            |ui| tree_row(ui, cache, palette, row),
        );
        if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
            let delta = pointer_pos - floated.response.rect.center();
            ui.ctx().transform_layer_shapes(
                layer_id,
                egui::emath::TSTransform::from_translation(delta),
            );
        }
        // A floating row takes no part in hit-testing.
        slot_response
    } else {
        // Drag hit-area first, row on top: clicks land on the row, drags on
        // the hit-area (see the module-level note above).
        let drag_area = ui
            .interact(slot_rect, id, egui::Sense::drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        let row_response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(slot_rect), |ui| {
                tree_row(ui, cache, palette, row)
            })
            .inner;
        drag_area | row_response
    };

    // Ring the hovered drop target while a row is in flight — egui
    // suppresses ordinary hover fills on non-dragged widgets mid-drag, so
    // the focus-ring token carries the affordance instead.
    if egui::DragAndDrop::has_payload_of_type::<RowDrag>(ui.ctx()) && response.contains_pointer() {
        ui.painter_at(response.rect).rect_stroke(
            response.rect,
            theme::RADIUS_MD,
            egui::Stroke::new(1.5_f32, palette.focus_ring),
            egui::StrokeKind::Inside,
        );
    }

    let drop_from = response
        .dnd_release_payload::<RowDrag>()
        .map(|payload| payload.0);

    ReorderableRow {
        response,
        drop_from,
    }
}
