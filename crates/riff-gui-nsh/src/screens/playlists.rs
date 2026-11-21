//! The user playlist stage: the list and its detail in one surface — header
//! (name, count, play actions, rename and delete), the entry rows in playlist
//! order, and the missing entries listed with the egui row's `(missing)`
//! treatment. A drag on a row reorders the list, committed as one store
//! transaction; playback skips dangling entries.
//!
//! The detail arrives through [`UiState::playlist_detail`], rebuilt by the
//! pump when the playlist or Library generation moves, so a rename, a
//! removal or a reorder lands without a rescan.

use nsh_base::input;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::ViewMode;
use riff_backend::domain::{PlaylistId, TrackId};

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::empty_state;
use crate::components::track_row::{row_of_in, TrackRowView};
use crate::model::{reads, PlaylistDetail, PlaylistRowView};
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// The stage: header, rename/confirm bands, the rows.
pub fn stage(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_1()
        .flex_col()
        .test_id("playlist-stage")
        .bg(move || palette.get().surface_row)
        .child(dynamic(move || header(model, state)))
        .child(dynamic(move || rename_band(model, state)))
        .child(dynamic(move || delete_band(model, state)))
        .child(dynamic(move || body(model, state)))
}

/// The header: name and count on the left, the actions on the right.
fn header(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(detail) = state.playlist_detail.get() else {
        return view().into_element();
    };
    let count = detail.rows.len();
    let play_detail = detail.clone();
    let shuffle_detail = detail.clone();
    let rename_name = detail.name.clone();
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .px(px(tokens::SPACE_LG))
        .py(px(tokens::SPACE_MD))
        .test_id("playlist-header")
        .child(
            text(move || detail.name.clone())
                .text_xl()
                .truncate()
                .text_color(move || palette.get().ink)
                .test_id("playlist-name"),
        )
        .child(
            text(move || format!("{count} tracks"))
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("playlist-count"),
        )
        .child(view().flex_1())
        .child(header_action(
            palette,
            "playlist-play",
            "Play playlist",
            "Play",
            move || crate::menus::play_batch(model, valid_ids(&play_detail)),
        ))
        .child(header_action(
            palette,
            "playlist-shuffle",
            "Shuffle playlist",
            "Shuffle",
            move || crate::menus::shuffle_batch(model, valid_ids(&shuffle_detail)),
        ))
        .child(icon_action(
            palette,
            "playlist-rename",
            "Rename playlist",
            Icon::Pencil,
            move || {
                state.playlist_renaming.set(true);
                state.playlist_draft.set(rename_name.clone());
                state.playlist_delete_confirm.set(false);
                state.playlist_error.set(None);
            },
        ))
        .child(icon_action(
            palette,
            "playlist-delete",
            "Delete playlist",
            Icon::Trash,
            move || {
                state.playlist_delete_confirm.set(true);
                state.playlist_renaming.set(false);
            },
        ))
        .into_element()
}

