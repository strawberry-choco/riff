//! The restyled sidebar widgets (Issue 07, restructured per design-handoff
//! issue 07).
//!
//! The design sidebar is: a focus-ring border, flat
//! sectioned nav (LIBRARY / SMART LISTS / PLAYLISTS) on 40px tree rows with
//! hover states and right-aligned live counts, an animated equalizer-bars
//! indicator on the now-playing row, playlist rows whose edit/delete
//! affordances reveal on hover, and an Add-folder / last-scan footer. Track
//! rows carry the favorite control in their leading cell, so every track
//! listing in the app speaks one row shape.
//!
//! Everything here is a pure widget seam: widgets paint from [`Palette`]
//! tokens (ADR 0004), report actions instead of mutating app state, and
//! render headlessly in `tests/ui_tests.rs` / `tests/golden_tests.rs`. The
//! Store flows behind the playlist actions stay in `app.rs`, which keeps this
//! module egui-only and behavior-free.

use eframe::egui;
use std::time::Duration;

use super::icons::{Icon, IconCache};
use super::menu::{FAVORITE_ADD_LABEL, FAVORITE_REMOVE_LABEL};
use super::theme::geometry::sidebar::{
    EQ_BAR_COUNT, FAVORITE_COL_W, HEART_SIZE, ICON_GAP, INDENT_BASE, INDENT_SCALE, INDENT_STEP,
    MIN_LABEL_FREE_W, ROW_COVER, ROW_H,
};
use super::theme::{self, Palette};
use riff_backend::domain::{Track, TrackId};

/// The three-level indent scale: 12/28/44px for levels 0/1/2, one step per
/// level. Levels beyond the pinned three keep stepping (see [`INDENT_STEP`])
/// so deep trees never fold into one edge.
#[must_use]
#[expect(clippy::cast_precision_loss)]
pub fn indent_px(level: usize) -> f32 {
    match level.checked_sub(INDENT_SCALE.len() - 1) {
        Some(extra) => INDENT_SCALE[INDENT_SCALE.len() - 1] + INDENT_STEP * extra as f32,
        None => INDENT_SCALE[level],
    }
}

// --- Equalizer bars -------------------------------------------------------------

/// The equalizer's repaint budget: 20 Hz, which is the cadence the app already
/// runs at while playback is running, so the bars cost no repaint the player
/// was not already paying for.
///
/// At the tempo below this is 8–10 samples per cycle per bar (0.41–0.50 s
/// cycles sampled every 50 ms), which is clean: no bar's peak or trough falls
/// between two samples. 60 Hz would triple the app's *continuous* repaint cost
/// for a 14x14 px decoration, and a decoration is the worst thing to spend a
/// frame budget on — the eye reads the tempo, not the frame rate. Kept as a
/// named constant so the budget is stated once, next to the reason, rather than
/// as a bare `50` in the painter.
///
/// A `Duration` and not a float on purpose: a float constant of a measured
/// type in a view module is what the dimension sweep is for (ADR 0004), and a
/// repaint interval is a frame budget, not a design token.
const EQ_REPAINT_INTERVAL: Duration = Duration::from_millis(50);

