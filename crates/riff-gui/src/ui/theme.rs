//! Design-token foundation for the riff UI redesign (Issue 01, ADR 0004).
//!
//! Every color, radius, and chrome dimension the redesign's design-token
//! sheet (`colors_and_type.css`) defines becomes a named constant here; view
//! code must style itself from these tokens (directly or through
//! [`Palette`]) instead of hardcoding colors.
//!
//! Two palettes ship:
//!
//! - **Dark** — the mockup tokens verbatim ([`Palette::dark`]).
//! - **Light** — derived by rule per ADR 0004: surfaces invert (channel-wise
//!   mirror), ink flips, brand amber is unchanged ([`Palette::light`]). The
//!   result is consciously approximate until a light design exists.
//!
//! High Contrast ([`Palette::high_contrast`]) is a token-set variant over
//! each base palette — never a third design.

use eframe::egui;
use egui::{Color32, CornerRadius, Stroke};

use super::fonts;

// --- Type scale (`text-xs/sm/xl/3xl`) ----------------------------------------
//
// Sized from the mockup pages' Tailwind usage (Issue 02): xs and sm carry
// nearly all UI text, xl heads sections, 3xl is the Now Playing title.

/// Tailwind `text-xs` — 12 px: muted labels, meta lines.
pub const TEXT_XS: f32 = 12.0;
/// Tailwind `text-sm` — 14 px: the workhorse size for body and buttons.
pub const TEXT_SM: f32 = 14.0;
/// Tailwind `text-xl` — 20 px: section headings (mockup h1s).
pub const TEXT_XL: f32 = 20.0;
/// Tailwind `text-3xl` — 30 px: the Now Playing title.
pub const TEXT_3XL: f32 = 30.0;

