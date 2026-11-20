//! The dev fixture screen: every color, radius, space, type size, font
//! weight, icon, shadow and the brand gradient the N2 port ships, drawn
//! from the live palette signal so all four variants switch at runtime.
//!
//! Reachable only in debug builds: the entry point lives under
//! `#[cfg(debug_assertions)]` in `crate::app`, so a release build compiles
//! this module out entirely. `n2-dark.png`, `n2-light.png` and
//! `n2-contrast.png` under `docs/execution-plans/evidence/nsh/n2/shots/`
//! are captured from this screen.

use nsh_core::BoxShadow;
use nsh_core::color::Hsla;
use nsh_core::prelude::*;

use crate::theme::Variant;
use crate::theme::fonts::{
    INTER_FACES, INTER_FAMILY, WEIGHT_BOLD, WEIGHT_MEDIUM, WEIGHT_REGULAR, WEIGHT_SEMIBOLD,
};
use crate::theme::icons::{Icon, icon};
use crate::theme::palette::{self, Palette};
use crate::theme::tokens;

/// The fixture's control surface, installed by the app under
/// `#[cfg(debug_assertions)]`: the four variant keys (mod+alt+1..4) and the
/// fixture toggle (mod+alt+t). Returns the toggle signal — debug builds
/// open on the fixture, mod+alt+t returns to the home view.
///
/// The shape is P0's palette-switching proof: a press (here a keystroke)
/// writes the signal, bindings read it. All signal writes stay on the
/// engine thread (P0 correction 9); `Palette` is `Copy`, so nothing here
/// needs `stored`.
pub fn install_switcher(palette: Signal<Palette>) -> Signal<bool> {
    let show = signal(true);
    on_key("mod+alt+t", move |_| show.update(|s| *s = !*s));
    let keys: [(&str, Variant); 4] = [
        ("mod+alt+1", Variant::Dark),
        ("mod+alt+2", Variant::Light),
        ("mod+alt+3", Variant::HighContrastDark),
        ("mod+alt+4", Variant::HighContrastLight),
    ];
    for (keystroke, variant) in keys {
        on_key(keystroke, move |_| palette.set(Palette::of(variant)));
    }
    show
}

/// The dev fixture: the switcher header plus the token sections, reading
/// `palette` in bindings so a variant switch re-themes the screen.
pub fn fixture(palette: Signal<Palette>) -> impl IntoElement {
    view()
        .flex_1()
        .flex_col()
        .bg(move || palette.get().background)
        .text_color(move || palette.get().ink)
        .child(header(palette))
        .child(sections(palette))
}

/// The fixture's header: title, the four variant buttons, and the debug
/// keys' legend.
fn header(palette: Signal<Palette>) -> impl IntoElement {
    let title = move || format!("riff dev tokens — {}", variant_name(palette));
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .p(px(tokens::SPACE_LG))
        .border_b(px(1.0))
        .border_color(move || palette.get().line)
        .child(
            text(title)
                .text_xl()
                .font_family(INTER_FAMILY)
                .font_weight(WEIGHT_SEMIBOLD)
                .text_color(move || palette.get().ink),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(tokens::SPACE_SM))
                .children(Variant::ALL.map(|variant| switch_button(palette, variant))),
        )
        .child(
            text("mod+alt+1..4 switch variants · mod+alt+t closes the fixture")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
}

/// The active variant's name, read through the signal so the header binding
/// re-runs on a switch (P0 correction 21).
fn variant_name(palette: Signal<Palette>) -> String {
    let p = palette.get();
    if p.high_contrast {
        format!("high contrast over {}", family(&p))
    } else {
        family(&p)
    }
}

fn family(p: &Palette) -> String {
    if p.dark {
        "dark".to_owned()
    } else {
        "light".to_owned()
    }
}

