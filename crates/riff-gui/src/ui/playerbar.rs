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
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use super::icons::{Icon, IconCache};
use super::linear;
use super::theme::geometry::playerbar::{
    CENTER_MIN_W, COVER, GHOST_BTN, MIN_INNER_H, MIN_TEXT_W, NOW_PLAYING_GAP, NOW_PLAYING_LINE_GAP,
    NOW_PLAYING_MIN_W, NOW_PLAYING_W, PLAY_BTN, QUEUE_LABEL_SPACE, QUEUE_PANEL_HEADER_H,
    QUEUE_PANEL_MAX_LIST_H, QUEUE_PANEL_W, TEXT_HIDE_META_W, VOLUME_THUMB, VOLUME_W,
};
use super::theme::geometry::seek::{TIME_LABEL_SPACE, TRACK_H};
use super::theme::geometry::sidebar::ROW_H;
use super::theme::{self, Palette};
use riff_backend::domain::{PlaybackState, RepeatMode, TrackId};

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

/// Retained monospace readouts for one seek row (allocation plan 2.4): the
/// elapsed side is cleared and refilled through `write!` into a fixed
/// buffer every frame, and the total side is rebuilt only when the track's
/// duration actually changes (`--:--` cached outright while unknown). The
/// caller owns the buffers across frames, so steady-state frames allocate
/// nothing for either label.
#[derive(Default)]
pub struct SeekReadouts {
    /// Refilled every frame; capacity is retained across frames.
    elapsed: String,
    /// The total's whole seconds the cached `total` label was built from.
    total_secs: Option<u64>,
    /// Rebuilt only when `total_secs` moves.
    total: String,
}

impl SeekReadouts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Refresh both labels against this frame's position and duration.
    pub fn sync(&mut self, position: Duration, total: Option<Duration>) {
        self.elapsed.clear();
        let secs = position.as_secs();
        let _ = write!(self.elapsed, "{:02}:{:02}", secs / 60, secs % 60);

        let total_secs = total.map(|duration| duration.as_secs());
        if total_secs != self.total_secs {
            self.total_secs = total_secs;
            self.total = total.map_or_else(|| "--:--".to_owned(), format_duration);
        }
    }

    /// The elapsed readout, e.g. `"02:00"`.
    #[must_use]
    pub fn elapsed(&self) -> &str {
        &self.elapsed
    }

    /// The total readout, e.g. `"04:05"` or `"--:--"` when unknown.
    #[must_use]
    pub fn total(&self) -> &str {
        &self.total
    }
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

// --- Content & actions ------------------------------------------------------------

/// Everything the playerbar needs to render one frame. A plain value struct:
/// the caller reads it out of the session, the widgets never touch state.
#[derive(Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each field is an independent playback or display state, as in UiFlags"
)]
pub struct PlayerBarContent<'a> {
    /// Real cover texture from the app's LRU cache; `None` paints the
    /// gradient placeholder.
    pub cover: Option<egui::TextureHandle>,
    /// Current track title; `None` renders the idle copy. An `Arc`-shared
    /// cache handout like [`NowPlayingContent`]'s labels (allocation plan
    /// 2.2), so fresh frames bump refcounts instead of rebuilding strings.
    ///
    /// [`NowPlayingContent`]: crate::ui::now_playing::NowPlayingContent
    pub title: Option<Arc<str>>,
    /// `"Artist - Album"` line under the title.
    pub meta_line: Option<Arc<str>>,
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
    /// Whether the queue panel (handoff issue 13) is open; the queue button
    /// carries the active tint and flipped label while it is.
    pub queue_open: bool,
    /// Whether the enlarged player view (Now Playing) is up; the expand
    /// button carries the active tint and flipped glyph while it is.
    pub expanded: bool,
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
    /// Open or close the queue panel (handoff issue 13).
    ToggleQueue,
    /// Enter or leave the enlarged player view (handoff issue 13).
    ToggleExpanded,
    /// Queue a track from the queue panel to play next (REQ-UI-005).
    PlayNext(TrackId),
}

// --- Entry point -------------------------------------------------------------------