/// The design type scale mapped onto egui's named text styles:
///
/// - [`egui::TextStyle::Small`] → `text-xs`
/// - [`egui::TextStyle::Body`] / [`egui::TextStyle::Monospace`] → `text-sm`
///   (monospace stays on [`egui::FontFamily::Monospace`] so seek/volume time
///   readouts align digit-for-digit)
/// - [`egui::TextStyle::Button`] → `text-sm` at Inter Medium (mockup buttons)
/// - [`egui::TextStyle::Heading`] → `text-xl` at Inter `SemiBold` (mockup h1s)
#[must_use]
pub fn text_styles() -> std::collections::BTreeMap<egui::TextStyle, egui::FontId> {
    use egui::{FontFamily, FontId, TextStyle};
    std::collections::BTreeMap::from([
        (
            TextStyle::Small,
            FontId::new(TEXT_XS, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(TEXT_SM, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(TEXT_SM, FontFamily::Monospace),
        ),
        (
            TextStyle::Button,
            FontId::new(TEXT_SM, fonts::family_medium()),
        ),
        (
            TextStyle::Heading,
            FontId::new(TEXT_XL, fonts::family_semibold()),
        ),
    ])
}

/// The Now Playing title font: `text-3xl` at Inter `SemiBold` — the mockup's
/// single `text-3xl font-semibold` usage, referenced by name from view code.
#[must_use]
pub fn hero_title_font() -> egui::FontId {
    egui::FontId::new(TEXT_3XL, fonts::family_semibold())
}

// --- Brand amber scale (`--riff-brand-*`) -----------------------------------
//
// The brand hue is identical in both palettes (ADR 0004); 500 is the primary.

/// `--riff-brand-50` — `#fff8e7`.
pub const BRAND_50: Color32 = Color32::from_rgb(0xff, 0xf8, 0xe7);
/// `--riff-brand-100` — `#ffefcc`.
pub const BRAND_100: Color32 = Color32::from_rgb(0xff, 0xef, 0xcc);
/// `--riff-brand-200` — `#ffe099`.
pub const BRAND_200: Color32 = Color32::from_rgb(0xff, 0xe0, 0x99);
/// `--riff-brand-300` — `#ffcc66`.
pub const BRAND_300: Color32 = Color32::from_rgb(0xff, 0xcc, 0x66);
/// `--riff-brand-400` — `#ffb833`.
pub const BRAND_400: Color32 = Color32::from_rgb(0xff, 0xb8, 0x33);
/// `--riff-brand-500` — `#f0821e`, the primary brand color.
pub const BRAND_500: Color32 = Color32::from_rgb(0xf0, 0x82, 0x1e);
/// `--riff-brand-600` — `#d98a0d`.
pub const BRAND_600: Color32 = Color32::from_rgb(0xd9, 0x8a, 0x0d);
/// `--riff-brand-700` — `#a66709`.
pub const BRAND_700: Color32 = Color32::from_rgb(0xa6, 0x67, 0x09);

// --- Dark surfaces (`--riff-bg`, `--riff-surface*`) --------------------------
//
// Deep-ink neutral ramp; darkest is the window background, lightest the
// raised accent surface.

/// `--riff-bg` — `#101013`, the window background.
pub const SURFACE_BG: Color32 = Color32::from_rgb(0x10, 0x10, 0x13);
/// `--riff-surface` — `#17171b`, panels and cards (the design's sidebar /
/// top-bar / player-bar panel fill).
pub const SURFACE: Color32 = Color32::from_rgb(0x17, 0x17, 0x1b);
/// `--riff-surface-2` — `#1e1e23`, hover fills and popovers.
pub const SURFACE_2: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x23);
/// `--riff-surface-3` — `#26262d`, raised accents.
pub const SURFACE_3: Color32 = Color32::from_rgb(0x26, 0x26, 0x2d);

// --- Dark ink ladder (`--riff-ink`, `--riff-ink-2`, `--riff-ink-3`) ----------

/// `--riff-ink` — `#ededf0`, primary text.
pub const INK: Color32 = Color32::from_rgb(0xed, 0xed, 0xf0);
/// `--riff-ink-2` — `#9a9aa6`, secondary text.
pub const INK_2: Color32 = Color32::from_rgb(0x9a, 0x9a, 0xa6);
/// `--riff-ink-3` — `#6b6b77`, tertiary/muted text.
pub const INK_3: Color32 = Color32::from_rgb(0x6b, 0x6b, 0x77);

// --- Row hover (`--riff-row-hover`) -------------------------------------------
//
// The design highlights rows with a warm amber wash rather than a surface-ramp
// step: a literal from the mockup's hovered sidebar/track rows.

/// `--riff-row-hover` — `#2a1c0e`, the amber wash painted under hovered rows.
pub const ROW_HOVER: Color32 = Color32::from_rgb(0x2a, 0x1c, 0x0e);

// --- Lines (`--riff-line`, `--riff-border`) ----------------------------------
//
// White overlays on the dark surfaces: 0.08 × 255 ≈ 20, 0.10 × 255 ≈ 26.

/// `--riff-line` — `rgba(255, 255, 255, 0.08)`, hairline separators.
pub const LINE: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 20);
/// `--riff-border` — `rgba(255, 255, 255, 0.10)`, widget borders.
pub const BORDER: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 26);

// --- Status colors (`--riff-state-*`) ----------------------------------------

/// Neutral multiply tint for drawing textures at their own colors (cover
/// art, icon glyphs): pure white leaves every texture pixel untouched.
/// Named here so view code never constructs a flat color literal (ADR 0004).
pub const TEXTURE_TINT: Color32 = Color32::WHITE;

/// `--riff-state-success` — `#22c55e`.
pub const STATE_SUCCESS: Color32 = Color32::from_rgb(0x22, 0xc5, 0x5e);
/// `--riff-state-warning` — aliases `--riff-brand-500`.
pub const STATE_WARNING: Color32 = BRAND_500;
/// `--riff-state-error` — `#ef4444`.
pub const STATE_ERROR: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
/// `--riff-state-info` — `#3b82f6`.
pub const STATE_INFO: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);

// --- Radius scale (`--riff-radius-*`) ----------------------------------------

