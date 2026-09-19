//! Frameless window chrome and the unified app shell (Issues 04 + 06).
//!
//! riff launches undecorated (`decorations(false)`) and draws its own
//! titlebar: a full-width drag region plus custom minimize/close controls.
//! The approach is the one egui itself validates in its `custom_window_frame`
//! example — register the drag-region interact first so the control buttons
//! drawn after it sit on top and win clicks over their slice of the strip.
//!
//! Since Issue 06 the titlebar is also the shell's top chrome: the former
//! top-bar content (scan status, theme / advanced / view toggles) is merged
//! into the same 56px strip, nav routes to exactly one visible View, and a
//! token-derived minimum window size keeps the fixed chrome from collapsing.
//!
//! Headless seams (tested in `tests/ui_tests.rs`): the launch viewport
//! configuration, the control→action contract, the drag-region gesture
//! decision, and the nav routing. The pixels are covered by the golden-image
//! harness (`tests/golden_tests.rs`, `shell_chrome_dark`).

use super::icons::{Icon, IconCache, icon_button};
use super::sidebar::{ghost_icon_button, search_ring_stroke};
use super::theme::geometry::sidebar::SEARCH_H;
use super::theme::geometry::titlebar::{
    CAPTION_BTN_H, CAPTION_BTN_W, CAPTION_GAP, SEARCH_EDGE_INSET, SEARCH_GAP, SEARCH_MAX_W,
    WORDMARK_GAP,
};
use super::theme::{self, Palette};
use eframe::egui;
use riff_backend::app::state::{BrowseMode, ViewMode};

/// Smallest main-stage area kept usable beside/between the fixed chrome.
pub const MIN_STAGE_SIZE: egui::Vec2 = egui::vec2(520.0, 456.0);

/// Full-texture UV rect for [`egui::Painter::image`] (sidebar precedent).
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

/// The static normalized bar heights of the wordmark's equalizer glyph — a
/// fixed brand mark that moved into the titlebar from the content top bar.
const WORDMARK_BARS: [f32; 4] = [0.55, 0.95, 0.7, 0.4];

/// Chrome-fitting minimum window size: sidebar + stage across, titlebar +
/// playerbar + stage down. The window can never shrink below this, so the
/// fixed 56/280/88 chrome never collapses.
pub const MIN_WINDOW_SIZE: egui::Vec2 = egui::vec2(
    theme::SIDEBAR_W + MIN_STAGE_SIZE.x,
    theme::TITLEBAR_H + theme::PLAYERBAR_H + MIN_STAGE_SIZE.y,
);

/// Launch viewport configuration for the frameless window: the decorated
/// window's launch size carries over unchanged, OS decorations are replaced
/// by riff's custom titlebar, and the minimum size fits the fixed shell.
#[must_use]
pub fn viewport_builder() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([MIN_WINDOW_SIZE.x, MIN_WINDOW_SIZE.y])
        .with_decorations(false)
}

/// Where a navigation action leads. Library and Folders are the two library
/// browse destinations; Settings is its own view. Now Playing is not a
/// destination — it REPLACES the active view (ADR: resolved gaps), so no
/// destination is highlighted while it is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDestination {
    /// The library explorer's track/artist browser.
    Library,
    /// The folder-tree browser.
    Folders,
    /// The settings stage.
    Settings,
}

impl NavDestination {
    /// Which destination the current state points at, or `None` while Now
    /// Playing replaces the view. Exactly one destination is ever active.
    #[must_use]
    pub fn active(view: ViewMode, browse: BrowseMode) -> Option<Self> {
        match view {
            ViewMode::Library => match browse {
                BrowseMode::Library => Some(Self::Library),
                BrowseMode::Folders => Some(Self::Folders),
            },
            ViewMode::Settings => Some(Self::Settings),
            ViewMode::NowPlaying => None,
        }
    }

    /// Route to this destination. Afterwards exactly that one View is
    /// visible: [`Self::Settings`] switches the view mode; [`Self::Library`]
    /// and [`Self::Folders`] land on the library view with the matching
    /// browse mode.
    pub fn apply(self, view: &mut ViewMode, browse: &mut BrowseMode) {
        match self {
            Self::Library => {
                *view = ViewMode::Library;
                *browse = BrowseMode::Library;
            }
            Self::Folders => {
                *view = ViewMode::Library;
                *browse = BrowseMode::Folders;
            }
            Self::Settings => {
                *view = ViewMode::Settings;
            }
        }
    }
}

