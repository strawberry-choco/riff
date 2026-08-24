//! The restyled player bar widgets (Issue 08).
//!
//! The mockup playerbar is: a 56×56 cover with a gradient placeholder (a
//! Mesh strip from surface-2 to surface-3) fed by the existing LRU texture
//! cache for real covers, circular ghost transport buttons around a 40px
//! primary-filled play, a 4px seek row with fill and monospace time
//! readouts, a styled volume slider (4px track, round thumb), shuffle and
//! repeat toggles, and a queue position label.
//!
//! Everything here is a pure widget seam, exactly like `sidebar.rs`: widgets
//! paint from [`Palette`] tokens (ADR 0004), report [`PlayerBarAction`]s
//! instead of mutating app state or sending engine commands, and render
//! headlessly in `tests/ui_tests.rs` / `tests/golden_tests.rs`. The
//! action→command wiring stays in `app.rs`, which keeps this module
//! egui-only and behavior-free.

use eframe::egui;
use std::time::Duration;

use super::icons::{Icon, IconCache};
use super::theme::{self, Palette};
use crate::domain::{PlaybackState, RepeatMode};

// --- Mockup dimensions ---------------------------------------------------------

/// Cover-art square (`size-14`): the now-playing cover is exactly 56×56.
pub const COVER: f32 = 56.0;

/// The primary play/pause button diameter.
pub const PLAY_BTN: f32 = 40.0;

/// Circular ghost transport button diameter (previous/next/stop/toggles).
pub const GHOST_BTN: f32 = 32.0;

/// Track height of the seek row and the volume slider (mockup: 4px).
pub const TRACK_H: f32 = 4.0;

/// Width of the volume slider track.
pub const VOLUME_W: f32 = 90.0;

/// Round thumb diameter on the volume slider.
const VOLUME_THUMB: f32 = 10.0;

/// Horizontal room reserved at each end of the seek row for the monospace
/// time readouts ("62:03" fits with margin).
const TIME_LABEL_SPACE: f32 = 44.0;

/// Horizontal room reserved for the queue position label ("999/999").
const QUEUE_LABEL_SPACE: f32 = 52.0;

/// Smallest useful inner height: a 16px seek-row hit area, an 8px gap, and
/// the 40px primary button. Below this the bar degrades gracefully instead
/// of overlapping its own rows.
const MIN_INNER_H: f32 = PLAY_BTN + 16.0 + 8.0;

/// Full-texture UV rect for [`egui::Painter::image`] (sidebar precedent).
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

// --- Readout helpers -------------------------------------------------------------

/// The monospace font for elapsed/total time readouts: `text-xs` on the
/// monospace family so digits align while counting (Issue 02 kept the family
/// registered for exactly this).
#[must_use]
pub fn time_font() -> egui::FontId {
    egui::FontId::new(theme::TEXT_XS, egui::FontFamily::Monospace)
}

/// Format a duration as `mm:ss` (minutes accumulate past an hour), the
/// shared readout format for the seek row's two ends.
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

