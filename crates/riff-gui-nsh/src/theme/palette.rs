//! The riff palette on nsh, ported verbatim from the egui design system's
//! store (`crates/riff-gui-theme/src/theme.rs`, frozen during the
//! coexistence window per ADR 0022). Every hex is that file's value; this
//! is a port, not a redesign.
//!
//! The authoritative runtime store is the `Signal<Palette>` in
//! `crate::app`: every riff surface reads
//! `move || palette.get().surface` and so on, which also gives runtime
//! dark/light and High Contrast switching without an nsh theme API
//! (ADR 0004's N2 amendment). `theme.json` only restyles `nsh_base`
//! defaults.
//!
//! The two families and the High Contrast token-set variant keep the
//! source's structure: light derives by rule (surfaces mirror channel-wise,
//! ink flips its faintness order, brand amber is unchanged), and High
//! Contrast is a variant over each base, never a third design.

use nsh_core::color::{Hsla, rgba};

/// `--riff-bg` — `#0e0d0c`, the dark window background.
pub const SURFACE_BG: Hsla = rgba(0x0e0d_0cff).to_hsla();

/// `--riff-surface` — `#161514`, panels and cards.
pub const SURFACE: Hsla = rgba(0x1615_14ff).to_hsla();

/// `--riff-surface-2` — `#1f1d1b`, hover fills and popovers.
pub const SURFACE_2: Hsla = rgba(0x1f1d_1bff).to_hsla();

/// `--riff-surface-3` — `#282521`, raised accents.
pub const SURFACE_3: Hsla = rgba(0x2825_21ff).to_hsla();

/// `--riff-surface-row` — `#121110`, the track/row plane.
pub const SURFACE_ROW: Hsla = rgba(0x1211_10ff).to_hsla();

/// `--riff-ink` — `#ededf0`, primary text on dark.
pub const INK: Hsla = rgba(0xeded_f0ff).to_hsla();

/// `--riff-ink-2` — `#a8a8b4`, secondary text on dark.
pub const INK_2: Hsla = rgba(0xa8a8_b4ff).to_hsla();

/// `--riff-ink-3` — `#8e8e9a`, tertiary/muted text on dark.
pub const INK_3: Hsla = rgba(0x8e8e_9aff).to_hsla();

/// `--riff-row-hover` — `#2a1c0e`, the amber wash under hovered rows.
pub const ROW_HOVER: Hsla = rgba(0x2a1c_0eff).to_hsla();

/// `--riff-line` — `rgba(255, 255, 255, 0.08)` on dark, hairline separators.
pub const LINE: Hsla = rgba(0xffff_ff14).to_hsla();

/// `--riff-border` — `rgba(255, 255, 255, 0.10)` on dark, widget borders.
pub const BORDER: Hsla = rgba(0xffff_ff1a).to_hsla();

/// `--riff-brand-500` — `#f0821e`, the primary brand color, identical in
/// both families (ADR 0004).
pub const BRAND_500: Hsla = rgba(0xf082_1eff).to_hsla();

/// `--riff-brand-400` — `#ffb833`, the brand gradient's light end.
pub const BRAND_400: Hsla = rgba(0xffb8_33ff).to_hsla();

/// The brand gradient's stops in paint order: the light rung first, the
/// primary second. Both stops are plain consts, not [`Palette`] slots, on
/// the source's argument: brand amber is family-invariant, so a
/// per-family slot for it would be a lie.
pub const BRAND_GRADIENT: [Hsla; 2] = [BRAND_400, BRAND_500];

/// `--riff-state-success` — `#22c55e`.
pub const STATE_SUCCESS: Hsla = rgba(0x22c5_5eff).to_hsla();

/// `--riff-state-warning` — `#eab308` on dark.
pub const STATE_WARNING: Hsla = rgba(0xeab3_08ff).to_hsla();

/// The dark family's `--riff-state-error` — `#ff5252`.
pub const STATE_ERROR: Hsla = rgba(0xff52_52ff).to_hsla();

/// `--riff-state-info` — `#3b82f6`.
pub const STATE_INFO: Hsla = rgba(0x3b82_f6ff).to_hsla();

