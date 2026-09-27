//! The design system: every visual value riff ships with is declared here and
//! read from here (ADR 0004) — colors, corner radii, the type scale, the
//! spacing scale, the chrome dimensions, and the component geometry grouped by
//! the surface that paints it. The color helpers below ([`blend_over`],
//! [`glow`], [`destructive_fill`]) are the only sanctioned way to derive one
//! color from another; view code never scales a token at a call site.
//!
//! Two guards in `tests/ui_tests.rs` keep that true rather than merely
//! intended: a source sweep that fails on a color derived outside this module,
//! and one that fails on a geometry constant declared inside a view. What
//! their absence had already produced is recorded in
//! `.scratch/design-handoff/review-2026-09-19.md`.
//!
//! Two palettes ship:
//!
//! - **Dark** — the mockup tokens verbatim ([`Palette::dark`]).
//! - **Light** — derived by rule per ADR 0004: surfaces invert (channel-wise
//!   mirror), ink flips its faintness order, brand amber is unchanged
//!   ([`Palette::light`]). The two muted ink rungs are the deliberate exception
//!   to the mirror: they are chosen so they clear WCAG AA on the light
//!   surfaces, which a flip of the dark ramp does not.
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

/// The brand gradient's first stop: [`BRAND_400`], the lighter end. A wash that
/// steps from here to [`BRAND_GRADIENT_BOTTOM`] reads as one lit shape; the
/// same two rungs at full strength read as two flat bands.
pub const BRAND_GRADIENT_TOP: Color32 = BRAND_400;
/// The brand gradient's second stop: [`BRAND_500`], the primary and the deeper
/// end.
///
/// Both stops are plain consts, not [`Palette`] slots: brand amber is
/// identical in both families (ADR 0004), so a per-family slot for a
/// family-invariant value would be a lie, and the light-mirror rule would then
/// have to carve an exception for it. No hex is introduced — the gradient is
/// the two rungs the scale already had, named for the gradient that reads
/// them.
pub const BRAND_GRADIENT_BOTTOM: Color32 = BRAND_500;

// --- Dark surfaces (`--riff-bg`, `--riff-surface*`) --------------------------
//
// Deep-ink ramp; darkest is the window background, lightest the raised accent
// surface. Two deliberate departures from the extracted mockup hexes
// (`#101013 / #17171b / #1e1e23 / #26262d`), both of them intentional
// amendments to the design handoff rather than drift:
//
// - **Wider separation.** The mockup's steps were 7 / 7 / 8, which read as one
//   flat field at a glance — a panel edge was hard to find. This ramp steps
//   9 / 9 over 4 / 4, so chrome, the row plane, cards and raised accents each
//   hold their own plane without any view changing a single rectangle. The
//   row plane takes the narrow half deliberately: it has to read as *under* a
//   card, so the card's edge is the thing that has to stay findable.
// - **A warm cast.** The mockup's neutrals were cool (blue channel highest),
//   which sits at odds with the amber brand every surface has to carry. These
//   wear a slight warm bias (red ≥ green ≥ blue, ~30° hue) so panels read as
//   part of the same system as the amber instead of gray behind it.
//
// The ink ladder is unchanged and gains headroom, not loses it: every surface
// went darker, so `ink_3` on `surface_3` measures ~4.70:1 where the mockup's
// pair measured 4.63:1. The WCAG test in `tests/ui_tests.rs` holds that floor.

/// `--riff-bg` — `#0e0d0c`, the window background.
pub const SURFACE_BG: Color32 = Color32::from_rgb(0x0e, 0x0d, 0x0c);
/// `--riff-surface` — `#161514`, panels and cards (the design's sidebar /
/// top-bar / player-bar panel fill).
pub const SURFACE: Color32 = Color32::from_rgb(0x16, 0x15, 0x14);
/// `--riff-surface-2` — `#1f1d1b`, hover fills and popovers.
pub const SURFACE_2: Color32 = Color32::from_rgb(0x1f, 0x1d, 0x1b);
/// `--riff-surface-3` — `#282521`, raised accents.
pub const SURFACE_3: Color32 = Color32::from_rgb(0x28, 0x25, 0x21);

/// `--riff-surface-row` — `#121110`, the track/row plane: the field a list of
/// rows is painted on, *below* the card plane rather than beside it.
///
/// The mockup draws that plane darker than its own cards, and the four-step
/// ramp above has no slot for it — `surface` is the card plane, so rows would
/// have had to share a field with the cards sitting on them. This const is the
/// extra step that hierarchy asks for, and it splits the existing
/// `background` → `surface` step of 24 in half rather than opening a new
/// cadence: every step the ramp already had keeps the value it had, and a card
/// still rises a full 12 off a row.
///
/// The mockup's own row hex is deliberately not this value, for two reasons,
/// both amendments to the handoff rather than drift:
///
/// - **Taken literally it inverts the slot.** `#15151b` sums to 69, which is
///   *above* `surface` at 63 — read against the ramp's own ordering it is a
///   card, not a row, so it cannot be the row plane.
/// - **It is cool-cast.** The ramp is deliberately warm (red ≥ green ≥ blue,
///   ~30°) so panels sit in the amber brand's system; a blue-leaning row plane
///   would be the one field in the app that does not. `#121110` is the
///   half-step in the ramp's own register: +4 per channel off the background,
///   carrying the same +1 hue the +8 steps carry.
///
/// The ink ladder is untouched and gains headroom here rather than losing it:
/// `ink_3` measures 5.83:1 on this plane, darker than the 5.63:1 it reads at
/// on `surface` but nowhere near the ramp's tight end. The WCAG test in
/// `tests/ui_tests.rs` holds that floor.
pub const SURFACE_ROW: Color32 = Color32::from_rgb(0x12, 0x11, 0x10);

// --- Dark ink ladder (`--riff-ink`, `--riff-ink-2`, `--riff-ink-3`) ----------
//
// `ink` is the mockup's. `ink_2` and `ink_3` are NOT the extracted hexes
// (`#9a9aa6`, `#6b6b77`): `ink_3` measured 3.61:1 on the window and 3.40:1 on
// a panel, and it carries required text — durations, counts, every muted meta
// line — so it was lifted until it clears WCAG 2.1 AA (4.5:1) on every fill it
// paints on, and `ink_2` lifted with it to keep the three rungs distinguishable
// rather than collapsing into one gray. The contrast test in `tests/ui_tests.rs`
// is what holds all of them up.

/// `--riff-ink` — `#ededf0`, primary text.
pub const INK: Color32 = Color32::from_rgb(0xed, 0xed, 0xf0);
/// `--riff-ink-2` — `#a8a8b4`, secondary text. Lifted from the mockup's
/// `#9a9aa6` for ladder headroom above [`INK_3`].
pub const INK_2: Color32 = Color32::from_rgb(0xa8, 0xa8, 0xb4);
/// `--riff-ink-3` — `#8e8e9a`, tertiary/muted text. Lifted from the mockup's
/// `#6b6b77`, which sat at 3.40:1 on a panel while carrying required text.
pub const INK_3: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x9a);

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

/// Fully transparent. The placeholder an icon glyph falls back to when
/// rasterization fails; at alpha 0 the channel values are theme-independent,
/// so this is the one color a view may want that no [`Palette`] slot supplies.
pub const TRANSPARENT: Color32 = Color32::TRANSPARENT;