/// Playback progress as a fraction of `total`: 0 when the duration is
/// unknown or zero-length, clamped into `0..=1` no matter what the engine
/// reports.
#[must_use]
pub fn seek_fraction(current: Duration, total: Option<Duration>) -> f32 {
    match total {
        Some(total) if total.as_secs_f32() > 0.0 => {
            (current.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

/// Pointer position along a horizontal bar as a clamped `0..=1` fraction —
/// the shared click/drag math behind the seek row and the volume slider.
#[must_use]
fn fraction_at(rect: egui::Rect, pos: egui::Pos2) -> f32 {
    if rect.width() <= 0.0 {
        return 0.0;
    }
    ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
}

// --- Content & actions ------------------------------------------------------------

/// Everything the playerbar needs to render one frame. A plain value struct:
/// the caller reads it out of `AppState`, the widgets never touch state.
#[derive(Clone)]
pub struct PlayerBarContent<'a> {
    /// Real cover texture from the app's LRU cache; `None` paints the
    /// gradient placeholder.
    pub cover: Option<egui::TextureHandle>,
    /// Drives which action the primary button reports.
    pub playback: PlaybackState,
    /// Elapsed playback position.
    pub position: Duration,
    /// Track duration; `None` disables seeking and shows `--:--`.
    pub total: Option<Duration>,
    /// Current volume in `0..=1`.
    pub volume: f32,
    /// Whether output is muted (icon flips; slider keeps its value).
    pub muted: bool,
    /// Whether shuffle is engaged (toggle carries the active tint).
    pub shuffle: bool,
    /// Current repeat mode (`One` swaps the glyph).
    pub repeat: RepeatMode,
    /// Preformatted queue position, e.g. `"3/12"`.
    pub queue_position: &'a str,
    /// Progressive disclosure (REQ-UI-006): reveals the Stop affordance.
    pub advanced: bool,
}

/// What the user did to the playerbar this frame. The app applies these
/// through its engine-command channel and state paths so every effect stays
/// testable headlessly.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerBarAction {
    /// Skip to the previous track.
    Previous,
    /// Pause playback (primary button while playing).
    Pause,
    /// Resume playback (primary button while paused).
    Resume,
    /// Start the selected track (primary button while stopped).
    PlaySelected,
    /// Skip to the next track.
    Next,
    /// Stop playback (advanced-only affordance).
    Stop,
    /// Seek to an absolute position within the current track.
    Seek(Duration),
    /// Set the output volume in `0..=1`.
    SetVolume(f32),
    /// Flip the mute flag.
    ToggleMute,
    /// Flip shuffle on the queue.
    ToggleShuffle,
    /// Cycle the repeat mode (off → all → one).
    ToggleRepeat,
}

// --- Entry point -------------------------------------------------------------------

/// Draw the playerbar across the full panel strip and report every control
/// interaction. Must run inside the shell's bottom panel of exactly
/// [`crate::ui::theme::PLAYERBAR_H`] height; layout inside is manual and
/// deterministic so the golden image pins real geometry.
pub fn show_player_bar(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &PlayerBarContent<'_>,
) -> Vec<PlayerBarAction> {
    let mut actions = Vec::new();
    let rect = ui.max_rect();
    // 16px side insets; vertical breathing room up to 12px, shrinking before
    // the bar's two rows are allowed to collide (host chrome may hand the
    // widget less than the full 88px strip).
    let vpad = ((rect.height() - MIN_INNER_H) / 2.0).clamp(0.0, 12.0);
    let inner = rect.shrink2(egui::vec2(16.0, vpad));
    let cy = inner.center().y;

    // --- Left: now-playing cover ------------------------------------------
    let cover_rect = egui::Rect::from_min_size(
        egui::pos2(inner.left(), cy - COVER / 2.0),
        egui::vec2(COVER, COVER),
    );
    paint_cover(
        ui,
        palette,
        content.cover.as_ref().map(egui::TextureHandle::id),
        cover_rect,
    );

    // --- Right cluster ------------------------------------------------------
    let queue_left = show_right_cluster(ui, cache, palette, content, inner, &mut actions);

    // --- Center column: seek row above centered transport -------------------
    let center = egui::Rect::from_min_max(
        egui::pos2(cover_rect.right() + 20.0, inner.top()),
        egui::pos2(queue_left - 20.0, inner.bottom()),
    );
    show_seek_row(ui, palette, content, center, &mut actions);
    transport_row(ui, cache, palette, content, center, &mut actions);

    actions
}

/// The right-hand cluster, laid right-to-left from the strip's right edge:
/// volume slider, mute toggle, repeat and shuffle toggles, and the queue
/// position label. Returns the label's left edge so the caller can bound the
/// center column.
fn show_right_cluster(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &PlayerBarContent<'_>,
    inner: egui::Rect,
    actions: &mut Vec<PlayerBarAction>,
) -> f32 {
    let cy = inner.center().y;
    let painter = ui.painter_at(inner);
    let mut x = inner.right();

    // Volume slider: 4px track + round thumb, click/drag to set.
    let vol_track = egui::Rect::from_min_size(
        egui::pos2(x - VOLUME_W, cy - TRACK_H / 2.0),
        egui::vec2(VOLUME_W, TRACK_H),
    );
    let vol_hit = egui::Rect::from_center_size(
        egui::pos2(x - VOLUME_W / 2.0, cy),
        egui::vec2(VOLUME_W, GHOST_BTN),
    );
    paint_slider(
        ui,
        palette,
        vol_track,
        vol_hit,
        content.volume,
        "Volume",
        actions,
    );
    x -= VOLUME_W + 8.0;

    // Mute toggle: icon flips between speaker and crossed-out speaker.
    let mute_rect = egui::Rect::from_center_size(
        egui::pos2(x - GHOST_BTN / 2.0, cy),
        egui::vec2(GHOST_BTN, GHOST_BTN),
    );
    let (mute_icon, mute_label) = if content.muted {
        (Icon::VolumeMuted, "Unmute")
    } else {
        (Icon::VolumeHigh, "Mute")
    };
    if ghost_circle_button(
        ui,
        cache,
        palette,
        mute_rect,
        egui::Id::new("playerbar_mute"),
        mute_icon,
        mute_label,
        content.muted,
    ) {
        actions.push(PlayerBarAction::ToggleMute);
    }
    x -= GHOST_BTN + 14.0;

    // Repeat toggle: cycles off → all → one; active tint while engaged.
    let repeat_rect = egui::Rect::from_center_size(
        egui::pos2(x - GHOST_BTN / 2.0, cy),
        egui::vec2(GHOST_BTN, GHOST_BTN),
    );
    let repeat_icon = if content.repeat == RepeatMode::One {
        Icon::RepeatOne
    } else {
        Icon::Repeat
    };
    if ghost_circle_button(
        ui,
        cache,
        palette,
        repeat_rect,
        egui::Id::new("playerbar_repeat"),
        repeat_icon,
        "Cycle repeat mode",
        content.repeat != RepeatMode::None,
    ) {
        actions.push(PlayerBarAction::ToggleRepeat);
    }
    x -= GHOST_BTN + 6.0;

    // Shuffle toggle: active tint while engaged.
    let shuffle_rect = egui::Rect::from_center_size(
        egui::pos2(x - GHOST_BTN / 2.0, cy),
        egui::vec2(GHOST_BTN, GHOST_BTN),
    );
    if ghost_circle_button(
        ui,
        cache,
        palette,
        shuffle_rect,
        egui::Id::new("playerbar_shuffle"),
        Icon::Shuffle,
        "Toggle shuffle",
        content.shuffle,
    ) {
        actions.push(PlayerBarAction::ToggleShuffle);
    }
    x -= GHOST_BTN + 10.0;

    // Queue position label ("3/12"), monospace like the time readouts,
    // sitting left of the shuffle toggle.
    queue_label(ui, &painter, palette, content.queue_position, x, cy)
}

/// The monospace queue position label ("3/12"), right-aligned at `right_x`
/// with a hover-only hit rect so assistive tech can read it. Returns the
/// label's left edge.
fn queue_label(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    palette: &Palette,
    text: &str,
    right_x: f32,
    cy: f32,
) -> f32 {
    let left = right_x - QUEUE_LABEL_SPACE;
    painter.text(
        egui::pos2(right_x, cy),
        egui::Align2::RIGHT_CENTER,
        text,
        time_font(),
        palette.ink_2,
    );
    let response = ui.interact(
        egui::Rect::from_min_max(
            egui::pos2(left, cy - GHOST_BTN / 2.0),
            egui::pos2(right_x, cy + GHOST_BTN / 2.0),
        ),
        egui::Id::new("playerbar_queue_position"),
        egui::Sense::hover(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    left
}

/// The seek row spanning the top of the center column: monospace elapsed and
/// total readouts around a 4px fill bar. Seeking is only offered while the
/// track duration is known; the absolute target rides in the action and the
/// app re-clamps it against the live total before sending it downstream.
fn show_seek_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    content: &PlayerBarContent<'_>,
    center: egui::Rect,
    actions: &mut Vec<PlayerBarAction>,
) {
    let painter = ui.painter_at(center);
    let seek_cy = center.top() + GHOST_BTN / 2.0;
    painter.text(
        egui::pos2(center.left(), seek_cy),
        egui::Align2::LEFT_CENTER,
        format_duration(content.position),
        time_font(),
        palette.ink_2,
    );
    painter.text(
        egui::pos2(center.right(), seek_cy),
        egui::Align2::RIGHT_CENTER,
        content
            .total
            .map_or_else(|| "--:--".to_string(), format_duration),
        time_font(),
        palette.ink_2,
    );
    let seek_track = egui::Rect::from_min_size(
        egui::pos2(center.left() + TIME_LABEL_SPACE, seek_cy - TRACK_H / 2.0),
        egui::vec2((center.width() - TIME_LABEL_SPACE * 2.0).max(40.0), TRACK_H),
    );
    let frac = seek_fraction(content.position, content.total);
    paint_seek_bar(ui, palette, seek_track, frac);

    if content.total.is_none() {
        return;
    }
    let seek_hit = egui::Rect::from_min_max(
        egui::pos2(seek_track.left(), center.top()),
        egui::pos2(seek_track.right(), center.top() + GHOST_BTN),
    );
    let seek_response = ui.interact(
        seek_hit,
        egui::Id::new("Seek"),
        egui::Sense::click_and_drag(),
    );
    if (seek_response.clicked() || seek_response.dragged())
        && let Some(pos) = seek_response.interact_pointer_pos()
        && let Some(total) = content.total
    {
        actions.push(PlayerBarAction::Seek(Duration::from_secs_f32(
            fraction_at(seek_hit, pos) * total.as_secs_f32(),
        )));
    }
    seek_response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Seek"));
}

/// The transport row centered in the column's lower half: circular ghost
/// previous/next around the primary-filled play, plus the advanced-only Stop
/// affordance (REQ-UI-006).
fn transport_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &PlayerBarContent<'_>,
    center: egui::Rect,
    actions: &mut Vec<PlayerBarAction>,
) {
    let transport_y = center.bottom() - PLAY_BTN / 2.0;
    let stop_extra = if content.advanced {
        GHOST_BTN + 12.0
    } else {
        0.0
    };
    let transport_w = GHOST_BTN + 12.0 + PLAY_BTN + 12.0 + GHOST_BTN + stop_extra;
    let mut tx = center.center().x - transport_w / 2.0;

    let prev_rect = egui::Rect::from_center_size(
        egui::pos2(tx + GHOST_BTN / 2.0, transport_y),
        egui::vec2(GHOST_BTN, GHOST_BTN),
    );
    tx += GHOST_BTN + 12.0;
    let play_rect = egui::Rect::from_center_size(
        egui::pos2(tx + PLAY_BTN / 2.0, transport_y),
        egui::vec2(PLAY_BTN, PLAY_BTN),
    );
    tx += PLAY_BTN + 12.0;
    let next_rect = egui::Rect::from_center_size(
        egui::pos2(tx + GHOST_BTN / 2.0, transport_y),
        egui::vec2(GHOST_BTN, GHOST_BTN),
    );
    tx += GHOST_BTN + 12.0;
    let stop_rect = egui::Rect::from_center_size(
        egui::pos2(tx + GHOST_BTN / 2.0, transport_y),
        egui::vec2(GHOST_BTN, GHOST_BTN),
    );

    if ghost_circle_button(
        ui,
        cache,
        palette,
        prev_rect,
        egui::Id::new("playerbar_previous"),
        Icon::SkipBack,
        "Previous track",
        false,
    ) {
        actions.push(PlayerBarAction::Previous);
    }

    // Primary play/pause: the one filled control on the bar.
    let (play_icon, play_label, play_action) = match content.playback {
        PlaybackState::Playing => (Icon::Pause, "Pause", PlayerBarAction::Pause),
        PlaybackState::Paused => (Icon::Play, "Play", PlayerBarAction::Resume),
        PlaybackState::Stopped => (Icon::Play, "Play", PlayerBarAction::PlaySelected),
    };
    if primary_play_button(ui, palette, play_rect, cache, play_icon, play_label) {
        actions.push(play_action);
    }

    if ghost_circle_button(
        ui,
        cache,
        palette,
        next_rect,
        egui::Id::new("playerbar_next"),
        Icon::SkipForward,
        "Next track",
        false,
    ) {
        actions.push(PlayerBarAction::Next);
    }

    // Stop stays an advanced-only affordance (REQ-UI-006).
    if content.advanced
        && ghost_circle_button(
            ui,
            cache,
            palette,
            stop_rect,
            egui::Id::new("playerbar_stop"),
            Icon::Square,
            "Stop",
            false,
        )
    {
        actions.push(PlayerBarAction::Stop);
    }
}

// --- Painters & controls ---------------------------------------------------------

/// The 56×56 now-playing cover: the real texture when the LRU cache has one,
/// otherwise a Mesh strip gradient from surface-2 down to surface-3 framed by
/// a hairline border.
fn paint_cover(
    ui: &mut egui::Ui,
    palette: &Palette,
    texture: Option<egui::TextureId>,
    rect: egui::Rect,
) {
    let painter = ui.painter_at(rect);
    if let Some(id) = texture {
        painter.image(id, rect, UV_FULL, theme::TEXTURE_TINT);
    } else {
        // Mesh strip: two triangles whose vertex colors interpolate from
        // surface-2 (top) to surface-3 (bottom).
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(rect.left_top(), palette.surface_2);
        mesh.colored_vertex(rect.right_top(), palette.surface_2);
        mesh.colored_vertex(rect.right_bottom(), palette.surface_3);
        mesh.colored_vertex(rect.left_bottom(), palette.surface_3);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    }
    painter.rect_stroke(
        rect,
        theme::RADIUS_MD,
        egui::Stroke::new(1.0_f32, palette.border),
        egui::StrokeKind::Inside,
    );
}

/// One styled bar control (seek row / volume slider): a 4px rounded track on
/// surface-3 with a brand-tinted fill up to `value`; the volume variant adds
/// the mockup's round thumb. Clicking or dragging reports the action at the
/// clicked fraction.
fn paint_slider(
    ui: &mut egui::Ui,
    palette: &Palette,
    track: egui::Rect,
    hit: egui::Rect,
    value: f32,
    label: &str,
    actions: &mut Vec<PlayerBarAction>,
) {
    let response = ui.interact(hit, egui::Id::new(label), egui::Sense::click_and_drag());
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        actions.push(PlayerBarAction::SetVolume(fraction_at(hit, pos)));
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, label));

    let painter = ui.painter_at(hit);
    let radius = TRACK_H / 2.0;
    painter.rect_filled(track, radius, palette.surface_3);
    let fill_w = (track.width() * value.clamp(0.0, 1.0)).max(radius * 2.0);
    painter.rect_filled(
        egui::Rect::from_min_size(track.min, egui::vec2(fill_w, TRACK_H)),
        radius,
        palette.brand_primary,
    );
    // Round thumb at the current value.
    let thumb_x = track.left() + track.width() * value.clamp(0.0, 1.0);
    painter.circle_filled(
        egui::pos2(thumb_x, track.center().y),
        VOLUME_THUMB / 2.0,
        palette.ink,
    );
}

