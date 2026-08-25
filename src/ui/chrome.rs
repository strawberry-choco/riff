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
use super::theme::{self, Palette};
use crate::app::state::{BrowseMode, ViewMode};
use eframe::egui;

/// Smallest main-stage area kept usable beside/between the fixed chrome.
pub const MIN_STAGE_SIZE: egui::Vec2 = egui::vec2(520.0, 456.0);

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
    /// Close deliberately routes through [`egui::ViewportCommand::Close`] —
    /// the same path as the OS close button — so close-to-tray (REQ-SI-001)
    /// keeps vetoing it into a hide on macOS/Windows. It must never bypass
    /// that logic with a hard exit, or closing would silently kill playback.
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
    /// Close the window (routes through the vetoable close-to-tray path).
    Close,
}

/// Draw the shell titlebar inside its panel: background, wordmark, scan
/// status, the drag region, and the control cluster at the right edge
/// (theme / Now Playing / Settings / Advanced toggles plus minimize/close).
///
/// Must run inside a top panel of exactly [`crate::ui::theme::TITLEBAR_H`]
/// height with no frame margins, so the drag region covers the full strip.
///
/// Observed actions are appended to `actions` — a buffer the caller owns and
/// clears per frame, so idle frames never build a fresh `Vec`. The caller
/// applies them to app state and viewport commands.
pub fn show_titlebar(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &TitleBarContent<'_>,
    actions: &mut Vec<TitleBarAction>,
) {
    let rect = ui.max_rect();

    // Register the drag region FIRST so the buttons added below sit on top
    // and win pointer events over their slice of the strip.
    let drag_response = ui.interact(
        rect,
        egui::Id::new("riff_titlebar_drag_region"),
        egui::Sense::click_and_drag(),
    );

    // Wordmark at the left edge.
    ui.painter().text(
        rect.left_center() + egui::vec2(16.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "riff",
        egui::FontId::proportional(18.0),
        palette.ink,
    );

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

    // Scan status sits next to the wordmark, muted.
    if let Some(status) = content.scan_status {
        ui.painter().text(
            rect.left_center() + egui::vec2(72.0, 0.0),
            egui::Align2::LEFT_CENTER,
            status,
            egui::FontId::proportional(theme::TEXT_SM),
            palette.ink_3,
        );
    }

    // Controls at the right edge, Windows order: minimize left of close.
    // Drawn after the drag region so they take priority over it.
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            show_titlebar_controls(ui, cache, palette, content, actions);
        },
    );
}

/// The right-edge control cluster: theme / Now Playing / Settings / Advanced
/// toggles plus the custom minimize/close pair. Runs inside a right-to-left
/// scope covering the full titlebar strip; observed actions append to
/// `actions`.
fn show_titlebar_controls(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &TitleBarContent<'_>,
    actions: &mut Vec<TitleBarAction>,
) {
    ui.spacing_mut().item_spacing.x = 4.0;
    ui.visuals_mut().button_frame = false;

    for (control, label, glyph) in [
        (WindowControl::Close, "Close", Icon::Close),
        (WindowControl::Minimize, "Minimize", Icon::Minimize),
    ] {
        if icon_button(ui, cache, glyph, label, 16.0, palette.ink_2)
            .on_hover_text(label)
            .clicked()
        {
            actions.push(match control {
                WindowControl::Close => TitleBarAction::Close,
                WindowControl::Minimize => TitleBarAction::Minimize,
            });
        }
    }
    ui.add_space(8.0);

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