/// Draw the playerbar across the full panel strip and report every control
/// interaction. Must run inside the shell's bottom panel of exactly
/// [`crate::ui::theme::PLAYERBAR_H`] height; layout inside is manual and
/// deterministic so the golden image pins real geometry.
///
/// Observed actions are appended to `actions` — a buffer the caller owns
/// and clears per frame — and the seek row's time labels refill the
/// caller's retained [`SeekReadouts`], so steady-state frames allocate
/// nothing here.
pub fn show_player_bar(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &PlayerBarContent<'_>,
    readouts: &mut SeekReadouts,
    actions: &mut Vec<PlayerBarAction>,
) {
    let rect = ui.max_rect();
    readouts.sync(content.position, content.total);
    // 16px side insets; vertical breathing room up to 12px, shrinking before
    // the bar's two rows are allowed to collide (host chrome may hand the
    // widget less than the full 88px strip).
    let vpad = ((rect.height() - MIN_INNER_H) / 2.0).clamp(0.0, 12.0);
    let inner = rect.shrink2(egui::vec2(16.0, vpad));
    let cy = inner.center().y;

    // --- Right cluster ------------------------------------------------------
    let queue_left = show_right_cluster(ui, cache, palette, content, inner, actions);

    // --- Left: now-playing zone (cover + title/meta lines) -------------------
    // The zone is elastic: the fixed right cluster and the protected center
    // column get their room first.
    let cluster_w = inner.right() - queue_left;
    let zone_w = now_playing_zone_width(inner.width(), cluster_w);
    let cover_rect = egui::Rect::from_min_size(
        egui::pos2(inner.left(), cy - COVER / 2.0),
        egui::vec2(COVER, COVER),
    );
    let text_w = zone_w - COVER - NOW_PLAYING_GAP;
    // Below `MIN_TEXT_W` no legible line survives: paint the cover alone —
    // the full title still rides in the zone's tooltip.
    let hide_text = text_w < MIN_TEXT_W;
    let zone_rect = if hide_text {
        cover_rect
    } else {
        egui::Rect::from_min_size(cover_rect.min, egui::vec2(zone_w, COVER))
    };
    let button = super::button::begin_icon_button(
        ui,
        zone_rect,
        egui::Id::new("playerbar_now_playing"),
        false,
    );
    paint_cover(
        ui,
        palette,
        content.cover.as_ref().map(egui::TextureHandle::id),
        cover_rect,
    );
    if !hide_text {
        paint_now_playing_text(
            ui,
            palette,
            content,
            egui::pos2(cover_rect.right() + NOW_PLAYING_GAP, cy),
            text_w,
        );
    }
    if super::button::finish_icon_button(ui, palette, &button, &now_playing_label(content)) {
        actions.push(PlayerBarAction::ToggleExpanded);
    }

    // --- Center column: seek row above centered transport -------------------
    let center = egui::Rect::from_min_max(
        egui::pos2(zone_rect.right() + 20.0, inner.top()),
        egui::pos2(queue_left - 20.0, inner.bottom()),
    );
    show_seek_row(ui, palette, content, readouts, center, actions);
    transport_row(ui, cache, palette, content, center, actions);
}

/// Width of the now-playing zone for one frame's bar geometry: the bar's
/// inner width minus the right cluster and the two 20px gaps protecting the
/// center column, clamped between [`NOW_PLAYING_MIN_W`] and [`NOW_PLAYING_W`]
/// (issue 01's layout contract). The floor is soft: when the bar cannot hold
/// both the zone floor and [`CENTER_MIN_W`], the protected center wins and
/// the zone yields to it (down to nothing, where the text rules below shed
/// the whole text column) — the bar degrades in the fixed order, never
/// overlaps.
#[must_use]
pub fn now_playing_zone_width(inner_w: f32, cluster_w: f32) -> f32 {
    let room = inner_w - cluster_w - 40.0 - CENTER_MIN_W;
    room.clamp(NOW_PLAYING_MIN_W, NOW_PLAYING_W)
        .min(room.max(0.0))
}

/// The zone's accessible name and hover tooltip: the full, un-elided
/// `"Title — Artist - Album"`, or the idle copy when no track is loaded.
fn now_playing_label(content: &PlayerBarContent<'_>) -> String {
    match (&content.title, &content.meta_line) {
        (Some(title), Some(meta)) => format!("{title} \u{2014} {meta}"),
        (Some(title), None) => title.to_string(),
        (None, Some(meta)) => meta.to_string(),
        (None, None) => "Nothing playing".to_owned(),
    }
}

