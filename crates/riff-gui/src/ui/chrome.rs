//! Frameless window chrome and the unified app shell (Issues 04 + 06).
//!
//! riff launches undecorated (`decorations(false)`) and draws its own
//! titlebar: a full-width drag region plus custom minimize/close controls.
//! The approach is the one egui itself validates in its `custom_window_frame`
//! example — register the drag-region interact first so the control buttons
//! drawn after it sit on top and win clicks over their slice of the strip.
//!
//! Since Issue 06 the titlebar is also the shell's top chrome: the former
//! top-bar content (scan status, theme / view toggles) is merged into the same
//! 56px strip, nav routes to exactly one visible View, and a token-derived
//! minimum window size keeps the fixed chrome from collapsing.
//!
//! Headless seams (tested in `tests/ui_tests.rs`): the launch viewport
//! configuration, the control→action contract, the drag-region gesture
//! decision, and the nav routing. The pixels are covered by the golden-image
//! harness (`tests/golden_tests.rs`, `shell_chrome_dark`). The equalizer mark
//! this module paints is also the one rasterized for the tray and the OS window
//! icon, so the three surfaces cannot disagree about what riff looks like.

use super::icons::{Icon, IconCache, icon_button};
use super::theme::geometry::sidebar::SEARCH_H;
#[cfg(not(target_os = "macos"))]
use super::theme::geometry::titlebar::{CAPTION_BTN_H, CAPTION_BTN_W};
use super::theme::geometry::titlebar::{
    CAPTION_GAP, SEARCH_EDGE_INSET, SEARCH_GAP, SEARCH_MAX_W, TRAFFIC_LIGHT_CLEARANCE,
    WORDMARK_GAP, WORDMARK_LEFT_INSET,
};
use super::theme::geometry::window;
use super::theme::{self, Palette};
use eframe::egui;
use riff_backend::app::state::{BrowseMode, ViewMode};

/// Full-texture UV rect for [`egui::Painter::image`] (sidebar precedent).
/// Only the caption cluster's glyphs use it here; that cluster is not
/// compiled on macOS, so neither is this.
#[cfg(not(target_os = "macos"))]
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

/// The static normalized bar heights of the wordmark's equalizer glyph — a
/// fixed brand mark that moved into the titlebar from the content top bar.
const WORDMARK_BARS: [f32; 4] = [0.55, 0.95, 0.7, 0.4];

/// Raster resolution of the brand mark for the OS surfaces that consume it as
/// pixels: larger than any tray slot, so the OS downsamples an anti-aliased
/// glyph instead of upscaling a blocky one.
pub const APP_ICON_PX: u32 = 64;

/// Which window-chrome convention this platform runs under — the single
/// branch point for the per-OS split (macos-native-title-bar issue 01,
/// amending ADR 0005). Both the launch viewport configuration and the
/// titlebar renderer consume this decision, so the branches cannot drift
/// apart; it is pure data, which is what makes the macOS branch assertable
/// from the Linux/Windows CI machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChromeMode {
    /// Windows/Linux: the window is frameless and riff draws its own
    /// Windows-convention caption controls in the strip's top-right corner.
    #[default]
    CustomCaption,
    /// macOS: the window keeps its `AppKit` decorations with a transparent
    /// title bar carrying riff's full-size content; the system's traffic
    /// lights are the window controls and riff draws none.
    NativeTrafficLights,
}

/// The per-OS chrome decision every chrome branch hangs off: native traffic
/// lights on macOS, custom caption controls elsewhere. One pure function —
/// the viewport builder and the titlebar renderer both read it, never their
/// own `cfg`s.
#[must_use]
pub fn chrome_mode() -> ChromeMode {
    #[cfg(target_os = "macos")]
    {
        ChromeMode::NativeTrafficLights
    }
    #[cfg(not(target_os = "macos"))]
    {
        ChromeMode::CustomCaption
    }
}

