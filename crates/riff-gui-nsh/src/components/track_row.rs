//! The track row: the anatomy every track listing shows (the plan's N6 line
//! — favorite, cover, title, artist, album, duration, missing marker,
//! playing marker), with the selection contract the egui crate's
//! `ui/app.rs` `interactive_track_row` ported: a press selects (the session
//! write mirrored to the selection signal, [`select_track`]), the heart
//! toggles the favorite flag (the egui heart's same durable write,
//! [`set_favorite`]).
//!
//! One display model, [`TrackRowView`]: the caller resolves the row's facts
//! (the list reads its warm window), and the builder reads them inside
//! every binding, beside the signals that move — so a recycled cell
//! re-resolves under its new row and a moved selection or playback
//! repaints without a structural rebuild (P0 corrections 12 and 21). The
//! row never touches the store: all reads ride the caller's closure and
//! the signal handles.

use std::time::Duration;

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::domain::TrackId;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

use super::cover;

/// One row's display facts, resolved by the caller (the egui `TrackRow`'s
/// shape, widened to the columns the nsh row shows).
#[derive(Debug, Clone)]
pub struct TrackRowView {
    /// The row's position in the listing — the traversal's address.
    pub index: usize,
    pub id: TrackId,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `None` paints the dash (unknown duration).
    pub duration: Option<Duration>,
    pub favorite: bool,
    /// A track the store lists whose file is gone. The flat All Tracks
    /// listing carries only scanned-and-present rows, so the list fills
    /// this `false`; the marker's proof is a synthetic row (recorded in the
    /// N6 trail). Playlists' dangling entries light it for real at N10.
    pub missing: bool,
}

/// One track row: band, favorite heart, cover, the three text columns,
/// duration, the missing and playing markers.
pub fn row_of(
    model: Stored<UiModel>,
    state: UiState,
    facts: impl Fn() -> TrackRowView + Clone + 'static,
    list_focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    let selection = state.selection;
    let playback = state.playback;
    let selected = {
        let facts = facts.clone();
        move || selection.get().as_ref() == Some(&facts().id)
    };
    let selected_a11y = selected.clone();
    // The egui row's now-playing fact: this row IS the player's current
    // track (the indicator paints whether or not playback is running, the
    // egui `is_current`).
    let playing = {
        let facts = facts.clone();
        move || {
            playback
                .get()
                .current
                .as_ref()
                .is_some_and(|current| current.id == facts().id)
        }
    };
    let heart_facts = facts.clone();
    let press_facts = facts.clone();
    let cover_facts = facts.clone();
    let missing_facts = facts.clone();
    let focused = {
        let facts = facts.clone();
        move || list_focus.get() == Some(facts().index)
    };
    let title_facts = facts.clone();
    let artist_facts = facts.clone();
    let album_facts = facts.clone();
    let duration_facts = facts.clone();
    let playing_tint = playing.clone();

    let row = view()
        .h(px(tokens::ROW_H))
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .px(px(tokens::SPACE_SM))
        .rounded(px(tokens::RADIUS_SM))
        .test_id("track-row")
        .accessibility_state(move || AccessibilityState {
            selected: selected_a11y(),
            ..AccessibilityState::default()
        })
        .bg(move || {
            if selected() {
                palette.get().surface_3
            } else {
                transparent_black()
            }
        })
        .hover(|s| s.bg(move || palette.get().row_hover))
        .on_press(move || select_track(model, press_facts().id))
        .child(favorite_cell(model, state, heart_facts))
        .child(
            view()
                .w(px(tokens::ROW_COVER))
                .h(px(tokens::ROW_COVER))
                .rounded(px(tokens::RADIUS_XS))
                .test_id("row-cover")
                .child(cover::tile(
                    model,
                    state,
                    move || cover_facts().id.0,
                    crate::bridge::covers::COVER_THUMB,
                )),
        )
        .child(columns(
            palette,
            playing_tint,
            title_facts,
            artist_facts,
            album_facts,
        ))
        .child(tail(palette, playing, missing_facts, duration_facts));
    crate::shell::ring_when(row, focused, palette)
}