/// One variant button: a riff-styled press target in the shape P0's
/// palette-switching proof used — a press writes the signal, bindings read
/// it. Active shows on the focus ring and the raised fill.
fn switch_button(palette: Signal<Palette>, variant: Variant) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .rounded(px(tokens::RADIUS_SM))
        .py(px(tokens::SPACE_XS))
        .px(px(tokens::SPACE_MD))
        .test_id("variant-button")
        .on_press(move || palette.set(Palette::of(variant)))
        .bg(move || {
            let p = palette.get();
            if p == Palette::of(variant) {
                p.surface_3
            } else {
                p.surface_2
            }
        })
        .border(px(1.0))
        .border_color(move || {
            let p = palette.get();
            if p == Palette::of(variant) {
                p.focus_ring
            } else {
                p.border
            }
        })
        .child(
            text(variant.name())
                .text_sm()
                .text_color(move || palette.get().ink),
        )
}

/// The token sections, in a scrolling column (`flex_1` is load-bearing:
/// P0 correction 21 — a scrolling child inside a flex container collapses
/// to nothing without it).
fn sections(palette: Signal<Palette>) -> impl IntoElement {
    scroll()
        .flex_1()
        .flex_col()
        .gap(px(tokens::SPACE_LG))
        .p(px(tokens::SPACE_LG))
        .child(colors_section(palette))
        .child(radii_section(palette))
        .child(spaces_section(palette))
        .child(type_section(palette))
        .child(icons_section(palette))
        .child(shadows_section(palette))
        .child(gradient_section(palette))
        .child(fonts_section(palette))
}

/// A titled section: the heading rides `ink_2`, its body keeps the
/// inherited `ink`.
fn section_title(palette: Signal<Palette>, title: &str) -> impl IntoElement {
    text(title.to_owned())
        .text_sm()
        .font_family(INTER_FAMILY)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(move || palette.get().ink_2)
}

/// One palette slot's name and its accessor, the table the color section
/// renders.
type SlotTable = [(&'static str, fn(&Palette) -> Hsla); 18];

/// Every color slot of the active palette: swatch, slot name, resolved hex.
/// Each swatch reads the signal, so the section is the palette switch made
/// visible.
fn colors_section(palette: Signal<Palette>) -> impl IntoElement {
    let slots: SlotTable = [
        ("background", |p| p.background),
        ("surface_row", |p| p.surface_row),
        ("surface", |p| p.surface),
        ("surface_2", |p| p.surface_2),
        ("surface_3", |p| p.surface_3),
        ("row_hover", |p| p.row_hover),
        ("ink", |p| p.ink),
        ("ink_2", |p| p.ink_2),
        ("ink_3", |p| p.ink_3),
        ("line", |p| p.line),
        ("border", |p| p.border),
        ("brand_primary", |p| p.brand_primary),
        ("on_brand", |p| p.on_brand),
        ("success", |p| p.success),
        ("warning", |p| p.warning),
        ("error", |p| p.error),
        ("info", |p| p.info),
        ("focus_ring", |p| p.focus_ring),
    ];
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(section_title(palette, "Palette — 18 slots"))
        .child(
            view()
                .flex_col()
                .gap(px(tokens::SPACE_XS))
                .children(slots.map(|(name, slot)| swatch_row(palette, name, slot))),
        )
}

/// One color slot's row: the swatch paints the slot's live color; the label
/// prints the resolved hex so a wrong port reads in the file too.
fn swatch_row(
    palette: Signal<Palette>,
    name: &'static str,
    slot: fn(&Palette) -> Hsla,
) -> impl IntoElement {
    let label = move || format!("{name}  {}", slot(&palette.get()).to_hex());
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .test_id("swatch-row")
        .child(
            view()
                .rounded(px(tokens::RADIUS_SM))
                .border(px(1.0))
                .border_color(move || palette.get().border)
                .w(px(40.0))
                .h(px(24.0))
                .bg(move || slot(&palette.get())),
        )
        .child(
            text(label)
                .text_xs()
                .font_family(INTER_FAMILY)
                .text_color(move || palette.get().ink),
        )
}

/// The six radius steps as chips of their own corner.
fn radii_section(palette: Signal<Palette>) -> impl IntoElement {
    let steps: [(&str, f32); 6] = [
        ("XS", tokens::RADIUS_XS),
        ("SM", tokens::RADIUS_SM),
        ("MD", tokens::RADIUS_MD),
        ("LG", tokens::RADIUS_LG),
        ("XL", tokens::RADIUS_XL),
        ("FULL", tokens::RADIUS_FULL),
    ];
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(section_title(palette, "Radii — 6 steps"))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(tokens::SPACE_MD))
                .children(steps.map(|(name, radius)| {
                    let label = move || format!("{name} {}", radius_label(radius));
                    view()
                        .flex_col()
                        .items_center()
                        .gap(px(tokens::SPACE_XS))
                        .child(
                            view()
                                .rounded(px(radius))
                                .w(px(56.0))
                                .h(px(32.0))
                                .bg(move || palette.get().surface_3)
                                .border(px(1.0))
                                .border_color(move || palette.get().border),
                        )
                        .child(
                            text(label)
                                .text_xs()
                                .text_color(move || palette.get().ink_3),
                        )
                })),
        )
}

