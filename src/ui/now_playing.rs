//! The restyled Now Playing stage (Issue 10).
//!
//! The mockup's `now-playing.html` page is a single centered column: a 240px
//! cover with the extra-large radius ([`theme::RADIUS_XL`]) wrapped in the
//! brand glow, the `text-3xl` semibold title, a muted meta line, the in-view
//! seek row, and the Up Next queue rows.
//!
//! Now Playing is a MODE over the active View (resolved navigation gaps): it
//! replaces whatever View was showing, and its close button always lands on
//! the Library View — there is no prior view to restore. The close decision
//! itself lives in [`crate::ui::app::apply_now_playing_action`]; this module
//! only reports [`NowPlayingAction`]s.
//!
//! Pure widget seam, exactly like `sidebar.rs` / `playerbar.rs`: paints from
//! [`Palette`] tokens (ADR 0004), mutates nothing, and renders headlessly in
//! `tests/ui_tests.rs` / `tests/golden_tests.rs`.

use eframe::egui;
use std::time::Duration;

use super::icons::{Icon, IconCache};
use super::library::{glow_color, GLOW_LAYERS};
use super::playerbar;
use super::sidebar::{self, TreeRow};
use super::theme::{self, Palette};
use crate::domain::{Track, TrackId, TrackMetadata};

// --- Mockup dimensions ---------------------------------------------------------

/// Cover-art square (`w-60 h-60`): the Now Playing cover is exactly 240px.
pub const COVER_SIZE: f32 = 240.0;

/// How many Up Next rows the stage previews (pre-restyle behavior).
pub const UP_NEXT_LIMIT: usize = 5;

/// Stage inset above the cover: 40px, clearing the widest glow layer
/// (36px spread) so the halo never clips against the panel's top edge.
const STAGE_INSET: f32 = 40.0;

/// Gap between the cover and the title (`mb-6`, widened to 40px so the
/// title clears the widest glow layer's 36px spread): 40px.
const COPY_GAP: f32 = 40.0;

/// Gap between the title and the meta line (`mt-2`): 8px.
const TITLE_META_GAP: f32 = 8.0;

/// Gap between the meta line and the details line (`mt-1`): 4px.
const META_DETAILS_GAP: f32 = 4.0;

/// Gap between the copy block and the seek row.
const SEEK_GAP: f32 = 20.0;

/// Hit-area height of the seek row.
const SEEK_H: f32 = 24.0;

/// Track thickness of the seek bar (mockup: 4px, as the playerbar's).
const TRACK_H: f32 = 4.0;

/// Horizontal room reserved at each end of the seek bar for the monospace
/// time readouts.
const TIME_LABEL_SPACE: f32 = 44.0;

/// Gap between the seek row and the Up Next section.
const SECTION_GAP: f32 = 16.0;

/// Height of the Up Next section header line.
const HEADER_H: f32 = 24.0;

/// Close-affordance diameter and its inset from the stage corner.
const CLOSE_BTN: f32 = 28.0;
const CLOSE_INSET: f32 = 12.0;

/// Full-texture UV rect for [`egui::Painter::image`] (sidebar precedent).
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

// --- Content & actions ------------------------------------------------------------

/// One clickable Up Next row: the queued track plus its display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpNextEntry {
    /// The queued track; rides back on [`NowPlayingAction::PlayNext`].
    pub id: TrackId,
    /// Preformatted row label, `"Artist - Title"`.
    pub label: String,
}

/// What the user did to the Now Playing stage this frame. The app applies
/// these through its state/command paths so every effect stays testable
/// headlessly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NowPlayingAction {
    /// Close the mode; ALWAYS lands on the Library View.
    Close,
    /// Queue an Up Next track to play next (REQ-UI-005).
    PlayNext(TrackId),
    /// Seek within the current track (in-view progress, REQ-UI-005).
    Seek(Duration),
}