/// Normalized equalizer-bar heights for `phase` seconds of playback time:
/// each bar bounces on its own sine wave with a small phase offset, so the
/// group reads as dancing rather than pulsing. Pure and deterministic — the
/// same phase always yields the same four heights, so a held phase (see
/// [`equalizer_phase`]) is simply a held argument and nothing about the
/// animation's shape needs a harness, a renderer or a frame to verify.
///
/// The rates are 12.5 / 15.4 / 13.7 / 14.6 rad/s against the 5.1 / 6.3 / 4.7 /
/// 5.9 this replaced, and they moved NON-uniformly, which is the whole point:
///
/// - **Tempo.** The periods come down from 1.232 / 0.997 / 1.336 / 1.065 s to
///   0.503 / 0.408 / 0.459 / 0.430 s — a mean of 1.158 s to a mean of 0.450 s,
///   2.57x faster. At a second per cycle the group sways; at half a second it
///   reads as playing. Scaling all four rates by a constant would have hit the
///   same tempo and left the next point untouched.
/// - **Spread.** 15.4 / 12.5 = 1.232x, down from 6.3 / 4.7 = 1.34x. The
///   offsets are what make four bars read as dancing; the spread is what made
///   them read as ONE shape breathing, because at 1.34x the fastest bar runs a
///   third of a cycle ahead of the slowest and they visibly slide into and out
///   of agreement. At 1.23x they stay neighbours.
/// - **Offsets.** Unchanged at 0.0 / 1.3 / 2.6 / 3.9 rad. The tempo was the
///   defect; the offsets were not.
///
/// They stay inline literals here rather than becoming a `const`: they are
/// algorithmic shape data for one hand-painted glyph, which is the category a
/// view is allowed to own, and a float `const` in a view module is exactly what
/// `test_view_code_declares_no_dimensions_of_its_own` exists to catch. Moving
/// them into `theme.rs` would put shape data in the design-token store, and
/// widening the sweep to allow it is not the failure mode that sweep has.
///
/// **The golden baseline's phase is set by the harness's frame count**, not by
/// a token: `sidebar_playing_dark` renders this row with `playing: true`
/// through `snapshot_animating`, so the baseline captures the bars mid-dance.
/// A tempo change here therefore moves that baseline, and that is a
/// re-baseline rather than a regression. What the phase is is worth recording
/// exactly, because it is not obvious: the harness never *sets* `input.time`,
/// it pins `predicted_dt` to a fixed 0.25 s step, and egui derives
/// `time = prev + predicted_dt` from a clock that also runs through the
/// harness's own build frames. Reading `input.time` therefore captured
/// 0.5167 s and 0.7667 s on the two painted frames — 0.25 s of build frame the
/// `ready` gate had already spent before the composition drew anything. The
/// held phase accumulates the same two 0.25 s steps from zero, so the baseline
/// now sits at exactly 0.5 s: a pure function of the frame count, which is the
/// determinism contract the harness's doc claims and did not quite have. The
/// idle sidebar baselines (`sidebar_dark`, `sidebar_light`) pin a row that has
/// never played, and a never-played row has no phase to advance, so they stay
/// byte-identical through any retune here.
#[must_use]
#[expect(clippy::cast_possible_truncation)]
pub fn equalizer_heights(phase: f64) -> [f32; EQ_BAR_COUNT] {
    let t = phase;
    [
        ((t * 12.5 + 0.0).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
        ((t * 15.4 + 1.3).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
        ((t * 13.7 + 2.6).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
        ((t * 14.6 + 3.9).sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32,
    ]
}

/// The equalizer phase this frame, in seconds of playback time, for the row
/// with id `row_id`.
///
/// **A paused row holds the last advanced phase; it does not reset it.**
/// Reading the phase as zero whenever the row is not playing snapped all four
/// bars to their phase-zero heights on every pause, which is a lopsided
/// staircase — phase zero is not a shape the four sine waves happen to be in
/// agreement on (they sit at 0.50 / 0.98 / 0.76 / 0.16 there), so pressing
/// pause visibly snapped the decoration into a shape that playing never
/// produces.
///
/// The phase therefore lives in egui's memory under the row's own id, salted so
/// it cannot collide with the row's focus or interaction state, and it only
/// advances while the row is playing. `Memory::data` is a map keyed on
/// `egui::Id` that egui never clears between frames — `insert_temp` means "not
/// written to `memory.json` on shutdown", not "per frame" — so the held value
/// survives the frame boundary it was read on, a window resize, and a re-layout
/// that keeps the row's id, without becoming a field on [`TreeRow`] or on
/// whatever read model the caller built the row from. A row that has never
/// played has no entry and paints phase zero, which is what keeps every
/// never-playing baseline byte-identical.
///
/// Not persisted, deliberately: a launch has no last advanced phase to restore,
/// and a phase carried across a restart would be a resume from an arbitrary
/// point in a cycle the user never reached.
///
/// egui's own `InputState::time` would be the alternative, and it is what this
/// used to read. It is the WRONG clock for a pause: it keeps running while
/// paused, so resuming would jump the bars forward by however long the pause
/// lasted rather than continuing from the shape they were frozen at. The
/// advance is `stable_dt` — egui's animation clock, the delta it expects the
/// next frame to take — clamped to a quarter of a second, because a stalled
/// frame (a blocked disk, a laptop waking) would otherwise fling the bars a
/// second and a half ahead. The clamp is inline rather than named: a float
/// constant in a view module is what the dimension sweep is for, and a frame
/// budget is not a design token (see [`EQ_REPAINT_INTERVAL`]).
fn equalizer_phase(ui: &egui::Ui, row_id: egui::Id, playing: bool) -> f64 {
    let key = row_id.with("eq_phase");
    let held = ui.memory(|m| m.data.get_temp::<f64>(key)).unwrap_or(0.0);
    if !playing {
        return held;
    }
    let phase = held + f64::from(ui.input(|i| i.stable_dt.clamp(0.0, 0.25)));
    ui.memory_mut(|m| m.data.insert_temp(key, phase));
    phase
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

// --- Search ring -------------------------------------------------------------------

/// The search field border: the hairline border token when idle, the palette's
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
            .image(tex_id, rect.shrink(4.0), UV_FULL, tint);
    }
    super::button::finish_icon_button(ui, palette, &button, label)
}

// --- Tree rows --------------------------------------------------------------------

/// The right-aligned value cluster on a track row: `N · M:SS` (present
/// parts joined with middle dots). Painted text only — the label carries
/// the accessibility name. Track rows (the All Tracks list, the album's
/// Tracks column) pass one; sidebar rows keep `None`.
pub struct RowMeta {
    /// Finished plays, straight from the store's play history.
    pub plays: Option<u32>,
    /// Track duration; formatted with `playerbar::format_duration`.
    pub time: Option<Duration>,
}

/// One 40px sidebar tree row: indent level, optional leading glyph, label,
/// selection state, and now-playing state. Rows paint their own hover fill
/// ([`Palette::row_hover`], the design's amber wash), selected fill
/// ([`Palette::surface_3`]), and — on the now-playing row — the animated
/// equalizer-bars indicator in the brand tint.
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent two-state facts about one row"
)]
pub struct TreeRow<'a> {
    /// Nesting depth; 0 is a top-level row (see [`indent_px`]).
    pub indent_level: usize,
    /// Optional leading Lucide glyph (e.g. sparkles for smart playlists).
    pub icon: Option<Icon>,
    /// Optional leading cover-art thumbnail (library track rows). `None`
    /// paints none; track rows pass the row's cover texture — real art when
    /// cached, otherwise the shared music-icon placeholder tile — so every
    /// track row reads with a uniform leading square.
    pub cover: Option<egui::TextureId>,
    /// Row text.
    pub label: &'a str,
    /// Live count shown right-aligned in muted ink (the design's count on
    /// every sidebar row, from the counts read model). `None` paints no
    /// count; the accessibility label gains a `(count)` suffix when present.
    pub count: Option<usize>,
    /// The right-aligned value cluster (`plays · time`) on track rows;
    /// `None` paints none (the sidebar's plain rows).
    pub meta: Option<RowMeta>,
    /// The track's favorite flag, on rows that ARE track rows (the All Tracks
    /// list, the album's Tracks column, a playlist's entries, the folder
    /// tree's tracks): `Some(..)` paints the interactive heart in the row's
    /// leading [`FAVORITE_COL_W`] cell — brand-tinted when the track IS a
    /// favorite, muted when not — and reports the toggle through
    /// [`TreeRowResponse::favorite_toggled`]. `None` paints no cell and
    /// leaves the row's click area whole, which is what every non-track row
    /// (sidebar sections, folders, smart lists, playlists, Up Next, the queue
    /// panel) wants.
    pub favorite: Option<bool>,
    /// Whether this row is the current selection.
    pub selected: bool,
    /// Whether this row IS the track currently loaded in the player; paints
    /// the equalizer indicator and brand-tints the label.
    pub now_playing: bool,
    /// Whether playback is actually running (bars animate only then).
    pub playing: bool,
    /// Whether the leading glyph occupies the same 32px square a cover tile
    /// does, so covered and uncovered rows of one tree start their label in
    /// the same place. The folder tree sets it; every other row keeps the
    /// compact 16px glyph strip.
    pub art_slot: bool,
}