/// Launch viewport configuration for the frameless window: the decorated
/// window's launch size carries over unchanged, OS decorations are replaced
/// by riff's custom titlebar, the minimum size fits the fixed shell, and the
/// OS gets riff's own mark rather than eframe's default `e` icon.
#[must_use]
pub fn viewport_builder() -> egui::ViewportBuilder {
    viewport_builder_for(chrome_mode())
}

/// The launch viewport configuration for one chrome branch — the launch
/// viewport's consumption of the [`chrome_mode`] decision, kept separate from
/// the wrapper so the native (macOS) branch stays executable and assertable
/// as data on every platform. The sizes and the riff mark are shared; only
/// the decoration differs:
///
/// - [`ChromeMode::CustomCaption`]: frameless exactly as before — OS
///   decorations are replaced by riff's custom titlebar.
/// - [`ChromeMode::NativeTrafficLights`]: the window keeps its `AppKit`
///   decorations with a full-size content view, a transparent title bar, and
///   a hidden title, so the strip renders behind the system title bar and the
///   traffic lights stay native and correctly placed. The traffic-light
///   buttons stay shown — they are the point.
#[must_use]
pub fn viewport_builder_for(mode: ChromeMode) -> egui::ViewportBuilder {
    let builder = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([window::MIN_WINDOW_SIZE.x, window::MIN_WINDOW_SIZE.y])
        .with_icon(window_icon());
    match mode {
        ChromeMode::CustomCaption => builder.with_decorations(false),
        ChromeMode::NativeTrafficLights => builder
            .with_decorations(true)
            .with_fullsize_content_view(true)
            // Two separate egui fields, because egui-winit maps each to exactly
            // one winit call (egui-winit 0.35 src/lib.rs):
            //
            //   title_shown      -> with_title_hidden(title_shown == false)
            //                      -> AppKit `titleVisibility = hidden`
            //   titlebar_shown   -> with_titlebar_transparent(titlebar_shown == false)
            //                      -> AppKit `titlebarAppearsTransparent = true`
            //
            // Nothing maps one to the other, so setting only `titlebar_shown`
            // yields a transparent bar that AppKit still paints the window
            // title text into — visible behind the strip. Both are required.
            .with_title_shown(false)
            .with_titlebar_shown(false),
    }
}

/// The strip's left-cluster clearance on the native-chrome branch, in egui
/// points: the macOS traffic lights' span, measured where eframe can measure
/// it (`eframe::WindowChromeMetrics`, reported in native scale and divided by
/// the zoom factor to land in egui points), floored at the documented
/// fallback [`TRAFFIC_LIGHT_CLEARANCE`] where it cannot. The measurement is
/// the target; the fixed constant is the acceptable floor — a clearance below
/// the real cluster would put the wordmark under the lights.
#[must_use]
pub fn traffic_light_clearance(measured_native_width: Option<f32>, zoom_factor: f32) -> f32 {
    measured_native_width.map_or(TRAFFIC_LIGHT_CLEARANCE, |w| {
        (w / zoom_factor).max(TRAFFIC_LIGHT_CLEARANCE)
    })
}

/// How far into the strip the left cluster (wordmark, scan status) starts, in
/// egui points. On the custom branch this is the small inset it always was;
/// on the native branch it is the traffic-light clearance, so the cluster can
/// never sit under the system's lights.
#[must_use]
pub fn titlebar_left_inset(mode: ChromeMode, traffic_clearance: f32) -> f32 {
    match mode {
        ChromeMode::CustomCaption => WORDMARK_LEFT_INSET,
        ChromeMode::NativeTrafficLights => traffic_clearance,
    }
}

