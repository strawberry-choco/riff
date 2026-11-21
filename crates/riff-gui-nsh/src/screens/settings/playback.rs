//! The Playback page: the ReplayGain switch and its Track/Album mode.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::ReplayGainMode;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::theme::tokens;

use super::{choice_row, heading, toggle_row};

/// The page.
pub fn page(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let enabled = move || {
        let _ = state.settings.get();
        model.with(|m| m.rt.playback.lock_or_recover().replaygain_enabled)
    };
    let mode = move || {
        let _ = state.settings.get();
        model.with(|m| m.rt.playback.lock_or_recover().replaygain_mode)
    };
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .test_id("settings-playback")
        .child(heading(state, "ReplayGain"))
        .child(toggle_row(
            state,
            "settings-replaygain",
            "ReplayGain",
            "Level tracks by their measured loudness.",
            enabled,
            move |on| {
                model.with(|m| {
                    m.rt.playback.lock_or_recover().replaygain_enabled = on;
                });
            },
        ))
        .child(choice_row(
            state,
            "settings-replaygain-mode",
            "ReplayGain mode",
            [
                (
                    "Track",
                    mode() == ReplayGainMode::Track,
                    Box::new({
                        move || {
                            model.with(|m| {
                                m.rt.playback.lock_or_recover().replaygain_mode =
                                    ReplayGainMode::Track;
                            });
                        }
                    }) as Box<dyn Fn()>,
                ),
                (
                    "Album",
                    mode() == ReplayGainMode::Album,
                    Box::new(move || {
                        model.with(|m| {
                            m.rt.playback.lock_or_recover().replaygain_mode =
                                ReplayGainMode::Album;
                        });
                    }),
                ),
            ],
        ))
        .into_element()
}