/// A one-row [`egui::text::LayoutJob`] for the now-playing zone's text
/// column: `text` laid out in `font` on `ink`, hard-capped to `max_w`. The
/// wrap settings mirror the sidebar's tree-row label elide — measurement-
/// driven, exactly one row, and `break_anywhere` so even a filename-style
/// unbroken token elides instead of overflowing the bar (issue 04). The
/// un-truncated string is what reaches the zone's accessible name and hover
/// tooltip, never this job's painted output.
#[must_use]
pub fn now_playing_text_job(
    text: &str,
    font: egui::FontId,
    ink: egui::Color32,
    max_w: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, ink, max_w);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    job
}

/// The zone's two text lines, vertically centered on the cover and
/// left-aligned at `pos`: the title at `TEXT_SM` on `ink`, the
/// `"Artist - Album"` meta at `TEXT_XS` on `ink_2`. With no track loaded the
/// same slots carry the idle copy on `ink_2`/`ink_3` — same geometry either
/// way, so the bar never reflows when playback starts. Both lines are
/// one-row galleys elided at `text_w` (see [`now_playing_text_job`]); the
/// first degradation step drops the meta line entirely below
/// `TEXT_HIDE_META_W` (it still rides in the zone's tooltip).
fn paint_now_playing_text(
    ui: &mut egui::Ui,
    palette: &Palette,
    content: &PlayerBarContent<'_>,
    pos: egui::Pos2,
    text_w: f32,
) {
    let title_font = super::now_playing::styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM);
    let meta_font = super::now_playing::styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS);
    let (title, title_ink, meta, meta_ink) = match content.title.as_deref() {
        Some(title) => (
            title,
            palette.ink,
            content.meta_line.as_deref().unwrap_or(""),
            palette.ink_2,
        ),
        None => (
            "Nothing playing",
            palette.ink_2,
            "Pick a track from your library to start",
            palette.ink_3,
        ),
    };
    let title_galley =
        ui.fonts_mut(|f| f.layout_job(now_playing_text_job(title, title_font, title_ink, text_w)));
    let meta_galley = (text_w >= TEXT_HIDE_META_W).then(|| {
        ui.fonts_mut(|f| f.layout_job(now_playing_text_job(meta, meta_font, meta_ink, text_w)))
    });

    let painter = ui.painter();
    let title_h = title_galley.size().y;
    let block_h = title_h
        + meta_galley
            .as_ref()
            .map_or(0.0, |g| NOW_PLAYING_LINE_GAP + g.size().y);
    let top = pos.y - block_h / 2.0;
    painter.galley(egui::pos2(pos.x, top), title_galley, title_ink);
    if let Some(galley) = meta_galley {
        painter.galley(
            egui::pos2(pos.x, top + title_h + NOW_PLAYING_LINE_GAP),
            galley,
            meta_ink,
        );
    }
}

/// The queue-open ghost button (handoff issue 13): the list-music glyph,
/// flipping its label and engaging the active tint while the panel is open.
fn queue_open_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    content: &PlayerBarContent<'_>,
) -> bool {
    let label = if content.queue_open {
        "Close queue"
    } else {
        "Open queue"
    };
    ghost_circle_button(
        ui,
        cache,
        palette,
        rect,
        egui::Id::new("playerbar_queue_open"),
        Icon::ListMusic,
        label,
        content.queue_open,
    )
}

/// The fullscreen/expand ghost button (handoff issue 13): the corner-bracket
/// glyph flips to the collapse variant while the enlarged player view is up.
fn expand_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    content: &PlayerBarContent<'_>,
) -> bool {
    let (icon, label) = if content.expanded {
        (Icon::Collapse, "Exit expanded player")
    } else {
        (Icon::Expand, "Expand player")
    };
    ghost_circle_button(
        ui,
        cache,
        palette,
        rect,
        egui::Id::new("playerbar_expand"),
        icon,
        label,
        content.expanded,
    )
}