/// The dark family's keyboard-focus ring — `#a78bfa`, a violet no other
/// token wears.
pub const FOCUS_RING: Hsla = rgba(0xa78b_faff).to_hsla();

/// The High Contrast focus ring on the dark family — `#ffd700`, the bright
/// gold that reads as keyboard focus on a dark surface.
pub const HC_FOCUS_RING: Hsla = rgba(0xffd7_00ff).to_hsla();

/// The High Contrast focus ring on the light family — `#7a5f00`, the deep
/// gold; the bright one reads 1.01:1 against a light panel.
pub const HC_FOCUS_RING_LIGHT: Hsla = rgba(0x7a5f_00ff).to_hsla();

/// Which of the four token sets is active. The switch axis the debug
/// fixture's controls write and the palette signal resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    /// The dark family, the app's opening look.
    Dark,
    /// The light family.
    Light,
    /// High Contrast over dark.
    HighContrastDark,
    /// High Contrast over light.
    HighContrastLight,
}

impl Variant {
    /// Every variant, in switcher order.
    pub const ALL: [Variant; 4] = [
        Variant::Dark,
        Variant::Light,
        Variant::HighContrastDark,
        Variant::HighContrastLight,
    ];

    /// The name the switcher labels and the screenshot filenames carry.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Variant::Dark => "dark",
            Variant::Light => "light",
            Variant::HighContrastDark => "high-contrast dark",
            Variant::HighContrastLight => "high-contrast light",
        }
    }
}

/// The semantic color set a themed surface reads from: every themed surface
/// reads its colors from an instance of this struct, never from the flat
/// constants, so switching the signal re-themes everything at once
/// (ADR 0004). All fields are colors; Reduce Motion is a separate flag, not
/// a color, and lives with the motion tokens in
/// `crate::theme::tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// `true` for the dark family, `false` for light.
    pub dark: bool,
    /// Whether this set is the High Contrast variant of its base.
    pub high_contrast: bool,
    /// Window background (`--riff-bg`).
    pub background: Hsla,
    /// Track/row plane, below the card plane (`--riff-surface-row`).
    pub surface_row: Hsla,
    /// Panel/card fill (`--riff-surface`).
    pub surface: Hsla,
    /// Hover fills and popovers (`--riff-surface-2`).
    pub surface_2: Hsla,
    /// Raised accents (`--riff-surface-3`).
    pub surface_3: Hsla,
    /// Amber wash painted under hovered rows (`--riff-row-hover`).
    pub row_hover: Hsla,
    /// Primary text (`--riff-ink`).
    pub ink: Hsla,
    /// Secondary text (`--riff-ink-2`).
    pub ink_2: Hsla,
    /// Tertiary/muted text (`--riff-ink-3`).
    pub ink_3: Hsla,
    /// Hairline separators (`--riff-line`).
    pub line: Hsla,
    /// Widget borders (`--riff-border`).
    pub border: Hsla,
    /// Primary brand fill (`--riff-primary`, brand-500 in both palettes).
    pub brand_primary: Hsla,
    /// Text painted on brand fills (`--riff-primary-foreground`).
    pub on_brand: Hsla,
    /// Success status (`--riff-state-success`).
    pub success: Hsla,
    /// Warning status — [`STATE_WARNING`] on dark, the deep amber end
    /// (`#854d0e`) on light, because this slot paints text.
    pub warning: Hsla,
    /// Error/destructive status — [`STATE_ERROR`] on dark, its deep end
    /// (`#991b1b`) on light, because this slot paints text.
    pub error: Hsla,
    /// Info status (`--riff-state-info`).
    pub info: Hsla,
    /// Keyboard-focus/selection ring: [`FOCUS_RING`] on dark, a deeper
    /// violet on light, and the High Contrast golds in both High Contrast
    /// variants. Never the brand amber.
    pub focus_ring: Hsla,
}

impl Palette {
    /// The dark palette: the ported source's surfaces, brand and `ink`, with
    /// the muted ink rungs, `warning`, `error` and `focus_ring` lifted off
    /// their extracted values, exactly as the egui store ships them.
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