/// Everything the stage needs to render one frame. A plain value struct: the
/// caller reads it out of `AppState`, the widgets never touch state.
#[derive(Default)]
pub struct NowPlayingContent {
    /// Real cover texture from the app's LRU cache; `None` paints the
    /// placeholder well.
    pub cover: Option<egui::TextureHandle>,
    /// Current track title; `None` renders the calm empty state.
    pub title: Option<String>,
    /// `"Artist - Album"` line under the title.
    pub meta_line: Option<String>,
    /// Optional secondary details line (year · genre · track/disc).
    pub details: Option<String>,
    /// Elapsed playback position.
    pub position: Duration,
    /// Track duration; `None` disables seeking and shows `--:--`.
    pub total: Option<Duration>,
    /// Up Next rows in Playback Queue order (see [`up_next_entries`]).
    pub up_next: Vec<UpNextEntry>,
}

// --- Pure helpers -------------------------------------------------------------------

/// Build the Up Next rows from the playback projection's resolved window:
/// the tracks after the current one, in the QUEUE's own order (shuffle
/// included), capped at `limit`. The queue-to-window mapping and the skip of
/// entries whose files have left the library live in
/// [`crate::app::projection::PlaybackProjection`]; this is the pure label
/// formatting over its result.
#[must_use]
pub fn up_next_entries(up_next: &[Track], limit: usize) -> Vec<UpNextEntry> {
    up_next
        .iter()
        .take(limit)
        .map(|t| UpNextEntry {
            id: t.id.clone(),
            label: format!(
                "{} - {}",
                t.metadata.display_artist(),
                t.metadata.display_title(&t.file_path)
            ),
        })
        .collect()
}

/// The optional secondary details line under the meta line: year, genre, and
/// track/disc joined with middle dots. Missing fields are hidden, never shown
/// as "Unknown"; with nothing available there is no line at all.
#[must_use]
pub fn metadata_details(metadata: &TrackMetadata) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(year) = metadata.year {
        parts.push(year.to_string());
    }
    if let Some(genre) = &metadata.genre {
        parts.push(genre.clone());
    }
    if let Some(track) = metadata.track_number {
        parts.push(format!(
            "Track {} / Disc {}",
            track,
            metadata.disc_number.unwrap_or(1)
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" \u{b7} "))
    }
}

// --- Entry point ----------------------------------------------------------------------

/// Draw the Now Playing stage across the full panel area and report every
/// interaction. Must run inside the shell's central stage panel; layout is
/// manual and deterministic so the golden image pins real geometry.
pub fn show_now_playing(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &NowPlayingContent,
) -> Vec<NowPlayingAction> {
    let mut actions = Vec::new();
    let stage = ui.max_rect();

    // Close affordance at the stage's top-right corner, registered every
    // frame so it is reachable in the empty state too.
    let close_center = egui::pos2(
        stage.right() - CLOSE_INSET - CLOSE_BTN / 2.0,
        stage.top() + CLOSE_INSET + CLOSE_BTN / 2.0,
    );
    if sidebar::ghost_icon_button(
        ui,
        cache,
        palette,
        egui::Rect::from_center_size(close_center, egui::vec2(CLOSE_BTN, CLOSE_BTN)),
        egui::Id::new("now_playing_close"),
        Icon::Close,
        "Close Now Playing",
        false,
    ) {
        actions.push(NowPlayingAction::Close);
    }

    let Some(title) = content.title.as_deref() else {
        paint_empty_state(ui);
        return actions;
    };

    // Resolve the design type scale through the CURRENT style: naming a
    // weight family directly would panic in the golden harness's first frame
    // (see `segmented_nav` / `empty_state_hero`). The title is `text-3xl` on
    // the Heading (semibold) family — the mockup's single 3xl usage.
    let title_font = styled_font(ui, egui::TextStyle::Heading, theme::TEXT_3XL);
    let body_font = styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM);
    let xs_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS);

    let cx = stage.center().x;
    let column_top = stage.top() + STAGE_INSET;

    paint_cover(ui, palette, content.cover.as_ref(), cx, column_top);

    let copy_top = column_top + COVER_SIZE + COPY_GAP;
    let seek_top = paint_copy_block(
        &ui.painter_at(stage),
        palette,
        title,
        content,
        (&title_font, &body_font, &xs_font),
        cx,
        copy_top,
    );

    if let Some(action) = seek_row(ui, &ui.painter_at(stage), palette, content, cx, seek_top) {
        actions.push(action);
    }

    let section_top = seek_top + SEEK_H + SECTION_GAP;
    up_next_section(
        ui,
        cache,
        palette,
        content,
        &mut actions,
        stage,
        cx,
        (&body_font, &xs_font),
        section_top,
    );

    actions
}

