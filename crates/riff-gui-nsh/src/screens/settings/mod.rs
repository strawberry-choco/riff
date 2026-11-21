//! The Settings screen: the five pages the egui `ui/settings.rs` shipped —
//! Library, Playback, Appearance, Advanced and About — each control writing
//! the same session field or backend lever it wrote there, so the pump's
//! frame-end diff-commit lands it in the Application Store.
//!
//! The screen is a view mode of the shell: the sidebar's Settings control
//! opens it, Back returns to the Library view.

pub mod about;
pub mod advanced;
pub mod appearance;
pub mod library;
pub mod playback;

use nsh_core::Checked;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::ViewMode;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::model::SettingsPage;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// The screen: the page list beside the selected page.
pub fn screen(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_1()
        .flex_row()
        .test_id("settings")
        .bg(move || palette.get().surface_row)
        .child(nav(model, state))
        .child(
            scroll()
                .flex_1()
                .flex_col()
                .p(px(tokens::SPACE_LG))
                .gap(px(tokens::SPACE_LG))
                .test_id("settings-page")
                .child(dynamic(move || {
                    // Rebuild on a session change so a toggle flips its own
                    // row without a page-local signal; typing in a field
                    // writes only that field's draft, so the rebuild never
                    // interrupts it.
                    let _ = state.settings.get();
                    let _ = state.library_paths.get();
                    page(model, state)
                })),
        )
}

/// The page list and Back.
fn nav(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .w(px(tokens::COLUMN_MIN_W))
        .flex_col()
        .gap(px(tokens::SPACE_XS))
        .p(px(tokens::SPACE_LG))
        .border_r(px(tokens::BORDER_HAIRLINE))
        .border_color(move || palette.get().line)
        .bg(move || palette.get().surface)
        .test_id("settings-nav")
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_SM))
                .pb(px(tokens::SPACE_MD))
                .child(icon(Icon::Settings, Icon::ICON_SIZE, move || {
                    palette.get().ink_2
                }))
                .child(
                    text("SETTINGS")
                        .text_xs()
                        .text_color(move || palette.get().ink_3),
                ),
        )
        .children(SettingsPage::ALL.iter().map(|page| {
            let page = *page;
            let active = move || state.settings_page.get() == page;
            view()
                .on_press(move || state.settings_page.set(page))
                .test_id(&format!("settings-page-{}", page.label().to_lowercase()))
                .label(page.label())
                .px(px(tokens::SPACE_SM))
                .py(px(tokens::SPACE_XS))
                .rounded(px(tokens::RADIUS_SM))
                .bg(move || {
                    if active() {
                        palette.get().surface_3
                    } else {
                        transparent_black()
                    }
                })
                .hover(|s| s.bg(move || palette.get().row_hover))
                .child(
                    text(page.label())
                        .text_sm()
                        .text_color(move || palette.get().ink_2),
                )
        }))
        .child(
            view()
                .on_press(move || close(model, state))
                .test_id("settings-back")
                .label("Back to the library")
                .mt(px(tokens::SPACE_LG))
                .px(px(tokens::SPACE_SM))
                .py(px(tokens::SPACE_XS))
                .rounded(px(tokens::RADIUS_SM))
                .hover(|s| s.bg(move || palette.get().row_hover))
                .child(
                    text("Back")
                        .text_sm()
                        .text_color(move || palette.get().ink_2),
                ),
        )
}

/// The selected page.
fn page(model: Stored<UiModel>, state: UiState) -> AnyElement {
    match state.settings_page.get() {
        SettingsPage::Library => library::page(model, state),
        SettingsPage::Playback => playback::page(model, state),
        SettingsPage::Appearance => appearance::page(model, state),
        SettingsPage::Advanced => advanced::page(model, state),
        SettingsPage::About => about::page(state),
    }
}

/// Open the Settings view, mirrored to the nav signal.
pub fn open(model: Stored<UiModel>, state: UiState) {
    let nav = state.nav;
    let mut view = nav.get();
    model.with(|m| {
        m.rt.library.lock_or_recover().view_mode = ViewMode::Settings;
    });
    view.view_mode = ViewMode::Settings;
    nav.set(view);
    state.clear_confirm.set(false);
    state.settings_status.set(None);
}