/// The traffic lights' measured width in native scale, straight from eframe's
/// window-chrome metrics — `None` wherever eframe cannot measure: no window
/// yet, a non-AppKit window handle, or any non-macOS build (the metrics are
/// macOS-only). Feed it to [`traffic_light_clearance`], which divides by the
/// zoom factor and floors at the documented fallback.
#[must_use]
pub fn measured_traffic_lights_width(frame: &eframe::Frame) -> Option<f32> {
    #[cfg(target_os = "macos")]
    {
        use raw_window_handle::HasWindowHandle;
        let window = frame.winit_window()?;
        let handle = window.window_handle().ok()?;
        eframe::WindowChromeMetrics::from_window_handle(&handle.as_raw())
            .map(|metrics| metrics.traffic_lights_size.x)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        None
    }
}

/// How far a measured frame may differ from its target before the `AppKit` shell
/// bothers to write it back, in points. The drift check runs every frame, so
/// the epsilon is what separates "a layout pass with no effect" from
/// "`AppKit` re-centred the cluster again" — it must sit above the sub-point
/// noise `AppKit`'s own layout produces and below the ~14pt jump it produces on
/// a real relayout. Not a design value: it is a tolerance on a measurement,
/// which is why it lives here with the algorithm rather than in the token store.
const TRAFFIC_LIGHT_DRIFT_EPSILON: f64 = 0.5;

/// Where the traffic lights sit vertically inside riff's strip, and how tall the
/// `AppKit` titlebar container must be to hold them: `(inset_y, container_h)`.
///
/// `AppKit` centres the standard buttons for ITS OWN titlebar height — 28pt
/// pre-Tahoe, 32pt on macOS 26, with 16pt/14pt button frames — so in riff's
/// 56pt strip they ride roughly 14pt above centre. This is the correction:
/// the inset is half of whatever is left after the measured button, so
/// `button_h + 2 * inset_y == strip_h` by construction. That equality is the
/// property worth protecting, because it makes the placement correct whether or
/// not the container is coordinate-flipped — only the container's *anchor* is
/// orientation-sensitive, and that lives in [`crate::ui::traffic_lights`].
///
/// `strip_h` is [`crate::ui::theme::TITLEBAR_H`] read, never a literal: the
/// strip is 56pt on every platform and this must not become a way to make it
/// otherwise. `button_h` is measured off the live `NSButton` rather than
/// hardcoded, so Apple's next titlebar change costs nothing.
#[must_use]
pub fn traffic_light_plan(strip_h: f64, button_h: f64) -> (f64, f64) {
    ((strip_h - button_h) / 2.0, strip_h)
}

/// Whether the measured geometry has drifted far enough from the target to be
/// worth writing. The steady state is the fast path: `AppKit` resets the
/// button positions on relayout (and `setTitle:` — which riff sends on every
/// track change — is a documented trigger), so the check is what puts them
/// back, but with the epsilon in place a settled window performs zero `AppKit`
/// writes.
#[must_use]
pub fn needs_reapply(measured_h: f64, measured_y: f64, target_h: f64, target_y: f64) -> bool {
    (measured_h - target_h).abs() > TRAFFIC_LIGHT_DRIFT_EPSILON
        || (measured_y - target_y).abs() > TRAFFIC_LIGHT_DRIFT_EPSILON
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
// `Eq` had to go when the traffic-light clearance joined: a float field on an
// otherwise discrete struct. `PartialEq` covers every comparison made on it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TitleBarContent<'a> {
    /// Library scan status line shown next to the wordmark.
    pub scan_status: Option<&'a str>,
    /// Whether the dark palette is active (drives the theme glyph).
    pub theme_dark: bool,
    /// Which nav destination is active; `None` while Now Playing replaces
    /// the view (then the Now Playing control carries the active tint).
    pub active_nav: Option<NavDestination>,
    /// The chrome branch this frame renders under — `chrome_mode()` in
    /// production, passed in so the native (macOS) branch stays executable
    /// and assertable as data on every platform.
    pub chrome: ChromeMode,
    /// The traffic-light clearance in egui points for the native branch, from
    /// [`traffic_light_clearance`]. Ignored on the custom branch.
    pub traffic_clearance: f32,
}