/// The empty state: calm copy instead of dangling controls (REQ-UI-005).
fn paint_empty_state(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.heading("Nothing Playing");
        ui.label("Select a track to start playback");
    });
}

/// The cover square at the column axis: layered brand glow behind the art
/// (or the placeholder well), framed by the extra-large radius hairline.
fn paint_cover(
    ui: &egui::Ui,
    palette: &Palette,
    texture: Option<&egui::TextureHandle>,
    cx: f32,
    top: f32,
) {
    let cover_rect = egui::Rect::from_center_size(
        egui::pos2(cx, top + COVER_SIZE / 2.0),
        egui::vec2(COVER_SIZE, COVER_SIZE),
    );
    let painter = ui.painter_at(cover_rect.expand(GLOW_LAYERS[0].spread));

    // The glow stands in for the design's box-shadow blur: concentric
    // translucent brand fills painted largest-first (library-hero precedent).
    for layer in &GLOW_LAYERS {
        painter.rect_filled(
            cover_rect.expand(layer.spread),
            theme::RADIUS_XL + layer.spread,
            glow_color(palette, *layer),
        );
    }

    if let Some(texture) = texture {
        painter.image(texture.id(), cover_rect, UV_FULL, theme::TEXTURE_TINT);
    } else {
        painter.rect_filled(cover_rect, theme::RADIUS_XL, palette.surface_2);
        painter.text(
            cover_rect.center(),
            egui::Align2::CENTER_CENTER,
            "\u{1F3B5}",
            egui::FontId::proportional(COVER_SIZE * 0.25),
            palette.ink_3,
        );
    }
    painter.rect_stroke(
        cover_rect,
        theme::RADIUS_XL,
        egui::Stroke::new(1.0_f32, palette.border),
        egui::StrokeKind::Inside,
    );
}

/// The centered copy block under the cover: 3xl title, meta line, optional
/// details line. Returns the y where the next row (the seek bar) starts.
fn paint_copy_block(
    painter: &egui::Painter,
    palette: &Palette,
    title: &str,
    content: &NowPlayingContent,
    fonts: (&egui::FontId, &egui::FontId, &egui::FontId),
    cx: f32,
    top: f32,
) -> f32 {
    let (title_font, body_font, xs_font) = fonts;
    let meta_galley = content
        .meta_line
        .as_ref()
        .map(|meta| painter.layout_no_wrap(meta.clone(), body_font.clone(), palette.ink_2));
    let details_galley = content
        .details
        .as_ref()
        .map(|details| painter.layout_no_wrap(details.clone(), xs_font.clone(), palette.ink_3));

    let mut y = top;
    let title_galley = painter.layout_no_wrap(title.to_owned(), title_font.clone(), palette.ink);
    painter.galley(
        egui::pos2(cx - title_galley.size().x / 2.0, y),
        title_galley.clone(),
        palette.ink,
    );
    y += title_galley.size().y + TITLE_META_GAP;
    for galley in meta_galley.iter().chain(details_galley.iter()) {
        painter.galley(
            egui::pos2(cx - galley.size().x / 2.0, y),
            galley.clone(),
            palette.ink_2,
        );
        y += galley.size().y + META_DETAILS_GAP;
    }
    y - META_DETAILS_GAP + SEEK_GAP
}

