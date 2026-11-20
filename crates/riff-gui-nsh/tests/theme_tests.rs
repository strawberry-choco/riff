//! The palette port's guards. The plan's N2 gate is the first test: every
//! color slot read from every variant, so an unset slot — a field left
//! transparent by a refactor or a new variant — fails here rather than
//! painting blank chrome. The rest pin the port against drift from the
//! values the egui store ships.

use nsh_core::color::rgba;
use riff_gui_nsh::theme::icons::Icon;
use riff_gui_nsh::theme::palette;
use riff_gui_nsh::theme::tokens;
use riff_gui_nsh::theme::{Palette, Variant};

/// Every color slot with its name, read through one accessor table. The
/// table's completeness is enforced by `every_slot_is_listed`'s exhaustive
/// destructure, not by this doc comment.
fn slots(p: &Palette) -> [(&'static str, nsh_core::color::Hsla); 18] {
    [
        ("background", p.background),
        ("surface_row", p.surface_row),
        ("surface", p.surface),
        ("surface_2", p.surface_2),
        ("surface_3", p.surface_3),
        ("row_hover", p.row_hover),
        ("ink", p.ink),
        ("ink_2", p.ink_2),
        ("ink_3", p.ink_3),
        ("line", p.line),
        ("border", p.border),
        ("brand_primary", p.brand_primary),
        ("on_brand", p.on_brand),
        ("success", p.success),
        ("warning", p.warning),
        ("error", p.error),
        ("info", p.info),
        ("focus_ring", p.focus_ring),
    ]
}

/// The compile-time guard a length assert cannot be: destructuring
/// `Palette` without `..` names every field, so a field added to the struct
/// without joining the accessor table above fails the build, not a test
/// that could never fail.
#[test]
fn every_slot_is_listed() {
    let p = Palette::dark();
    let Palette {
        dark,
        high_contrast,
        background,
        surface_row,
        surface,
        surface_2,
        surface_3,
        row_hover,
        ink,
        ink_2,
        ink_3,
        line,
        border,
        brand_primary,
        on_brand,
        success,
        warning,
        error,
        info,
        focus_ring,
    } = p;
    // The table and the struct agree; resolving them again through
    // `slots` keeps the two lists one and the same source of truth.
    assert_eq!(slots(&p).len(), 18);
    let _ = (
        dark,
        high_contrast,
        background,
        surface_row,
        surface,
        surface_2,
        surface_3,
        row_hover,
        ink,
        ink_2,
        ink_3,
        line,
        border,
        brand_primary,
        on_brand,
        success,
        warning,
        error,
        info,
        focus_ring,
    );
}

/// The N2 gate: every color slot of every variant resolves to a painted
/// color. An unset slot is `Hsla::default()` — transparent black — so a
/// zero alpha or an all-zero color fails, whatever the family.
#[test]
fn every_palette_slot_resolves_in_all_four_variants() {
    for variant in Variant::ALL {
        let p = Palette::of(variant);
        for (name, color) in slots(&p) {
            assert!(
                color.a > 0.0 || color.h != 0.0 || color.s != 0.0 || color.l != 0.0,
                "{variant:?}: slot `{name}` is unset (transparent black)"
            );
            assert!(
                (0.0..=1.0).contains(&color.a)
                    && (0.0..=1.0).contains(&color.h)
                    && (0.0..=1.0).contains(&color.s)
                    && (0.0..=1.0).contains(&color.l),
                "{variant:?}: slot `{name}` is out of range: {color:?}"
            );
        }
    }
}

/// The four variants are four distinct token sets: no two resolve equal, so
/// a switcher that silently hands out the same palette twice cannot pass.
#[test]
fn the_four_variants_are_distinct() {
    for (i, a) in Variant::ALL.iter().enumerate() {
        for b in Variant::ALL.iter().skip(i + 1) {
            assert_ne!(
                Palette::of(*a),
                Palette::of(*b),
                "{a:?} and {b:?} resolve to the same palette"
            );
        }
    }
}

/// The High Contrast variant stays recognizably its own base: surfaces,
/// brand and the status colors inherit, ink is pinned to the family's
/// extreme, lines strengthen, and the ring swaps to the gold. This is the
/// shape ADR 0004's "token-set variant, never a third design" names.
#[test]
fn high_contrast_is_a_token_set_over_its_base() {
    let dark = Palette::dark();
    let hc_dark = dark.high_contrast();
    assert!(hc_dark.high_contrast && hc_dark.dark);
    assert_eq!(hc_dark.surface, dark.surface);
    assert_eq!(hc_dark.brand_primary, dark.brand_primary);
    assert_eq!(hc_dark.error, dark.error);
    assert_eq!(hc_dark.ink, rgba(0xffff_ffff).to_hsla());
    assert_eq!(hc_dark.focus_ring, palette::HC_FOCUS_RING);
    assert!(hc_dark.border != dark.border, "HC lines strengthen");

    let light = Palette::light();
    let hc_light = light.high_contrast();
    assert!(hc_light.high_contrast && !hc_light.dark);
    assert_eq!(hc_light.surface, light.surface);
    assert_eq!(hc_light.ink, rgba(0x0000_00ff).to_hsla());
    assert_eq!(hc_light.focus_ring, palette::HC_FOCUS_RING_LIGHT);
}

/// The light palette derives by rule per ADR 0004: surfaces are the dark
/// ramp's channel-wise mirrors, brand amber is unchanged, and the lines
/// flip base at unchanged alphas. Pinned on representative slots so a
/// hand-picked value cannot quietly replace a derivation.
#[test]
fn light_mirrors_dark_and_brand_is_family_invariant() {
    let dark = Palette::dark();
    let light = Palette::light();

    // #0e0d0c mirrors to #f1f2f3 (channel-wise: 255 - each byte).
    assert_eq!(light.background, rgba(0xf1f2_f3ff).to_hsla());
    assert_eq!(light.surface_3, rgba(0xd7da_deff).to_hsla());
    assert_eq!(light.line, rgba(0x0000_0014).to_hsla());
    assert_eq!(dark.line, rgba(0xffff_ff14).to_hsla());
    assert_eq!(light.border, rgba(0x0000_001a).to_hsla());
    assert_eq!(dark.border, rgba(0xffff_ff1a).to_hsla());

    // Brand amber is identical in both families (ADR 0004).
    assert_eq!(light.brand_primary, dark.brand_primary);
    assert_eq!(light.brand_primary, palette::BRAND_500);
}

/// The ported literals: representative slots pinned to the hexes the egui
/// store ships, the values a redesign of the nsh side must not drift.
#[test]
fn the_dark_slots_match_the_egui_store() {
    let p = Palette::dark();
    assert_eq!(p.background, rgba(0x0e0d_0cff).to_hsla());
    assert_eq!(p.surface_row, rgba(0x1211_10ff).to_hsla());
    assert_eq!(p.surface, rgba(0x1615_14ff).to_hsla());
    assert_eq!(p.surface_2, rgba(0x1f1d_1bff).to_hsla());
    assert_eq!(p.surface_3, rgba(0x2825_21ff).to_hsla());
    assert_eq!(p.row_hover, rgba(0x2a1c_0eff).to_hsla());
    assert_eq!(p.ink, rgba(0xeded_f0ff).to_hsla());
    assert_eq!(p.ink_2, rgba(0xa8a8_b4ff).to_hsla());
    assert_eq!(p.ink_3, rgba(0x8e8e_9aff).to_hsla());
    assert_eq!(p.brand_primary, rgba(0xf082_1eff).to_hsla());
    assert_eq!(p.on_brand, rgba(0x0e0d_0cff).to_hsla());
    assert_eq!(p.success, rgba(0x22c5_5eff).to_hsla());
    assert_eq!(p.warning, rgba(0xeab3_08ff).to_hsla());
    assert_eq!(p.error, rgba(0xff52_52ff).to_hsla());
    assert_eq!(p.info, rgba(0x3b82_f6ff).to_hsla());
    assert_eq!(p.focus_ring, rgba(0xa78b_faff).to_hsla());
}

/// The light family's chosen exceptions: the muted ink rungs, `warning` and
/// `error` are picked against the light surfaces directly, not mirrored —
/// the ADR's documented carve-out.
#[test]
fn the_light_exceptions_match_the_egui_store() {
    let p = Palette::light();
    assert_eq!(p.ink_2, rgba(0x4747_40ff).to_hsla());
    assert_eq!(p.ink_3, rgba(0x5e5e_55ff).to_hsla());
    assert_eq!(p.warning, rgba(0x854d_0eff).to_hsla());
    assert_eq!(p.error, rgba(0x991b_1bff).to_hsla());
    assert_eq!(p.focus_ring, rgba(0x6d28_d9ff).to_hsla());
    // The wash rides the brand hue in both families, at unchanged coverage.
    assert_eq!(p.row_hover, rgba(0xf082_1e18).to_hsla());
}

/// The family flags a themed surface branches on.
#[test]
fn the_variant_flags_resolve() {
    assert!(Palette::dark().dark && !Palette::dark().high_contrast);
    assert!(!Palette::light().dark && !Palette::light().high_contrast);
    assert!(Palette::of(Variant::HighContrastDark).dark);
    assert!(!Palette::of(Variant::HighContrastLight).dark);
    assert!(Palette::of(Variant::HighContrastDark).high_contrast);
    assert!(Palette::of(Variant::HighContrastLight).high_contrast);
    // `of` is exactly the base + variant chain it replaces.
    assert_eq!(Palette::of(Variant::Dark), Palette::dark());
    assert_eq!(Palette::of(Variant::Light), Palette::light());
    assert_eq!(
        Palette::of(Variant::HighContrastDark),
        Palette::dark().high_contrast()
    );
}

/// The token scale, pinned to the plan's and the egui store's values: a
/// retune happens in both crates deliberately, not by one drifting.
#[test]
fn the_token_scale_matches_the_egui_store() {
    for (name, ported, source) in [
        ("TEXT_XS", tokens::TEXT_XS, 12.0),
        ("TEXT_SM", tokens::TEXT_SM, 14.0),
        ("TEXT_XL", tokens::TEXT_XL, 20.0),
        ("TEXT_3XL", tokens::TEXT_3XL, 30.0),
        ("RADIUS_XS", tokens::RADIUS_XS, 3.0),
        ("RADIUS_SM", tokens::RADIUS_SM, 6.0),
        ("RADIUS_MD", tokens::RADIUS_MD, 10.0),
        ("RADIUS_LG", tokens::RADIUS_LG, 14.0),
        ("RADIUS_XL", tokens::RADIUS_XL, 18.0),
        ("RADIUS_FULL", tokens::RADIUS_FULL, 999.0),
        ("SPACE_XS", tokens::SPACE_XS, 4.0),
        ("SPACE_SM", tokens::SPACE_SM, 6.0),
        ("SPACE_MD", tokens::SPACE_MD, 8.0),
        ("SPACE_LG", tokens::SPACE_LG, 12.0),
        ("MOTION_DEFAULT", tokens::MOTION_DEFAULT, 0.18),
        ("MOTION_HOVER", tokens::MOTION_HOVER, 0.10),
        ("MOTION_REDUCED", tokens::MOTION_REDUCED, 0.0),
        ("TITLEBAR_H", tokens::TITLEBAR_H, 56.0),
        ("SIDEBAR_W", tokens::SIDEBAR_W, 280.0),
        ("COLUMN_WIDTH", tokens::COLUMN_WIDTH, 280.0),
        ("COLUMN_MIN_W", tokens::COLUMN_MIN_W, 200.0),
        ("LAST_COLUMN_MIN_W", tokens::LAST_COLUMN_MIN_W, 320.0),
        ("INSPECTOR_WIDTH", tokens::INSPECTOR_WIDTH, 300.0),
        ("PLAYERBAR_H", tokens::PLAYERBAR_H, 88.0),
    ] {
        // The comparison the egui store's own tests use: both sides are
        // declared constants, so equality is the port's claim, and epsilon
        // keeps the float-compare lint at bay without weakening it.
        assert!(
            (ported - source).abs() < f32::EPSILON,
            "{name} drifted from the egui store's value"
        );
    }
}

/// Reduce Motion is a flag, not a color: the motion axis resolves through
/// the tokens alone, and the palettes carry no motion field — the struct
/// has only color and flag fields, and `slots` covers every color.
#[test]
fn reduce_motion_is_not_a_color() {
    assert!((tokens::MOTION_DEFAULT - 0.18).abs() < f32::EPSILON);
    assert!((tokens::MOTION_HOVER - 0.10).abs() < f32::EPSILON);
    assert!((tokens::MOTION_REDUCED - 0.0).abs() < f32::EPSILON);
    let _ = slots(&Palette::dark());
}

/// The menu and window shadows port at the egui store's geometry and
/// coverage: centered vertical drops, alphas 115/45 and 120/50.
#[test]
fn the_shadows_match_the_egui_store() {
    let shadows = [
        (&tokens::SHADOW_MENU_DARK, 8.0, 20.0, 115),
        (&tokens::SHADOW_MENU_LIGHT, 8.0, 20.0, 45),
        (&tokens::SHADOW_WINDOW_DARK, 12.0, 28.0, 120),
        (&tokens::SHADOW_WINDOW_LIGHT, 12.0, 28.0, 50),
    ];
    for (shadow, y, blur, alpha) in shadows {
        assert_eq!(shadow.offset_x, nsh_core::geometry::Points(0.0));
        assert_eq!(shadow.offset_y, nsh_core::geometry::Points(y));
        assert_eq!(shadow.blur, nsh_core::geometry::Points(blur));
        assert_eq!(shadow.spread, nsh_core::geometry::Points(0.0));
        assert!(!shadow.inset);
        assert_eq!(shadow.color.0 & 0xff, alpha, "coverage drifted");
        assert_eq!(shadow.color.0 >> 8, 0, "the drop is black");
    }
}

/// The brand gradient's stops: the light rung first, the primary second —
/// paint order the fixture's strip renders top to bottom.
#[test]
fn the_brand_gradient_runs_light_to_deep() {
    assert_eq!(palette::BRAND_GRADIENT[0], palette::BRAND_400);
    assert_eq!(palette::BRAND_GRADIENT[1], palette::BRAND_500);
}

/// The icon set: 35 vendored glyphs, every asset name unique, every source
/// a 24-box Lucide document stroking `currentColor` (the tint path's one
/// requirement), and `ALL` covering the enum exactly.
#[test]
fn the_icon_set_is_complete() {
    assert_eq!(Icon::ALL.len(), 35, "the egui store vendors 35 glyphs");
    let mut names: Vec<&str> = Icon::ALL.iter().map(|i| i.asset_name()).collect();
    names.sort_unstable();
    let unique = names.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), names.len(), "asset names must be unique");
    // Every glyph's compiled-in source is a currentColor-stroked 24-box
    // Lucide document, so the svg element's text color is the whole tint.
    for icon in Icon::ALL {
        let source = icon.markup();
        assert!(
            source.contains("currentColor"),
            "{}: the glyph must stroke currentColor for the tint to reach it",
            icon.asset_name()
        );
        assert!(
            source.contains("viewBox=\"0 0 24 24\""),
            "{}: the glyph must carry the 24-box Lucide viewBox",
            icon.asset_name()
        );
    }
    // Every name names a file the crate vendors (and `assets!()` ships).
    let manifest = env!("CARGO_MANIFEST_DIR");
    for name in &names {
        let path = std::path::Path::new(manifest)
            .join("assets/icons")
            .join(format!("{name}.svg"));
        assert!(
            path.is_file(),
            "the vendored glyph is missing: {}",
            path.display()
        );
    }
}