/// What the user did to the titlebar this frame. The app applies these
/// through its state/viewport-command paths so every effect stays testable
/// headlessly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleBarAction {
    /// Flip between the light and dark palettes.
    ToggleTheme,
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
/// at the right edge (theme / Now Playing / Settings toggles plus
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

    // Wordmark after the left inset: on the custom branch the small inset it
    // always had; on the native branch past the traffic-light clearance. The
    // text is measured so the scan status starts clear of it.
    let mark_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.left() + titlebar_left_inset(content.chrome, content.traffic_clearance) + 9.0,
            rect.center().y,
        ),
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
                // Custom chrome only: on the native branch the system's own
                // double-click-titlebar preference governs the titlebar band
                // (its drag surface), and riff hardcodes no maximize toggle
                // there — a mac owner's system setting wins.
                if content.chrome == ChromeMode::CustomCaption {
                    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
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

    // Window controls: on the custom branch, three caption-style hit strips
    // flush to the top-right corner (Windows convention — minimize |
    // maximize | close, zero gap between them), drawn after the drag region
    // so they win clicks over their slice of the strip. On the native branch
    // the system's traffic lights are the window controls and riff emits no
    // window-control actions at all — the caption code is not even compiled
    // on macOS.
    let minimize_left = match content.chrome {
        ChromeMode::NativeTrafficLights => rect.right(),
        #[cfg(not(target_os = "macos"))]
        ChromeMode::CustomCaption => draw_caption_controls(ui, cache, palette, rect, actions),
        // chrome_mode() never answers CustomCaption on macOS, so this arm
        // exists only to keep the match exhaustive there — it renders the
        // same right edge as the native arm. The caption code above stays
        // cfg'd out: no dead button code on the native branch.
        #[cfg(target_os = "macos")]
        ChromeMode::CustomCaption => rect.right(),
    };

    // Nav controls (theme / Now Playing / Settings toggles) at the
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

    // The shared single-line text field: it reads focus and paints its own
    // ring, edits `search_query` in place, offers the clear affordance, and
    // dismisses on Escape. Returns the field's response so this caller can keep
    // driving the Ctrl+K request-focus shortcut.
    let id = egui::Id::new("riff_global_search");
    super::text_field::text_field(
        ui,
        cache,
        palette,
        search_query,
        &super::text_field::TextField {
            id,
            rect: search_rect,
            hint: "Search or jump to…",
            leading_icon: Some(Icon::Search),
            clear_label: Some("Clear search"),
            dismiss_on_escape: true,
        },
    )
}

/// The minimize | maximize | close caption strips: three caption-style hit
/// areas flush to the top-right corner (Windows convention, zero gap between
/// them), observed actions appended to `actions`. Returns the minimize
/// strip's left edge — the nav cluster ends one [`CAPTION_GAP`] left of it.
///
/// Custom chrome only: not compiled on macOS, where the system's traffic
/// lights are the window controls.
#[cfg(not(target_os = "macos"))]
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
#[cfg(not(target_os = "macos"))]
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
    let button = super::button::begin_icon_button(ui, rect, id, false);
    let painter = ui.painter_at(rect);

    // Press replaces the hover fill rather than compositing with it, so a held
    // window control never looks like a hovered one that is somehow also held —
    // and the pressed fill is the framework's own active fill, read off the
    // style, so these controls agree with egui's stock widgets about what a
    // press is. Instant (rule 4 of the motion rule in `theme`). The destructive
    // close's error red is a *hover* treatment, so it steps aside for the press
    // exactly as the neutral one does.
    if button.pressed {
        painter.rect_filled(rect, theme::RADIUS_SM, super::button::active_fill(ui));
    } else if button.hovered {
        let fill = if danger {
            palette.error
        } else {
            palette.surface_2
        };
        painter.rect_filled(rect, theme::RADIUS_SM, fill);
    }
    // On the close hover fill the glyph flips to the on-brand ink so it
    // stays readable over the error red.
    let tint = if button.hovered && danger {
        palette.on_brand
    } else if button.hovered {
        palette.ink
    } else {
        palette.ink_2
    };
    let tex_id = cache.texture(ui.ctx(), icon, 16.0, tint);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
    painter.image(tex_id, icon_rect, UV_FULL, tint);

    super::button::finish_icon_button(ui, palette, &button, label)
}