/// The row's right-aligned value cluster text: the present parts of
/// [`RowMeta`] joined with middle dots (`12 · 3:45`). Painted only — the
/// row's accessibility name stays the label.
fn meta_cluster(meta: &RowMeta) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(2);
    if let Some(plays) = meta.plays {
        parts.push(plays.to_string());
    }
    if let Some(time) = meta.time {
        parts.push(super::playerbar::format_duration(time));
    }
    parts.join(" \u{b7} ")
}

/// Paint the row's label and, on track rows, the right-aligned meta cluster
/// (`plays · time`) beside it. The label lays out to the left of the
/// cluster, truncating with an ellipsis when a long title would overdraw it;
/// a row without a cluster keeps the plain single-line label. `count_w` is
/// the measured width of the row's right-aligned count, if any, so the
/// cluster dodges it.
#[expect(clippy::too_many_arguments, reason = "one row-paint call")]
fn paint_row_label_and_meta(
    ui: &egui::Ui,
    painter: &egui::Painter,
    palette: &Palette,
    row: &TreeRow<'_>,
    ink: egui::Color32,
    font: egui::FontId,
    x: f32,
    rect: egui::Rect,
    count_w: Option<f32>,
) {
    let Some(meta) = row.meta.as_ref() else {
        painter.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            row.label,
            font,
            ink,
        );
        return;
    };
    let text = meta_cluster(meta);
    let meta_w = ui
        .fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), palette.ink_3))
        .size()
        .x;
    let meta_right =
        rect.right() - INDENT_BASE - count_w.unwrap_or(0.0) - count_w.map_or(0.0, |_| 12.0);
    painter.text(
        egui::pos2(meta_right, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        text,
        font.clone(),
        palette.ink_3,
    );
    let label_w = (meta_right - meta_w - 12.0 - x).max(MIN_LABEL_FREE_W);
    let mut job = egui::text::LayoutJob::simple(row.label.to_owned(), font, ink, label_w);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(
        egui::pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
}

/// What one tree row reported this frame.
pub struct TreeRowResponse {
    /// The row's own response: clicks, double-clicks, hover, and context
    /// menus all stay with the caller, exactly as they do for a plain row.
    pub response: egui::Response,
    /// `Some(new_value)` when the row's favorite control was clicked this
    /// frame — the flag's NEW value, so the caller commits exactly that and
    /// never re-derives it. `None` otherwise, and always `None` on rows
    /// without a control.
    pub favorite_toggled: Option<bool>,
}

/// One row's allocated cells: the favorite control (track rows only), the row
/// body, and their responses.
struct RowCells {
    /// The favorite control's cell and response; `None` on rows without one.
    heart: Option<(egui::Rect, egui::Response)>,
    /// The row body: everything right of the favorite cell.
    rect: egui::Rect,
    /// The row body's response — the one callers hand to their menu wiring.
    response: egui::Response,
    /// Row body plus favorite cell: the shape the fills and the focus ring
    /// cover, so the row reads as one 40px band.
    whole: egui::Rect,
}

impl RowCells {
    /// Whether the row reads as hovered. The favorite cell counts: the wash
    /// must not blink off while the pointer is on the heart.
    fn hovered(&self) -> bool {
        self.response.hovered() || self.heart.as_ref().is_some_and(|(_, r)| r.hovered())
    }
}

/// Allocate one row's cells. A track row's favorite control gets its own
/// [`FAVORITE_COL_W`] cell at the row's leading edge, laid out BEFORE the row
/// body so the tab walk reaches the heart before the title (the handoff
/// order), and the body's click area starts to the right of it — clicking
/// the heart can never select, or play, the row. Rows without a control keep
/// the single full-width click area every sidebar row has.
fn allocate_row_cells(ui: &mut egui::Ui, row: &TreeRow<'_>) -> RowCells {
    if row.favorite.is_none() {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_H),
            egui::Sense::click(),
        );
        return RowCells {
            heart: None,
            rect,
            response,
            whole: rect,
        };
    }
    let (heart_rect, heart_response, rect, response) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (heart_rect, heart_response) =
                ui.allocate_exact_size(egui::vec2(FAVORITE_COL_W, ROW_H), egui::Sense::click());
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), ROW_H),
                egui::Sense::click(),
            );
            (heart_rect, heart_response, rect, response)
        })
        .inner;
    RowCells {
        heart: Some((heart_rect, heart_response)),
        rect,
        response,
        whole: heart_rect.union(rect),
    }
}