/// A custom window control button in the titlebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControl {
    /// Collapse the window to the taskbar.
    Minimize,
    /// Close the window.
    Close,
}

impl WindowControl {
    /// The viewport command this control issues when clicked.
    ///
    /// Minimize collapses the window. Close is only consumed on Linux, where
    /// there is no tray: it sends the real [`egui::ViewportCommand::Close`],
    /// which quits. On macOS/Windows the custom X is a hide gesture — the app
    /// sends it through the frontend-local visibility channel
    /// ([`crate::ui::window_visibility::VisibilityMessage(false)`]), never
    /// through this method: with the close-to-tray veto gone, every `Close`
    /// that reaches eframe quits, so a hard exit here would quit instead of
    /// stowing to the tray.
    #[must_use]
    pub fn viewport_command(self) -> egui::ViewportCommand {
        match self {
            Self::Minimize => egui::ViewportCommand::Minimized(true),
            Self::Close => egui::ViewportCommand::Close,
        }
    }
}

/// What a pointer gesture on the drag region means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragRegionAction {
    /// Begin an OS window move (winit `drag_window`).
    StartDrag,
    /// Toggle maximize/restore (titlebar double-click convention).
    ToggleMaximize,
}

/// Decide what a gesture on the drag region means from the egui response
/// flags. Double-click wins over drag-start: both can be observed in the same
/// frame for a jittery double-click, and maximizing must win or the window
/// would move instead.
#[must_use]
pub fn drag_region_action(drag_started: bool, double_clicked: bool) -> Option<DragRegionAction> {
    if double_clicked {
        Some(DragRegionAction::ToggleMaximize)
    } else if drag_started {
        Some(DragRegionAction::StartDrag)
    } else {
        None
    }
}

/// Everything the shell titlebar needs to render one frame (Issue 06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TitleBarContent<'a> {
    /// Library scan status line shown next to the wordmark.
    pub scan_status: Option<&'a str>,
    /// Whether the dark palette is active (drives the theme glyph).
    pub theme_dark: bool,
    /// Progressive-disclosure flag (REQ-UI-006) reflected by the toggle.
    pub advanced_mode: bool,
    /// Which nav destination is active; `None` while Now Playing replaces
    /// the view (then the Now Playing control carries the active tint).
    pub active_nav: Option<NavDestination>,
}

/// What the user did to the titlebar this frame. The app applies these
/// through its state/viewport-command paths so every effect stays testable
/// headlessly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleBarAction {
    /// Flip between the light and dark palettes.
    ToggleTheme,
    /// Flip progressive disclosure (REQ-UI-006).
    ToggleAdvanced,
    /// Open/close Now Playing over the active view.
    ToggleNowPlaying,
    /// Route to the Settings view.
    GoSettings,
    /// Collapse the window to the taskbar.
    Minimize,
    /// Toggle maximize/restore.
    ToggleMaximize,
    /// Close the window. Split-close-paths: the app hides to the tray on
    /// macOS/Windows (frontend-local visibility message) and really closes on
    /// Linux (no tray).
    Close,
}