/// `--riff-state-success` — `#22c55e`.
pub const STATE_SUCCESS: Color32 = Color32::from_rgb(0x22, 0xc5, 0x5e);
/// Warning — `#eab308`. This was `--riff-state-warning`, which the design
/// sheet aliased to `--riff-brand-500`, so amber meant "primary action",
/// "playback progress", "keyboard focus" and "this field differs" at once
/// (design-handoff review P2-17). Brand amber keeps the first two; warning is
/// the yellow it reads as everywhere else — and it is yellow, not the brand
/// orange, at every call site that paints warning *text* with it.
pub const STATE_WARNING: Color32 = Color32::from_rgb(0xea, 0xb3, 0x08);
/// `--riff-state-error` — lifted from the mockup's `#ef4444` for the same
/// reason [`INK_3`] was: this slot paints text (a destructive label, a failed
/// scan's error line, an unreadable path), and the extracted red read 4.41:1 on
/// a hover fill and 3.99:1 on a raised one. It is the red ramp's bright accent
/// rather than a new hue — the same red a step up the same scale, which is why
/// a lift this small does not read as a restyle.
pub const STATE_ERROR: Color32 = Color32::from_rgb(0xff, 0x52, 0x52);

/// [`STATE_ERROR`]'s deep end — `#991b1b`, the same red pushed the way a light
/// panel needs. One shared value cannot serve both families: this bright accent
/// reads 2.15:1 on a light panel, so the two wear opposite ends of one hue,
/// which is the arrangement [`STATE_WARNING`] already makes for amber. Dark
/// takes the light end, light the deep one, and both stay unmistakably red.
pub const STATE_ERROR_LIGHT: Color32 = Color32::from_rgb(0x99, 0x1b, 0x1b);

/// `--riff-state-info` — `#3b82f6`.
pub const STATE_INFO: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);

/// The keyboard-focus / selection ring — `#a78bfa`, a violet no other token in
/// the system wears. `--riff-ring` was the brand amber, which is the collision
/// [`STATE_WARNING`] documents; High Contrast has always overridden the ring
/// ([`HC_FOCUS_RING`]) and the base palettes now separate it the same way, so
/// "this is the focused control" is never a shade of "this is the primary
/// action".
pub const FOCUS_RING: Color32 = Color32::from_rgb(0xa7, 0x8b, 0xfa);

// --- Radius scale (`--riff-radius-*`) ----------------------------------------
//
// Lifted two px per step off the extracted mockup scale (4 / 8 / 12 / 16):
// the mockup's small controls were nearly square, and at 12 px type the extra
// softness is what makes a 24 px-tall control read as a control rather than a
// rectangle. A corner radius is not a dimension — it moves no box edge and
// costs no layout — so this is the one "generosity" lever the pinned chrome
// geometry never had a say in.

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

// --- Elevation (`--riff-shadow-*`) --------------------------------------------
//
// The mockup speaks elevation in Tailwind box-shadows — a soft, near-vertical
// drop under popovers and floating chrome — and `style_from` is where that
// language maps onto egui's [`egui::Shadow`]. egui's stock shadows carry a
// hard diagonal offset (`[10, 20]`), which reads as library-default rather
// than designed, so both families restate the mockup's centered drop instead.
// Shadows paint outside every rect they wrap, so they move no layout; the
// alphas are family-dependent only because a light canvas needs far less ink
// for the same perceived depth.

/// Menus, popovers and tooltips on the dark family: a tight vertical drop,
/// the mockup's `shadow-lg` read over `--riff-surface`.
pub const SHADOW_MENU_DARK: egui::Shadow = egui::Shadow {
    offset: [0, 8],
    blur: 20,
    spread: 0,
    color: Color32::from_black_alpha(115),
};

/// Menus, popovers and tooltips on the light family: the same geometry at the
/// lower coverage a bright canvas needs.
pub const SHADOW_MENU_LIGHT: egui::Shadow = egui::Shadow {
    offset: [0, 8],
    blur: 20,
    spread: 0,
    color: Color32::from_black_alpha(45),
};

/// Floating windows on the dark family: the menu drop, one step larger.
pub const SHADOW_WINDOW_DARK: egui::Shadow = egui::Shadow {
    offset: [0, 12],
    blur: 28,
    spread: 0,
    color: Color32::from_black_alpha(120),
};

/// Floating windows on the light family: the same drop at light coverage.
pub const SHADOW_WINDOW_LIGHT: egui::Shadow = egui::Shadow {
    offset: [0, 12],
    blur: 28,
    spread: 0,
    color: Color32::from_black_alpha(50),
};

// --- Spacing scale ------------------------------------------------------------
//
// The gaps view code leaves between items. Every step is a value the app
// already used before the scale was declared, so adopting it moved numbers
// rather than changing them.

/// 4 px — the tightest gap: a glyph beside its label inside one control.
pub const SPACE_XS: f32 = 4.0;
/// 6 px — a thumbnail beside its text in a list row.
pub const SPACE_SM: f32 = 6.0;
/// 8 px — the default gap between sibling controls in a row.
pub const SPACE_MD: f32 = 8.0;
/// 12 px — controls that read as one group: caption buttons, breadcrumbs.
pub const SPACE_LG: f32 = 12.0;
/// 16 px — a heading above the content it names.
pub const SPACE_XL: f32 = 16.0;
/// 24 px — the hero-scale gaps (the mockup's `mb-6`).
pub const SPACE_XXL: f32 = 24.0;

// --- Chrome dimensions (`--riff-titlebar-h`, `--riff-sidebar-w`,
// `--riff-playerbar-h`) ---------------------------------------------------------

/// `--riff-titlebar-h` — 56 px top bar.
pub const TITLEBAR_H: f32 = 56.0;
/// `--riff-sidebar-w` — 280 px sidebar.
pub const SIDEBAR_W: f32 = 280.0;

/// Preferred width of an entity list column in the elastic column stage
/// (the elastic-column spec): every non-last list column keeps this width
/// while the window allows it.
pub const COLUMN_WIDTH: f32 = 280.0;
/// Minimum width an entity list column shrinks to before proportional
/// shrinking takes over. The stage never scrolls horizontally; in narrow
/// windows every column shrinks toward its floor instead.
pub const COLUMN_MIN_W: f32 = 200.0;
/// Minimum width of the stage's last (absorbing) column — the tracks column
/// or a single full-width listing — before proportional shrinking takes
/// over. Kept wider than [`COLUMN_MIN_W`] because the last column usually
/// carries the track table.
pub const LAST_COLUMN_MIN_W: f32 = 320.0;
/// Width of the collapsible inspector (the former selection panel): the
/// stage's rightmost column, shown only while a selection exists.
pub const INSPECTOR_WIDTH: f32 = 300.0;
/// `--riff-playerbar-h` — 88 px bottom player bar.
pub const PLAYERBAR_H: f32 = 88.0;

// --- Component geometry ---------------------------------------------------------
//
// The dimensions the views paint with, grouped by the surface that owns them.
// Grouped rather than flattened because two surfaces each calling a bar
// `HEADER_H` — 28px in the browser column, 24px in Now Playing — is exactly how
// duplicate, disagreeing tokens got written before; each keeps its own name
// under its surface now. A shared module holds what two surfaces genuinely
// paint with the same numbers: `glow` for the brand halo behind the hero disc
// and the Now Playing cover, `seek` for the scrub track both bars draw.
//
// These are the views' own values moved, not a redesign: every number is what
// the surface already painted with. Derived math belongs here beside the token
// it derives from, and a value that only an algorithm could produce — scroll
// and paging math, texture cache keys, how many entries a view asks its read
// model for — belongs in the view that computes it.

pub mod geometry {
    /// The `ToggleSwitch` pill and knob.
    pub mod toggle {
        /// Pill width (`w-9`): exactly 36px.
        pub const TOGGLE_W: f32 = 36.0;
        /// Pill height (`h-5`): exactly 20px.
        pub const TOGGLE_H: f32 = 20.0;
        /// Knob diameter (`w-4 h-4`): exactly 16px.
        pub const KNOB_SIZE: f32 = 16.0;
        /// Knob inset from the pill edge (`top-0.5 left-0.5`): 2px.
        pub const KNOB_INSET: f32 = 2.0;
        /// Horizontal knob travel when checked (`peer-checked:translate-x-4`):
        /// 16px.
        pub const KNOB_TRAVEL: f32 = 16.0;
    }