/// The next ghost button's square, centered in the right-to-left layout
/// cursor — the one spelling of that rect for the strip's icon buttons.
fn ghost_btn_rect(x: f32, cy: f32) -> egui::Rect {
    egui::Rect::from_center_size(
        egui::pos2(x - GHOST_BTN / 2.0, cy),
        egui::vec2(GHOST_BTN, GHOST_BTN),
    )
}

/// The right-hand cluster, laid right-to-left from the strip's right edge:
/// the expand/fullscreen button at the corner, the volume slider, the mute
/// toggle, the queue-open button, the repeat and shuffle toggles, and the
/// queue position label. Returns the label's left edge so the caller can
/// bound the center column.
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

    // Expand/fullscreen at the strip's right corner (handoff issue 13):
    // enters or leaves the enlarged player view; the glyph flips between
    // the corner brackets while the enlarged view is up.
    let expand_rect = ghost_btn_rect(x, cy);
    if expand_button(ui, cache, palette, expand_rect, content) {
        actions.push(PlayerBarAction::ToggleExpanded);
    }
    x -= GHOST_BTN + 16.0;

    // Volume slider: 4px track + round thumb, click/drag/keyboard to set.
    draw_volume_slider(ui, palette, x, cy, content.volume, actions);
    x -= VOLUME_W + 8.0;

    // Mute toggle: icon flips between speaker and crossed-out speaker.
    let mute_rect = ghost_btn_rect(x, cy);
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

    // Queue-open button (handoff issue 13): reveals the Up Next / queue
    // panel; the label flips and the tint engages while it is open.
    let queue_rect = ghost_btn_rect(x, cy);
    if queue_open_button(ui, cache, palette, queue_rect, content) {
        actions.push(PlayerBarAction::ToggleQueue);
    }
    x -= GHOST_BTN + 6.0;

    // Repeat toggle: cycles off → all → one; active tint while engaged.
    let repeat_rect = ghost_btn_rect(x, cy);
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
    let shuffle_rect = ghost_btn_rect(x, cy);
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
/// Both labels come from the caller's retained [`SeekReadouts`] (already
/// synced against this frame's content).
fn show_seek_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    content: &PlayerBarContent<'_>,
    readouts: &SeekReadouts,
    center: egui::Rect,
    actions: &mut Vec<PlayerBarAction>,
) {
    let painter = ui.painter_at(center);
    let seek_cy = center.top() + GHOST_BTN / 2.0;
    painter.text(
        egui::pos2(center.left(), seek_cy),
        egui::Align2::LEFT_CENTER,
        readouts.elapsed(),
        time_font(),
        palette.ink_2,
    );
    painter.text(
        egui::pos2(center.right(), seek_cy),
        egui::Align2::RIGHT_CENTER,
        readouts.total(),
        time_font(),
        palette.ink_2,
    );
    let seek_track = egui::Rect::from_min_size(
        egui::pos2(center.left() + TIME_LABEL_SPACE, seek_cy - TRACK_H / 2.0),
        egui::vec2((center.width() - TIME_LABEL_SPACE * 2.0).max(40.0), TRACK_H),
    );
    let seek_hit = egui::Rect::from_min_max(
        egui::pos2(seek_track.left(), center.top()),
        egui::pos2(seek_track.right(), center.top() + GHOST_BTN),
    );
    let frac = seek_fraction(content.position, content.total);
    // A track with an unknown or zero duration is not seekable: the control
    // paints but takes no pointer/keyboard input.
    let new_frac = linear::linear_control(
        ui,
        palette,
        &linear::LinearControl {
            id: egui::Id::new("Seek"),
            track: seek_track,
            hit: seek_hit,
            value: frac,
            thumb: None,
            // A seek bar grows a thumb of the seek surface's own diameter while
            // the pointer is on it, and thickens while it is dragged. The
            // diameter is not passed here: it is a token of that surface, so this
            // call site names the affordance and declares no dimension of its own.
            hover_thumb: true,
            interactive: content.total.is_some(),
            label: "Seek",
        },
    );
    if let (Some(f), Some(total)) = (new_frac, content.total) {
        actions.push(PlayerBarAction::Seek(Duration::from_secs_f32(
            f * total.as_secs_f32(),
        )));
    }
}

