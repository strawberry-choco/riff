//! The inline tag editor: the eleven editable values the egui
//! `ui/app/tag_editor.rs` edits — title, artist, album, album artist,
//! genre, year, track number and the four ReplayGain facts — with the album
//! aggregate's `(different)` and `(none)` display, a per-track save and an
//! album save over `rt.tag_edits`.
//!
//! The draft lives in the bridge signals; the form mounts once per edit
//! session (a `when` on the editor slot), so typing never rebuilds the
//! fields. Outcomes are polled on an interval and paired with the file each
//! request named, so a failure reads as `<path>: <reason>`.

use std::path::PathBuf;

use nsh_base::input;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest};
use riff_backend::app::traits::{ReplayGainTags, TagEdit};
use riff_backend::domain::{Track, TrackId};

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::model::{reads, TagDraft, TAG_FIELDS};
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// How often the mounted editor asks the tag-edit service for outcomes.
const POLL: std::time::Duration = std::time::Duration::from_millis(150);

/// Open the editor for `id`: select the track and raise the editor signal.
pub fn begin(model: Stored<UiModel>, id: TrackId) {
    let state = model.with(|m| m.signals());
    model.with(|m| {
        m.rt.library.lock_or_recover().selected_track = Some(id.clone());
    });
    let draft = model.with(|m| {
        reads::selected_track(&mut m.rt.session_views, &id).map_or_else(TagDraft::default, |track| {
            TagDraft::of_track(&track)
        })
    });
    state.selection.set(Some(id.clone()));
    state.tag_edit.set(Some(id));
    state.tag_album_mode.set(false);
    state.tag_draft.set(draft);
    state.tag_status.set(None);
    state.tag_pending.set(Vec::new());
    state.tag_saved.set(0);
    state.tag_failures.set(Vec::new());
    state.tag_saving.set(false);
}

/// Close the editor without saving.
pub fn cancel(model: Stored<UiModel>) {
    model.with(|m| m.state.tag_edit).set(None);
}

/// The mounted editor's poller: pairs each finished outcome with the file
/// its request named and settles the status when none are left. Installed
/// once by the inspector's mount; idle ticks cost one signal read.
pub fn install(model: Stored<UiModel>, state: UiState) {
    set_interval(POLL, move || {
        if !state.tag_saving.get() {
            return;
        }
        let mut pending = state.tag_pending.get();
        let mut saved = state.tag_saved.get();
        let mut failures = state.tag_failures.get();
        let mut moved = false;
        while let Some(outcome) = model.with(|m| m.rt.tag_edits.poll()) {
            let path = if pending.is_empty() {
                None
            } else {
                Some(pending.remove(0))
            };
            match outcome {
                TagEditOutcome::Saved => saved += 1,
                TagEditOutcome::Failed { reason } => {
                    failures.push(format!("{}: {reason}", path_label(path.as_ref())));
                }
            }
            moved = true;
        }
        if !moved {
            return;
        }
        state.tag_pending.set(pending.clone());
        state.tag_saved.set(saved);
        state.tag_failures.set(failures.clone());
        if pending.is_empty() {
            state.tag_saving.set(false);
            state
                .tag_status
                .set(Some(status_line(saved, &failures)));
            if failures.is_empty() {
                state.tag_edit.set(None);
            }
        }
    });
}

fn path_label(path: Option<&PathBuf>) -> String {
    path.map_or_else(|| "Unknown file".to_owned(), |path| path.display().to_string())
}

fn status_line(saved: usize, failures: &[String]) -> String {
    if failures.is_empty() {
        return match saved {
            1 => "Saved".to_owned(),
            n => format!("Saved {n} tracks"),
        };
    }
    format!(
        "Saved {saved}, {} failed: {}",
        failures.len(),
        failures.join("; ")
    )
}