/// Paint one row: the background band, the favorite control, the leading glyph
/// strip, then the label and its value clusters. Split from [`tree_row`] so the
/// row's cells are laid out and registered before anything paints, and split
/// per band below so no single piece grows past a screenful.
fn paint_row(
    ui: &egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    row: &TreeRow<'_>,
    cells: &RowCells,
) {
    let painter = ui.painter_at(cells.whole);
    paint_row_band(ui, palette, row, cells, &painter);
    paint_favorite(ui, cache, palette, row, cells, &painter);
    let x = paint_row_leading(ui, cache, palette, row, cells, &painter);
    paint_row_label(ui, palette, row, cells, &painter, x);
}

/// The row's background band: the selected fill, the hover wash, and the row's
/// own focus ring — all delegated to the shared neutral row frame so a tree
/// row and a browser row read their interaction states from one owner. The
/// favorite cell counts as hovered, so the wash never blinks off while the
/// pointer crosses the heart.
fn paint_row_band(
    ui: &egui::Ui,
    palette: &Palette,
    row: &TreeRow<'_>,
    cells: &RowCells,
    painter: &egui::Painter,
) {
    let focused = ui.memory(|m| m.has_focus(cells.response.id));
    super::row::paint_row_band(
        ui,
        painter,
        palette,
        cells.whole,
        cells.response.id,
        row.selected,
        cells.hovered(),
        focused,
    );
}