    /// The brand halo that stands in for the design's `.riff-disc-glow`
    /// box-shadow. Shared because the Library empty-state disc and the Now
    /// Playing cover both paint the same three layers, one behind a circle and
    /// one behind a rounded square.
    pub mod glow {
        /// One translucent layer: how far its radius reaches past the shape it
        /// wraps, and how strong the brand tint burns there.
        #[derive(Debug, Clone, Copy)]
        pub struct GlowLayer {
            /// Radius offset beyond the edge, in px.
            pub spread: f32,
            /// Brand-alpha fraction; the mocked box-shadow peaks at 15% brand.
            pub alpha: f32,
        }

        /// The layered approximation of `0 0 60px -20px brand@15%`: egui cannot
        /// blur, so three concentric fills declared largest-first stack into a
        /// soft step gradient, their alphas falling off toward the outside
        /// under the shadow's 15% peak. Each layer's tint is `theme::glow`
        /// applied to its `alpha`.
        pub const LAYERS: [GlowLayer; 3] = [
            GlowLayer {
                spread: 36.0,
                alpha: 0.04,
            },
            GlowLayer {
                spread: 24.0,
                alpha: 0.07,
            },
            GlowLayer {
                spread: 12.0,
                alpha: 0.11,
            },
        ];
    }

    /// The Library stage's empty-state hero: the glowing disc, its glyph, and
    /// the two lines of copy under it.
    pub mod hero {
        /// Disc-circle diameter (`w-40 h-40`): 160px.
        pub const DISC_SIZE: f32 = 160.0;
        /// Disc glyph size inside the circle (`w-20 h-20`): 80px.
        pub const DISC_ICON_SIZE: f32 = 80.0;
        /// Gap between the disc circle and the title (`mb-6`): 24px.
        pub const TITLE_GAP: f32 = 24.0;
        /// Gap between the title and the subtitle (`mb-1`): 4px.
        pub const SUBTITLE_GAP: f32 = 4.0;
        /// Stage inset around the hero group (`p-8`): 32px.
        pub const STAGE_INSET: f32 = 32.0;
    }

    /// The custom titlebar's clusters: wordmark, nav, scan status, the search
    /// band and the OS-convention caption buttons. The band's own height is
    /// [`TITLEBAR_H`](crate::ui::theme::TITLEBAR_H).
    pub mod titlebar {
        /// Caption-button hit area width: Windows-convention caption buttons
        /// are wide, full-height strips (not floating icon chips), so
        /// minimize/maximize/close get real pointer targets.
        pub const CAPTION_BTN_W: f32 = 44.0;
        /// Caption-button hit area height, inside the 56px band.
        pub const CAPTION_BTN_H: f32 = 36.0;
        /// Gap between the nav-control cluster and the caption-button pair.
        pub const CAPTION_GAP: f32 = 12.0;
        /// Gap between the wordmark's equalizer glyph and its "riff" text (and
        /// between the wordmark and the scan status line).
        pub const WORDMARK_GAP: f32 = 16.0;
        /// Gap between the titlebar search field and the clusters on either
        /// side; the field shrinks before either cluster moves as the window
        /// narrows.
        pub const SEARCH_GAP: f32 = 16.0;
        /// Upper bound on the titlebar search field's width so it stays a
        /// field, not a second window; it shrinks with the window before the
        /// clusters move.
        pub const SEARCH_MAX_W: f32 = 520.0;
        /// Left inset of the titlebar search field when the window is wide
        /// enough to hit [`SEARCH_MAX_W`] — the same inset the content top bar
        /// used.
        pub const SEARCH_EDGE_INSET: f32 = 12.0;
    }

    /// The sidebar's tree row — the row height every list in the app lays out
    /// with (the detail column, Now Playing's Up Next and the player bar's
    /// queue panel all read [`ROW_H`] from here), plus what sits inside one.
    pub mod sidebar {
        /// Tree-row height (`h-10`): every sidebar row is exactly 40px tall.
        pub const ROW_H: f32 = 40.0;
        /// Track-row cover thumbnail on library track rows: a square cover-art
        /// tile sized `ROW_H - 8` (32×32), so width and height always match
        /// and the tile never exceeds the 40px row it sits in.
        pub const ROW_COVER: f32 = ROW_H - 8.0;
        /// Column width of a track row's favorite control: the heart's own cell
        /// at the row's leading edge. The cell sits INSIDE the row (not beside
        /// it), so the hover and selection washes cover it and the row reads as
        /// one 40px band.
        pub const FAVORITE_COL_W: f32 = 24.0;
        /// Glyph size of the favorite control's heart.
        pub const HEART_SIZE: f32 = 14.0;
        /// Search-box height (`h-8`).
        pub const SEARCH_H: f32 = 32.0;
        /// First-level indent: content starts 12px into the row. Deeper levels
        /// add [`INDENT_STEP`] each, so this is the whole indent at level 0.
        pub const INDENT_BASE: f32 = 12.0;
        /// The three-level indent scale: 12 / 28 / 44px, one
        /// [`super::super::SPACE_XL`] per level. The mockup's 12 / 44 / 80 was
        /// twice as wide, which spent a third of a 280px column on nesting
        /// alone.
        pub const INDENT_SCALE: [f32; 3] = [12.0, 28.0, 44.0];
        /// Indent step between tree levels past the pinned third; deep levels
        /// keep stepping so deep trees never fold into one edge.
        pub const INDENT_STEP: f32 = 16.0;
        /// Horizontal padding of the icon strip inside a row.
        pub const ICON_GAP: f32 = 8.0;
        /// Floor under the label's wrap width when a row carries a right-aligned
        /// meta cluster: a pathologically narrow row keeps a readable title
        /// instead of letting the text column collapse.
        pub const MIN_LABEL_FREE_W: f32 = 24.0;
        /// The equalizer-bars indicator: four bars, like the mockup's
        /// now-playing glyph.
        pub const EQ_BAR_COUNT: usize = 4;
    }

    /// The browser column's list mode: the 48px row that fits a cover
    /// thumbnail beside two lines of text, and the header strip above it. Its
    /// rows are taller than [`super::sidebar::ROW_H`] because they carry art.
    pub mod browser {
        /// Row height of the browser column's list mode: room for a 36px cover
        /// thumbnail (the artist variant's "small cover thumbnail") with
        /// breathing room.
        pub const ROW_H: f32 = 48.0;
        /// Edge size of a row's cover thumbnail.
        pub const THUMB_SIZE: f32 = 36.0;
        /// Left room before the thumbnail, thumbnail size, and gap to the text:
        /// the text column starts 52px into the row.
        pub const THUMB_TEXT_GAP: f32 = 6.0 + THUMB_SIZE + 10.0;
        /// Right padding on the text column so wrapped lines don't touch the
        /// pane edge.
        pub const TEXT_RIGHT_PAD: f32 = 6.0;
        /// Floor under the text column's wrap width: a pathologically narrow
        /// pane keeps a usable (still wrapping) column instead of collapsing
        /// it.
        pub const MIN_TEXT_W: f32 = 60.0;
        /// Gap between the row's label line and its muted detail line.
        pub const TEXT_GAP: f32 = 4.0;
        /// Vertical inset of a wrapped row's text block within its grown row.
        pub const TEXT_INSET_Y: f32 = 8.0;
        /// Height of the header strip above the rows (sort control, genre
        /// chips).
        pub const HEADER_H: f32 = 28.0;
    }

    /// The scrub bar, which two surfaces paint: the player bar's seek row and
    /// Now Playing's larger one. Both reserve the same room at each end for
    /// the monospace time readouts and draw the same hairline track, so those
    /// two numbers are one token rather than two that have to agree by
    /// accident.
    pub mod seek {
        /// Track height of the seek row (and the volume slider riding the same
        /// row): 4px.
        pub const TRACK_H: f32 = 4.0;
        /// Horizontal room reserved at each end of the seek row for the
        /// monospace time readouts ("62:03" fits with margin).
        pub const TIME_LABEL_SPACE: f32 = 44.0;
        /// The fraction a focused linear control moves per keyboard arrow
        /// press (a seek or volume nudge).
        pub const KEY_STEP: f32 = 0.05;
    }