/// `--riff-radius-sm` — 4 px: small controls (buttons, inputs).
pub const RADIUS_SM: f32 = 4.0;
/// `--riff-radius-md` — 8 px: cards, menus, popovers.
pub const RADIUS_MD: f32 = 8.0;
/// `--riff-radius-lg` — 12 px: windows and large containers.
pub const RADIUS_LG: f32 = 12.0;
/// `--riff-radius-xl` — 16 px: hero surfaces such as the Now Playing cover.
pub const RADIUS_XL: f32 = 16.0;
/// `--riff-radius-full` — 999 px: pills and circular elements.
pub const RADIUS_FULL: f32 = 999.0;

// --- Chrome dimensions (`--riff-titlebar-h`, `--riff-sidebar-w`,
// `--riff-playerbar-h`) ---------------------------------------------------------

/// `--riff-titlebar-h` — 56 px top bar.
pub const TITLEBAR_H: f32 = 56.0;
/// 48 px content top bar (design-handoff issue 06): the second content strip
/// between the frameless titlebar and the library stage, carrying the
/// wordmark, the global search field, and the list/grid view toggles.
pub const TOPBAR_H: f32 = 48.0;
/// `--riff-sidebar-w` — 280 px sidebar.
pub const SIDEBAR_W: f32 = 280.0;

/// Width of the browser column pane (design-handoff issue 08): the first
/// pane of the three-pane explorer, right of the nav sidebar.
pub const BROWSER_W: f32 = 320.0;
/// Width of the selection panel (design-handoff issue 10): the third pane
/// of the explorer — the design's 300×750 panel between the detail column
/// and the window edge.
pub const SELECT_PANEL_W: f32 = 300.0;
/// `--riff-playerbar-h` — 88 px bottom player bar.
pub const PLAYERBAR_H: f32 = 88.0;

// --- Semantic palette ---------------------------------------------------------

/// The focus-ring color High Contrast variants swap in for the brand ring
/// (REQ-UI-007): a bright yellow that reads as "keyboard focus" on both
/// palettes.
const HC_FOCUS_RING: Color32 = Color32::from_rgb(0xff, 0xd7, 0x00);

/// A semantic color set resolved from the raw tokens above: every themed
/// surface reads its colors from an instance of this struct, never from the
/// flat constants, so switching palettes re-themes everything at once
/// (ADR 0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// `true` for the dark family, `false` for light (mirrors
    /// [`egui::Visuals::dark_mode`]).
    pub dark: bool,
    /// Whether this set is the High Contrast variant of its base; the style
    /// builder thickens focus strokes when set.
    pub high_contrast: bool,
    /// Window background (`--riff-bg`).
    pub background: Color32,
    /// Panel/card fill (`--riff-surface`).
    pub surface: Color32,
    /// Hover fills and popovers (`--riff-surface-2`).
    pub surface_2: Color32,
    /// Raised accents (`--riff-surface-3`).
    pub surface_3: Color32,
    /// Amber wash painted under hovered rows (`--riff-row-hover`).
    pub row_hover: Color32,
    /// Primary text (`--riff-ink`).
    pub ink: Color32,
    /// Secondary text (`--riff-ink-2`).
    pub ink_2: Color32,
    /// Tertiary/muted text (`--riff-ink-3`).
    pub ink_3: Color32,
    /// Hairline separators (`--riff-line`).
    pub line: Color32,
    /// Widget borders (`--riff-border`).
    pub border: Color32,
    /// Primary brand fill (`--riff-primary`, brand-500 in both palettes).
    pub brand_primary: Color32,
    /// Text painted on brand fills (`--riff-primary-foreground`).
    pub on_brand: Color32,
    /// Success status (`--riff-state-success`).
    pub success: Color32,
    /// Warning status (`--riff-state-warning`).
    pub warning: Color32,
    /// Error/destructive status (`--riff-state-error`).
    pub error: Color32,
    /// Info status (`--riff-state-info`).
    pub info: Color32,
    /// Keyboard-focus/selection ring (`--riff-ring`).
    pub focus_ring: Color32,
}