/// The form: the eleven fields, the album toggle, Save track, Save album,
/// Cancel, and the status line. Mounted once per edit session.
pub fn form(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let draft = state.tag_draft.get();
    let album_mode = state.tag_album_mode.get();
    let mut fields = view().flex_col().gap(px(tokens::SPACE_SM));
    for (index, label) in TAG_FIELDS.iter().enumerate() {
        let value = signal(draft.field(index).to_owned());
        let write = value;
        let draft_signal = state.tag_draft;
        let placeholder = draft.placeholder(index);
        fields = fields.child(
            view()
                .flex_col()
                .gap(px(tokens::SPACE_XS))
                .child(
                    text(*label)
                        .text_xs()
                        .text_color(move || palette.get().ink_3),
                )
                .child(
                    input()
                        .value(value)
                        .on_change(move |text: &str| {
                            write.set(text.to_owned());
                            draft_signal.update(|draft| draft.set_field(index, text.to_owned()));
                        })
                        .placeholder(placeholder)
                        .test_id(&format!("tag-field-{index}"))
                        .label(*label)
                        .text_sm()
                        .p(px(tokens::SPACE_SM))
                        .rounded(px(tokens::RADIUS_SM))
                        .bg(move || palette.get().surface_2)
                        .text_color(move || palette.get().ink),
                ),
        );
    }
    view()
        .flex_col()
        .gap(px(tokens::SPACE_LG))
        .test_id("tag-editor")
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_SM))
                .child(
                    text("EDIT TAGS")
                        .flex_1()
                        .text_xs()
                        .text_color(move || palette.get().ink_3),
                )
                .child(
                    view()
                        .on_press(move || toggle_album(model, state))
                        .test_id("tag-album-toggle")
                        .label("Edit album tags")
                        .px(px(tokens::SPACE_SM))
                        .py(px(tokens::SPACE_XS))
                        .rounded(px(tokens::RADIUS_SM))
                        .bg(move || {
                            if album_mode {
                                palette.get().surface_3
                            } else {
                                transparent_black()
                            }
                        })
                        .hover(|s| s.bg(move || palette.get().row_hover))
                        .child(
                            text("Album")
                                .text_xs()
                                .text_color(move || palette.get().ink_2),
                        ),
                )
                .child(
                    view()
                        .on_press(move || cancel(model))
                        .test_id("tag-cancel")
                        .label("Cancel tag edit")
                        .p(px(tokens::SPACE_SM))
                        .rounded(px(tokens::RADIUS_FULL))
                        .hover(|s| s.bg(move || palette.get().row_hover))
                        .child(icon(Icon::Close, Icon::ICON_SIZE, move || {
                            palette.get().ink_2
                        })),
                ),
        )
        .child(fields)
        .child(
            view()
                .flex_row()
                .gap(px(tokens::SPACE_SM))
                .child(
                    view()
                        .on_press(move || save_track(model, state))
                        .test_id("tag-save-track")
                        .label("Save tags for this track")
                        .px(px(tokens::SPACE_MD))
                        .py(px(tokens::SPACE_XS))
                        .rounded(px(tokens::RADIUS_SM))
                        .bg(move || palette.get().brand_primary)
                        .child(
                            text("Save track")
                                .text_xs()
                                .text_color(move || palette.get().on_brand),
                        ),
                )
                .child(
                    view()
                        .on_press(move || save_album(model, state))
                        .test_id("tag-save-album")
                        .label("Save tags for the whole album")
                        .px(px(tokens::SPACE_MD))
                        .py(px(tokens::SPACE_XS))
                        .rounded(px(tokens::RADIUS_SM))
                        .border(px(tokens::BORDER_HAIRLINE))
                        .border_color(move || palette.get().border)
                        .hover(|s| s.bg(move || palette.get().row_hover))
                        .child(
                            text("Save album")
                                .text_xs()
                                .text_color(move || palette.get().ink_2),
                        ),
                ),
        )
        .child(dynamic(move || {
            let Some(status) = state.tag_status.get() else {
                return view().into_element();
            };
            text(move || status.clone())
                .text_xs()
                .text_color(move || palette.get().ink_2)
                .test_id("tag-status")
                .into_element()
        }))
}

