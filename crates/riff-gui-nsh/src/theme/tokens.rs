//! The riff token scale on nsh, ported verbatim from the egui design
//! system's store (`crates/riff-gui-theme/src/theme.rs`): the type scale,
//! the radius scale, the spacing scale, the motion durations, the chrome
//! dimensions, and the menu/window shadows.
//!
//! The four type sizes are also nsh's own named scale values
//! (`text-xs` 12, `text-sm` 14, `text-xl` 20, `text-3xl` 30), because both
//! scales are lifted from the same Tailwind steps — a view may ride nsh's
//! named classes and stay on riff's scale. Everything else is applied
//! through the numeric style setters (`text_size(px(…))`,
//! `rounded(px(…))`), so the values here are the design's, not the
//! framework's.

use nsh_core::BoxShadow;
use nsh_core::Color;
use nsh_core::geometry::Points;

// --- Type scale (`text-xs/sm/xl/3xl`) ----------------------------------------
//
// Sized from the mockup pages' Tailwind usage: xs and sm carry nearly all
// UI text, xl heads sections, 3xl is the Now Playing title.

/// Tailwind `text-xs` — 12 px: muted labels, meta lines.
pub const TEXT_XS: f32 = 12.0;
/// Tailwind `text-sm` — 14 px: the workhorse size for body and buttons.
pub const TEXT_SM: f32 = 14.0;
/// Tailwind `text-xl` — 20 px: section headings (mockup h1s).
pub const TEXT_XL: f32 = 20.0;
/// Tailwind `text-3xl` — 30 px: the Now Playing title.
pub const TEXT_3XL: f32 = 30.0;

// --- Radius scale (`--riff-radius-*`) -----------------------------------------
//
// Lifted two px per step off the extracted mockup scale: at 12 px type the
// extra softness is what makes a 24 px-tall control read as a control.

/// `--riff-radius-xs` — 3 px: hairline-affordance corners.
pub const RADIUS_XS: f32 = 3.0;
/// `--riff-radius-sm` — 6 px: small controls (buttons, inputs).
pub const RADIUS_SM: f32 = 6.0;
/// `--riff-radius-md` — 10 px: cards, menus, popovers.
pub const RADIUS_MD: f32 = 10.0;
/// `--riff-radius-lg` — 14 px: windows and large containers.
pub const RADIUS_LG: f32 = 14.0;
/// `--riff-radius-xl` — 18 px: hero surfaces such as the Now Playing cover.
pub const RADIUS_XL: f32 = 18.0;
/// `--riff-radius-full` — 999 px: pills and circular elements.
pub const RADIUS_FULL: f32 = 999.0;

// --- Spacing scale -------------------------------------------------------------
//
// The gaps view code leaves between items. Every step is a value the app
// already used before the scale was declared.

/// 4 px — the tightest gap: a glyph beside its label inside one control.
pub const SPACE_XS: f32 = 4.0;
/// 6 px — a thumbnail beside its text in a list row.
pub const SPACE_SM: f32 = 6.0;
/// 8 px — the default gap between sibling controls in a row.
pub const SPACE_MD: f32 = 8.0;
/// 12 px — the gap at a boundary between grouped regions (the inspector's
/// sections).
pub const SPACE_LG: f32 = 12.0;

// --- Motion --------------------------------------------------------------------
//
// Durations, in seconds — the one family whose unit is not a pixel. Motion
// is family-invariant (four motion token sets would be four things to keep
// coherent for no gain), so these are flat consts, not [`Palette`] slots,
// exactly as the source has them.

/// The global default duration, in seconds, for everything the framework
/// animates on its own.
pub const MOTION_DEFAULT: f32 = 0.18;
/// The hover duration, in seconds, for riff-authored hover washes only: a
/// little over half the default, because a hover wash is the one tween the
/// user is looking at when it runs.
pub const MOTION_HOVER: f32 = 0.10;
/// The reduced-motion duration, in seconds: `0.0`, the value every motion
/// token collapses to when the Reduce Motion preference is on. Reduce
/// Motion is a flag, not a color — a separate axis, never a `Palette`
/// slot.
pub const MOTION_REDUCED: f32 = 0.0;

// --- Chrome dimensions (`--riff-titlebar-h`, `--riff-sidebar-w`,
// `--riff-playerbar-h`) ---------------------------------------------------------