impl Palette {
    /// The mockup's dark palette verbatim.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            dark: true,
            high_contrast: false,
            background: SURFACE_BG,
            surface: SURFACE,
            surface_2: SURFACE_2,
            surface_3: SURFACE_3,
            row_hover: ROW_HOVER,
            ink: INK,
            ink_2: INK_2,
            ink_3: INK_3,
            line: LINE,
            border: BORDER,
            brand_primary: BRAND_500,
            on_brand: SURFACE_BG,
            success: STATE_SUCCESS,
            warning: STATE_WARNING,
            error: STATE_ERROR,
            info: STATE_INFO,
            focus_ring: BRAND_500,
        }
    }

    /// The light palette derived by rule per ADR 0004: surfaces invert
    /// (channel-wise mirror of the dark ramp), ink flips, lines flip their
    /// base white→black at unchanged alphas, and brand amber plus status
    /// colors are untouched. Known-imperfect by design until a proper light
    /// design exists.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            dark: false,
            high_contrast: false,
            // Channel-wise mirrors of the dark surfaces (#101013 → #efefec …).
            background: Color32::from_rgb(0xef, 0xef, 0xec),
            surface: Color32::from_rgb(0xe8, 0xe8, 0xe4),
            surface_2: Color32::from_rgb(0xe1, 0xe1, 0xdc),
            surface_3: Color32::from_rgb(0xd9, 0xd9, 0xd2),
            // The wash is brand-derived, so it is NOT mirrored (that would
            // turn it blue): the unchanged brand amber at the ~9% coverage
            // the dark wash reads over its surface keeps the hover warm on
            // light too.
            row_hover: Color32::from_rgba_unmultiplied_const(0xf0, 0x82, 0x1e, 24),
            // Mirrored ink ladder (#ededf0 → #12120f …); brightness inversion
            // preserves the faintness hierarchy against the flipped surfaces.
            ink: Color32::from_rgb(0x12, 0x12, 0x0f),
            ink_2: Color32::from_rgb(0x65, 0x65, 0x59),
            ink_3: Color32::from_rgb(0x94, 0x94, 0x88),
            // Black-based lines at the dark alphas (20 / 26).
            line: Color32::from_rgba_unmultiplied_const(0, 0, 0, 20),
            border: Color32::from_rgba_unmultiplied_const(0, 0, 0, 26),
            // Brand amber is unchanged across palettes (ADR 0004), so text on
            // amber stays deep ink too.
            brand_primary: BRAND_500,
            on_brand: SURFACE_BG,
            success: STATE_SUCCESS,
            warning: STATE_WARNING,
            error: STATE_ERROR,
            info: STATE_INFO,
            focus_ring: BRAND_500,
        }
    }

    /// The High Contrast token-set variant over this base palette (ADR 0004):
    /// text pinned to the extreme of the base, secondary ink strengthened,
    /// line alphas roughly doubled, and the focus ring swapped to
    /// [`HC_FOCUS_RING`]. Surfaces, brand, and status colors inherit the base
    /// so each variant stays recognizably its own design.
    #[must_use]
    pub fn high_contrast(&self) -> Self {
        let mut variant = *self;
        variant.high_contrast = true;
        if self.dark {
            variant.ink = Color32::WHITE;
            variant.ink_2 = Color32::from_gray(200);
            variant.line = Color32::from_rgba_unmultiplied_const(255, 255, 255, 40);
            variant.border = Color32::from_rgba_unmultiplied_const(255, 255, 255, 50);
        } else {
            variant.ink = Color32::BLACK;
            variant.ink_2 = Color32::from_gray(55);
            variant.line = Color32::from_rgba_unmultiplied_const(0, 0, 0, 40);
            variant.border = Color32::from_rgba_unmultiplied_const(0, 0, 0, 50);
        }
        variant.focus_ring = HC_FOCUS_RING;
        variant
    }
}