/// Draw the shell titlebar inside its panel: background, wordmark, scan
/// status, the global search field, the drag region, and the control cluster
/// at the right edge (theme / Now Playing / Settings / Advanced toggles plus
/// minimize/close).
///
/// Must run inside a top panel of exactly [`crate::ui::theme::TITLEBAR_H`]
/// height with no frame margins, so the drag region covers the full strip.
///
/// The global "Search or jump to…" field is shared chrome: it renders
/// centered between the wordmark/scan-status cluster and the nav/caption
/// cluster, capped at [`SEARCH_MAX_W`], and shrinks before either side as
/// the window narrows. It edits `search_query` in place — pass the session's
/// `search_query` so typing filters the library immediately — and keeps the
/// former content-top-bar interaction contract: Ctrl+K request-focus,
/// Escape clears + surrenders focus. Returns the field's response so the
/// caller can keep driving the Ctrl+K request-focus shortcut.
///
/// Observed actions are appended to `actions` — a buffer the caller owns and
/// clears per frame, so idle frames never build a fresh `Vec`. The caller
/// applies them to app state and viewport commands.
pub fn show_titlebar(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &TitleBarContent<'_>,
    search_query: &mut String,
    actions: &mut Vec<TitleBarAction>,
) -> egui::Response {
    let rect = ui.max_rect();

    // Register the drag region FIRST so the buttons added below sit on top
    // and win pointer events over their slice of the strip.
    let drag_response = ui.interact(
        rect,
        egui::Id::new("riff_titlebar_drag_region"),
        egui::Sense::click_and_drag(),
    );

    // Wordmark at the left edge: the sound-wave equalizer glyph plus "riff",
    // both in the brand orange. The cluster sits at the very top-left of the
    // window; the text is measured so the scan status starts clear of it.
    let mark_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 16.0 + 9.0, rect.center().y),
        egui::vec2(18.0, 20.0),
    );
    paint_equalizer_mark(&ui.painter_at(mark_rect), mark_rect, palette.brand_primary);
    let wordmark_right = paint_wordmark_text(ui, mark_rect, rect.center().y, palette);

    if let Some(action) = drag_region_action(
        drag_response.drag_started_by(egui::PointerButton::Primary),
        drag_response.double_clicked_by(egui::PointerButton::Primary),
    ) {
        match action {
            DragRegionAction::StartDrag => ui.send_viewport_cmd(egui::ViewportCommand::StartDrag),
            DragRegionAction::ToggleMaximize => {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
        }
    }

    // The left cluster's right edge: the wordmark, plus the scan status
    // (measured so the search field never starts under it) when present.
    let left_cluster_right = wordmark_right
        + match content.scan_status {
            Some(status) => {
                let status_w = ui
                    .painter()
                    .layout_no_wrap(
                        status.to_owned(),
                        egui::FontId::proportional(theme::TEXT_SM),
                        palette.ink_3,
                    )
                    .size()
                    .x;
                // Scan status sits next to the wordmark, muted.
                ui.painter().text(
                    egui::pos2(wordmark_right + WORDMARK_GAP, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    status,
                    egui::FontId::proportional(theme::TEXT_SM),
                    palette.ink_3,
                );
                WORDMARK_GAP + status_w
            }
            None => 0.0,
        };

    // Window controls at the top-right corner: three caption-style hit strips
    // flush to the edge (Windows convention — minimize | maximize | close,
    // zero gap between them). Drawn after the drag region so they win clicks
    // over their slice of the strip.
    let minimize_left = draw_caption_controls(ui, cache, palette, rect, actions);

    // Nav controls (theme / Now Playing / Settings / Advanced toggles) at the
    // right edge, Windows order, ending one gap left of the caption pair.
    // Drawn after the drag region so they take priority over it.
    let nav_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(minimize_left - CAPTION_GAP, rect.max.y),
    );
    // The cluster's left edge: with the right-to-left layout the cursor's
    // right edge lands left of the last (leftmost) control, exactly where
    // the search field must clear.
    let nav_left = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(nav_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                show_titlebar_controls(ui, cache, palette, content, actions);
                ui.cursor().max.x
            },
        )
        .inner;

    // Global search field: centered between the left cluster and the nav
    // cluster, capped at its maximum width. The field shrinks first as the
    // window narrows — the clusters never move for it — and the minimum gap
    // on both sides means it never collides with either cluster at the
    // minimum window size.
    let band_left = (left_cluster_right + SEARCH_GAP).max(rect.left() + SEARCH_EDGE_INSET);
    let band_right = nav_left - SEARCH_GAP;
    let search_w = (band_right - band_left).clamp(0.0, SEARCH_MAX_W);
    let search_rect = egui::Rect::from_center_size(
        egui::pos2(f32::midpoint(band_left, band_right), rect.center().y),
        egui::vec2(search_w, SEARCH_H),
    );

    // Read focus BEFORE painting so the ring lands on the same frame the
    // field gains focus (sidebar precedent).
    let id = egui::Id::new("riff_global_search");
    let focused = ui.memory(|m| m.has_focus(id));
    paint_search_well(ui, palette, search_rect, focused);

    let response = show_search_field(ui, cache, palette, search_query, search_rect, id);

    handle_search_dismiss(ui, id, focused, search_query);

    response
}