/// The three text columns: the title (brand-tinted while the row is the
/// player's current track), the artist, the album — each truncating, each
/// reading its facts inside its own binding.
fn columns(
    palette: Signal<crate::theme::Palette>,
    playing: impl Fn() -> bool + Clone + 'static,
    title: impl Fn() -> TrackRowView + Clone + 'static,
    artist: impl Fn() -> TrackRowView + Clone + 'static,
    album: impl Fn() -> TrackRowView + Clone + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .flex_row()
        .gap(px(tokens::SPACE_SM))
        .child(
            text(move || title().title)
                .flex_1()
                .text_sm()
                .truncate()
                .text_color(move || {
                    if playing() {
                        palette.get().brand_primary
                    } else {
                        palette.get().ink
                    }
                })
                .test_id("row-title"),
        )
        .child(
            text(move || artist().artist)
                .flex_1()
                .text_sm()
                .truncate()
                .text_color(move || palette.get().ink_2)
                .test_id("row-artist"),
        )
        .child(
            text(move || album().album)
                .flex_1()
                .text_sm()
                .truncate()
                .text_color(move || palette.get().ink_2)
                .test_id("row-album"),
        )
}

/// The row's tail: the missing marker, the playing marker, the duration.
fn tail(
    palette: Signal<crate::theme::Palette>,
    playing: impl Fn() -> bool + Clone + 'static,
    missing: impl Fn() -> TrackRowView + Clone + 'static,
    duration: impl Fn() -> TrackRowView + Clone + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .child(
            when(
                move || missing().missing,
                move || missing_marker(palette).into_element(),
            )
            .otherwise(|| view().into_element()),
        )
        .child(
            when(playing, move || now_playing_marker(palette).into_element())
                .otherwise(|| view().into_element()),
        )
        .child(
            text(move || duration_line(duration().duration))
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("row-duration"),
        )
}

/// The favorite cell: the heart the egui row carries in its leading cell —
/// brand-tinted when the track IS a favorite, muted ink when not, and a
/// press commits the flip (the egui heart's same durable write, whose
/// committed mutation bumps the generation the pump answers).
fn favorite_cell(
    model: Stored<UiModel>,
    state: UiState,
    facts: impl Fn() -> TrackRowView + Clone + 'static,
) -> impl IntoElement {
    let palette = state.palette;
    let favorite = facts.clone();
    let press_facts = facts;
    view()
        .w(px(tokens::FAVORITE_COL_W))
        .items_center()
        .justify_center()
        .test_id("row-favorite")
        .on_press(move || set_favorite(model, press_facts().id.clone(), !press_facts().favorite))
        .child(icon(Icon::Heart, tokens::PLACEHOLDER_GLYPH, move || {
            if favorite().favorite {
                palette.get().brand_primary
            } else {
                palette.get().ink_3
            }
        }))
}

/// The missing marker: the egui playlist-entry row's `(missing)` suffix, as
/// its own muted label.
fn missing_marker(palette: Signal<crate::theme::Palette>) -> impl IntoElement {
    text("(missing)")
        .text_xs()
        .text_color(move || palette.get().ink_3)
        .test_id("row-missing")
}

/// The playing marker: the glyph the egui row paints on the track currently
/// loaded in the player, brand-tinted like the egui row's label.
fn now_playing_marker(palette: Signal<crate::theme::Palette>) -> impl IntoElement {
    icon(Icon::Play, tokens::PLACEHOLDER_GLYPH, move || {
        palette.get().brand_primary
    })
    .test_id("row-playing")
}

/// The duration's right-aligned value: `m:ss`, or the dash for an unknown
/// length (the egui `TrackRow`'s `None` contract).
pub(crate) fn duration_line(duration: Option<Duration>) -> String {
    let Some(duration) = duration else {
        return "-".to_owned();
    };
    let secs = duration.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// One selection: the egui row's click write, through the library guard and
/// mirrored to the selection signal outside the borrow (the sidebar's
/// `apply` shape — a selection write mutates no store, so the pump is not
/// its carrier).
pub(crate) fn select_track(model: Stored<UiModel>, id: TrackId) {
    let selection = model.with(|m| m.state.selection);
    model.with(|m| {
        m.rt.library.lock_or_recover().selected_track = Some(id.clone());
    });
    selection.set(Some(id));
}

/// Commit one track's favorite flag: the egui `commit_track_favorite` — one
/// immediate durable write through the mutations port, a failure logged and
/// otherwise silent (the committed mutation bumps the generation, so the
/// projections and the pump answer with no caller action).
pub(crate) fn set_favorite(model: Stored<UiModel>, id: TrackId, favorite: bool) {
    model.with(|m| {
        if let Err(e) = m.rt.library_mutations.set_track_favorite(&id, favorite) {
            tracing::warn!("Failed to commit the favorite flag for {}: {e}", id.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_duration_reads_as_a_dash() {
        assert_eq!(duration_line(None), "-");
        assert_eq!(duration_line(Some(Duration::from_secs(201))), "3:21");
    }
}
