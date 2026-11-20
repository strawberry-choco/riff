//! The root component. N3 moves the backend runtime in: the app spawns the
//! Composition Root's `AppRuntime`, the bridge owns it in a `stored` model,
//! and the pump — installed once here — writes the signals at runtime (the
//! debug fixture and readout controls also write, on the same thread). The
//! palette now comes out of the bridge's `UiState` (the same one
//! `Signal<Palette>` store ADR 0004's amendment names; only its
//! construction moved from this file into `bridge::state`).
//!
//! Debug builds open on the bridge readout (mod+alt+r toggles it, mod+alt+t
//! opens the N2 fixture); release builds keep the N1 home view.

use nsh_core::prelude::*;

use crate::bridge::{UiModel, install, runtime::spawn_runtime};
use crate::theme::Palette;
use crate::theme::fonts::{INTER_FAMILY, WEIGHT_SEMIBOLD};

use crate::APP_NAME;

/// The app's root: the bridge model on the engine thread, and the palette-
/// driven content over it.
#[component]
pub fn App() -> impl IntoElement {
    // The vendored Inter set registers before the first frame, from bytes
    // compiled in (see `theme::fonts` for why not the `asset:/` path).
    crate::theme::fonts::register_inter_fonts();

    // The bridge: spawn the runtime over the synthetic N3 store, move it
    // into the engine-thread model, and start the pump (the one signal
    // writer). A store failure is fatal: `spawn_runtime` logs and exits
    // non-zero.
    let (rt, lifecycle) = spawn_runtime();
    let model = stored(UiModel::new(rt, Some(lifecycle)));
    install(model);

    // The signals, copied out once. Every binding below reads handles and
    // never re-borrows the model's store (P0 correction 20).
    let state = model.with(|m| m.signals());
    let palette: Signal<Palette> = state.palette;

    // The dev fixture's control surface (N2), plus the N3 readout's. Both
    // are compiled out in release with their screens.
    #[cfg(debug_assertions)]
    let show_fixture = crate::screens::dev_tokens::install_switcher(palette);
    // N3 opens on the readout; mod+alt+t still opens the fixture.
    #[cfg(debug_assertions)]
    show_fixture.set(false);
    #[cfg(debug_assertions)]
    let show_readout = crate::screens::bridge_readout::install();
    #[cfg(debug_assertions)]
    crate::screens::bridge_readout::install_transport_keys(model);
    #[cfg(debug_assertions)]
    let content = when(show_readout, move || {
        crate::screens::bridge_readout::readout(model, state)
    })
    .otherwise(move || {
        when(show_fixture, move || {
            crate::screens::dev_tokens::fixture(palette)
        })
        .otherwise(move || home(palette))
    });
    #[cfg(not(debug_assertions))]
    let content = home(palette);

    view()
        .flex_1()
        .flex_col()
        .bg(move || palette.get().background)
        .text_color(move || palette.get().ink)
        .child(content)
}

/// The N1 home view, reading the palette signal instead of the `nsh_base`
/// theme: the riff name at the hero step (text-3xl at Inter `SemiBold`, the
/// egui store's `hero_title_font` pairing) over the version line.
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