fn radius_label(radius: f32) -> String {
    if radius >= tokens::RADIUS_FULL {
        "999".to_owned()
    } else {
        format!("{radius:.0}")
    }
}

/// The four spacing steps as bars of their own width.
fn spaces_section(palette: Signal<Palette>) -> impl IntoElement {
    let steps: [(&str, f32); 4] = [
        ("XS", tokens::SPACE_XS),
        ("SM", tokens::SPACE_SM),
        ("MD", tokens::SPACE_MD),
        ("LG", tokens::SPACE_LG),
    ];
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(section_title(palette, "Spacing — 4 steps"))
        .child(
            view()
                .flex_col()
                .gap(px(tokens::SPACE_XS))
                .children(steps.map(|(name, space)| {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(tokens::SPACE_SM))
                        .child(
                            view()
                                .rounded(px(tokens::RADIUS_XS))
                                .w(px(space))
                                .h(px(16.0))
                                .bg(move || palette.get().brand_primary),
                        )
                        .child(
                            text(format!("{name} {space:.0}"))
                                .text_xs()
                                .text_color(move || palette.get().ink_2),
                        )
                })),
        )
}

/// The type scale at its own sizes, ending at the hero step's weight.
fn type_section(palette: Signal<Palette>) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(section_title(palette, "Type scale"))
        .child(
            view()
                .flex_col()
                .gap(px(tokens::SPACE_SM))
                .child(text("text-xs — 12px muted labels").text_size(px(tokens::TEXT_XS)))
                .child(text("text-sm — 14px the workhorse size").text_size(px(tokens::TEXT_SM)))
                .child(text("text-xl — 20px section headings").text_size(px(tokens::TEXT_XL)))
                .child(
                    text("text-3xl — 30px Now Playing title")
                        .text_size(px(tokens::TEXT_3XL))
                        .font_family(INTER_FAMILY)
                        .font_weight(WEIGHT_SEMIBOLD),
                ),
        )
}

/// The 35 vendored Lucide glyphs, tinted to the live `ink` through the
/// reactive tint, each with its asset name under it.
fn icons_section(palette: Signal<Palette>) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(section_title(palette, "Icons — 35 Lucide glyphs"))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(tokens::SPACE_MD))
                .children(Icon::ALL.iter().map(|glyph| {
                    let name = glyph.asset_name().to_owned();
                    view()
                        .flex_col()
                        .items_center()
                        .gap(px(tokens::SPACE_XS))
                        .w(px(72.0))
                        .child(icon(*glyph, Icon::ICON_SIZE, move || palette.get().ink))
                        .child(text(name).text_xs().text_color(move || palette.get().ink_3))
                })),
        )
}