/// The favorite control (track rows): a heart in the brand tint when the track
/// IS a favorite, muted when not, painted into the row's leading cell. It
/// carries its own focus ring, so the keyboard can see where it is without
/// moving the row's. Rows without a control paint nothing here.
///
/// The heart's two wordings are borrowed from `menu`, which owns them beside the
/// rest of the action vocabulary: a Track's menu offers the Favourite as a
/// second path to this same durable change, and the two surfaces must not
/// spell it independently. This module is the composite and `menu` the
/// primitive, so the borrow points the way the component layer requires.
fn paint_favorite(
    ui: &egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    row: &TreeRow<'_>,
    cells: &RowCells,
    painter: &egui::Painter,
) {
    let (Some(favorite), Some((heart_rect, heart_response))) = (row.favorite, cells.heart.as_ref())
    else {
        return;
    };
    let (label, tint) = if favorite {
        (FAVORITE_REMOVE_LABEL, palette.brand_primary)
    } else {
        (FAVORITE_ADD_LABEL, palette.ink_3)
    };
    if let Some(ring) =
        theme::focus_ring_stroke(palette, ui.memory(|m| m.has_focus(heart_response.id)))
    {
        painter.rect_stroke(
            heart_rect.shrink(2.0),
            theme::RADIUS_SM,
            ring,
            egui::StrokeKind::Inside,
        );
    }
    let texture = cache.texture(ui.ctx(), Icon::Heart, HEART_SIZE, tint);
    painter.image(
        texture,
        egui::Rect::from_center_size(heart_rect.center(), egui::vec2(HEART_SIZE, HEART_SIZE)),
        UV_FULL,
        tint,
    );
    heart_response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    heart_response.clone().on_hover_text(label);
}

