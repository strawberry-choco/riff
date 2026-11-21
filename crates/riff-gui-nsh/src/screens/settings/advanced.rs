//! The Advanced page: progressive disclosure, notifications, close-to-quit,
//! the thumbnail cache, and the library-wide `ReplayGain` pass.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::replaygain_pass::PassCommand;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::theme::tokens;

use super::{action_row, heading, status_line, toggle_row};

/// The page.
#[allow(clippy::too_many_lines)]
pub fn page(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let flags = move || {
        let _ = state.settings.get();
        model.with(|m| m.rt.library.lock_or_recover().ui_flags)
    };
    let pass_prefs = move || {
        let _ = state.settings.get();
        model.with(|m| m.rt.library.lock_or_recover().pass_prefs)
    };
    let progress = move || {
        let (done, total) = state.pass_progress.get();
        if total == 0 {
            String::new()
        } else {
            format!("{done}/{total}")
        }
    };
    let running = move || {
        let _ = state.pass_progress.get();
        model.with(|m| m.rt.passes.is_running())
    };
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .test_id("settings-advanced")
        .child(heading(state, "Interface"))
        .child(toggle_row(
            state,
            "settings-advanced-mode",
            "Advanced mode",
            "Show the power features, including Never Played and Lost Gems.",
            move || flags().advanced_mode,
            move |on| {
                model.with(|m| {
                    m.rt.library.lock_or_recover().ui_flags.advanced_mode = on;
                });
            },
        ))
        .child(toggle_row(
            state,
            "settings-notifications",
            "System notifications",
            "Raise an OS notification when background work finishes.",
            move || flags().system_notifications,
            move |on| {
                model.with(|m| {
                    m.rt.library.lock_or_recover().ui_flags.system_notifications = on;
                });
            },
        ))
        .child(toggle_row(
            state,
            "settings-close-quits",
            "Quit on close",
            "Closing the window quits riff instead of hiding it.",
            move || flags().close_quits_app,
            move |on| {
                model.with(|m| {
                    m.rt.library.lock_or_recover().ui_flags.close_quits_app = on;
                });
            },
        ))
        .child(heading(state, "Thumbnail cache"))
        .child(action_row(
            state,
            "settings-clear-thumbnails",
            "Clear thumbnail cache",
            move || model.with(|m| m.rt.covers.clear_cache()),
        ))
        .child(dynamic(move || {
            let Some(status) = state.cache_status.get() else {
                return view().into_element();
            };
            text(move || status.clone())
                .text_xs()
                .text_color(move || palette.get().ink_2)
                .test_id("settings-cache-status")
                .into_element()
        }))
        .child(heading(state, "ReplayGain pass"))
        .child(
            text("Measure the whole library and write the values you pick.")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(toggle_row(
            state,
            "settings-pass-track",
            "Track values",
            "Write each track's own measured pair.",
            move || pass_prefs().track_values,
            move |on| {
                model.with(|m| {
                    m.rt.library.lock_or_recover().pass_prefs.track_values = on;
                });
            },
        ))
        .child(toggle_row(
            state,
            "settings-pass-album",
            "Album values",
            "Write each album's aggregate to its tracks.",
            move || pass_prefs().album_values,
            move |on| {
                model.with(|m| {
                    m.rt.library.lock_or_recover().pass_prefs.album_values = on;
                });
            },
        ))
        .child(toggle_row(
            state,
            "settings-pass-force",
            "Re-measure already measured tracks",
            "Ignore the stored values and measure everything again.",
            move || state.pass_force.get(),
            move |on| state.pass_force.set(on),
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_MD))
                .child(action_row(
                    state,
                    "settings-pass-start",
                    "Start ReplayGain pass",
                    move || start_pass(model, state),
                ))
                .child(action_row(
                    state,
                    "settings-pass-cancel",
                    "Cancel pass",
                    move || model.with(|m| m.rt.passes.cancel()),
                ))
                .child(
                    text(progress)
                        .text_xs()
                        .text_color(move || palette.get().ink_3)
                        .test_id("settings-pass-progress"),
                )
                .children(running().then(|| {
                    text("Running")
                        .text_xs()
                        .text_color(move || palette.get().brand_primary)
                })),
        )
        .child(dynamic(move || {
            let Some(status) = state.pass_status.get() else {
                return view().into_element();
            };
            text(move || status.clone())
                .text_xs()
                .text_color(move || palette.get().ink_2)
                .test_id("settings-pass-status")
                .into_element()
        }))
        .child(status_line(state))
        .into_element()
}

fn start_pass(model: Stored<UiModel>, state: UiState) {
    let prefs = model.with(|m| m.rt.library.lock_or_recover().pass_prefs);
    if !prefs.track_values && !prefs.album_values {
        state
            .settings_status
            .set(Some("Pick Track values, Album values, or both".to_owned()));
        return;
    }
    model.with(|m| {
        m.rt.passes.submit(PassCommand::LibraryWide {
            track_values: prefs.track_values,
            album_values: prefs.album_values,
            force: state.pass_force.get(),
        });
    });
}