/// The seek row (REQ-UI-005): monospace times around a 4px fill bar.
/// Clicking or dragging reports an absolute [`NowPlayingAction::Seek`];
/// seeking is only offered while the track duration is known. Returns the
/// action observed this frame, if any.
fn seek_row(
    ui: &egui::Ui,
    painter: &egui::Painter,
    palette: &Palette,
    content: &NowPlayingContent,
    cx: f32,
    top: f32,
) -> Option<NowPlayingAction> {
    let seek_cy = top + SEEK_H / 2.0;
    let bar_w = (ui.max_rect().width() - TIME_LABEL_SPACE * 2.0 - 48.0).clamp(160.0, 360.0);
    let bar_rect = egui::Rect::from_min_size(
        egui::pos2(cx - bar_w / 2.0, seek_cy - TRACK_H / 2.0),
        egui::vec2(bar_w, TRACK_H),
    );
    painter.text(
        egui::pos2(bar_rect.left() - 8.0, seek_cy),
        egui::Align2::RIGHT_CENTER,
        playerbar::format_duration(content.position),
        playerbar::time_font(),
        palette.ink_2,
    );
    painter.text(
        egui::pos2(bar_rect.right() + 8.0, seek_cy),
        egui::Align2::LEFT_CENTER,
        content
            .total
            .map_or_else(|| "--:--".to_string(), playerbar::format_duration),
        playerbar::time_font(),
        palette.ink_2,
    );

    let radius = TRACK_H / 2.0;
    painter.rect_filled(bar_rect, radius, palette.surface_3);
    let frac = playerbar::seek_fraction(content.position, content.total);
    let fill_w = bar_rect.width() * frac;
    if fill_w > 0.0 {
        painter.rect_filled(
            egui::Rect::from_min_size(bar_rect.min, egui::vec2(fill_w, TRACK_H)),
            radius,
            palette.brand_primary,
        );
    }

    let total = content.total?;
    let hit = bar_rect.expand2(egui::vec2(0.0, (SEEK_H - TRACK_H) / 2.0));
    let response = ui.interact(
        hit,
        egui::Id::new("now_playing_seek"),
        egui::Sense::click_and_drag(),
    );
    let action = if response.clicked() || response.dragged() {
        response.interact_pointer_pos().map(|pos| {
            let fraction = ((pos.x - hit.left()) / hit.width()).clamp(0.0, 1.0);
            NowPlayingAction::Seek(Duration::from_secs_f32(fraction * total.as_secs_f32()))
        })
    } else {
        None
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Seek"));
    action
}

/// The Up Next section: a small header plus the queue rows — bounded to a
/// centered column so they align with the composition above — in the
/// remaining stage height. Rows report [`NowPlayingAction::PlayNext`] for
/// their own track; the list scrolls when the fixed column above leaves it
/// too little room.
#[allow(clippy::too_many_arguments)]
fn up_next_section(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &NowPlayingContent,
    actions: &mut Vec<NowPlayingAction>,
    stage: egui::Rect,
    cx: f32,
    fonts: (&egui::FontId, &egui::FontId),
    top: f32,
) {
    let (body_font, xs_font) = fonts;
    let painter = ui.painter_at(stage);
    painter.text(
        egui::pos2(cx, top + HEADER_H / 2.0),
        egui::Align2::CENTER_CENTER,
        "Up Next",
        xs_font.clone(),
        palette.ink_3,
    );

    let column_w = (stage.width() - 48.0).min(480.0);
    let list_rect = egui::Rect::from_min_max(
        egui::pos2(cx - column_w / 2.0, top + HEADER_H),
        egui::pos2(cx + column_w / 2.0, stage.bottom()),
    );

    if content.up_next.is_empty() {
        painter.text(
            egui::pos2(cx, list_rect.top() + sidebar::ROW_H / 2.0),
            egui::Align2::CENTER_CENTER,
            "Queue is empty",
            body_font.clone(),
            palette.ink_3,
        );
        return;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("now_playing_up_next")
            .auto_shrink(false)
            .show_rows(ui, sidebar::ROW_H, content.up_next.len(), |ui, range| {
                for i in range {
                    let Some(entry) = content.up_next.get(i) else {
                        continue;
                    };
                    let response = sidebar::tree_row(
                        ui,
                        cache,
                        palette,
                        TreeRow {
                            indent_level: 0,
                            icon: None,
                            label: &entry.label,
                            selected: false,
                            now_playing: false,
                            playing: false,
                            disclosure: None,
                        },
                    );
                    if response.clicked() {
                        actions.push(NowPlayingAction::PlayNext(entry.id.clone()));
                    }
                    response.on_hover_text("Queue this track to play next");
                }
            });
    });
}

/// A design-scale [`egui::FontId`] at `size`, riding the family the installed
/// token style mapped onto `key` (so weight families resolve even before the
/// vendored fonts are installed).
fn styled_font(ui: &egui::Ui, key: egui::TextStyle, size: f32) -> egui::FontId {
    let family = ui
        .style()
        .text_styles
        .get(&key)
        .map_or(egui::FontFamily::Proportional, |font| font.family.clone());
    egui::FontId::new(size, family)
}
