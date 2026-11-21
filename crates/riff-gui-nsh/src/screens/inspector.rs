//! The inspector: the stage's rightmost column, the readout of the selected
//! track — the egui crate's `ui/selection.rs` + `ui/app/selection_pane.rs`
//! ported at this unit's scope. It follows the selection signal: cover (the
//! card box), title, artist subtitle, the details list (plays, last played,
//! path), and the tag section as a read-only placeholder (the full
//! editable section is the tag editor's own unit). With nothing selected it
//! shows the empty state, the egui panel's `Nothing selected` copy.
//!
//! A READOUT, not a control surface: it reports nothing, dispatches
//! nothing. Its one outward effect is the cover request, which rides an
//! effect on the selection — a handler context, not a render binding, so
//! the cache's sets stay out of the render path.

use nsh_core::prelude::*;
use riff_backend::domain::Track;
use std::sync::Arc;
use std::time::SystemTime;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::cover;
use crate::components::empty_state;
use crate::model::reads;
use crate::theme::tokens;

/// The panel: the column and its content. The egui column renders only
/// while a selection exists; here it stays mounted and shows the empty
/// state instead (the acceptance's no-selection surface).
pub fn panel(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    request_cover_on_selection(model, state);
    view()
        .w(px(tokens::INSPECTOR_WIDTH))
        .flex_col()
        .border_l(px(tokens::BORDER_HAIRLINE))
        .border_color(move || palette.get().line)
        .bg(move || palette.get().surface)
        .test_id("inspector")
        .child(header(state))
        .child(
            scroll()
                .flex_1()
                .flex_col()
                .p(px(tokens::SPACE_LG))
                .gap(px(tokens::SPACE_LG))
                .test_id("inspector-body")
                .child(dynamic(move || {
                    let Some(track) = selected_track(model, state) else {
                        return nothing_selected(palette).into_element();
                    };
                    readout(model, state, track).into_element()
                })),
        )
}

/// The `SELECTION` header, with the readout kind's chip at the right edge
/// (the egui header's shape).
fn header(state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_row()
        .items_center()
        .p(px(tokens::SPACE_LG))
        .test_id("inspector-header")
        .child(
            text("SELECTION")
                .text_xs()
                .flex_1()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            text(move || selected_word(state.selection.get().is_some()))
                .text_xs()
                .text_color(move || palette.get().ink_2),
        )
}

/// The chip: `Track` while a selection stands (the egui panel's kind chip;
/// the entity readouts join at N7), nothing when the panel is empty.
fn selected_word(selected: bool) -> &'static str {
    if selected { "Track" } else { "" }
}

/// The no-selection state: the egui panel's copy, through the shared empty
/// state.
fn nothing_selected(palette: Signal<crate::theme::Palette>) -> impl IntoElement {
    empty_state::state(
        palette,
        "Nothing selected",
        "Select a track in the browser to see it here.",
    )
}

/// The selected track, read through the seam at dispatch time — a
/// `dynamic` block runs on the engine thread (its reads ride the model's
/// stored borrow), and a stale id (the store dropped the track) reads as
/// the empty state, the egui resolve's fall-through. The signal reads ride
/// the copied handles OUTSIDE the borrow: a read inside `with` subscribes
/// nothing (the dynamic would never re-run on a moved selection).
fn selected_track(model: Stored<UiModel>, state: UiState) -> Option<Arc<Track>> {
    let selection = state.selection.get();
    let _ = state.list_revision.get();
    let _ = state.library_revision.get();
    model.with(|m| {
        selection.and_then(|id| reads::selected_track(&mut m.rt.session_views, &id).map(Arc::new))
    })
}

/// The readout: art, title, subtitle, the tag section placeholder, the
/// details list (the egui `track_inspector`'s body, in its boundary order).
fn readout(model: Stored<UiModel>, state: UiState, track: Arc<Track>) -> impl IntoElement {
    let palette = state.palette;
    let track_id = track.id.clone();
    let title_track = Arc::clone(&track);
    let subtitle_track = Arc::clone(&track);
    view()
        .flex_col()
        .gap(px(tokens::SPACE_LG))
        .child(
            view()
                .w_full()
                .h(px(tokens::INSPECTOR_ART_H))
                .test_id("inspector-cover")
                .child(cover::tile(
                    model,
                    state,
                    move || track_id.0.clone(),
                    crate::bridge::covers::COVER_CARD,
                )),
        )
        .child(
            text(move || title_track.metadata.display_title(&title_track.file_path))
                .text_xl()
                .test_id("inspector-title"),
        )
        .child(
            text(move || subtitle_track.metadata.display_artist())
                .text_sm()
                .text_color(move || palette.get().ink_2)
                .test_id("inspector-subtitle"),
        )
        .child(tag_section(state, &track))
        .child(details(state, &track))
}