    /// The light palette derived by rule per ADR 0004, with the source's
    /// chosen exceptions: surfaces mirror channel-wise (the warm cast flips
    /// to cool), ink flips its faintness order, lines flip white to black at
    /// unchanged alphas, brand amber is untouched, and the muted ink rungs,
    /// `warning` and `error` are chosen against the light surfaces directly
    /// because each of them carries text there.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            dark: false,
            high_contrast: false,
            // Channel-wise mirrors of the dark surfaces (#0e0d0c -> #f1f2f3,
            // #282521 -> #d7dade), so the separation survives the flip.
            background: rgba(0xf1f2_f3ff).to_hsla(),
            surface_row: rgba(0xedee_efff).to_hsla(),
            surface: rgba(0xe9ea_ebff).to_hsla(),
            surface_2: rgba(0xe0e2_e4ff).to_hsla(),
            surface_3: rgba(0xd7da_deff).to_hsla(),
            // The wash is brand-derived, so it is NOT mirrored (that would
            // turn it blue): brand-500 at the ~9% coverage the dark wash
            // reads over its surface (24/255).
            row_hover: rgba(0xf082_1e18).to_hsla(),
            // Mirrored ink ladder (#ededf0 -> #12120f). The two muted rungs
            // are the exception to the mirror rule: light surfaces need
            // darker text, not merely inverted text, so they are chosen
            // against the light surfaces directly at the same AA floor.
            ink: rgba(0x1212_0fff).to_hsla(),
            ink_2: rgba(0x4747_40ff).to_hsla(),
            ink_3: rgba(0x5e5e_55ff).to_hsla(),
            // Black-based lines at the dark alphas (20 / 26).
            line: rgba(0x0000_0014).to_hsla(),
            border: rgba(0x0000_001a).to_hsla(),
            brand_primary: BRAND_500,
            on_brand: SURFACE_BG,
            success: STATE_SUCCESS,
            // Warning carries text, and bright yellow on a light panel reads
            // at 1.25:1, so light wears the deep amber end of the same hue.
            warning: rgba(0x854d_0eff).to_hsla(),
            // Error carries text for the same reason, so light wears the
            // deep end of the same red.
            error: rgba(0x991b_1bff).to_hsla(),
            info: STATE_INFO,
            // Violet again rather than the dark family's lavender: the ring
            // needs 3:1 against the surfaces it sits on.
            focus_ring: rgba(0x6d28_d9ff).to_hsla(),
        }
    }

    /// The High Contrast token-set variant over this base palette
    /// (ADR 0004): text pinned to the extreme of the base, secondary ink
    /// strengthened, line alphas roughly doubled, and the focus ring swapped
    /// to the High Contrast gold. Surfaces, brand and status colors inherit
    /// the base so each variant stays recognizably its own design.
    #[must_use]
    pub fn high_contrast(self) -> Self {
        let mut variant = self;
        variant.high_contrast = true;
        if self.dark {
            variant.ink = rgba(0xffff_ffff).to_hsla();
            variant.ink_2 = rgba(0xc8c8_c8ff).to_hsla();
            variant.line = rgba(0xffff_ff28).to_hsla();
            variant.border = rgba(0xffff_ff32).to_hsla();
            variant.focus_ring = HC_FOCUS_RING;
        } else {
            variant.ink = rgba(0x0000_00ff).to_hsla();
            variant.ink_2 = rgba(0x3737_37ff).to_hsla();
            variant.line = rgba(0x0000_0028).to_hsla();
            variant.border = rgba(0x0000_0032).to_hsla();
            variant.focus_ring = HC_FOCUS_RING_LIGHT;
        }
        variant
    }

    /// Resolve the active [`Palette`] for a [`Variant`]: the base family per
    /// ADR 0004 with High Contrast applied as a token-set variant over it,
    /// never a third design. The single resolution path the palette signal's
    /// switcher writes through.
    #[must_use]
    pub fn of(variant: Variant) -> Self {
        match variant {
            Variant::Dark => Self::dark(),
            Variant::Light => Self::light(),
            Variant::HighContrastDark => Self::dark().high_contrast(),
            Variant::HighContrastLight => Self::light().high_contrast(),
        }
    }
}
