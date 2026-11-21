//! The Appearance page: dark, light, system, the two high-contrast
//! variants, and Reduce Motion. Every choice writes the palette signal, so
//! the whole shell repaints without a restart.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::model::{AppearanceMode, SettingsPage};
use crate::theme::tokens;

use super::{heading, toggle_row};

/// The page.
pub fn page(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let reduce_motion = move || {
        let _ = state.settings.get();
        model.with(|m| m.rt.library.lock_or_recover().ui_flags.reduce_motion)
    };
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .test_id("settings-appearance")
        .child(heading(state, "Theme"))
        .child(
            view()
                .flex_col()
                .gap(px(tokens::SPACE_XS))
                .children(AppearanceMode::ALL.iter().map(|mode| {
                    let mode = *mode;
                    let selected = move || state.appearance.get() == mode;
                    view()
                        .on_press(move || choose(model, state, mode))
                        .test_id(&format!(
                            "settings-appearance-{}",
                            mode.label().to_lowercase().replace(' ', "-")
                        ))
                        .label(mode.label())
                        .flex_row()
                        .items_center()
                        .gap(px(tokens::SPACE_SM))
                        .px(px(tokens::SPACE_SM))
                        .py(px(tokens::SPACE_XS))
                        .rounded(px(tokens::RADIUS_SM))
                        .bg(move || {
                            if selected() {
                                palette.get().surface_3
                            } else {
                                transparent_black()
                            }
                        })
                        .hover(|s| s.bg(move || palette.get().row_hover))
                        .child(
                            view()
                                .w(px(tokens::SWITCH_H))
                                .h(px(tokens::SWITCH_H))
                                .rounded(px(tokens::RADIUS_FULL))
                                .bg(move || {
                                    if selected() {
                                        palette.get().brand_primary
                                    } else {
                                        palette.get().line
                                    }
                                }),
                        )
                        .child(
                            text(mode.label())
                                .text_sm()
                                .text_color(move || palette.get().ink),
                        )
                        .into_element()
                })),
        )
        .child(heading(state, "Motion"))
        .child(toggle_row(
            state,
            "settings-reduce-motion",
            "Reduce motion",
            "Suppress non-essential transitions.",
            reduce_motion,
            move |on| {
                model.with(|m| {
                    m.rt.library.lock_or_recover().ui_flags.reduce_motion = on;
                });
            },
        ))
        .child(
            text(match state.settings_page.get() {
                SettingsPage::Appearance => {
                    "High contrast is one of the theme choices above."
                }
                _ => "",
            })
            .text_xs()
            .text_color(move || palette.get().ink_3),
        )
        .into_element()
}

/// One appearance choice: the mode, the palette, and the persisted High
/// Contrast flag move together.
pub fn choose(model: Stored<UiModel>, state: UiState, mode: AppearanceMode) {
    let high_contrast = matches!(
        mode,
        AppearanceMode::DarkHighContrast | AppearanceMode::LightHighContrast
    );
    model.with(|m| {
        m.rt.library.lock_or_recover().ui_flags.high_contrast = high_contrast;
    });
    state.appearance.set(mode);
    state.palette.set(mode.palette());
}