    /// The 88px bottom player bar: cover, transport, the seek and volume
    /// rows, and the queue sheet it opens. Its height is
    /// [`PLAYERBAR_H`](crate::ui::theme::PLAYERBAR_H); its scrub track comes
    /// from [`seek`](super::seek).
    pub mod playerbar {
        /// Cover-art square (`size-14`): the now-playing cover is exactly
        /// 56×56.
        pub const COVER: f32 = 56.0;
        /// The primary play/pause button diameter.
        pub const PLAY_BTN: f32 = 40.0;
        /// Circular ghost transport button diameter
        /// (previous/next/stop/toggles).
        pub const GHOST_BTN: f32 = 32.0;
        /// Width of the volume slider track.
        pub const VOLUME_W: f32 = 90.0;
        /// Round thumb diameter on the volume slider.
        pub const VOLUME_THUMB: f32 = 10.0;
        /// Horizontal room reserved for the queue position label
        /// ("999/999").
        pub const QUEUE_LABEL_SPACE: f32 = 52.0;
        /// Smallest useful inner height: a 16px seek-row hit area, an 8px gap,
        /// and the 40px primary button. Below this the bar degrades gracefully
        /// instead of overlapping its own rows.
        pub const MIN_INNER_H: f32 = PLAY_BTN + 16.0 + 8.0;
        /// Queue panel width (a compact side sheet, not a second stage).
        pub const QUEUE_PANEL_W: f32 = 320.0;
        /// Tallest the queue panel's row list grows before it scrolls.
        pub const QUEUE_PANEL_MAX_LIST_H: f32 = 320.0;
        /// Height of the panel's "Up Next" header line.
        pub const QUEUE_PANEL_HEADER_H: f32 = 28.0;
        /// Width of the now-playing zone (cover + two text lines) at its
        /// widest.
        pub const NOW_PLAYING_W: f32 = 250.0;
        /// Narrowest the zone shrinks to before it starts shedding lines.
        pub const NOW_PLAYING_MIN_W: f32 = 140.0;
        /// Gap between the cover and the text column (the `SPACE_LG` value).
        pub const NOW_PLAYING_GAP: f32 = 12.0;
        /// Gap between the title line and the artist/album line.
        pub const NOW_PLAYING_LINE_GAP: f32 = 2.0;
        /// Smallest center column the bar protects: the transport row
        /// (`GHOST_BTN * 2 + PLAY_BTN + 12 * 2` = 128) plus room for a usable
        /// seek row and its two time readouts (24).
        pub const CENTER_MIN_W: f32 = GHOST_BTN * 2.0 + PLAY_BTN + 12.0 * 2.0 + 24.0;
        /// Text-column width below which the meta line is dropped.
        pub const TEXT_HIDE_META_W: f32 = 96.0;
        /// Text-column width below which only the cover is painted.
        pub const MIN_TEXT_W: f32 = 56.0;
    }

    /// The Now Playing stage: the 240px cover, its copy block, the seek row,
    /// the Up Next list and the close affordance. [`HEADER_H`] is the Up Next
    /// section's line, 24px — the same word as the browser column's 28px
    /// [`HEADER_H`](super::browser::HEADER_H) because each names its own
    /// surface's header, and neither is the other's.
    pub mod now_playing {
        /// Cover-art square (`w-60 h-60`): the Now Playing cover is exactly
        /// 240px.
        pub const COVER_SIZE: f32 = 240.0;
        /// Stage inset above the cover: 40px, clearing the widest glow layer
        /// (36px spread) so the halo never clips against the panel's top edge.
        pub const STAGE_INSET: f32 = 40.0;
        /// Gap between the cover and the title (`mb-6`, widened to 40px so the
        /// title clears the widest glow layer's 36px spread): 40px.
        pub const COPY_GAP: f32 = 40.0;
        /// Gap between the title and the meta line (`mt-2`): 8px.
        pub const TITLE_META_GAP: f32 = 8.0;
        /// Gap between the meta line and the details line (`mt-1`): 4px.
        pub const META_DETAILS_GAP: f32 = 4.0;
        /// Gap between the copy block and the seek row.
        pub const SEEK_GAP: f32 = 20.0;
        /// Hit-area height of the seek row.
        pub const SEEK_H: f32 = 24.0;
        /// Gap between the seek row and the Up Next section.
        pub const SECTION_GAP: f32 = 16.0;
        /// Height of the Up Next section header line.
        pub const HEADER_H: f32 = 24.0;
        /// Close-affordance diameter.
        pub const CLOSE_BTN: f32 = 28.0;
        /// Inset of the close affordance from the stage corner.
        pub const CLOSE_INSET: f32 = 12.0;
    }

    /// The inspector column (the former selection panel): the album readout
    /// shown while a selection exists. Its width is
    /// [`INSPECTOR_WIDTH`](crate::ui::theme::INSPECTOR_WIDTH); the rows inside
    /// it are the sidebar's.
    pub mod inspector {
        /// Art block height (design: the 268×200 cover block under the
        /// header).
        pub const ART_H: f32 = 200.0;
        /// Height of the Play album button (design: the 32px action row).
        pub const PLAY_H: f32 = 32.0;
    }

    /// The shell's size policy: the fixed 56/280/88 chrome plus the least
    /// main stage that stays usable beside it. Below [`MIN_WINDOW_SIZE`] the
    /// panels would collapse, so the viewport refuses to shrink that far.
    pub mod window {
        /// Smallest main-stage area kept usable beside/between the fixed
        /// chrome.
        pub const MIN_STAGE_SIZE: egui::Vec2 = egui::vec2(520.0, 456.0);
        /// Chrome-fitting minimum window size: sidebar + stage across, titlebar
        /// + playerbar + stage down.
        pub const MIN_WINDOW_SIZE: egui::Vec2 = egui::vec2(
            crate::ui::theme::SIDEBAR_W + MIN_STAGE_SIZE.x,
            crate::ui::theme::TITLEBAR_H + crate::ui::theme::PLAYERBAR_H + MIN_STAGE_SIZE.y,
        );
    }

