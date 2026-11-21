//! The Up Next panel over the player bar: the egui `show_queue_panel`
//! behavior — a fixed-size list anchored above the bar, newest ordering by
//! the queue's own order, the current row marked, a click promoting a row to
//! play next, and a Clear control that stops and empties the queue.
//!
//! The rows read the `PlaybackView` signal (`up_next` is the projection's
//! window) and every action rides `rt.ui_transport`, so the panel owns no
//! state of its own beyond the open flag the player bar toggles.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::PlaybackQueue;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// How many rows the panel shows — the egui panel's cap.
const QUEUE_ROWS: usize = 50;

/// The panel: header, rows (virtual-free: the cap is 50), empty copy.
pub fn panel(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .h(px(tokens::QUEUE_PANEL_H))
        .w_full()
        .flex_row()
        .justify_end()
        .px(px(tokens::SPACE_LG))
        .child(
            view()
                .w(px(tokens::QUEUE_PANEL_W))
                .flex_col()
                .rounded(px(tokens::RADIUS_MD))
                .border(px(tokens::BORDER_HAIRLINE))
                .border_color(move || palette.get().border)
                .bg(move || palette.get().surface)
                .test_id("queue-panel")
                .child(header(model, state))
                .child(
                    scroll()
                        .flex_1()
                        .flex_col()
                        .test_id("queue-rows")
                        .child(dynamic(move || rows(model, state))),
                ),
        )
}

/// `Up Next` and Clear; Clear stops playback and empties the queue.
fn header(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .p(px(tokens::SPACE_MD))
        .test_id("queue-header")
        .child(
            text("Up Next")
                .flex_1()
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            view()
                .on_press(move || clear(model))
                .test_id("queue-clear")
                .label("Clear queue")
                .px(px(tokens::SPACE_SM))
                .py(px(tokens::SPACE_XS))
                .rounded(px(tokens::RADIUS_SM))
                .hover(|s| s.bg(move || palette.get().row_hover))
                .child(
                    text("Clear")
                        .text_xs()
                        .text_color(move || palette.get().ink_2),
                ),
        )
}

/// The rows, or the empty copy. Rebuilt when the playback view moves.
fn rows(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let up_next = state.playback.get().up_next;
    if up_next.is_empty() {
        return view()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(tokens::SPACE_XS))
            .py(px(tokens::SPACE_LG))
            .test_id("queue-empty")
            .child(
                text("Queue is empty")
                    .text_sm()
                    .text_color(move || palette.get().ink_2),
            )
            .child(
                text("Play a track to start your queue.")
                    .text_xs()
                    .text_color(move || palette.get().ink_3),
            )
            .into_element();
    }
    let current = state
        .playback
        .get()
        .current
        .map_or_else(String::new, |track| track.id.0.clone());
    view()
        .flex_col()
        .children(up_next.iter().take(QUEUE_ROWS).map(|track| {
            let id = track.id.clone();
            let is_current = id.0 == current;
            let title = track.metadata.display_title(&track.file_path);
            let artist = track.metadata.display_artist();
            let press_id = id.clone();
            view()
                .h(px(tokens::ROW_H))
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_SM))
                .px(px(tokens::SPACE_MD))
                .rounded(px(tokens::RADIUS_SM))
                .test_id("queue-row")
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
                        .text_sm()
                        .truncate()
                        .text_color(move || {
                            if is_current {
                                palette.get().brand_primary
                            } else {
                                palette.get().ink
                            }
                        }),
                )
                .child(
                    text(move || artist.clone())
                        .text_xs()
                        .truncate()
                        .text_color(move || palette.get().ink_3),
                )
                .children(is_current.then(|| {
                    icon(Icon::Play, Icon::ICON_SIZE, move || {
                        palette.get().brand_primary
                    })
                }))
                .into_element()
        }))
        .into_element()
}

/// Stop playback and empty the queue. The engine reads the shared session,
/// so the cleared queue is what the next continuation tick sees.
fn clear(model: Stored<UiModel>) {
    model.with(|m| {
        m.rt.ui_transport.stop();
        let mut session = m.rt.playback.lock_or_recover();
        session.queue = PlaybackQueue::new(Vec::new());
        session.playback_state = riff_backend::app::state::PlaybackState::Stopped;
    });
}
