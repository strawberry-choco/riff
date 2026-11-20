//! The root component: the palette-driven shell. The authoritative theme
//! store is the `Signal<Palette>` created here (ADR 0004's N2 amendment):
//! every riff surface reads its colors from that signal, so dark/light and
//! High Contrast switch at runtime without an nsh theme API. `theme.json`
//! only restyles `nsh_base` defaults.

use nsh_core::prelude::*;

use crate::theme::{
    Palette,
    fonts::{INTER_FAMILY, WEIGHT_SEMIBOLD},
};

use crate::APP_NAME;

/// The app's root: its name and version on the live palette, plus the dev
/// fixture in debug builds.
#[component]
pub fn App() -> impl IntoElement {
    // The vendored Inter set registers before the first frame, from bytes
    // compiled in (see `theme::fonts` for why not the `asset:/` path).
    crate::theme::fonts::register_inter_fonts();

    // The theme store. `Palette` is `Copy + PartialEq`, so the signal
    // dedups by equality and every read below is a cheap copy.
    let palette = signal(Palette::dark());

    // The dev fixture's control surface. Compiled out in release, with the
    // fixture itself: the screen is reachable only in debug builds. Debug
    // builds open on the fixture; mod+alt+t returns to the home view.
    #[cfg(debug_assertions)]
    let show_tokens = crate::screens::dev_tokens::install_switcher(palette);
    #[cfg(debug_assertions)]
    let content = when(show_tokens, move || {
        crate::screens::dev_tokens::fixture(palette)
    })
    .otherwise(move || home(palette));
    #[cfg(not(debug_assertions))]
    let content = home(palette);

    view()
        .flex_1()
        .flex_col()
        .bg(move || palette.get().background)
        .text_color(move || palette.get().ink)
        .child(content)
}

/// The N1 home view, now reading the palette signal instead of the
/// `nsh_base` theme: the riff name at the hero step (text-3xl at Inter
/// `SemiBold`, the egui store's `hero_title_font` pairing) over the version
/// line.
fn home(palette: Signal<Palette>) -> impl IntoElement {
    // `flex_1` is load-bearing: a view that sizes to its children leaves
    // the centering no space to centre in (P0 correction 21).
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .bg(move || palette.get().background)
        .text_color(move || palette.get().ink)
        .child(
            text(APP_NAME)
                .text_3xl()
                .font_family(INTER_FAMILY)
                .font_weight(WEIGHT_SEMIBOLD),
        )
        .child(text(concat!("v", env!("CARGO_PKG_VERSION"))).text_sm())
}