// --- Generated placeholder colour (design-handoff issue 14) -------------------
//
// The derivation is colour math over the palette family, so it lives beside
// the tokens it derives from; the cache mechanics that materialize it into a
// texture live in `cover_placeholder`.

/// Muted band for the dark family: saturation 28–42%, lightness 20–30%.
/// Bright enough to read against the near-black surfaces, dark enough to
/// never flash white.
const DARK_SATURATION: (f32, f32) = (0.28, 0.42);
const DARK_LIGHTNESS: (f32, f32) = (0.20, 0.30);

/// Mirror band for the light family (ADR 0004 derives light by rule):
/// pastel saturation 24–38%, lightness 76–86% — visible against the light
/// surfaces without washout.
const LIGHT_SATURATION: (f32, f32) = (0.24, 0.38);
const LIGHT_LIGHTNESS: (f32, f32) = (0.76, 0.86);

/// Derive the generated cover placeholder colour for one identity (`seed` —
/// a track's [`TrackId`](riff_persistence::track::TrackId) string or an
/// album key) on the `dark` or light palette family. Deterministic: the hash
/// is a fixed-key `DefaultHasher`, so the colour is stable across runs, not
/// just frames.
#[must_use]
pub fn generated_colour(seed: &str, dark: bool) -> Color32 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    let bits = hasher.finish();

    // Independent bit fields drive hue, saturation, and lightness so the
    // three vary without stepping on each other. The fields are all below
    // 1024, so the u16 conversions cannot fail.
    let hue = f32::from(u16::try_from((bits >> 8) % 360).unwrap_or(0));
    let (saturation, lightness) = if dark {
        (
            band(DARK_SATURATION, bits >> 40),
            band(DARK_LIGHTNESS, bits >> 52),
        )
    } else {
        (
            band(LIGHT_SATURATION, bits >> 40),
            band(LIGHT_LIGHTNESS, bits >> 52),
        )
    };
    hsl_to_rgb(hue, saturation, lightness)
}

/// Map one hash field into a `(lo, hi)` band.
fn band((lo, hi): (f32, f32), bits: u64) -> f32 {
    let fraction = f32::from(u16::try_from(bits % 1024).unwrap_or(0)) / 1024.0;
    lo + fraction * (hi - lo)
}