/// The minimize | maximize | close caption strips: three caption-style hit
/// areas flush to the top-right corner (Windows convention, zero gap between
/// them), observed actions appended to `actions`. Returns the minimize
/// strip's left edge — the nav cluster ends one [`CAPTION_GAP`] left of it.
fn draw_caption_controls(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    actions: &mut Vec<TitleBarAction>,
) -> f32 {
    let btn_top = rect.center().y - CAPTION_BTN_H / 2.0;
    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
    let close_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - CAPTION_BTN_W, btn_top),
        egui::vec2(CAPTION_BTN_W, CAPTION_BTN_H),
    );
    let maximize_rect = egui::Rect::from_min_size(
        egui::pos2(close_rect.left() - CAPTION_BTN_W, btn_top),
        egui::vec2(CAPTION_BTN_W, CAPTION_BTN_H),
    );
    let minimize_rect = egui::Rect::from_min_size(
        egui::pos2(maximize_rect.left() - CAPTION_BTN_W, btn_top),
        egui::vec2(CAPTION_BTN_W, CAPTION_BTN_H),
    );
    if window_control_button(
        ui,
        cache,
        palette,
        minimize_rect,
        egui::Id::new("riff_titlebar_minimize"),
        Icon::Minimize,
        "Minimize",
        false,
    ) {
        actions.push(TitleBarAction::Minimize);
    }
    let (max_icon, max_label) = if maximized {
        (Icon::Collapse, "Restore")
    } else {
        (Icon::Expand, "Maximize")
    };
    if window_control_button(
        ui,
        cache,
        palette,
        maximize_rect,
        egui::Id::new("riff_titlebar_maximize"),
        max_icon,
        max_label,
        false,
    ) {
        actions.push(TitleBarAction::ToggleMaximize);
    }
    if window_control_button(
        ui,
        cache,
        palette,
        close_rect,
        egui::Id::new("riff_titlebar_close"),
        Icon::Close,
        "Close",
        true,
    ) {
        actions.push(TitleBarAction::Close);
    }
    minimize_rect.left()
}

/// One caption-style window-control strip: transparent until hovered (then a
/// surface fill — or the error fill for the destructive close), carrying a
/// tinted glyph centered in the full hit area. The label doubles as the hover
/// tooltip and the assistive-tech name.
#[expect(clippy::too_many_arguments)]
fn window_control_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    id: egui::Id,
    icon: Icon,
    label: &str,
    danger: bool,
) -> bool {
    let response = ui.interact(rect, id, egui::Sense::click());
    let painter = ui.painter_at(rect);

    if response.hovered() {
        let fill = if danger {
            palette.error
        } else {
            palette.surface_2
        };
        painter.rect_filled(rect, theme::RADIUS_SM, fill);
    }
    // On the close hover fill the glyph flips to the on-brand ink so it
    // stays readable over the error red.
    let tint = if response.hovered() && danger {
        palette.on_brand
    } else if response.hovered() {
        palette.ink
    } else {
        palette.ink_2
    };
    let tex_id = cache.texture(ui.ctx(), icon, 16.0, tint);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
    painter.image(tex_id, icon_rect, UV_FULL, tint);

    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_text(label).clicked()
}

/// The right-edge nav-control cluster: theme / Now Playing / Settings /
/// Advanced toggles. The minimize/close caption pair is drawn by
/// [`show_titlebar`] itself, flush to the window corner. Runs inside a
/// right-to-left scope covering the strip left of that pair; observed actions
/// append to `actions`.
fn show_titlebar_controls(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &TitleBarContent<'_>,
    actions: &mut Vec<TitleBarAction>,
) {
    ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
    ui.visuals_mut().button_frame = false;

    let advanced_label = if content.advanced_mode {
        "Advanced: On"
    } else {
        "Advanced: Off"
    };
    if ui
        .button(advanced_label)
        .on_hover_text(
            "Reveals power features: tag editing, smart playlists, \
             and extra transport controls (stop, repeat).",
        )
        .clicked()
    {
        actions.push(TitleBarAction::ToggleAdvanced);
    }

    let settings_tint = if content.active_nav == Some(NavDestination::Settings) {
        palette.brand_primary
    } else {
        palette.ink_2
    };
    if icon_button(ui, cache, Icon::Settings, "Settings", 18.0, settings_tint)
        .on_hover_text("Settings")
        .clicked()
    {
        actions.push(TitleBarAction::GoSettings);
    }

    // Now Playing replaces the view, which is exactly when no nav
    // destination is active — so it carries the active tint then.
    let now_playing_tint = if content.active_nav.is_none() {
        palette.brand_primary
    } else {
        palette.ink_2
    };
    if icon_button(
        ui,
        cache,
        Icon::Music,
        "Now Playing",
        18.0,
        now_playing_tint,
    )
    .on_hover_text("Now Playing")
    .clicked()
    {
        actions.push(TitleBarAction::ToggleNowPlaying);
    }

    let (theme_icon, theme_hover) = if content.theme_dark {
        (Icon::Sun, "Switch to light theme")
    } else {
        (Icon::Moon, "Switch to dark theme")
    };
    if icon_button(ui, cache, theme_icon, "Theme", 18.0, palette.ink_2)
        .on_hover_text(theme_hover)
        .clicked()
    {
        actions.push(TitleBarAction::ToggleTheme);
    }
    ui.add_space(8.0);
}