    /// The Settings modal: its card and left nav, the library pane's rows and
    /// actions, the preference rows and their controls. [`SECTION_GAP`] is the
    /// gap between Settings `<section>`s — the same word as Now Playing's
    /// 16px [`SECTION_GAP`](super::now_playing::SECTION_GAP), a different
    /// measurement.
    ///
    /// Every token in here is consumed by the Settings view alone. They are
    /// distinct from the now-playing rhythm, which deliberately carries its own
    /// same-named gap token — [`SECTION_GAP`](super::now_playing::SECTION_GAP)
    /// above is 32px where Now Playing's is 16px — so a reader must not assume
    /// one shared value behind a shared name.
    pub mod settings {
        /// Gap between the section header and its card. Revalued 16 -> 12 for the
        /// tightened mockup rhythm the two-column Library pane reads with.
        pub const HEADER_GAP: f32 = 12.0;
        /// Width of a settings card's own hairline — the mockup's 1px
        /// `border`, and the edge that *defines* the card.
        ///
        /// A settings card is the card plane plus this line: the mockup gives a
        /// card no fill of its own, so the boundary between "card" and "the pane
        /// behind it" is entirely the stroke. The width was a bare `1.0_f32` at
        /// each of the five card call sites; the value is unchanged, only named,
        /// so no pinned golden moves. Deliberately not a radius or a spacing
        /// step — a stroke moves no box edge, which is what lets a card gain an
        /// edge without reflowing a single pixel of its contents.
        pub const CARD_BORDER_W: f32 = 1.0;
        /// Gap between sections. Revalued 32 -> 12 for the same reason.
        pub const SECTION_GAP: f32 = 12.0;
        /// Stage inset around the full-stage Settings page.
        pub const PAGE_PAD: f32 = 26.0;
        /// Gap between the left nav column and the content pane.
        pub const NAV_GAP: f32 = 20.0;
        /// Gap between the two Library-pane columns.
        pub const COLUMN_GAP: f32 = 24.0;
        /// Width at which the Library pane is allowed to split into two columns.
        pub const MIN_TWO_COL_W: f32 = 620.0;
        /// Pane inset around its content.
        pub const PANE_PAD: f32 = 24.0;
        /// Height of one library row (`px-4 py-3` over ~24px of content).
        pub const LIBRARY_ROW_H: f32 = 48.0;
        /// Row width below which a row STACKS instead of laying its content out
        /// on one line. Revalued never — introduced with the narrow-width
        /// reflow (the min-stage fallback), and derived, not guessed:
        ///
        /// - the library row's right-hand control cluster measures **254.41px**
        ///   (readiness dot + label, Scan, Watch, trash, their 16px gaps and
        ///   the 16px right padding). It is width-independent — it measures the
        ///   same 254.41 at 605px and at 965px.
        /// - a legible path line at `TEXT_SM` needs ~186px, about 24
        ///   characters of the truncated path.
        ///
        /// 254.41 + 186 = 440.4, rounded to **440.0**.
        ///
        /// It has to land strictly inside the validated band. The narrowest
        /// *already-validated* row is a preference row inside the two-column
        /// Library layout, 461.5px, which must keep its single-line form; the
        /// widest unvalidated one is the min-stage pane at 205px, which must
        /// stack. 440 sits between them, so the reflow cannot move a golden
        /// that is already pinned.
        pub const LIBRARY_ROW_STACK_W: f32 = 440.0;
        /// Height of the Add Library / Scan All actions row (`px-4 py-4`).
        pub const ACTIONS_ROW_H: f32 = 64.0;
        /// Gap between the Add Library and Scan All buttons when they share a
        /// line. Previously a bare `12.0`; value unchanged.
        pub const ACTIONS_ROW_GAP: f32 = 12.0;
        /// A row button's own horizontal padding: leading `16`, an `8` gap
        /// after the icon, and `32` of trailing pad, summed. Previously spelled
        /// inline as `16.0 + 8.0 + label + 32.0`; the value is unchanged.
        pub const ROW_BTN_PAD: f32 = 56.0;
        /// Height of one preference row (`px-4 py-3` over title +
        /// description).
        pub const PREF_ROW_H: f32 = 60.0;
        /// Horizontal inset from a preference row's edge to its text. The
        /// row's own padding, previously bare `16.0` literals; the value is
        /// unchanged, only named, so no pinned golden moves.
        pub const PREF_ROW_PAD: f32 = 16.0;
        /// Vertical inset from a preference row's top and bottom to its text.
        /// Previously bare `11.0` literals; value unchanged.
        pub const PREF_ROW_TEXT_INSET: f32 = 11.0;
        /// Gap between a preference row's title and its description when the
        /// description wraps onto more than one line. New with the narrow-width
        /// reflow; the single-line form does not read it.
        pub const PREF_ROW_TEXT_GAP: f32 = 4.0;
        /// Status-dot diameter (`w-2 h-2`): 8px.
        pub const DOT_SIZE: f32 = 8.0;
        /// Gap inside a readiness pair, between the dot and its label, and
        /// between the pair and the control cluster. Previously bare `8.0`
        /// literals; value unchanged.
        pub const READINESS_GAP: f32 = 8.0;
        /// Gap between a library row's path and the track count beside it.
        /// Previously a bare `12.0`; value unchanged.
        pub const LIBRARY_ROW_TEXT_GAP: f32 = 12.0;
        /// Secondary-button height (`px-3 py-1.5` at `text-xs`).
        pub const SMALL_BTN_H: f32 = 27.0;
        /// Primary/secondary action-button height (`px-4 py-2` at `text-sm`).
        pub const ACTION_BTN_H: f32 = 34.0;
        /// Trash affordance hit area (`w-7 h-7`): 28px square.
        pub const TRASH_BTN: f32 = 28.0;
        /// Watch checkbox square size (a native checkbox at xs text ≈ 14px).
        pub const WATCH_BOX: f32 = 14.0;
        /// Height of one format chip — literally the small secondary button,
        /// named separately because it is a different control.
        pub const CHIP_H: f32 = SMALL_BTN_H;
        /// Horizontal padding inside a format chip around its label.
        pub const CHIP_LABEL_PAD: f32 = 12.0;
        /// Gap between adjacent format chips. Used on both axes: the same gap
        /// separates chips on one line and the chip rows when the card wraps.
        pub const CHIP_GAP: f32 = 8.0;
        /// Narrowest content column in which all seven format chips still fit on
        /// one line, measured with the vendored Inter set at [`TEXT_XS`]:
        ///
        /// ```text
        /// label widths   26.19 + 27.34 + 25.56 + 33.53 + 27.16 + 30.72 + 27.44
        ///               = 197.94
        /// chip padding   CHIP_LABEL_PAD * 2 * 7  = 24 * 7       = 168.00
        /// inter-chip gap CHIP_GAP * 6                            =  48.00
        ///               ---------------------------------------------
        /// cluster                                                  413.94
        /// widest single chip  CHIP_LABEL_PAD * 2 + 33.53 (OPUS)  =  57.53
        ///               ---------------------------------------------
        /// threshold = max(cluster, widest chip)                    413.94
        /// ```
        ///
        /// Rounded up to whole pixels, so `413.94` fits and `413.93` wraps.
        /// Below it the chip row wraps and the card grows; at or above it every
        /// pinned wide width (the 920-stage goldens at 581px of content and the
        /// 1280 two-column golden at 445.5px) keeps the single-line layout the
        /// pre-wrap code produced.
        pub const CHIP_ROW_NO_WRAP_W: f32 = 414.0;
        /// Height of the last-full-scan card. Revalued 76 -> 96 for the
        /// three-line card (stamp, files indexed, errors on its own line).
        /// Sized for three lines plus the card's vertical padding, so the card
        /// never has to resize as a scan's error count changes.
        pub const SCAN_CARD_H: f32 = 96.0;
        /// Inset from the scan card's edges to its text and its action. The
        /// card's own padding, previously bare `16.0` literals.
        pub const SCAN_CARD_PAD: f32 = 16.0;
        /// Horizontal padding inside a small button around its icon and label.
        /// The scan card's Rescan action, the former clear row and the footer
        /// all sized their small buttons with this bare `24.0`.
        pub const SMALL_BTN_LABEL_PAD: f32 = 24.0;
        /// Gap between the page footer's two right-hand actions. New with the
        /// destructive ghost joining Done in the footer.
        pub const FOOTER_ACTION_GAP: f32 = 12.0;
        /// Height of the page footer's action row. Revalued 48 -> 36 for the
        /// full-stage footer, which carries the destructive Clear Library
        /// ghost beside Done at `ACTION_BTN_H` and so needs less band than
        /// the old two-button row reserved.
        pub const FOOTER_H: f32 = 36.0;
        /// Height of the page header row (title + Back control). Replaces the
        /// dropped `MODAL_HEADER_H` at the same value: the header lost its
        /// modal card but kept its height.
        pub const PAGE_HEADER_H: f32 = 56.0;
        /// Left-nav column width. Revalued 180 -> 176 so the nav plus the
        /// hairline and `NAV_GAP` land on the mockup's rhythm.
        pub const NAV_W: f32 = 176.0;
        /// Inset from the body's top to the nav column's first row, so the
        /// column does not start hard against the header. Pre-existing value,
        /// named here because the full-stage frame reuses it.
        pub const NAV_TOP_INSET: f32 = 8.0;
        /// Width of the hairline drawn between the nav column and the pane.
        pub const NAV_HAIRLINE_W: f32 = 1.0;
        /// One left-nav row's height (`py-2` at text-sm).
        pub const NAV_ITEM_H: f32 = 32.0;
    }
}

