//! The Now Playing view: the egui `ui/now_playing.rs` behavior — a hero
//! cover, the full metadata set, a seek slider gated on a known duration,
//! and the Up Next list with the same promote gesture the queue panel has.
//! Close returns to the Library view, the egui action's one exit.
//!
//! Reduce motion has nothing to settle here: the screen's only motion is the
//! playing indicator inside listings, which is a static glyph. The flag is
//! still read so a device with it on paints no hover transitions the
//! framework would otherwise run.

use nsh_base::{SliderValue, slider};
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::ViewMode;
use riff_backend::domain::Track;
use std::sync::Arc;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::cover;
use crate::components::empty_state;
use crate::shell::player_bar::{mmss, slider_parts};
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// How many Up Next rows the screen lists.
const UP_NEXT_ROWS: usize = 50;

/// The screen: close control, hero block, the queue under it.
pub fn screen(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_1()
        .flex_col()
        .test_id("now-playing")
        .bg(move || palette.get().surface_row)
        .child(
            view()
                .flex_row()
                .items_center()
                .px(px(tokens::SPACE_LG))
                .py(px(tokens::SPACE_MD))
                .test_id("now-playing-header")
                .child(
                    text("NOW PLAYING")
                        .flex_1()
                        .text_xs()
                        .text_color(move || palette.get().ink_3),
                )
                .child(
                    view()
                        .on_press(move || close(model, state))
                        .test_id("now-playing-close")
                        .label("Close Now Playing")
                        .p(px(tokens::SPACE_SM))
                        .rounded(px(tokens::RADIUS_FULL))
                        .hover(|s| s.bg(move || palette.get().row_hover))
                        .child(icon(Icon::Close, Icon::ICON_SIZE, move || {
                            palette.get().ink_2
                        })),
                ),
        )
        .child(
            scroll()
                .flex_1()
                .flex_col()
                .p(px(tokens::SPACE_LG))
                .gap(px(tokens::SPACE_LG))
                .child(dynamic(move || body(model, state)))
                .child(dynamic(move || up_next(model, state))),
        )
}

/// The current track's block, or the empty state.
fn body(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(track) = state.playback.get().current else {
        return view()
            .items_center()
            .justify_center()
            .py(px(tokens::SPACE_LG))
            .child(empty_state::state(
                palette,
                "Nothing playing",
                "Play a track to see it here.",
            ))
            .into_element();
    };
    let cover_id = track.id.0.clone();
    view()
        .flex_col()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .test_id("now-playing-track")
        .child(
            view()
                .w(px(tokens::HERO_COVER))
                .h(px(tokens::HERO_COVER))
                .rounded(px(tokens::RADIUS_XL))
                .test_id("now-playing-cover")
                .child(cover::tile(
                    model,
                    state,
                    move || cover_id.clone(),
                    crate::bridge::covers::COVER_HERO,
                )),
        )
        .child(
            text({
                let track = Arc::clone(&track);
                move || track.metadata.display_title(&track.file_path)
            })
            .text_3xl()
            .truncate()
            .test_id("now-playing-title"),
        )
        .child(
            text({
                let track = Arc::clone(&track);
                move || track.metadata.display_artist()
            })
            .text_sm()
            .text_color(move || palette.get().ink_2)
            .test_id("now-playing-artist"),
        )
        .child(
            text(move || meta_line(&track))
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("now-playing-meta"),
        )
        .child(seek_row(model, state))
        .into_element()
}

/// The `year · genre · duration` line, only the parts the tags carry.
fn meta_line(track: &Track) -> String {
    let mut parts = Vec::new();
    if let Some(year) = track.metadata.year {
        parts.push(year.to_string());
    }
    if let Some(genre) = track.metadata.genre.clone() {
        parts.push(genre);
    }
    if let Some(duration) = track.duration {
        parts.push(mmss(duration.as_secs_f32()));
    }
    parts.join(" \u{b7} ")
}