/// Toggle the album aggregate: fill the draft with the album's common
/// values and mark the fields its tracks disagree on, or return to the
/// selected track's own values.
fn toggle_album(model: Stored<UiModel>, state: UiState) {
    let Some(id) = state.tag_edit.get() else {
        return;
    };
    let next = !state.tag_album_mode.get();
    let draft = model.with(|m| {
        let Some(track) = reads::selected_track(&mut m.rt.session_views, &id) else {
            return TagDraft::default();
        };
        if !next {
            return TagDraft::of_track(&track);
        }
        let tracks = album_tracks(m, &track);
        if tracks.is_empty() {
            TagDraft::of_track(&track)
        } else {
            TagDraft::of_album(&tracks)
        }
    });
    state.tag_album_mode.set(next);
    state.tag_draft.set(draft);
    state.tag_status.set(None);
}

/// The selected track's album's tracks, in canonical order. Empty when the
/// tags name no album, which keeps the album save a track save.
fn album_tracks(model: &mut UiModel, track: &Track) -> Vec<Track> {
    let Some(album) = track.metadata.album.clone() else {
        return Vec::new();
    };
    let artist = track
        .metadata
        .album_artist
        .clone()
        .or_else(|| track.metadata.artist.clone())
        .unwrap_or_default();
    reads::album_tracks(&mut model.rt.session_views, &artist, &album).to_vec()
}

/// Save one track: the draft's non-empty fields and ReplayGain values.
fn save_track(model: Stored<UiModel>, state: UiState) {
    let Some(id) = state.tag_edit.get() else {
        return;
    };
    let draft = state.tag_draft.get();
    let track = model.with(|m| reads::selected_track(&mut m.rt.session_views, &id));
    let Some(track) = track else {
        state
            .tag_status
            .set(Some("Track is no longer in the library".to_owned()));
        return;
    };
    if !draft.differs_from(&track) {
        state.tag_status.set(Some("No changes".to_owned()));
        return;
    }
    let edit = draft.edit();
    let replaygain = draft.replaygain();
    if edit.is_empty() && replaygain.is_empty() {
        state
            .tag_status
            .set(Some("Enter a value to save".to_owned()));
        return;
    }
    submit(
        model,
        state,
        vec![(track.id.clone(), track.file_path.clone(), edit, replaygain)],
    );
}

/// Save the album: one request per track whose values the draft changes.
fn save_album(model: Stored<UiModel>, state: UiState) {
    let Some(id) = state.tag_edit.get() else {
        return;
    };
    let draft = state.tag_draft.get();
    let tracks = model.with(|m| {
        reads::selected_track(&mut m.rt.session_views, &id)
            .map_or_else(Vec::new, |track| album_tracks(m, &track))
    });
    if tracks.len() < 2 {
        save_track(model, state);
        return;
    }
    let edit = draft.edit();
    let replaygain = draft.replaygain();
    let requests: Vec<(TrackId, PathBuf, TagEdit, ReplayGainTags)> = tracks
        .iter()
        .filter(|track| draft.differs_from(track))
        .map(|track| {
            (
                track.id.clone(),
                track.file_path.clone(),
                edit.clone(),
                replaygain,
            )
        })
        .collect();
    if requests.is_empty() {
        state.tag_status.set(Some("No changes".to_owned()));
        return;
    }
    submit(model, state, requests);
}

fn submit(
    model: Stored<UiModel>,
    state: UiState,
    requests: Vec<(TrackId, PathBuf, TagEdit, ReplayGainTags)>,
) {
    let pending: Vec<PathBuf> = requests.iter().map(|(_, path, _, _)| path.clone()).collect();
    let count = requests.len();
    model.with(|m| {
        for (id, path, edit, replaygain) in requests {
            m.rt.tag_edits.submit(TagEditRequest {
                track_id: id,
                path,
                edit,
                replaygain,
            });
        }
    });
    state.tag_pending.set(pending);
    state.tag_saved.set(0);
    state.tag_failures.set(Vec::new());
    state.tag_saving.set(true);
    state.tag_status.set(Some(format!("Saving {count}...")));
}