/// The ported elevation: both families' menu and window drops side by side,
/// since a shadow's coverage is per family while this section shows both.
/// nsh reads CSS box-shadows natively ([`BoxShadow`]), so the four egui
/// tokens port one for one.
fn shadows_section(palette: Signal<Palette>) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(section_title(
            palette,
            "Shadows — menu and window, both families",
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(tokens::SPACE_LG))
                .child(shadow_box(
                    tokens::SHADOW_MENU_DARK,
                    palette::Palette::dark().surface_2,
                    palette::Palette::dark().ink_3,
                    "menu / dark",
                ))
                .child(shadow_box(
                    tokens::SHADOW_MENU_LIGHT,
                    palette::Palette::light().surface_2,
                    palette::Palette::light().ink_3,
                    "menu / light",
                ))
                .child(shadow_box(
                    tokens::SHADOW_WINDOW_DARK,
                    palette::Palette::dark().surface_2,
                    palette::Palette::dark().ink_3,
                    "window / dark",
                ))
                .child(shadow_box(
                    tokens::SHADOW_WINDOW_LIGHT,
                    palette::Palette::light().surface_2,
                    palette::Palette::light().ink_3,
                    "window / light",
                )),
        )
}

/// One shadow sample: a card of the family's raised surface over the
/// section's background, wearing one ported drop.
fn shadow_box(shadow: BoxShadow, face: Hsla, label_ink: Hsla, label: &str) -> impl IntoElement {
    view()
        .flex_col()
        .items_center()
        .gap(px(tokens::SPACE_XS))
        .child(
            view()
                .rounded(px(tokens::RADIUS_MD))
                .w(px(96.0))
                .h(px(48.0))
                .bg(face)
                .shadow([shadow])
                .items_center()
                .justify_center()
                .child(text(label.to_owned()).text_xs().text_color(label_ink)),
        )
}

/// The brand gradient: nsh draws linear-gradient backgrounds natively, so
/// the two brand rungs port as a top-to-bottom strip. The stops are
/// family-invariant consts (the source's argument), so the strip is the one
/// sample that does not switch; its label follows the live palette.
fn gradient_section(palette: Signal<Palette>) -> impl IntoElement {
    let [top, bottom] = palette::BRAND_GRADIENT;
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(
            text("Brand gradient — brand-400 to brand-500")
                .text_sm()
                .font_family(INTER_FAMILY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(move || palette.get().ink_2),
        )
        .child(
            view()
                .rounded(px(tokens::RADIUS_MD))
                .w_full()
                .h(px(48.0))
                .bg_linear_to_b()
                .gradient_from(top)
                .gradient_to(bottom),
        )
}

/// The four vendored Inter weights, on one family line each, under the
/// family name the asset macros read from the TTFs at compile time (the
/// one `font_family` value riff text names; registration itself happens
/// through `assets!()`).
fn fonts_section(palette: Signal<Palette>) -> impl IntoElement {
    let samples: [(&str, FontWeight); 4] = [
        ("Regular 400 — body text", WEIGHT_REGULAR),
        ("Medium 500 — buttons, row labels", WEIGHT_MEDIUM),
        ("SemiBold 600 — headers, hero title", WEIGHT_SEMIBOLD),
        ("Bold 700 — wordmark accents", WEIGHT_BOLD),
    ];
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .child(
            text(format!(
                "Fonts — the four Inter weights (family {:?})",
                INTER_FACES[0].family
            ))
            .text_sm()
            .font_family(INTER_FAMILY)
            .font_weight(WEIGHT_MEDIUM)
            .text_color(move || palette.get().ink_2),
        )
        .child(
            view()
                .flex_col()
                .gap(px(tokens::SPACE_SM))
                .children(samples.map(|(label, weight)| {
                    text(label.to_owned())
                        .text_size(px(tokens::TEXT_SM))
                        .font_family(INTER_FAMILY)
                        .font_weight(weight)
                        .text_color(move || palette.get().ink)
                })),
        )
}