/// The volume slider: a round-thumb [`linear::LinearControl`] at `x`'s left.
/// The thumb rides the stored `volume` even while muted, so mute never
/// discards the level; a pointer/keyboard change reports [`PlayerBarAction::
/// SetVolume`] at the new fraction.
fn draw_volume_slider(
    ui: &egui::Ui,
    palette: &Palette,
    x: f32,
    cy: f32,
    volume: f32,
    actions: &mut Vec<PlayerBarAction>,
) {
    let vol_track = egui::Rect::from_min_size(
        egui::pos2(x - VOLUME_W, cy - TRACK_H / 2.0),
        egui::vec2(VOLUME_W, TRACK_H),
    );
    let vol_hit = egui::Rect::from_center_size(
        egui::pos2(x - VOLUME_W / 2.0, cy),
        egui::vec2(VOLUME_W, GHOST_BTN),
    );
    if let Some(v) = linear::linear_control(
        ui,
        palette,
        &linear::LinearControl {
            id: egui::Id::new("Volume"),
            track: vol_track,
            hit: vol_hit,
            value: volume,
            thumb: Some(VOLUME_THUMB),
            // Issue 08 added the seek bar's hover-driven grab affordance as its
            // own field, so this literal has to answer it: `false` is the volume
            // slider's pre-existing behaviour, unchanged — its thumb is the
            // permanent one above, and it never thickens its track.
            hover_thumb: false,
            interactive: true,
            label: "Volume",
        },
    ) {
        actions.push(PlayerBarAction::SetVolume(v));
    }
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

// --- Queue panel -------------------------------------------------------------------

/// How many Up Next rows the queue panel's projection resolves (design-handoff
/// issue 13). A sheet, not the whole queue — the scroll list is bounded like
/// the Now Playing stage's window, just deeper. This is how many entries the
/// view asks the read model for, not a dimension it paints, so it stays here.
pub const QUEUE_PANEL_LIMIT: usize = 50;

/// Reveal the queue panel (handoff issue 13): a floating sheet anchored
/// above the player bar's right edge, listing the existing Up Next read
/// model in queue order. Clicking a row reports
/// [`PlayerBarAction::PlayNext`] for its track — the same queue-to-play-next
/// intent the Now Playing stage's Up Next rows report. Call while
/// `queue_open` is set; the panel owns no state of its own.
#[expect(
    clippy::cast_precision_loss,
    reason = "row counts stay far below f32's integer precision"
)]
pub fn show_queue_panel(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    entries: &[super::sidebar::UpNextEntry],
    actions: &mut Vec<PlayerBarAction>,
) {
    let list_h = (entries.len() as f32 * ROW_H).min(QUEUE_PANEL_MAX_LIST_H);

    egui::Area::new(egui::Id::new("playerbar_queue_panel"))
        .order(egui::Order::Foreground)
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-16.0, -(theme::PLAYERBAR_H + 8.0)),
        )
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border))
                .corner_radius(theme::RADIUS_MD)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(QUEUE_PANEL_W - 16.0);
                    ui.set_height(QUEUE_PANEL_HEADER_H + list_h);

                    ui.painter().text(
                        egui::pos2(
                            ui.max_rect().left() + 4.0,
                            ui.max_rect().top() + QUEUE_PANEL_HEADER_H / 2.0,
                        ),
                        egui::Align2::LEFT_CENTER,
                        "Up Next",
                        egui::FontId::new(theme::TEXT_XS, egui::FontFamily::Proportional),
                        palette.ink_3,
                    );

                    if entries.is_empty() {
                        show_queue_panel_empty(ui, palette);
                        return;
                    }

                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
                            egui::pos2(
                                ui.max_rect().left(),
                                ui.max_rect().top() + QUEUE_PANEL_HEADER_H,
                            ),
                            egui::pos2(ui.max_rect().right(), ui.max_rect().bottom()),
                        )),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("playerbar_queue_panel_rows")
                                .auto_shrink(false)
                                .show_rows(ui, ROW_H, entries.len(), |ui, range| {
                                    for i in range {
                                        let Some(entry) = entries.get(i) else {
                                            continue;
                                        };
                                        let response = super::sidebar::up_next_row(
                                            ui,
                                            cache,
                                            palette,
                                            reduce_motion,
                                            entry,
                                        );
                                        if response.clicked() {
                                            actions
                                                .push(PlayerBarAction::PlayNext(entry.id.clone()));
                                        }
                                        response.on_hover_text("Queue this track to play next");
                                    }
                                });
                        },
                    );
                });
        });
}