/// The row's leading strip, painted left to right: the cover tile, the leading
/// glyph, and the equalizer indicator. Returns where the label starts.
fn paint_row_leading(
    ui: &egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    row: &TreeRow<'_>,
    cells: &RowCells,
    painter: &egui::Painter,
) -> f32 {
    let mut x = cells.rect.left() + indent_px(row.indent_level);

    if let Some(cover_id) = row.cover {
        // A square cover-art tile on the row's leading edge, centered in
        // the row and sized so it never exceeds the row height. The tile is
        // already the shared placeholder texture when the item is artless, so
        // this block never needs an idle well of its own.
        let cover_rect = egui::Rect::from_min_size(
            egui::pos2(x + 4.0, cells.rect.center().y - ROW_COVER / 2.0),
            egui::vec2(ROW_COVER, ROW_COVER),
        );
        super::artwork::paint(
            painter,
            palette,
            &super::artwork::Artwork {
                rect: cover_rect,
                texture: Some(cover_id),
                fit: super::artwork::Fit::Fill,
                tint: theme::TEXTURE_TINT,
                placeholder: None,
                border: Some(theme::RADIUS_SM),
            },
        );
        x += 4.0 + ROW_COVER + ICON_GAP;
    }

    if let Some(icon) = row.icon {
        let tint = if row.now_playing || row.selected {
            palette.brand_primary
        } else {
            palette.ink_2
        };
        // A tree row that CAN carry cover art owns that slot whatever it shows:
        // an uncovered folder paints its glyph into the same 32px square, so
        // covered and uncovered rows start their label in the same place.
        let (inset, box_px) = if row.art_slot {
            (4.0, ROW_COVER)
        } else {
            (0.0, 16.0)
        };
        let tex_id = cache.texture(ui.ctx(), icon, box_px, tint);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(x + inset + box_px / 2.0, cells.rect.center().y),
            egui::vec2(box_px, box_px),
        );
        painter.image(tex_id, icon_rect, UV_FULL, tint);
        x += inset + box_px + ICON_GAP;
    }

    if row.now_playing {
        // The animated equalizer-bars indicator replaces the old play glyph.
        // The phase is the row's own, held across a pause — see
        // [`equalizer_phase`].
        let phase = equalizer_phase(ui, cells.response.id, row.playing);
        let heights = equalizer_heights(phase);
        let eq_rect = egui::Rect::from_center_size(
            egui::pos2(x + 7.0, cells.rect.center().y),
            egui::vec2(14.0, 14.0),
        );
        paint_equalizer(painter, eq_rect, palette.brand_primary, &heights);
        x += 14.0 + ICON_GAP;
        if row.playing {
            // Keep the bars dancing between repaints, at the budget the
            // tempo was chosen to suit.
            ui.ctx().request_repaint_after(EQ_REPAINT_INTERVAL);
        }
    }

    x
}

/// The row's label and its right-aligned value clusters: the meta cluster
/// (`Album · plays · time`) on track rows and the live count on sidebar rows,
/// with the label laid out to their left so a long title truncates instead of
/// overdrawing them.
fn paint_row_label(
    ui: &egui::Ui,
    palette: &Palette,
    row: &TreeRow<'_>,
    cells: &RowCells,
    painter: &egui::Painter,
    x: f32,
) {
    let ink = if row.now_playing {
        palette.brand_primary
    } else {
        palette.ink
    };
    let font = egui::FontId::new(theme::TEXT_SM, egui::FontFamily::Proportional);

    let right_edge = cells.rect.right() - INDENT_BASE;
    let count_w = row.count.map(|count| {
        ui.fonts_mut(|f| f.layout_no_wrap(count.to_string(), font.clone(), palette.ink_3))
            .size()
            .x
    });
    paint_row_label_and_meta(ui, painter, palette, row, ink, font, x, cells.rect, count_w);

    if let Some(count) = row.count {
        painter.text(
            egui::pos2(right_edge, cells.rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::new(theme::TEXT_SM, egui::FontFamily::Proportional),
            palette.ink_3,
        );
    }

    // The accessibility label folds the live count in so assistive tech and
    // the kittest harness read the same "Name (count)" shape the playlist
    // rows paint — registered through the shared neutral row frame.
    let labeled = match row.count {
        Some(count) => format!("{} ({count})", row.label),
        None => row.label.to_owned(),
    };
    super::row::register_row_a11y(&cells.response, row.selected, labeled);
}

/// Draw one tree row and return what it reported — clicks, double-clicks,
/// hover, and context menus all stay with the caller so existing behaviors
/// (selection, play-on-double-click, context menus) are untouched. Track rows
/// (`favorite: Some(..)`) additionally report their favorite control's toggle
/// through [`TreeRowResponse::favorite_toggled`], with the flag's NEW value.
pub fn tree_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    row: TreeRow<'_>,
) -> TreeRowResponse {
    let cells = allocate_row_cells(ui, &row);
    paint_row(ui, cache, palette, &row, &cells);
    let favorite_toggled = match (row.favorite, cells.heart.as_ref()) {
        (Some(favorite), Some((_, heart))) if heart.clicked() => Some(!favorite),
        _ => None,
    };
    TreeRowResponse {
        response: cells.response,
        favorite_toggled,
    }
}

