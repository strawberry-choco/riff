//! The empty state: the one block every empty surface paints instead of
//! nothing — the egui crate's `ui/browser.rs` `empty_state` (title over
//! hint, centered, muted). The inspector's no-selection state and an empty
//! listing share it; the caller supplies the copy.

use nsh_core::prelude::*;

use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// The empty state: glyph, title, hint. The title is the surface's own
/// words; the hint says what would fill the space.
pub fn state(
    palette: Signal<crate::theme::Palette>,
    title: &'static str,
    hint: &'static str,
) -> impl IntoElement {
    view()
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(tokens::SPACE_SM))
        .test_id("empty-state")
        .child(icon(Icon::Music, Icon::ICON_SIZE, move || {
            palette.get().ink_3
        }))
        .child(
            text(title)
                .text_sm()
                .text_color(move || palette.get().ink_2),
        )
        .child(text(hint).text_xs().text_color(move || palette.get().ink_3))
}