/// The seek row's 4px bar: brand fill over a surface-3 track. Pure painting —
/// the interaction lives in [`show_player_bar`] where the track total is at
/// hand.
fn paint_seek_bar(ui: &mut egui::Ui, palette: &Palette, track: egui::Rect, frac: f32) {
    let painter = ui.painter_at(track);
    let radius = TRACK_H / 2.0;
    painter.rect_filled(track, radius, palette.surface_3);
    let fill_w = track.width() * frac.clamp(0.0, 1.0);
    if fill_w > 0.0 {
        painter.rect_filled(
            egui::Rect::from_min_size(track.min, egui::vec2(fill_w, TRACK_H)),
            radius,
            palette.brand_primary,
        );
    }
}

/// A circular ghost transport button: invisible until hovered (then a
/// surface-2 disc), carrying a tinted glyph — brand-tinted while `active`.
/// The label doubles as the hover tooltip, so every icon-only control
/// explains itself (Issue 12).
#[expect(clippy::too_many_arguments)]
fn ghost_circle_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    id: egui::Id,
    icon: Icon,
    label: &str,
    active: bool,
) -> bool {
    let response = ui.interact(rect, id, egui::Sense::click());
    let painter = ui.painter_at(rect);

    if response.hovered() {
        painter.circle_filled(rect.center(), rect.width() / 2.0, palette.surface_2);
    }
    let tint = if active {
        palette.brand_primary
    } else if response.hovered() {
        palette.ink
    } else {
        palette.ink_2
    };
    let tex = cache.texture(ui.ctx(), icon, 16.0, tint);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
    painter.image(tex.id(), icon_rect, UV_FULL, tint);

    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_text(label).clicked()
}

/// The 40px primary-filled play/pause circle: brand fill, on-brand glyph.
/// The label doubles as the hover tooltip (Issue 12).
fn primary_play_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: egui::Rect,
    cache: &mut IconCache,
    icon: Icon,
    label: &str,
) -> bool {
    let response = ui.interact(rect, egui::Id::new("playerbar_play"), egui::Sense::click());
    let painter = ui.painter_at(rect);

    painter.circle_filled(rect.center(), rect.width() / 2.0, palette.brand_primary);
    if response.hovered() {
        painter.circle_stroke(
            rect.center(),
            rect.width() / 2.0,
            egui::Stroke::new(1.5_f32, palette.border),
        );
    }
    let tex = cache.texture(ui.ctx(), icon, 18.0, palette.on_brand);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0));
    painter.image(tex.id(), icon_rect, UV_FULL, palette.on_brand);

    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_text(label).clicked()
}
