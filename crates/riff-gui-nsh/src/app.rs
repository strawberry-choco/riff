//! The root component: the whole of the N1 UI.

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::APP_NAME;

/// The app's root: its name and version, centred on the theme's colours.
#[component]
pub fn App() -> impl IntoElement {
    // `flex_1` is load-bearing: a view that sizes to its children leaves the
    // centring no space to centre in (P0 correction 21). The colours are read
    // inside `move ||` so a theme change re-runs the binding.
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(text(APP_NAME).text_3xl())
        .child(text(concat!("v", env!("CARGO_PKG_VERSION"))).text_sm())
}