fn header_action(
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

/// The rename band: a prefilled name field and Save/Cancel, with the store's
/// or the built-in rule's refusal under it.
fn rename_band(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    if !state.playlist_renaming.get() {
        return view().into_element();
    }
    let draft = state.playlist_draft;
    let write = draft;
    view()
        .flex_col()
        .gap(px(tokens::SPACE_XS))
        .px(px(tokens::SPACE_LG))
        .pb(px(tokens::SPACE_MD))
        .test_id("playlist-rename-band")
        .child(
            input()
                .value(draft)
                .on_change(move |text: &str| write.set(text.to_owned()))
                .placeholder("Playlist name")
                .test_id("playlist-rename-input")
                .label("Playlist name")
                .text_sm()
                .p(px(tokens::SPACE_SM))
                .rounded(px(tokens::RADIUS_SM))
                .bg(move || palette.get().surface_2)
                .text_color(move || palette.get().ink),
        )
        .child(
            view()
                .flex_row()
                .gap(px(tokens::SPACE_SM))
                .child(header_action(palette, "playlist-rename-save", "Save name", "Save", {
                    move || rename(model, state)
                }))
                .child(header_action(
                    palette,
                    "playlist-rename-cancel",
                    "Cancel",
                    "Cancel",
                    move || {
                        state.playlist_renaming.set(false);
                        state.playlist_draft.set(String::new());
                        state.playlist_error.set(None);
                    },
                )),
        )
        .child(error_line(state))
        .into_element()
}

/// The delete confirmation: nothing destructive until Confirm.
fn delete_band(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    if !state.playlist_delete_confirm.get() {
        return view().into_element();
    }
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .px(px(tokens::SPACE_LG))
        .pb(px(tokens::SPACE_MD))
        .test_id("playlist-delete-band")
        .child(
            text("Delete this playlist? Its tracks stay in your library.")
                .flex_1()
                .text_sm()
                .text_color(move || palette.get().ink_2),
        )
        .child(header_action(
            palette,
            "playlist-delete-confirm",
            "Delete playlist",
            "Delete",
            move || delete(model, state),
        ))
        .child(header_action(
            palette,
            "playlist-delete-cancel",
            "Keep playlist",
            "Cancel",
            move || state.playlist_delete_confirm.set(false),
        ))
        .into_element()
}

/// The rows: the detail's entries over a virtual list.
fn body(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(detail) = state.playlist_detail.get() else {
        return view()
            .flex_1()
            .p(px(tokens::SPACE_LG))
            .child(empty_state::state(
                palette,
                "Playlist not found",
                "It may have been deleted. Pick another list from the sidebar.",
            ))
            .into_element();
    };
    if detail.rows.is_empty() {
        return view()
            .flex_1()
            .p(px(tokens::SPACE_LG))
            .child(empty_state::state(
                palette,
                "This playlist is empty",
                "Right-click a track and choose Add to Playlist.",
            ))
            .into_element();
    }
    let drag_from = signal(None::<usize>);
    let playlist = detail.id.clone();
    let never_focus = signal(None::<usize>);
    view()
        .flex_1()
        .flex_col()
        .p(px(tokens::SPACE_LG))
        .child(
            virtual_list(
                move || {
                    state
                        .playlist_detail
                        .get()
                        .map_or(0, |detail| detail.rows.len())
                },
                move |at| {
                    let rows = state.playlist_detail;
                    let playlist = playlist.clone();
                    let facts = move || {
                        rows.get()
                            .and_then(|detail| detail.rows.get(at.index()).cloned())
                    };
                    row(model, state, facts, playlist, never_focus, drag_from, at.index())
                },
            )
            .estimated_item_size(px(tokens::ROW_H))
            .flex_1()
            .test_id("playlist-list"),
        )
        .into_element()
}

fn row(
    model: Stored<UiModel>,
    state: UiState,
    facts: impl Fn() -> Option<PlaylistRowView> + Clone + 'static,
    playlist: PlaylistId,
    never_focus: Signal<Option<usize>>,
    drag_from: Signal<Option<usize>>,
    index: usize,
) -> impl IntoElement {
    let palette = state.palette;
    let row_facts = facts.clone();
    let missing = move || {
        row_facts()
            .is_some_and(|row| row.track.is_none())
    };
    let start = drag_from;
    let end = drag_from;
    let end_playlist = playlist.clone();
    let missing_facts = facts.clone();
    let resolved_facts = facts;
    let end_len = move || {
        state
            .playlist_detail
            .get()
            .map_or(0, |detail| detail.rows.len())
    };
    view()
        .on_pan_start(move |_: &PanEvent| start.set(Some(index)))
        .on_pan_end(move |event: &PanEvent| {
            let Some(from) = end.get() else { return };
            end.set(None);
            #[allow(clippy::cast_possible_truncation)]
            let delta = (event.translation.y.0 / tokens::ROW_H).round() as isize;
            let len = end_len();
            if len == 0 {
                return;
            }
            let last = isize::try_from(len).unwrap_or(isize::MAX) - 1;
            let to = (isize::try_from(from).unwrap_or(0) + delta).clamp(0, last);
            let Ok(to) = usize::try_from(to) else { return };
            if to != from {
                reorder(model, end_playlist.clone(), from, to);
            }
        })
        .child(
            when(missing, move || {
                missing_row(palette, missing_facts.clone()).into_element()
            })
            .otherwise(move || {
                resolved_row(model, state, resolved_facts.clone(), playlist.clone(), never_focus)
                    .into_element()
            }),
        )
}

fn resolved_row(
    model: Stored<UiModel>,
    state: UiState,
    facts: impl Fn() -> Option<PlaylistRowView> + Clone + 'static,
    playlist: PlaylistId,
    never_focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let view_facts = move || {
        facts().map_or_else(
            || TrackRowView {
                index: 0,
                id: TrackId(String::new()),
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                duration: None,
                favorite: false,
                missing: true,
            },
            |row| {
                let track = row.track;
                track.map_or_else(
                    || TrackRowView {
                        index: 0,
                        id: row.id,
                        title: String::new(),
                        artist: String::new(),
                        album: String::new(),
                        duration: None,
                        favorite: false,
                        missing: true,
                    },
                    |track| TrackRowView {
                        index: 0,
                        id: track.id.clone(),
                        title: track.metadata.display_title(&track.file_path),
                        artist: track.metadata.display_artist(),
                        album: track.metadata.display_album().into_owned(),
                        duration: track.duration,
                        favorite: track.favorite,
                        missing: false,
                    },
                )
            },
        )
    };
    row_of_in(model, state, view_facts, never_focus, Some(playlist))
}

/// A dangling entry: its file path, struck through, with the `(missing)`
/// marker. No press action: there is nothing to play or select.
fn missing_row(
    palette: Signal<crate::theme::Palette>,
    facts: impl Fn() -> Option<PlaylistRowView> + Clone + 'static,
) -> impl IntoElement {
    view()
        .h(px(tokens::ROW_H))
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .px(px(tokens::SPACE_SM))
        .rounded(px(tokens::RADIUS_SM))
        .test_id("playlist-missing")
        .child(
            text(move || facts().map(|row| row.id.0).unwrap_or_default())
                .flex_1()
                .text_sm()
                .truncate()
                .line_through()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            text("(missing)")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
}

fn error_line(state: UiState) -> AnyElement {
    let Some(reason) = state.playlist_error.get() else {
        return view().into_element();
    };
    let palette = state.palette;
    text(move || reason.clone())
        .text_xs()
        .text_color(move || palette.get().error)
        .test_id("playlist-error")
        .into_element()
}

/// Rename: trim, the built-in rule, then the store's own decision.
fn rename(model: Stored<UiModel>, state: UiState) {
    let Some(detail) = state.playlist_detail.get() else {
        return;
    };
    let name = state.playlist_draft.get().trim().to_owned();
    if name.is_empty() {
        state
            .playlist_error
            .set(Some("Enter a playlist name".to_owned()));
        return;
    }
    if let Some(reason) = riff_backend::app::playlist_name::builtin_smart_list_name_conflict(&name)
    {
        state.playlist_error.set(Some(reason));
        return;
    }
    match model.with(|m| m.rt.playlists.rename_playlist(&detail.id, &name)) {
        Ok(true) => {
            state.playlist_renaming.set(false);
            state.playlist_draft.set(String::new());
            state.playlist_error.set(None);
        }
        Ok(false) => state
            .playlist_error
            .set(Some("That playlist no longer exists".to_owned())),
        Err(e) => state.playlist_error.set(Some(e.to_string())),
    }
}

/// Delete after confirmation: the row and its entries go in one transaction;
/// the stage returns to the Library, its slots cleared.
fn delete(model: Stored<UiModel>, state: UiState) {
    let Some(detail) = state.playlist_detail.get() else {
        return;
    };
    match model.with(|m| m.rt.playlists.delete_playlist(&detail.id)) {
        Ok(_) => {
            let nav = state.nav;
            let mut view = nav.get();
            view.playlist = None;
            nav.set(view);
            model.with(|m| {
                m.open_playlist = None;
                m.rt.library.lock_or_recover().view_mode = ViewMode::Library;
            });
            state.playlist_detail.set(None);
            state.playlist_delete_confirm.set(false);
            state.playlist_error.set(None);
        }
        Err(e) => state.playlist_error.set(Some(e.to_string())),
    }
}

/// Commit one drag: the current entry order with `from` moved to `to`, one
/// `reorder_playlist_entries` transaction. The order is read fresh from the
/// seam so a detail signal one tick behind never reorders stale ids.
pub fn reorder(model: Stored<UiModel>, playlist: PlaylistId, from: usize, to: usize) {
    let ids = model.with(|m| {
        reads::playlist_view(&mut m.rt.session_views, &playlist).map(|view| {
            view.rows
                .iter()
                .map(|(id, _, _)| id.clone())
                .collect::<Vec<TrackId>>()
        })
    });
    let Some(mut ids) = ids else { return };
    if from >= ids.len() || to >= ids.len() {
        return;
    }
    let moved = ids.remove(from);
    ids.insert(to, moved);
    model.with(|m| {
        if let Err(e) = m.rt.playlists.reorder_playlist_entries(&playlist, &ids) {
            tracing::warn!("Failed to reorder the playlist: {e}");
        }
    });
}

/// One playlist's valid ids, in order — the header's Play/Shuffle batch.
#[must_use]
pub fn valid_ids(detail: &PlaylistDetail) -> Vec<TrackId> {
    detail
        .rows
        .iter()
        .filter(|row| row.valid)
        .map(|row| row.id.clone())
        .collect()
}