/// The queue panel's empty state: the shared labelled
/// [`super::empty_state::empty_state_in_rect`] composition over the panel body,
/// plus a hover widget so assistive tech (and the harness) can read it.
fn show_queue_panel_empty(ui: &egui::Ui, palette: &Palette) {
    let body = egui::Rect::from_min_max(
        egui::pos2(
            ui.max_rect().left(),
            ui.max_rect().top() + QUEUE_PANEL_HEADER_H,
        ),
        ui.max_rect().max,
    );
    super::empty_state::empty_state_in_rect(
        ui.painter(),
        palette,
        body,
        "Queue is empty",
        "Play a track to start your queue.",
    );
    ui.interact(
        body,
        egui::Id::new("playerbar_queue_panel_empty"),
        egui::Sense::hover(),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Queue is empty"));
}

// --- Painters & controls ---------------------------------------------------------

/// The 56×56 now-playing cover: the real texture when the LRU cache has one,
/// otherwise the palette's gradient well, framed by a hairline border.
fn paint_cover(
    ui: &mut egui::Ui,
    palette: &Palette,
    texture: Option<egui::TextureId>,
    rect: egui::Rect,
) {
    let [top, bottom] = theme::placeholder_gradient_stops(palette);
    super::artwork::paint(
        &ui.painter_at(rect),
        palette,
        &super::artwork::Artwork {
            rect,
            texture,
            fit: super::artwork::Fit::Fill,
            tint: theme::TEXTURE_TINT,
            placeholder: Some(super::artwork::Placeholder::Gradient { top, bottom }),
            border: Some(theme::RADIUS_MD),
        },
    );
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
    let button = super::button::begin_icon_button(ui, rect, id, false);
    let painter = ui.painter_at(rect);

    // Press replaces hover, and drops the hover ring with it: a held button
    // must not look like a hovered one that is somehow also held. Instant, like
    // every press — rule 4 of the motion rule in `theme`.
    if button.pressed {
        painter.circle_filled(
            rect.center(),
            rect.width() / 2.0,
            super::button::active_fill(ui),
        );
    } else if button.hovered {
        painter.circle_filled(rect.center(), rect.width() / 2.0, palette.surface_2);
    }
    let tint = if active {
        palette.brand_primary
    } else if button.hovered || button.pressed {
        palette.ink
    } else {
        palette.ink_2
    };
    let tex_id = cache.texture(ui.ctx(), icon, 16.0, tint);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
    painter.image(tex_id, icon_rect, super::artwork::UV_FULL, tint);

    super::button::finish_icon_button(ui, palette, &button, label)
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
    let button = super::button::begin_icon_button(ui, rect, egui::Id::new("playerbar_play"), false);
    let painter = ui.painter_at(rect);

    // The FAB is the app's most-reached primary action, so it wears the same
    // lit-from-above face and the same accent bloom as a `Primary` text button —
    // through the same authority, so the two cannot drift.
    //
    // Press replaces that face, and the hover ring with it: the app's most
    // pressed control is the one that must acknowledge a press instantly (rule
    // 4 of the motion rule in `theme`), and the framework's own active fill is
    // what a stock egui button would show here — so a held FAB and a held egui
    // button are the same event wearing the same face. The bloom is left in
    // place: it reaches past the button, and a held button that also dimmed its
    // glow would read as disabled rather than as pressed.
    if button.pressed {
        painter.circle_filled(
            rect.center(),
            rect.width() / 2.0,
            super::button::active_fill(ui),
        );
    } else {
        super::button::paint_primary_face(ui, palette, rect, rect.width() / 2.0);
        if button.hovered {
            painter.circle_stroke(
                rect.center(),
                rect.width() / 2.0,
                egui::Stroke::new(1.5_f32, palette.border),
            );
        }
    }
    let tex_id = cache.texture(ui.ctx(), icon, 18.0, palette.on_brand);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0));
    painter.image(tex_id, icon_rect, super::artwork::UV_FULL, palette.on_brand);

    super::button::finish_icon_button(ui, palette, &button, label)
}
