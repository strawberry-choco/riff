//! The SMART LISTS stage: the built-in lists' read-only track listings, the
//! saved custom lists' listings, and the custom rule editor. The header of
//! every listing carries its name, its count and the whole-list Play and
//! Shuffle actions; a custom list adds Edit and Delete.
//!
//! Tracks arrive through the bridge signals — the built-in list's detail is
//! rebuilt by the pump, a custom list's rows are read through the seam's
//! bounded window by its virtual list.

use nsh_core::prelude::*;
use riff_backend::app::state::TrackSort;
use riff_backend::domain::{Track, TrackId};
use std::sync::Arc;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::track_row::{TrackRowView, row_of};
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

pub mod date_picker;
pub mod editor;
pub mod rules;

/// The stage: the editor when it is open, the opened custom list, or the
/// opened built-in list.
pub fn stage(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    when(
        move || state.smart_editing.get(),
        move || editor::screen(model, state).into_element(),
    )
    .otherwise(move || {
        if state.nav.get().custom_list.is_some() {
            custom_list_stage(model, state).into_element()
        } else {
            builtin_stage(model, state).into_element()
        }
    })
}

/// The built-in list: header over the shared track listing.
fn builtin_stage(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let listing = {
        let never_focus = signal(None::<usize>);
        view()
            .flex_1()
            .flex_col()
            .test_id("smart-list-stage")
            .child(dynamic(move || builtin_header(model, state)))
            .child(
                virtual_list(
                    move || {
                        state
                            .smart_list_detail
                            .get()
                            .map_or(0, |detail| detail.tracks.len())
                    },
                    move |at| {
                        let detail = state.smart_list_detail;
                        let facts = move || {
                            detail
                                .get()
                                .and_then(|detail| detail.tracks.get(at.index()).cloned())
                                .map_or_else(TrackRowView::empty, row_view)
                        };
                        row_of(model, state, facts, never_focus)
                    },
                )
                .estimated_item_size(px(tokens::ROW_H))
                .flex_1()
                .test_id("smart-list-list"),
            )
    };
    crate::screens::library::browse::stage_shell(model, state, listing)
}

/// The built-in header: name, count, read-only note, Play and Shuffle.
fn builtin_header(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(detail) = state.smart_list_detail.get() else {
        return view().into_element();
    };
    let name = detail.name.clone();
    let count = detail.tracks.len();
    let play = Arc::clone(&detail.tracks);
    let shuffle = Arc::clone(&detail.tracks);
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .pb(px(tokens::SPACE_MD))
        .test_id("smart-list-header")
        .child(
            text(move || name.clone())
                .text_xl()
                .truncate()
                .text_color(move || palette.get().ink)
                .test_id("smart-list-name"),
        )
        .child(
            text(move || format!("{count} tracks, read-only"))
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("smart-list-count"),
        )
        .child(view().flex_1())
        .child(action(
            palette,
            "smart-list-play",
            "Play smart list",
            "Play",
            move || crate::menus::play_batch(model, ids_of(&play)),
        ))
        .child(action(
            palette,
            "smart-list-shuffle",
            "Shuffle smart list",
            "Shuffle",
            move || crate::menus::shuffle_batch(model, ids_of(&shuffle)),
        ))
        .into_element()
}

/// The saved custom list: header with Edit/Delete, the delete confirmation,
/// and its virtual listing.
fn custom_list_stage(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let listing = view()
        .flex_1()
        .flex_col()
        .test_id("custom-list-stage")
        .child(dynamic(move || custom_header(model, state)))
        .child(dynamic(move || delete_band(model, state)))
        .child(
            virtual_list(
                move || state.custom_list_total.get(),
                move |at| {
                    let id = state.nav.get().custom_list;
                    let facts = move || {
                        let Some(id) = id.clone() else {
                            return TrackRowView::empty();
                        };
                        model
                            .with(|m| {
                                m.rt.session_views
                                    .custom_list_row(&id, TrackSort::NumberAsc, None, at.index())
                                    .ok()
                                    .flatten()
                            })
                            .map_or_else(TrackRowView::empty, |track| row_view((*track).clone()))
                    };
                    row_of(model, state, facts, signal(None::<usize>))
                },
            )
            .estimated_item_size(px(tokens::ROW_H))
            .flex_1()
            .test_id("custom-list-list"),
        )
        .bg(move || palette.get().surface_row);
    view()
        .flex_1()
        .flex_row()
        .child(
            view()
                .flex_1()
                .flex_col()
                .p(px(tokens::SPACE_LG))
                .child(listing),
        )
        .child(crate::screens::inspector::panel(model, state))
}