/// The right-edge nav-control cluster: theme / Now Playing / Settings
/// toggles. The minimize/close caption pair is drawn by
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
fn paint_equalizer_mark(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    for bar in equalizer_bars() {
        let scaled = egui::Rect::from_min_size(
            rect.min + bar.min.to_vec2() * rect.size(),
            bar.size() * rect.size(),
        );
        painter.rect_filled(scaled, scaled.width() / 2.0, color);
    }
}

/// The wordmark's bars as rects normalized into a unit box — each axis read as
/// a fraction of the box's own extent. The titlebar scales them into its mark
/// rect and [`mark_svg`] scales them into an SVG canvas, so the glyph the
/// window paints and the glyph the tray shows cannot drift apart.
#[must_use]
#[expect(clippy::cast_precision_loss)]
fn equalizer_bars() -> [egui::Rect; 4] {
    let n = WORDMARK_BARS.len() as f32;
    let bar_w = 1.0 / (n * 1.6);
    let gap = (1.0 - bar_w * n) / (n - 1.0);
    std::array::from_fn(|i| {
        let h = WORDMARK_BARS[i];
        egui::Rect::from_min_size(
            egui::pos2(i as f32 * (bar_w + gap), (1.0 - h) / 2.0),
            egui::vec2(bar_w, h),
        )
    })
}

/// The brand mark as a square SVG document tinted through the shell's
/// `currentColor` convention, sized at [`APP_ICON_PX`].
///
/// Drawn with a margin: the tray shrinks the mark hard, and a glyph that
/// fills its frame edge to edge reads as a blob at that size.
#[expect(clippy::cast_precision_loss)]
fn mark_svg() -> String {
    let canvas = APP_ICON_PX as f32;
    let inset = canvas * 0.1;
    let side = canvas - inset * 2.0;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{canvas:.0}\" \
         height=\"{canvas:.0}\" viewBox=\"0 0 {canvas:.0} {canvas:.0}\">"
    );
    for bar in equalizer_bars() {
        let x = inset + bar.min.x * side;
        let y = inset + bar.min.y * side;
        let w = bar.width() * side;
        let h = bar.height() * side;
        // `format_push_string` would rather `write!`, which hands back an
        // infallible `fmt::Result` to discard on every bar.
        #[expect(clippy::format_push_string)]
        svg.push_str(&format!(
            "<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{w:.3}\" height=\"{h:.3}\" \
             rx=\"{:.3}\" fill=\"currentColor\"/>",
            w / 2.0
        ));
    }
    svg.push_str("</svg>");
    svg
}

/// The brand mark as straight-alpha RGBA8 at [`APP_ICON_PX`], for the OS
/// surfaces that take bytes rather than egui textures — the system tray.
#[must_use]
pub fn icon_rgba() -> Option<Vec<u8>> {
    super::icons::rasterize_rgba(&mark_svg(), APP_ICON_PX as usize, theme::BRAND_500)
}

/// The brand mark for the OS window and taskbar button.
///
/// `with_icon` wants decoded pixels, not an encoded image, so this is the same
/// raster the tray takes — one rasterization, one source of truth, no asset to
/// regenerate when the mark changes.
#[must_use]
pub fn window_icon() -> egui::IconData {
    let rgba = icon_rgba().expect("the brand mark must rasterize");
    egui::IconData {
        rgba,
        width: APP_ICON_PX,
        height: APP_ICON_PX,
    }
}