// --- Up Next rows (shared by Now Playing and the player bar's queue sheet) ----

// The Up Next entry model, the label builder, and the row renderer used to live
// in a module of their own (`ui::up_next`). Both the Now Playing stage and the
// player bar's queue sheet consume them, so neither supplies the other's row
// data — which is what removes the `playerbar` ↔ `now_playing` module cycle.
// That is why they sit here, on the one row primitive both surfaces already
// shared, rather than being re-derived at either call site. Presentation-only:
// the queue-to-window ordering and the skip of departed files live in the read
// model (`SessionViews::playback_up_next`), and each surface keeps its own row
// limit and maps the returned row click to its own typed Play Next action.

/// One clickable Up Next row: the queued track plus its display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpNextEntry {
    /// The queued track; rides back on the surface's Play Next intent.
    pub id: TrackId,
    /// Preformatted row label, `"Artist - Title"`.
    pub label: String,
}

/// Build the Up Next rows from the playback projection's resolved window:
/// the tracks after the current one, in the QUEUE's own order (shuffle
/// included), capped at `limit`. The queue-to-window mapping and the skip of
/// entries whose files have left the library live in
/// [`crate::app::views::SessionViews`]; this is the pure label formatting over
/// its result.
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

/// Render one Up Next row through the shared neutral tree row and return its
/// response. Both the Now Playing stage and the queue sheet paint the same
/// Track row this way, then map a click to their own Play Next action.
pub fn up_next_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    entry: &UpNextEntry,
) -> egui::Response {
    tree_row(
        ui,
        cache,
        palette,
        TreeRow {
            indent_level: 0,
            icon: None,
            cover: None,
            label: &entry.label,
            count: None,
            meta: None,
            favorite: None,
            selected: false,
            now_playing: false,
            playing: false,
            art_slot: false,
        },
    )
    .response
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

    let focused = ui.memory(|m| m.has_focus(response.id));
    super::row::paint_row_band(
        ui,
        &painter,
        palette,
        rect,
        response.id,
        selected,
        response.hovered(),
        focused,
    );

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

    super::row::register_row_a11y(&response, selected, name);
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
            cover: None,
            label: "Add folder",
            count: None,
            meta: None,
            favorite: None,
            selected: false,
            now_playing: false,
            playing: false,
            art_slot: false,
        },
    )
    .response
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
    /// The wrapped row's favorite control, when it carries one:
    /// `Some(new_value)` on the frame it was clicked, else `None`.
    pub favorite_toggled: Option<bool>,
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

    let (response, favorite_toggled) = if ui.ctx().is_being_dragged(id) {
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
            let delta = pointer_pos - floated.inner.response.rect.center();
            ui.ctx().transform_layer_shapes(
                layer_id,
                egui::emath::TSTransform::from_translation(delta),
            );
        }
        // A floating row takes no part in hit-testing, and its controls do
        // not fire: the drag gesture in flight owns the interaction.
        (slot_response, None)
    } else {
        // Drag hit-area first, row on top: clicks land on the row, drags on
        // the hit-area (see the module-level note above).
        let drag_area = ui
            .interact(slot_rect, id, egui::Sense::drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        let row = ui
            .scope_builder(egui::UiBuilder::new().max_rect(slot_rect), |ui| {
                tree_row(ui, cache, palette, row)
            })
            .inner;
        let favorite_toggled = row.favorite_toggled;
        (drag_area | row.response, favorite_toggled)
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
        favorite_toggled,
    }
}