/// The tag section, read-only placeholder: the metadata fields' values with
/// the egui panel's `(none)` for what the track does not carry. The editable
/// eleven-field section and its draft flow are the tag editor's unit.
fn tag_section(state: UiState, track: &Track) -> impl IntoElement {
    let palette = state.palette;
    let title = track.metadata.title.clone();
    let artist = track.metadata.artist.clone();
    let album = track.metadata.album.clone();
    let genre = track.metadata.genre.clone();
    let year = track.metadata.year.map(|y| y.to_string());
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .test_id("inspector-tags")
        .child(
            text("TAGS")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(tag_row(palette, "Title", title))
        .child(tag_row(palette, "Artist", artist))
        .child(tag_row(palette, "Album", album))
        .child(tag_row(palette, "Genre", genre))
        .child(tag_row(palette, "Year", year))
}

/// One tag row: the field's muted label over its value, `(none)` when the
/// track carries none (the egui `TagRowState::None`'s word).
fn tag_row(
    palette: Signal<crate::theme::Palette>,
    label: &'static str,
    value: Option<String>,
) -> impl IntoElement {
    let carried = value.is_some();
    view()
        .flex_col()
        .gap(px(tokens::SPACE_XS))
        .child(
            text(label)
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            text(move || value.clone().unwrap_or_else(|| "(none)".to_owned()))
                .text_sm()
                .text_color(move || {
                    if carried {
                        palette.get().ink
                    } else {
                        palette.get().ink_3
                    }
                })
                .test_id("inspector-tag"),
        )
}

/// The details list: the egui track readout's three rows (plays, last
/// played, path), the label-above-value rhythm.
fn details(state: UiState, track: &Track) -> impl IntoElement {
    let palette = state.palette;
    let plays = track.play_count.to_string();
    let last_played = last_played_line(track.last_played);
    let path = track.file_path.to_string_lossy().into_owned();
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .test_id("inspector-details")
        .child(
            text("DETAILS")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(detail_row(palette, "Plays", plays))
        .child(detail_row(palette, "Last played", last_played))
        .child(detail_row(palette, "Path", path))
}

/// One detail row: the muted label over the value.
fn detail_row(
    palette: Signal<crate::theme::Palette>,
    label: &'static str,
    value: String,
) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(tokens::SPACE_XS))
        .child(
            text(label)
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            text(move || value.clone())
                .text_sm()
                .truncate()
                .text_color(move || palette.get().ink),
        )
}

/// The `Last played` line: never, or the ago-words the egui footer stamps
/// carry (the same granularity, not the same seconds).
fn last_played_line(last_played: Option<SystemTime>) -> String {
    match last_played {
        None => "Never".to_owned(),
        Some(at) => match at.elapsed() {
            Ok(ago) if ago.as_secs() < 60 => "Just now".to_owned(),
            Ok(ago) if ago.as_secs() < 3600 => format!("{}m ago", ago.as_secs() / 60),
            Ok(ago) if ago.as_secs() < 86400 => format!("{}h ago", ago.as_secs() / 3600),
            Ok(ago) => format!("{}d ago", ago.as_secs() / 86400),
            Err(_) => "Never".to_owned(),
        },
    }
}

/// Ask the View half for the selected track's card-box cover when the
/// selection moves — and when the covers revision moves, because an
/// eviction can drop the card the panel is showing (the egui panel asked
/// and resolved every frame; here the effect's re-runs are the frame, and
/// the want call is idempotent per `(identity, box)`, so the repeats are
/// set lookups). An effect, not a render binding: the request mutates the
/// decision half's sets, and a handler context is where mutation belongs.
fn request_cover_on_selection(model: Stored<UiModel>, state: UiState) {
    effect(move || {
        let Some(id) = state.selection.get() else {
            return;
        };
        let _ = state.covers.get();
        model.with(|m| {
            let Some(track) = reads::selected_track(&mut m.rt.session_views, &id) else {
                return;
            };
            m.covers.want_track(
                m.rt.covers.as_ref(),
                track.id.clone(),
                track.file_path.clone(),
                crate::bridge::covers::COVER_CARD,
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_played_line_reads_ago_words_and_never() {
        assert_eq!(last_played_line(None), "Never");
        assert_eq!(
            last_played_line(Some(SystemTime::now() - std::time::Duration::from_secs(90))),
            "1m ago"
        );
    }

    #[test]
    fn the_empty_panel_chips_nothing() {
        assert_eq!(selected_word(true), "Track");
        assert_eq!(selected_word(false), "");
    }
}
