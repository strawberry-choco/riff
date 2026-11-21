//! The About page: the version, the offline statement and the third-party
//! notices.

use nsh_core::prelude::*;

use crate::bridge::state::UiState;
use crate::theme::tokens;

/// The page.
pub fn page(state: UiState) -> AnyElement {
    let palette = state.palette;
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .test_id("settings-about")
        .child(
            text(format!("{} {}", crate::APP_NAME, env!("CARGO_PKG_VERSION")))
                .text_xl()
                .test_id("about-version"),
        )
        .child(
            text("A local-first music player. Your library, playlists and settings stay on this machine.")
                .text_sm()
                .text_color(move || palette.get().ink_2)
                .test_id("about-offline"),
        )
        .child(
            text("riff works offline. Nothing about your listening leaves your computer.")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            text("Licensed under the MIT License.")
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("about-license"),
        )
        .child(
            text("Bundled: Inter (SIL Open Font License), Lucide icons (ISC License), nsh (MIT).")
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("about-third-party"),
        )
        .into_element()
}