// --- Semantic palette ---------------------------------------------------------

/// The focus-ring color the High Contrast variants swap in for the base ring
/// (REQ-UI-007): a bright gold that reads as "keyboard focus" on a dark
/// surface — and, because a light panel needs the same hue pushed the other
/// way, a deep gold for the light family. The bright one measures 1.01:1
/// against a light panel, so sharing it across families would hide the ring in
/// exactly the mode that exists to make it obvious.
const HC_FOCUS_RING: Color32 = Color32::from_rgb(0xff, 0xd7, 0x00);

/// See [`HC_FOCUS_RING`].
const HC_FOCUS_RING_LIGHT: Color32 = Color32::from_rgb(0x7a, 0x5f, 0x00);

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
    /// Track/row plane — the field rows are painted on, below the card plane
    /// (`--riff-surface-row`).
    pub surface_row: Color32,
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
    /// Warning status — [`STATE_WARNING`] on dark, its deep-amber end on light,
    /// because this slot paints text.
    pub warning: Color32,
    /// Error/destructive status — [`STATE_ERROR`] on dark, its deep end
    /// ([`STATE_ERROR_LIGHT`]) on light, because this slot paints text.
    pub error: Color32,
    /// Info status (`--riff-state-info`).
    pub info: Color32,
    /// Keyboard-focus/selection ring: [`FOCUS_RING`] on dark, a deeper violet on
    /// light, and [`HC_FOCUS_RING`] in both High Contrast variants. Never the
    /// brand amber.
    pub focus_ring: Color32,
}

impl Palette {
    /// The dark palette: the mockup's surfaces, brand and `ink`, with the
    /// muted ink rungs, `warning`, `error` and `focus_ring` lifted off their
    /// extracted values — the first three for AA contrast, the last to stop the
    /// focus ring wearing the brand's hue.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            dark: true,
            high_contrast: false,
            background: SURFACE_BG,
            surface_row: SURFACE_ROW,
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
            focus_ring: FOCUS_RING,
        }
    }

    /// The light palette derived by rule per ADR 0004: surfaces invert
    /// (channel-wise mirror of the dark ramp), ink flips its faintness order,
    /// lines flip their base white→black at unchanged alphas, and brand amber
    /// is untouched. The muted ink rungs, `warning` and `error` are the
    /// exceptions, chosen against the light surfaces directly because each of
    /// them carries text there and a light surface needs *darker* text, not
    /// merely inverted text. `success` and `info` do inherit unchanged, which
    /// is the rule holding rather than an oversight: those two are only ever an
    /// 8 px readiness dot, never a glyph, so the text floor does not reach
    /// them. The rest stays consciously approximate until a proper light design
    /// exists.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            dark: false,
            high_contrast: false,
            // Channel-wise mirrors of the dark surfaces, so the warm cast and
            // the wider separation survive the flip: the dark ramp's warm bias
            // (red highest) mirrors to a cool bias (blue highest), which is
            // exactly what ADR 0004's rule predicts and what keeps light from
            // reading as beige (#0e0d0c → #f1f2f3, #282521 → #d7dade).
            background: Color32::from_rgb(0xf1, 0xf2, 0xf3),
            // The row plane mirrors with the rest of the ramp (#121110 →
            // #edeeef), which is what keeps it between `surface` and
            // `background` in luminance: the half-step survives the flip
            // because the flip is channel-wise and therefore order-preserving
            // in sum.
            surface_row: Color32::from_rgb(0xed, 0xee, 0xef),
            surface: Color32::from_rgb(0xe9, 0xea, 0xeb),
            surface_2: Color32::from_rgb(0xe0, 0xe2, 0xe4),
            surface_3: Color32::from_rgb(0xd7, 0xda, 0xde),
            // The wash is brand-derived, so it is NOT mirrored (that would
            // turn it blue): the unchanged brand amber at the ~9% coverage
            // the dark wash reads over its surface keeps the hover warm on
            // light too.
            row_hover: Color32::from_rgba_unmultiplied_const(0xf0, 0x82, 0x1e, 24),
            // Mirrored ink ladder (#ededf0 → #12120f); brightness inversion
            // preserves the faintness hierarchy against the flipped surfaces.
            ink: Color32::from_rgb(0x12, 0x12, 0x0f),
            // The two muted rungs are the exception to the mirror rule: the
            // channel-wise flip of an AA-compliant dark gray (`#8e8e9a` →
            // `#717165`) reads at 3.48:1 on a light panel, because a light
            // surface needs *darker* text, not merely inverted text. So they
            // are chosen against the light surfaces directly, at the same AA
            // floor, keeping `ink_2` the darker of the two.
            ink_2: Color32::from_rgb(0x47, 0x47, 0x40),
            ink_3: Color32::from_rgb(0x5e, 0x5e, 0x55),
            // Black-based lines at the dark alphas (20 / 26).
            line: Color32::from_rgba_unmultiplied_const(0, 0, 0, 20),
            border: Color32::from_rgba_unmultiplied_const(0, 0, 0, 26),
            // Brand amber is unchanged across palettes (ADR 0004), so text on
            // amber stays deep ink too.
            brand_primary: BRAND_500,
            on_brand: SURFACE_BG,
            success: STATE_SUCCESS,
            // Warning carries text — the Clear Library confirmation, a tag
            // row's `(different)` state, an unindexed path's status — and a
            // bright yellow on a light panel reads at 1.25:1, so light wears
            // the deep amber end of the same hue instead: the same job, legible
            // on its own surfaces.
            warning: Color32::from_rgb(0x85, 0x4d, 0x0e),
            // Error carries text for the same reason warning does — the
            // destructive label, a failed scan's error line, an unreadable
            // path — and the shared bright red reads 3.06:1 on a light panel,
            // so light wears the deep end of the same red instead: the same
            // job, legible on its own surfaces.
            error: STATE_ERROR_LIGHT,
            info: STATE_INFO,
            // Violet again rather than the dark family's lavender: the ring is
            // a UI component and needs 3:1 against the surfaces it sits on,
            // which `#a78bfa` does not clear on light.
            focus_ring: Color32::from_rgb(0x6d, 0x28, 0xd9),
        }
    }

    /// The High Contrast token-set variant over this base palette (ADR 0004):
    /// text pinned to the extreme of the base, secondary ink strengthened,
    /// line alphas roughly doubled, and the focus ring swapped to
    /// [`HC_FOCUS_RING`] / [`HC_FOCUS_RING_LIGHT`]. Surfaces, brand, and status
    /// colors inherit the base so each variant stays recognizably its own
    /// design.
    #[must_use]
    pub fn high_contrast(&self) -> Self {
        let mut variant = *self;
        variant.high_contrast = true;
        if self.dark {
            variant.ink = Color32::WHITE;
            variant.ink_2 = Color32::from_gray(200);
            variant.line = Color32::from_rgba_unmultiplied_const(255, 255, 255, 40);
            variant.border = Color32::from_rgba_unmultiplied_const(255, 255, 255, 50);
            variant.focus_ring = HC_FOCUS_RING;
        } else {
            variant.ink = Color32::BLACK;
            variant.ink_2 = Color32::from_gray(55);
            variant.line = Color32::from_rgba_unmultiplied_const(0, 0, 0, 40);
            variant.border = Color32::from_rgba_unmultiplied_const(0, 0, 0, 50);
            variant.focus_ring = HC_FOCUS_RING_LIGHT;
        }
        variant
    }
}

