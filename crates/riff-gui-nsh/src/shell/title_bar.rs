//! The title bar: the port of the egui chrome's strip (`ui/chrome.rs`'s
//! `show_titlebar`) — drag region first, then the controls that sit on top
//! of it — with the platform's own bar hidden and its height reserved. The
//! search field is the Ctrl+K target; the status line is the titlebar
//! feedback slot the egui app's `FeedbackBoard` filled, fed here from the
//! notices signal's newest entry.
//!
//! Under `TitleBarStyle::Hidden` the platform draws the system caption
//! buttons inside the `title_bar().right` region (P0 correction 23), and
//! they work — the live answer on Windows — so the shell draws no second
//! set and keeps the reservation as its only gesture toward them.

use nsh_base::input;
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::{current_window, title_bar};
use nsh_core::prelude::*;

use crate::APP_NAME;
use crate::bridge::state::UiState;
use crate::theme::Palette;
use crate::theme::fonts::{INTER_FAMILY, WEIGHT_SEMIBOLD};
use crate::theme::tokens;

use super::focus;
use super::ring;

/// The window's chrome: the platform's title bar hidden and reserved at the
/// token height, so the strip below is the only bar and the system's
/// caption buttons land inside the `title_bar().right` region the strip
/// pads past. Idempotent: the same options every time.
pub fn install_chrome() {
    let mut bar = TitleBarOptions::default();
    bar.style = TitleBarStyle::Hidden;
    bar.height = Some(px(tokens::TITLEBAR_H));
    let mut attributes = WindowAttributes::default();
    attributes.title_bar = Some(bar);
    current_window().set(attributes);
}

/// The strip: drag region first (the whole bar), then the children that
/// take pointer events over their slice of it. The right padding reserves
/// the platform caption buttons' region reactively, as the chrome spike
/// proved the widths report.
pub fn bar(state: UiState, search: &ElementRef, focus: Signal<Option<usize>>) -> impl IntoElement {
    let palette = state.palette;
    view()
        .h(px(tokens::TITLEBAR_H))
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_LG))
        .window_drag(true)
        .bg(move || palette.get().surface)
        .border_b(px(1.0))
        .border_color(move || palette.get().line)
        .pl(move || title_bar().left + px(tokens::SPACE_LG))
        .pr(move || title_bar().right + px(tokens::SPACE_LG))
        .test_id("titlebar")
        .child(wordmark(palette))
        .child(status(state))
        .child(search_field(state, search, focus))
}

/// The wordmark, at the egui store's hero pairing (`SemiBold` Inter).
fn wordmark(palette: Signal<Palette>) -> impl IntoElement {
    text(APP_NAME)
        .text_sm()
        .font_family(INTER_FAMILY)
        .font_weight(WEIGHT_SEMIBOLD)
        .text_color(move || palette.get().ink)
}

/// The status line: the newest typed notice, or nothing. `flex_1` gives
/// every leftover width to it so the search field sits right of the
/// wordmark cluster like the egui strip's did, and `truncate` keeps a long
/// message from pushing the bar's layout around.
fn status(state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let line = move || {
        state
            .notices
            .get()
            .last()
            .map_or_else(String::new, |notice| notice.message.clone())
    };
    text(line)
        .flex_1()
        .text_xs()
        .truncate()
        .text_color(move || palette.get().ink_3)
        .test_id("status")
}

/// The global search field — the placeholder Ctrl+K focuses. Its text is
/// local until N7 wires the query to the backend search; the value signal
/// keeps it a controlled field so Escape's blur leaves the text as typed.
fn search_field(
    state: UiState,
    search: &ElementRef,
    focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    let query = signal(String::new());
    ring(
        input()
            .element_ref(search)
            .value(query)
            .on_change(move |text: &str| query.set(text.to_owned()))
            .placeholder("Search")
            .test_id("search")
            .label("Search")
            .text_sm()
            .w(px(tokens::SIDEBAR_W))
            .p(px(tokens::SPACE_MD))
            .rounded(px(tokens::RADIUS_FULL))
            .bg(move || palette.get().surface_2)
            .text_color(move || palette.get().ink),
        focus::SEARCH,
        focus,
        palette,
    )
}