/// Close: back to the Library view.
pub fn close(model: Stored<UiModel>, state: UiState) {
    let nav = state.nav;
    let mut view = nav.get();
    model.with(|m| {
        m.rt.library.lock_or_recover().view_mode = ViewMode::Library;
    });
    view.view_mode = ViewMode::Library;
    nav.set(view);
    state.clear_confirm.set(false);
}

// --- Shared controls -------------------------------------------------------

/// A section heading.
pub(crate) fn heading(state: UiState, title: &'static str) -> impl IntoElement {
    let palette = state.palette;
    text(title)
        .text_sm()
        .text_color(move || palette.get().ink)
        .test_id("settings-heading")
}

/// A boolean row: label, optional detail line, and a toggle that writes
/// through `on_change`.
pub(crate) fn toggle_row(
    state: UiState,
    id: &'static str,
    label: &'static str,
    detail: &'static str,
    checked: impl Fn() -> bool + Copy + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .py(px(tokens::SPACE_XS))
        .test_id(id)
        .child(
            view()
                .flex_1()
                .flex_col()
                .child(text(label).text_sm().text_color(move || palette.get().ink))
                .child(
                    text(detail)
                        .text_xs()
                        .text_color(move || palette.get().ink_3),
                ),
        )
        .child(
            view()
                .on_press(move || on_change(!checked()))
                .test_id(&format!("{id}-toggle"))
                .label(label)
                .accessibility_state(move || AccessibilityState {
                    checked: Some(if checked() {
                        Checked::True
                    } else {
                        Checked::False
                    }),
                    ..AccessibilityState::default()
                })
                .w(px(tokens::SWITCH_W))
                .h(px(tokens::SWITCH_H))
                .rounded(px(tokens::RADIUS_FULL))
                .bg(move || {
                    if checked() {
                        palette.get().brand_primary
                    } else {
                        palette.get().surface_3
                    }
                }),
        )
}

/// A plain action row.
pub(crate) fn action_row(
    state: UiState,
    id: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let palette = state.palette;
    view()
        .on_press(on_press)
        .test_id(id)
        .label(label)
        .px(px(tokens::SPACE_MD))
        .py(px(tokens::SPACE_XS))
        .rounded(px(tokens::RADIUS_SM))
        .border(px(tokens::BORDER_HAIRLINE))
        .border_color(move || palette.get().border)
        .hover(|s| s.bg(move || palette.get().row_hover))
        .child(
            text(label)
                .text_xs()
                .text_color(move || palette.get().ink_2),
        )
}

/// A choice row: the current value and its alternatives, each a button.
pub(crate) fn choice_row(
    state: UiState,
    id: &'static str,
    label: &'static str,
    choices: impl IntoIterator<Item = (&'static str, bool, Box<dyn Fn()>)>,
) -> impl IntoElement {
    let palette = state.palette;
    let mut row = view()
        .flex_col()
        .gap(px(tokens::SPACE_XS))
        .test_id(id)
        .child(text(label).text_sm().text_color(move || palette.get().ink));
    let mut buttons = view().flex_row().gap(px(tokens::SPACE_SM));
    for (text_label, selected, on_press) in choices {
        buttons = buttons.child(
            view()
                .on_press(on_press)
                .px(px(tokens::SPACE_MD))
                .py(px(tokens::SPACE_XS))
                .rounded(px(tokens::RADIUS_SM))
                .bg(move || {
                    if selected {
                        palette.get().brand_primary
                    } else {
                        palette.get().surface_2
                    }
                })
                .child(text(text_label).text_xs().text_color(move || {
                    if selected {
                        palette.get().on_brand
                    } else {
                        palette.get().ink_2
                    }
                })),
        );
    }
    row = row.child(buttons);
    row
}

/// The page's status line, when one stands.
pub(crate) fn status_line(state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(status) = state.settings_status.get() else {
        return view().into_element();
    };
    text(move || status.clone())
        .text_xs()
        .text_color(move || palette.get().ink_2)
        .test_id("settings-status")
        .into_element()
}