/// Source-over blend of one straight-alpha colour over an opaque one,
/// returning an opaque result. Colour math lives here beside the tokens it
/// derives from (the no-hardcoded-colors scan exempts this module); view
/// code composes palette colors through helpers like this instead of
/// constructing them.
#[must_use]
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn blend_over(bottom: egui::Color32, top: egui::Color32) -> egui::Color32 {
    let alpha = f32::from(top.a()) / 255.0;
    let channel =
        |b: u8, t: u8| (f32::from(t) * alpha + f32::from(b) * (1.0 - alpha)).round() as u8;
    egui::Color32::from_rgba_unmultiplied(
        channel(bottom.r(), top.r()),
        channel(bottom.g(), top.g()),
        channel(bottom.b(), top.b()),
        u8::MAX,
    )
}

/// The **dark** family's accent-glow strength: the mockup's accent wash
/// `rgba(238, 122, 42, .35)` — 35% coverage.
///
/// The mockup's only stated glow strength, and it was measured on a dark
/// surface, so it is the dark family's value verbatim. Only the strength comes
/// from the mockup: its channels are a near-miss on brand-500 rather than a
/// rung of the scale, and [`glow`] scales the palette's `brand_primary`, which
/// supplies the hue.
///
/// **Per family, not one number.** A translucent wash's perceived strength
/// depends on the backdrop's luminance, and this one is not family-invariant:
/// over the dark card plane (`#161514`, L\* 6.8) 35% lifts the surface by
/// **22.0 L\***, while the same 35% over the light plane (`#e9eaeb`, L\* 92.7)
/// only drops it by **10.4 L\*** — 47% of the dark presence. The shared const
/// was therefore reading *louder on dark*, not louder on light. Use
/// [`glow_alpha`] to pick per family rather than reading this one directly.
pub const GLOW_ALPHA: f32 = 0.35;

/// The **light** family's accent-glow strength.
///
/// A light plane cannot glow: it already sits at the top of the luminance
/// range, so an accent wash can only *tint* it — darken toward the brand — and
/// no coverage turns that back into a bloom. Matching the dark family's 22.0 L\*
/// here would mean dragging a near-white surface down by a fifth of its
/// lightness, which reads as a dirty cast rather than as light. So the light
/// value is set by how much ink a light canvas needs for the same *read*, and
/// the app already answers that question elsewhere: [`SHADOW_MENU_LIGHT`] is
/// 45 against [`SHADOW_MENU_DARK`]'s 115, a 0.39× split, for exactly the reason
/// [`style_from`] gives — "a light canvas needs far less ink for the same
/// perceived depth". This is that same 0.39× of [`GLOW_ALPHA`], landing the
/// light wash at 4.4 L\* against the dark plane's 22.0.
///
/// High Contrast needs no value of its own: it inherits its base family's
/// surfaces unchanged, so a HC plane is the base plane and the wash over it is
/// the wash the base already had.
pub const GLOW_ALPHA_LIGHT: f32 = 0.14;

/// The accent-glow strength for `palette`'s family — [`GLOW_ALPHA`] on the dark
/// families, [`GLOW_ALPHA_LIGHT`] on the light ones. What
/// [`paint_accent_glow`] reads, and what a view should read rather than
/// naming either constant.
#[must_use]
pub fn glow_alpha(palette: &Palette) -> f32 {
    if palette.dark {
        GLOW_ALPHA
    } else {
        GLOW_ALPHA_LIGHT
    }
}

/// How far the accent glow's halo reaches beyond the control's own edge.
///
/// The mockup's lit-from-above flourish is a wide, faint bloom rather than a
/// tight rim, so this is generous relative to the 27–40px controls it sits
/// behind: roughly a third of a control's half-height. Wider than this and the
/// bloom stops reading as light coming off the button and starts reading as a
/// smudge on the pane behind it.
pub const GLOW_SPREAD: f32 = 10.0;

/// The brand glow wash at `alpha`: the palette's primary scaled by a layer's
/// alpha fraction. The only sanctioned way to dim a palette color — view code
/// reads a tint from here instead of scaling one at a call site (ADR 0004),
/// which is what the color sweep in `tests/ui_tests.rs` enforces.
///
/// **Two kinds of caller, deliberately.** An *accent* glow — the wash behind a
/// primary control — passes [`glow_alpha`] (formerly this module's single
/// `GLOW_ALPHA`), because that strength is a per-family design decision with
/// the measurements behind it. The other callers pass **a layer's own alpha**:
/// `library.rs` and `now_playing.rs` each carry a `layer.alpha` that is a
/// property of that layer's depth, not a restyle of the accent. Those are not
/// mis-wired and must not be "fixed" onto `glow_alpha` — a layer that fades
/// with its own alpha is the behaviour they are for.
#[must_use]
pub fn glow(palette: &Palette, alpha: f32) -> Color32 {
    palette.brand_primary.gamma_multiply(alpha)
}

/// The brand gradient's two stops, in paint order: the light rung first.
///
/// The direction lives here, beside the tokens, rather than at each call site —
/// the mockup lights its primary actions from above, so the gradient always
/// runs top-to-bottom and the [`BRAND_GRADIENT_TOP`] / [`BRAND_GRADIENT_BOTTOM`]
/// naming already says which end is which. A call site that passes these two
/// therefore cannot paint the gradient upside down.
///
/// egui 0.35 has no `LinearGradient` type, so this hands back the pair a
/// gradient wants rather than a finished object; [`paint_gradient_shape`] is
/// what turns it into pixels for a shape with rounded corners.
#[must_use]
pub fn brand_gradient_stops() -> [Color32; 2] {
    [BRAND_GRADIENT_TOP, BRAND_GRADIENT_BOTTOM]
}

/// A rounded shape's outline as points around its perimeter, clockwise from the
/// left end of its top edge. A `radius` of half the shorter side collapses the
/// straight edges to points, which is how this module asks for a circle.
///
/// **Requires a positive `radius`.** At zero every arc collapses onto its own
/// corner, so the outline degenerates to each corner repeated once per sample
/// and 24 of the 28 fan triangles are zero-area. That still *paints* correctly —
/// the four non-degenerate triangles cover the rect and `at_height` is affine in
/// `y`, so the interpolation stays exact — but it leans on the rasteriser
/// discarding degenerate triangles, which is an accident rather than a property
/// anyone can read off the code. [`paint_gradient_shape`] therefore takes radius
/// zero as its own rectangular path and never calls this with it; if you are
/// about to pass `0.0`, that is the sign you want the rectangle, not this
/// function.
fn rounded_outline(rect: egui::Rect, radius: f32, per_corner: u16) -> Vec<egui::Pos2> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    // Each corner's arc centre and the angle it starts at, in order.
    let corners = [
        (
            egui::pos2(rect.right() - radius, rect.top() + radius),
            -std::f32::consts::FRAC_PI_2,
        ),
        (
            egui::pos2(rect.right() - radius, rect.bottom() - radius),
            0.0,
        ),
        (
            egui::pos2(rect.left() + radius, rect.bottom() - radius),
            std::f32::consts::FRAC_PI_2,
        ),
        (
            egui::pos2(rect.left() + radius, rect.top() + radius),
            std::f32::consts::PI,
        ),
    ];
    let mut points = Vec::with_capacity(corners.len() * (per_corner as usize + 1));
    for (centre, start) in corners {
        for step in 0..=per_corner {
            let angle =
                start + std::f32::consts::FRAC_PI_2 * f32::from(step) / f32::from(per_corner);
            points.push(egui::pos2(
                centre.x + radius * angle.cos(),
                centre.y + radius * angle.sin(),
            ));
        }
    }
    points
}

/// One channel of a linear blend between two gradient stops, rounded.
///
/// Both ends are `u8` and `t` is a fraction clamped to `0..=1`, so the result
/// is in range by construction and the cast is only the rounding step — the
/// same trade [`blend_over`] makes for the same reason.
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn blend_channel(from: u8, to: u8, t: f32) -> u8 {
    (f32::from(from) + (f32::from(to) - f32::from(from)) * t).round() as u8
}

