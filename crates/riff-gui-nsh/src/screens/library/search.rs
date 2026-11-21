//! The Ctrl+K search flow: the titlebar field is already mounted (N4); this
//! module owns what the field's edits mean. A keystroke writes the Library
//! session's `search_query` — the backend's matching rules take it from
//! there, each stage's reads scope themselves — and mirrors it to the
//! [`UiState::search`](crate::bridge::state::UiState::search) signal the
//! render bindings subscribe to (the sidebar's `apply` shape: a search
//! write mutates no store, so the pump cannot carry it; the pump reads the
//! session directly and retargets the listing, and the row window's reset
//! is announced through the list revision). Escape restores the full list
//! and surrenders the field, the egui text field's `dismiss_on_escape`
//! contract; while the field holds the keyboard focus the framework's
//! `When::Auto` bindings give way to it, so Space types a space exactly
//! where the egui app gated Space on `!wants_keyboard_input`.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// One keystroke in the field: the session write and the signal mirror,
/// outside the borrow. The pump's next tick retargets the listing (the
/// window re-stamps, drops its rows and bumps the revision) — the
/// `set_sort` seam, now answering for the search needle too.
pub fn query_input(model: Stored<UiModel>, text: &str) {
    let search = model.with(|m| m.state.search);
    model.with(|m| {
        text.clone_into(&mut m.rt.library.lock_or_recover().search_query);
    });
    search.set(text.to_owned());
}

/// Escape while the field is focused: the query clears (the full list
/// restores through the same retarget a keystroke takes) and the field
/// blurs. The field's own visible text stays until typed over — a
/// controlled value reset does not rewrite a native control the user
/// edited (the framework last pushed the empty initial, the native edit
/// happened outside its diff), the windowed drive's one open gap,
/// recorded in the trail.
pub fn escape(model: Stored<UiModel>, value: Signal<String>, search: ElementRef) {
    query_input(model, "");
    value.set(String::new());
    search.blur();
}

/// The empty state a listing shows, query-aware: the stage's own copy when
/// no needle stands, the filtered copy (the query in the hint) when one
/// does. The egui columns' empty copies ported; the anatomy is the shared
/// empty state's, rebuilt here because the hint carries the query.
pub fn empty_state_for(
    palette: Signal<crate::theme::Palette>,
    query: impl Fn() -> String + Clone + 'static,
    plain: (&'static str, &'static str),
    filtered: (&'static str, &'static str),
) -> impl IntoElement {
    let is_plain = {
        let query = query.clone();
        move || query().is_empty()
    };
    when(is_plain, {
        move || crate::components::empty_state::state(palette, plain.0, plain.1).into_element()
    })
    .otherwise({
        let query = query.clone();
        move || {
            let query = query.clone();
            let filtered_title = filtered.0;
            let filtered_hint = filtered.1;
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
                    text(filtered_title)
                        .text_sm()
                        .text_color(move || palette.get().ink_2),
                )
                .child(
                    text(move || format!("{} '{}'.", filtered_hint, query()))
                        .text_xs()
                        .text_color(move || palette.get().ink_3),
                )
                .into_element()
        }
    })
}