/// Standard HSL → sRGB conversion (no external dependency: the token crate
/// rule keeps colour math beside the tokens it serves).
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> Color32 {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let hue_prime = hue / 60.0;
    let x = chroma * (1.0 - (hue_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hue_prime as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let channel = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(r1), channel(g1), channel(b1))
}

/// Convert a radius token (px) into an egui [`CornerRadius`], clamping the
/// scale's 999 px "full" step to egui's u8 corner representation.
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn corner(radius: f32) -> CornerRadius {
    CornerRadius::same(radius.clamp(0.0, f32::from(u8::MAX)).round() as u8)
}

/// Build the global [`egui::Style`] for `palette`: backgrounds, widget
/// visuals per kind, corner radii, and strokes — every value from tokens,
/// none from hardcoded colors.
#[must_use]
pub fn style_from(palette: &Palette) -> egui::Style {
    let mut style = egui::Style::default();
    let v = &mut style.visuals;

    // Typography: the design type scale (Issue 02) rides along with the
    // token style so every install re-applies it.
    style.text_styles = text_styles();

    // High Contrast variants thicken focus-bearing strokes (REQ-UI-007).
    let focus_width = if palette.high_contrast {
        2.0_f32
    } else {
        1.0_f32
    };
    let focus_stroke_color = if palette.high_contrast {
        palette.focus_ring
    } else {
        palette.border
    };

    v.dark_mode = palette.dark;
    v.override_text_color = Some(palette.ink);

    // Backgrounds: chrome panels read as cards over the window background;
    // text-edit wells use --riff-input (aliases surface-2); striped rows get
    // the hairline tint.
    v.panel_fill = palette.surface;
    v.window_fill = palette.background;
    v.extreme_bg_color = palette.surface_2;
    v.faint_bg_color = palette.line;
    v.code_bg_color = palette.surface_2;

    // Window chrome: border stroke, lg corners; menus pop at md.
    v.window_stroke = Stroke::new(focus_width, palette.border);
    v.window_corner_radius = corner(RADIUS_LG);
    v.menu_corner_radius = corner(RADIUS_MD);

    // Links and status text.
    v.hyperlink_color = palette.brand_primary;
    v.warn_fg_color = palette.warning;
    v.error_fg_color = palette.error;

    // Selection / keyboard-focus ring.
    v.selection.bg_fill = palette.brand_primary.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(focus_width, palette.focus_ring);

    // Widget states, each from tokens: sm corners everywhere; hover fills on
    // surface-2; pressed fills on surface-3; strokes from the line tokens.
    let w = &mut v.widgets;

    w.noninteractive.bg_fill = palette.surface;
    w.noninteractive.weak_bg_fill = palette.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, palette.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, palette.ink_2);
    w.noninteractive.corner_radius = corner(RADIUS_SM);

    w.inactive.bg_fill = palette.surface_2;
    w.inactive.weak_bg_fill = palette.surface;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, palette.border);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, palette.ink);
    w.inactive.corner_radius = corner(RADIUS_SM);

    w.hovered.bg_fill = palette.surface_2;
    w.hovered.weak_bg_fill = palette.surface_2;
    w.hovered.bg_stroke = Stroke::new(focus_width, focus_stroke_color);
    w.hovered.fg_stroke = Stroke::new(focus_width, palette.ink);
    w.hovered.corner_radius = corner(RADIUS_SM);

    w.active.bg_fill = palette.surface_3;
    w.active.weak_bg_fill = palette.surface_3;
    w.active.bg_stroke = Stroke::new(focus_width, focus_stroke_color);
    w.active.fg_stroke = Stroke::new(focus_width, palette.ink);
    w.active.corner_radius = corner(RADIUS_SM);

    w.open.bg_fill = palette.surface_2;
    w.open.weak_bg_fill = palette.surface_2;
    w.open.bg_stroke = Stroke::new(1.0_f32, palette.border);
    w.open.fg_stroke = Stroke::new(1.0_f32, palette.ink);
    w.open.corner_radius = corner(RADIUS_SM);

    style
}

/// The keyboard-focus ring for a custom-painted row or cell (handoff issue
/// 16): `Some` ring stroke while the widget holds keyboard focus, `None`
/// when idle — a row paints nothing extra unless it IS the focused widget.
/// The ring rides the palette's `focus_ring` token and thickens in High
/// Contrast mode (REQ-UI-007), matching the search well's ring.
///
/// Callers pass the memory-focus read (`ui.memory(|m| m.has_focus(id))`),
/// not `Response::has_focus`, so the ring lands on the same frame the focus
/// changes — the search-box precedent.
#[must_use]
pub fn focus_ring_stroke(palette: &Palette, focused: bool) -> Option<Stroke> {
    if !focused {
        return None;
    }
    let width = if palette.high_contrast {
        2.0_f32
    } else {
        1.5_f32
    };
    Some(Stroke::new(width, palette.focus_ring))
}

/// Resolve the active [`Palette`] for a `(dark, high_contrast)` theme
/// selection: the base family per ADR 0004 with High Contrast applied as a
/// token-set variant over it, never a third design. The single resolution
/// path shared by the global style install and any view code that needs the
/// active palette's semantic slots.
#[must_use]
pub fn resolve(dark: bool, high_contrast: bool) -> Palette {
    let mut palette = if dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    if high_contrast {
        palette = palette.high_contrast();
    }
    palette
}

/// Apply `palette` globally to `ctx` in one call: pins egui's theme
/// preference to the palette's family and installs the token-built style for
/// it, so every subsequent frame renders from this token set.
pub fn install(ctx: &egui::Context, palette: &Palette) {
    let theme = if palette.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.set_style_of(theme, style_from(palette));
}