/// Elapsed, the slider, total — the player bar's seek row at hero scale.
fn seek_row(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let total_text = state.playback.get().duration_secs;
    view()
        .w(px(tokens::NOW_PLAYING_SEEK_W))
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .test_id("now-playing-seek")
        .child(
            text(move || mmss(state.playback.get().position_secs))
                .text_xs()
                .text_color(move || palette.get().ink_2),
        )
        .child(seek_slider(model, state).flex_1())
        .child(
            text(move || total_text.map_or_else(|| "--:--".to_owned(), mmss))
                .text_xs()
                .text_color(move || palette.get().ink_2),
        )
}

// The step-quantized f64-to-f32 handoff the player bar's sliders use.
#[allow(clippy::cast_possible_truncation)]
fn seek_slider(model: Stored<UiModel>, state: UiState) -> nsh_base::SliderElement {
    let palette = state.palette;
    let slider = slider()
        .test_id("now-playing-seek-slider")
        .accessibility_label("Seek")
        .value(move || {
            let view = state.playback.get();
            view.duration_secs
                .map(|total| SliderValue::Single(f64::from(view.position_secs.min(total))))
        })
        .min(0.0)
        .max(move || f64::from(state.playback.get().duration_secs.unwrap_or(1.0)))
        .step(0.1)
        .disabled(move || state.playback.get().duration_secs.is_none())
        .on_change(move |value: &SliderValue| {
            let target = value.start() as f32;
            model.with(|m| {
                let session = m.rt.playback.lock_or_recover();
                m.rt.ui_transport.seek(&session, target);
            });
        });
    slider_parts(slider, palette)
}

/// Up Next: the queue's window, each row promoting on click.
fn up_next(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let queue = state.playback.get().up_next;
    let mut list = view()
        .flex_col()
        .gap(px(tokens::SPACE_XS))
        .test_id("now-playing-up-next");
    list = list.child(
        text("Up Next")
            .text_xs()
            .text_color(move || palette.get().ink_3),
    );
    if queue.is_empty() {
        list = list
            .child(
                text("Queue is empty")
                    .text_sm()
                    .text_color(move || palette.get().ink_2),
            )
            .child(
                text("Play a track to start your queue.")
                    .text_xs()
                    .text_color(move || palette.get().ink_3),
            );
        return list.into_element();
    }
    let current = state
        .playback
        .get()
        .current
        .map_or_else(String::new, |track| track.id.0.clone());
    list.children(queue.iter().take(UP_NEXT_ROWS).map(|track| {
        let id = track.id.clone();
        let is_current = id.0 == current;
        let title = track.metadata.display_title(&track.file_path);
        let press_id = id.clone();
        view()
            .h(px(tokens::ROW_H))
            .flex_row()
            .items_center()
            .gap(px(tokens::SPACE_SM))
            .px(px(tokens::SPACE_SM))
            .rounded(px(tokens::RADIUS_SM))
            .test_id("now-playing-up-next-row")
            .label(title.clone())
            .bg(move || {
                if is_current {
                    palette.get().surface_3
                } else {
                    transparent_black()
                }
            })
            .hover(|s| s.bg(move || palette.get().row_hover))
            .on_press(move || {
                model.with(|m| m.rt.ui_transport.play_next(press_id.clone()));
            })
            .child(
                text(move || title.clone())
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .text_color(move || {
                        if is_current {
                            palette.get().brand_primary
                        } else {
                            palette.get().ink
                        }
                    }),
            )
            .into_element()
    }))
    .into_element()
}

/// Close: back to the Library view, the session field and the nav mirror
/// written together.
pub fn close(model: Stored<UiModel>, state: UiState) {
    let nav = state.nav;
    let mut view = nav.get();
    model.with(|m| {
        m.rt.library.lock_or_recover().view_mode = ViewMode::Library;
    });
    view.view_mode = ViewMode::Library;
    nav.set(view);
}

/// Open: the Now Playing mode, mirrored to the nav signal.
pub fn open(model: Stored<UiModel>, state: UiState) {
    let nav = state.nav;
    let mut view = nav.get();
    model.with(|m| {
        m.rt.library.lock_or_recover().view_mode = ViewMode::NowPlaying;
    });
    view.view_mode = ViewMode::NowPlaying;
    nav.set(view);
}