/// The wordmark's "riff" title, painted at the equalizer glyph's right edge
/// in the brand orange. Returns the text's right edge so the scan status can
/// start clear of it.
fn paint_wordmark_text(
    ui: &mut egui::Ui,
    mark_rect: egui::Rect,
    center_y: f32,
    palette: &Palette,
) -> f32 {
    let wordmark = ui.painter().layout_no_wrap(
        "riff".to_owned(),
        egui::FontId::proportional(18.0),
        palette.brand_primary,
    );
    let pos = egui::pos2(
        mark_rect.right() + WORDMARK_GAP,
        center_y - wordmark.size().y / 2.0,
    );
    ui.painter()
        .galley(pos, wordmark.clone(), palette.brand_primary);
    pos.x + wordmark.size().x
}

/// Paint the wordmark's static equalizer glyph: four rounded bars of fixed
/// heights ([`WORDMARK_BARS`]) in one color.
#[expect(clippy::cast_precision_loss)]
fn paint_equalizer_mark(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let n = WORDMARK_BARS.len() as f32;
    let bar_w = rect.width() / (n * 1.6);
    let gap = (rect.width() - bar_w * n) / (n - 1.0);
    for (i, h) in WORDMARK_BARS.iter().enumerate() {
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

/// The rounded input well behind the titlebar search field: surface-2 fill
/// with the sidebar search's ring border — hairline when idle, focus ring
/// when the field has keyboard focus. Moved here with the search field from
/// the deleted content top bar.
fn paint_search_well(ui: &egui::Ui, palette: &Palette, rect: egui::Rect, focused: bool) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_2);
    painter.rect_stroke(
        rect,
        theme::RADIUS_MD,
        search_ring_stroke(palette, focused),
        egui::StrokeKind::Inside,
    );
}

/// The field inside the search well: search glyph, frameless text edit with
/// the "Search or jump to…" hint, and a clear affordance while the query is
/// non-empty. Returns the text edit's response for the caller's focus logic.
fn show_search_field(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    query: &mut String,
    search_rect: egui::Rect,
    id: egui::Id,
) -> egui::Response {
    let inner = search_rect.shrink2(egui::vec2(10.0_f32, 4.0_f32));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_MD;

            let tex_id = cache.texture(ui.ctx(), Icon::Search, 16.0, palette.ink_3);
            let sized = egui::load::SizedTexture::new(tex_id, egui::vec2(16.0, 16.0));
            ui.add(egui::Image::from_texture(sized));

            let response = ui.add(
                egui::TextEdit::singleline(query)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .hint_text("Search or jump to…")
                    .desired_width(ui.available_width() - 20.0),
            );

            if !query.is_empty() {
                let clear_rect = egui::Rect::from_center_size(
                    egui::pos2(inner.right() - 10.0, search_rect.center().y),
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
    )
    .inner
}

/// Keyboard dismissal (REQ-UI-007 parity): while the field has focus,
/// Escape clears the query and gives the focus back, so a keyboard user can
/// operate — and dismiss — the search entirely from the keyboard.
///
/// The gate is *last frame's* focus, not this frame's: egui itself clears
/// keyboard focus during pass begin when Escape is pressed, so by the time
/// widget code runs on the Escape frame the field no longer reports focus.
fn handle_search_dismiss(ui: &egui::Ui, id: egui::Id, focused: bool, query: &mut String) {
    let focus_key = id.with("had_focus");
    let had_focus = focused || ui.memory(|m| m.data.get_temp::<bool>(focus_key).unwrap_or(false));
    ui.memory_mut(|m| m.data.insert_temp(focus_key, focused));
    if had_focus && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        query.clear();
        ui.memory_mut(|m| m.surrender_focus(id));
    }
}
