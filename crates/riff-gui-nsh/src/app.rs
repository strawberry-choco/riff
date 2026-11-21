//! The root component. The bridge owns the backend runtime and the signals
//! (see `bridge`), and this mounts the N4 app shell over it: the
//! titlebar/sidebar/player-bar chrome reading the palette and view signals,
//! the transport intents dispatching through the model, and the window
//! geometry loop in `shell::layout`. The palette still comes out of the
//! bridge's `UiState` — the same one `Signal<Palette>` store ADR 0004's
//! amendment names.
//!
//! Debug builds open on the bridge readout (mod+alt+r toggles it, mod+alt+t
//! opens the N2 fixture) and mount the shell only when the readout is
//! hidden; release builds mount the shell at the root.

use nsh_core::prelude::*;

use crate::bridge::{UiModel, install, runtime::spawn_runtime};
use crate::theme::Palette;

/// The app's root: the bridge model on the engine thread, and the shell
/// over it.
#[component]
pub fn App() -> impl IntoElement {
    // The vendored Inter set registers before the first frame, from bytes
    // compiled in (see `theme::fonts` for why not the `asset:/` path).
    crate::theme::fonts::register_inter_fonts();

    // The bridge: spawn the runtime over the synthetic store, move it into
    // the engine-thread model, and start the pump (the one signal writer).
    // A store failure is fatal: `spawn_runtime` logs and exits non-zero.
    let (rt, lifecycle) = spawn_runtime();
    let model = stored(UiModel::new(rt, Some(lifecycle)));
    install(model);

    // The signals, copied out once. Every binding below reads handles and
    // never re-borrows the model's store (P0 correction 20).
    let state = model.with(|m| m.signals());
    let palette: Signal<Palette> = state.palette;

    // The persisted High Contrast flag opens the app on its variant; the
    // base family stays the session's Dark default (the store carries no
    // light/dark column, exactly as the egui store did not).
    let high_contrast = model.with(|m| {
        use riff_backend::app::MutexExt;
        m.rt.library.lock_or_recover().ui_flags.high_contrast
    });
    if high_contrast {
        palette.set(Palette::dark().high_contrast());
        state.appearance.set(crate::model::AppearanceMode::DarkHighContrast);
    }

    // The dev fixture's control surface (N2), plus the N3 readout's toggles.
    // Both are compiled out in release with their screens.
    #[cfg(debug_assertions)]
    let show_fixture = crate::screens::dev_tokens::install_switcher(palette);
    #[cfg(debug_assertions)]
    show_fixture.set(false);
    #[cfg(debug_assertions)]
    let show_readout = crate::screens::bridge_readout::install();
    #[cfg(debug_assertions)]
    crate::screens::bridge_readout::install_transport_keys(model);
    // The readout and the shell are mutually exclusive subtrees, so their
    // window-level key bindings never live at once: the readout carries its
    // transport keys while it is up, the shell's map (space, mod+k, the
    // arrows) while it is.
    #[cfg(debug_assertions)]
    let content = when(show_readout, move || {
        crate::screens::bridge_readout::readout(model, state)
    })
    .otherwise(move || {
        when(show_fixture, move || {
            crate::screens::dev_tokens::fixture(palette)
        })
        .otherwise(move || crate::shell::install(model))
    });
    #[cfg(not(debug_assertions))]
    let content = crate::shell::install(model);

    view()
        .flex_1()
        .flex_col()
        .bg(move || palette.get().background)
        .text_color(move || palette.get().ink)
        .child(content)
}