/// The artless-artwork placeholder's gradient stops: the two raised-surface
/// rungs, light on top.
///
/// **The one system gradient every placeholder wears** (decision D2). It lives
/// here rather than in the artwork module so the pair is a token choice like any
/// other: a caller reads these and hands them to the placeholder variant, and
/// nothing in a view names a rung. One pair for all placeholders is also what
/// keeps the shared placeholder tile cacheable — per-identity stops would make
/// every artless item its own texture again.
#[must_use]
pub fn placeholder_gradient_stops(palette: &Palette) -> [Color32; 2] {
    [palette.surface_2, palette.surface_3]
}

/// Paint `rect` as a rounded shape filled with `stops` running **vertically**,
/// `stops[0]` at the top edge and `stops[1]` at the bottom, each vertex taking
/// the color its own height implies.
///
/// A mesh rather than `Shape::gradient_rect`, for two reasons: a gradient rect
/// has square corners, and every control this paints for is rounded or
/// circular. Colored vertices interpolate in the tessellator, so the shape's
/// edge is antialiased by the same path a flat fill takes and the falloff costs
/// one primitive. `radius` is a [`RADIUS_MD`] token for a button, or half the
/// rect's width for a circle.
///
/// Paint-only: the mesh is bounded by `rect`, so it moves no box edge and
/// changes no hit target.
pub fn paint_gradient_shape(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    [top, bottom]: [Color32; 2],
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let at_height = |y: f32| {
        let t = ((y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        Color32::from_rgb(
            blend_channel(top.r(), bottom.r(), t),
            blend_channel(top.g(), bottom.g(), t),
            blend_channel(top.b(), bottom.b(), t),
        )
    };
    let mut mesh = egui::Mesh::default();

    // A plain rectangle is its own path, and the honest one: four corners, two
    // triangles, no centre vertex and no per-corner sampling. Rounding it to
    // zero would produce the same pixels via [`rounded_outline`], but only
    // because the fan's degenerate triangles happen to cover nothing — so the
    // rectangle is spelled out here rather than leaned on.
    if radius <= 0.0 {
        mesh.colored_vertex(rect.left_top(), top);
        mesh.colored_vertex(rect.right_top(), top);
        mesh.colored_vertex(rect.right_bottom(), bottom);
        mesh.colored_vertex(rect.left_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
        return;
    }

    // A rounded shape: a fan from the centre over its outline, which is exact
    // because the shape is convex.
    let outline = rounded_outline(rect, radius, 6);
    mesh.colored_vertex(rect.center(), at_height(rect.center().y));
    for point in &outline {
        mesh.colored_vertex(*point, at_height(point.y));
    }
    // A closed outline needs one triangle per edge, so the last one wraps back
    // to the first outline vertex. Dropping that wrap leaves out the triangle
    // spanning the shape's top edge and the centre — which, for a wide control,
    // is most of its upper half.
    let count = u32::try_from(outline.len()).expect("a rounded outline is a handful of points");
    for index in 1..=count {
        let next = if index == count { 1 } else { index + 1 };
        mesh.add_triangle(0, index, next);
    }
    painter.add(mesh);
}

/// Paint the accent glow that sits **behind** a primary control: the brand
/// accent at [`GLOW_ALPHA`] at the control's centre, falling to fully
/// transparent [`GLOW_SPREAD`] beyond its edge.
///
/// A fan of triangles from the centre out to an ellipse around the control, so
/// the bloom hugs the control's own proportions — circular for the play FAB,
/// wide and shallow for a text button — instead of stamping a fixed disc that
/// would pool in the corners of a wide control. Vertex interpolation supplies
/// the falloff, so the halo has no hard edge anywhere and costs one primitive.
///
/// Call this BEFORE the control's face: the centre is the brightest part and the
/// face covers it, which is what leaves a rim of light rather than a disc.
///
/// Paint-only, and deliberately unclipped — the halo is meant to reach past the
/// control onto the pane behind it, so a painter scoped to the control's own
/// rect (egui's `painter_at`) would erase it. What bounds it is the enclosing
/// panel's clip.
pub fn paint_accent_glow(painter: &egui::Painter, palette: &Palette, rect: egui::Rect) {
    /// Segments around the halo. Enough that the rim does not read as a
    /// polygon on a 40px control.
    const SEGMENTS: u16 = 32;
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let halo = rect.expand(GLOW_SPREAD);
    let centre = rect.center();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, glow(palette, glow_alpha(palette)));
    for step in 0..SEGMENTS {
        let angle = std::f32::consts::TAU * f32::from(step) / f32::from(SEGMENTS);
        mesh.colored_vertex(
            egui::pos2(
                centre.x + halo.width() / 2.0 * angle.cos(),
                centre.y + halo.height() / 2.0 * angle.sin(),
            ),
            TRANSPARENT,
        );
    }
    for step in 0..SEGMENTS {
        mesh.add_triangle(0, 1 + u32::from(step), 1 + u32::from((step + 1) % SEGMENTS));
    }
    painter.add(mesh);
}

/// The tint a hero glyph is rasterized with: the palette's muted ink at the
/// mockup's `muted-foreground/40` strength.
#[must_use]
pub fn hero_glyph(palette: &Palette) -> Color32 {
    palette.ink_3.gamma_multiply(0.4)
}

/// The coverage a role's hover wash is painted at — the mockup's
/// `hover:bg-*/10`, and the number that makes the accent wash and the
/// destructive wash one gesture in two hues rather than two different
/// strengths. [`destructive_fill`] carries the same 0.1 inline; this names it
/// for the accent side, and both are the mockup's `/10`.
pub const ACCENT_HOVER_COVERAGE: f32 = 0.1;

/// The destructive ghost button's fill: transparent until hovered, then the
/// error token at the mockup's 10% (`hover:bg-destructive/10`).
#[must_use]
pub fn destructive_fill(palette: &Palette, hovered: bool) -> Color32 {
    if hovered {
        palette.error.gamma_multiply(0.1)
    } else {
        TRANSPARENT
    }
}

/// The accent tier's hover wash: transparent until hovered, then the brand
/// token at the mockup's 10% (`hover:bg-primary/10`) — the identical gesture
/// [`destructive_fill`] makes, in the brand hue rather than the error hue.
///
/// The counterpart helper for [`destructive_fill`], and for the same reason: a
/// translucent role wash is derived here rather than at a call site, so the
/// coverage cannot drift between the two and view code never scales a token.
#[must_use]
pub fn accent_fill(palette: &Palette, hovered: bool) -> Color32 {
    if hovered {
        palette.brand_primary.gamma_multiply(ACCENT_HOVER_COVERAGE)
    } else {
        TRANSPARENT
    }
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

    // Elevation: popovers and floating chrome drop from the shadow tokens —
    // a centered, near-vertical fall — instead of egui's stock diagonal
    // smear, which reads as library-default rather than designed (ADR 0004:
    // every visual value declared here). A shadow paints outside every rect
    // it wraps, so this moves no layout; the two families differ only in how
    // much ink a canvas needs for the same perceived depth.
    v.popup_shadow = if palette.dark {
        SHADOW_MENU_DARK
    } else {
        SHADOW_MENU_LIGHT
    };
    v.window_shadow = if palette.dark {
        SHADOW_WINDOW_DARK
    } else {
        SHADOW_WINDOW_LIGHT
    };

    // Links and status text.
    v.hyperlink_color = palette.brand_primary;
    v.warn_fg_color = palette.warning;
    v.error_fg_color = palette.error;

    // Selection / keyboard-focus ring. The wash sits at 22% brand so the ink
    // it selects stays the loudest thing on its row; the ring is the focus
    // token, never the brand amber (review P2-17).
    v.selection.bg_fill = palette.brand_primary.gamma_multiply(0.22);
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