/// `--riff-titlebar-h` — 56 px top bar.
pub const TITLEBAR_H: f32 = 56.0;
/// The titlebar brand mark's box: the egui strip's 18 x 20 equalizer glyph.
pub const WORDMARK_MARK_W: f32 = 18.0;
pub const WORDMARK_MARK_H: f32 = 20.0;
/// The gap between the mark and the "riff" text: the egui strip's 16 px.
pub const WORDMARK_GAP: f32 = 16.0;
/// The "riff" wordmark's type size: the egui strip's 18 px `SemiBold`.
pub const WORDMARK_TEXT: f32 = 18.0;
/// `--riff-sidebar-w` — 280 px sidebar.
pub const SIDEBAR_W: f32 = 280.0;
/// Preferred width of an entity list column in the elastic column stage:
/// every non-last list column keeps this width while the window allows it.
pub const COLUMN_WIDTH: f32 = 280.0;
/// Minimum width an entity list column shrinks to before proportional
/// shrinking takes over.
pub const COLUMN_MIN_W: f32 = 200.0;
/// Minimum width of the stage's last (absorbing) column, kept wider than
/// [`COLUMN_MIN_W`] because it usually carries the track table.
pub const LAST_COLUMN_MIN_W: f32 = 320.0;
/// Width of the collapsible inspector (the former selection panel).
pub const INSPECTOR_WIDTH: f32 = 300.0;
/// Height of a track-listing row, and the row estimate the virtual list
/// lays unmeasured rows out with (the plan's N6 line).
pub const ROW_H: f32 = 44.0;
/// The leading cover tile in a track-listing row: the egui store's
/// `ROW_H - 8` square, carried over.
pub const ROW_COVER: f32 = 32.0;
/// Width of the favorite cell at a track row's leading edge, before the
/// cover tile (the egui row's heart cell).
pub const FAVORITE_COL_W: f32 = 24.0;
/// Height of the inspector's art block (the egui store's `ART_H`).
pub const INSPECTOR_ART_H: f32 = 200.0;
/// The music glyph inside a placeholder well: the egui tile's glyph is
/// drawn at half the tile, carried over.
pub const PLACEHOLDER_GLYPH: f32 = 16.0;
/// `--riff-playerbar-h` — 88 px bottom player bar.
pub const PLAYERBAR_H: f32 = 88.0;
/// The Now Playing hero cover's edge.
pub const HERO_COVER: f32 = 320.0;
/// The Now Playing seek row's width (the hero plus its readouts' room).
pub const NOW_PLAYING_SEEK_W: f32 = 440.0;
/// The Up Next panel above the player bar: its width and height.
pub const QUEUE_PANEL_W: f32 = 360.0;
pub const QUEUE_PANEL_H: f32 = 280.0;
/// The settings switch's box: a pill twice as wide as it is tall.
pub const SWITCH_W: f32 = 24.0;
pub const SWITCH_H: f32 = 12.0;

/// The 1 px hairline border: the width every separator, panel edge and
/// focus ring paints at. The egui shell spoke this width as a bare `1.0`
/// at each call site; it is a token here so the shell's hairlines move
/// together.
pub const BORDER_HAIRLINE: f32 = 1.0;

// --- Elevation (`--riff-shadow-*`) ---------------------------------------------
//
// The source speaks elevation in Tailwind box-shadows — a soft,
// near-vertical drop under popovers and floating chrome — and nsh reads the
// same language natively ([`BoxShadow`]), so the four egui shadows port one
// for one. The alphas are family-dependent only because a light canvas
// needs far less ink for the same perceived depth.

/// Menus, popovers and tooltips on the dark family: a tight vertical drop,
/// the mockup's `shadow-lg` read over `--riff-surface`.
pub const SHADOW_MENU_DARK: BoxShadow = BoxShadow {
    offset_x: Points(0.0),
    offset_y: Points(8.0),
    blur: Points(20.0),
    spread: Points(0.0),
    color: Color(0x0000_0073), // black at 115/255
    inset: false,
};

/// Menus, popovers and tooltips on the light family: the same geometry at
/// the lower coverage a bright canvas needs.
pub const SHADOW_MENU_LIGHT: BoxShadow = BoxShadow {
    offset_x: Points(0.0),
    offset_y: Points(8.0),
    blur: Points(20.0),
    spread: Points(0.0),
    color: Color(0x0000_002d), // black at 45/255
    inset: false,
};

/// Floating windows on the dark family: the menu drop, one step larger.
pub const SHADOW_WINDOW_DARK: BoxShadow = BoxShadow {
    offset_x: Points(0.0),
    offset_y: Points(12.0),
    blur: Points(28.0),
    spread: Points(0.0),
    color: Color(0x0000_0078), // black at 120/255
    inset: false,
};

/// Floating windows on the light family: the same drop at light coverage.
pub const SHADOW_WINDOW_LIGHT: BoxShadow = BoxShadow {
    offset_x: Points(0.0),
    offset_y: Points(12.0),
    blur: Points(28.0),
    spread: Points(0.0),
    color: Color(0x0000_0032), // black at 50/255
    inset: false,
};