fn custom_header(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(id) = state.nav.get().custom_list else {
        return view().into_element();
    };
    let name = state
        .custom_lists
        .get()
        .iter()
        .find(|row| row.id == id)
        .map_or_else(|| id.0.clone(), |row| row.name.clone());
    let count = state.custom_list_total.get();
    let edit_id = id.clone();
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .pb(px(tokens::SPACE_MD))
        .test_id("custom-list-header")
        .child(
            text(move || name.clone())
                .text_xl()
                .truncate()
                .text_color(move || palette.get().ink)
                .test_id("custom-list-name"),
        )
        .child(
            text(move || format!("{count} tracks"))
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("custom-list-count"),
        )
        .child(view().flex_1())
        .child(action(palette, "custom-list-play", "Play list", "Play", {
            let id = id.clone();
            move || {
                let ids = model.with(|m| editor::open_list_ids(m, &id));
                crate::menus::play_batch(model, ids);
            }
        }))
        .child(action(
            palette,
            "custom-list-shuffle",
            "Shuffle list",
            "Shuffle",
            {
                let id = id.clone();
                move || {
                    let ids = model.with(|m| editor::open_list_ids(m, &id));
                    crate::menus::shuffle_batch(model, ids);
                }
            },
        ))
        .child(icon_action(
            palette,
            "custom-list-edit",
            "Edit rules",
            Icon::Pencil,
            move || editor::edit_list(model, state, edit_id.clone()),
        ))
        .child(icon_action(
            palette,
            "custom-list-delete",
            "Delete list",
            Icon::Trash,
            move || state.smart_delete_confirm.set(true),
        ))
        .into_element()
}

fn delete_band(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    if !state.smart_delete_confirm.get() {
        return view().into_element();
    }
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .pb(px(tokens::SPACE_MD))
        .test_id("custom-list-delete-band")
        .child(
            text("Delete this smart list? Your library is untouched.")
                .flex_1()
                .text_sm()
                .text_color(move || palette.get().ink_2),
        )
        .child(action(
            palette,
            "custom-list-delete-confirm",
            "Delete list",
            "Delete",
            move || editor::delete_open(model, state),
        ))
        .child(action(
            palette,
            "custom-list-delete-cancel",
            "Keep list",
            "Cancel",
            move || state.smart_delete_confirm.set(false),
        ))
        .into_element()
}

fn action(
    palette: Signal<crate::theme::Palette>,
    id: &'static str,
    label: &'static str,
    text_label: &'static str,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .on_press(on_press)
        .test_id(id)
        .label(label)
        .px(px(tokens::SPACE_SM))
        .py(px(tokens::SPACE_XS))
        .rounded(px(tokens::RADIUS_SM))
        .hover(|s| s.bg(move || palette.get().row_hover))
        .child(
            text(text_label)
                .text_xs()
                .text_color(move || palette.get().ink_2),
        )
}

fn icon_action(
    palette: Signal<crate::theme::Palette>,
    id: &'static str,
    label: &'static str,
    glyph: Icon,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .on_press(on_press)
        .test_id(id)
        .label(label)
        .p(px(tokens::SPACE_SM))
        .rounded(px(tokens::RADIUS_FULL))
        .hover(|s| s.bg(move || palette.get().row_hover))
        .child(icon(glyph, Icon::ICON_SIZE, move || palette.get().ink_2))
}

fn ids_of(tracks: &[Track]) -> Vec<TrackId> {
    tracks.iter().map(|track| track.id.clone()).collect()
}

/// One store row as the display view the shared row renders.
fn row_view(track: Track) -> TrackRowView {
    TrackRowView {
        index: 0,
        id: track.id.clone(),
        title: track.metadata.display_title(&track.file_path),
        artist: track.metadata.display_artist(),
        album: track.metadata.display_album().into_owned(),
        duration: track.duration,
        favorite: track.favorite,
        missing: false,
    }
}
