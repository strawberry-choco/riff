//! The All Tracks stage: the virtual track list over the bounded Session
//! Projection (`virtual_list` over `track_count` and `track_row`), with the
//! inspector beside it. The egui crate's `ui/app.rs` `render_flat_view` is
//! the behavior contract: the flat list's reads, its empty copy, and the
//! row selection.
//!
//! The read discipline is P0 correction 22's: the direct per-row read
//! pattern misses the scroll budget by orders of magnitude, so the store is
//! only ever read by the **prefetch window** — the viewability handler
//! keeps a warm window of rows around the viewport (reads issued in
//! ascending index order, so the projection's page fetches stay sequential)
//! and the row bindings index into it with no store read; a binding the
//! window does not cover (the first frame, a jump faster than the report)
//! falls back to the seam's own read. Covers ride the same window: the
//! handler asks the View half for every row it read, ahead of the viewport
//! (P0 correction 24), and the arrivals land through the covers revision
//! signal.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::{BrowseMode, LibrarySection, ViewMode};
use riff_backend::domain::Track;
use std::sync::Arc;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::empty_state;
use crate::components::track_row::{TrackRowView, row_of};
use crate::model::NavView;
use crate::model::reads;
use crate::screens::inspector;
use crate::theme::tokens;

/// How far behind the viewport the warm window keeps rows, in rows (one
/// viewport's worth: the reverse scroll's grace).
pub const PREFETCH_LOOKBACK: usize = 50;
/// How far ahead of the viewport the warm window reaches, in rows — cover
/// decode is the limiter (P0 correction 24), so the ask goes out before the
/// row can be scrolled to. With the projection's 8-window page cache this
/// also bounds the read burst per report.
pub const PREFETCH_LOOKAHEAD: usize = 250;

/// Whether the nav state names the All Tracks listing — the one stage this
/// unit mounts (the same liveness conditions the sidebar's All Tracks row
/// highlights under).
#[must_use]
pub fn is_all_tracks(nav: &NavView) -> bool {
    nav.view_mode == ViewMode::Library
        && nav.browse_mode == BrowseMode::Library
        && nav.smart_list.is_none()
        && nav.playlist.is_none()
        && nav.library_section == LibrarySection::AllTracks
}

/// The stage: the list beside the inspector. The list keeps its scroll
/// while mounted (the framework's own) and its element ref reaches the
/// shell's traversal, whose Enter selects and whose steps reveal through
/// `scroll_to_index`.
pub fn stage(
    model: Stored<UiModel>,
    state: UiState,
    list: ElementRef,
    list_focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_1()
        .flex_row()
        .test_id("library-stage")
        .bg(move || palette.get().surface_row)
        .child(
            view()
                .flex_1()
                .flex_col()
                .p(px(tokens::SPACE_LG))
                .child(list_view(model, state, list, list_focus)),
        )
        .child(inspector::panel(model, state))
}

/// The listing: the virtual list, its empty state, and the viewability
/// handler that drives the prefetch window.
fn list_view(
    model: Stored<UiModel>,
    state: UiState,
    list: ElementRef,
    list_focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    let total = state.list_total;
    let empty = empty_state::state(
        palette,
        "No tracks yet",
        "Add a folder from the sidebar to start scanning your library.",
    );
    virtual_list(
        move || total.get(),
        move |at| {
            // The index is read INSIDE the bindings (P0 correction 12):
            // copying it out here would freeze the row the cell shows. The
            // list revision rides the copied handle, read beside the
            // window (a read inside the borrow subscribes nothing): a pump
            // reset re-resolves every visible row.
            let revision = state.list_revision;
            let facts = move || {
                let _ = revision.get();
                facts_of(model, at.index())
            };
            row_of(model, state, facts, list_focus)
        },
    )
    .estimated_item_size(px(tokens::ROW_H))
    .flex_1()
    .element_ref(&list)
    .test_id("track-list")
    .on_viewable_items_change(move |items: &ViewableItems| {
        // The report's own pair drives the window; a None report (no
        // viewable row) skips the fill.
        if let Some(viewable) = items.viewable {
            prefetch(model, &state, viewable);
        }
    })
    .empty(empty)
}

/// One row's facts, out of the warm window — a miss falls back to the
/// seam's own read (the first frame before the first report; the fallback
/// runs inside the binding, the spike's direct pattern, and costs no more
/// than the pattern it replaces).
fn facts_of(model: Stored<UiModel>, index: usize) -> TrackRowView {
    let track = model.with(|m| {
        m.list_window
            .row(index)
            .or_else(|| reads::track_row(&mut m.rt.session_views, "", m.list_window.sort(), index))
    });
    facts_from(index, track)
}

/// The row view out of a store row: `Option` because a listing's tail can
/// be asked past its end while the count signal and the rows disagree for
/// one frame — the egui flat view's `continue`.
#[must_use]
fn facts_from(index: usize, track: Option<Arc<Track>>) -> TrackRowView {
    match track {
        Some(track) => TrackRowView {
            index,
            id: track.id.clone(),
            title: track.metadata.display_title(&track.file_path),
            artist: track.metadata.display_artist(),
            album: track.metadata.display_album().into_owned(),
            duration: track.duration,
            favorite: track.favorite,
            missing: false,
        },
        None => TrackRowView {
            index,
            id: riff_backend::domain::TrackId(String::new()),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration: None,
            favorite: false,
            missing: false,
        },
    }
}

/// The prefetch window: the report's viewable range widened by the lookback
/// and the lookahead, clamped to the listing. Rows the window already holds
/// are kept (no re-read); the missing ones are read in ascending index
/// order — the sorted-read discipline P0 correction 22 asks for — and every
/// row's cover is asked for at the thumbnail box, ahead of the viewport.
/// No signal is written here: the covers repaint through the pump's
/// revision bump, the rows through the cells' own rebinds.
pub fn prefetch(model: Stored<UiModel>, state: &UiState, viewable: (usize, usize)) {
    let (first, last) = viewable;
    let total = state.list_total.get();
    let Some((start, end)) = window_bounds(first, last, total) else {
        return;
    };
    model.with(|m| {
        // The sort is the session's — read under this handler's borrow, the
        // same value the pump stamps the window with.
        let sort = m.rt.library.lock_or_recover().track_sort;
        m.list_window.set_sort(sort);
        refill_window(m, start, end);
        want_covers(m);
    });
}

/// The prefetch window's clamped bounds over `[start, end]`, or `None` when
/// the range holds nothing: a report computed against an older, larger
/// total can clamp below its own start once the listing shrinks under it
/// (a library shrink while scrolled deep), and an inverted range would
/// underflow the refill's length arithmetic.
#[must_use]
fn window_bounds(first: usize, last: usize, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let start = first.saturating_sub(PREFETCH_LOOKBACK);
    let end = last.saturating_add(PREFETCH_LOOKAHEAD).min(total - 1);
    (start <= end).then_some((start, end))
}

/// Fill the window over `[start, end]`, keeping the rows it already holds
/// and reading the missing ones ascending. `start` may not exceed `end` —
/// [`window_bounds`] guarantees it for the handler.
fn refill_window(model: &mut UiModel, start: usize, end: usize) {
    if let Some((held_start, held_end)) = model.list_window.covers()
        && held_start <= start
        && end <= held_end
    {
        return;
    }
    let (old_start, old_rows) = model.list_window.take();
    let len = end - start + 1;
    let mut rows: Vec<Option<Arc<Track>>> = std::iter::repeat_n(None, len).collect();
    let mut missing = Vec::new();
    for (at, row) in rows.iter_mut().enumerate() {
        let index = start + at;
        let held = index
            .checked_sub(old_start)
            .and_then(|offset| old_rows.get(offset))
            .and_then(Clone::clone);
        match held {
            Some(track) => *row = Some(track),
            None => missing.push(index),
        }
    }
    // The reads, ascending — the projection's page fetches stay sequential
    // and the FIFO window cache never thrashes (the span is bounded).
    for index in &missing {
        rows[*index - start] = reads::track_row(
            &mut model.rt.session_views,
            "",
            model.list_window.sort(),
            *index,
        );
    }
    model.list_window.fill(start, rows);
}

/// Ask the View half for every row the window serves, in ascending row
/// order; the decision half's arrived and in-flight sets absorb the repeats.
fn want_covers(model: &mut UiModel) {
    let Some((window_start, window_end)) = model.list_window.covers() else {
        return;
    };
    for index in window_start..=window_end {
        let Some(track) = model.list_window.row(index) else {
            continue;
        };
        if model.covers.want_track(
            model.rt.covers.as_ref(),
            track.id.clone(),
            track.file_path.clone(),
            crate::bridge::covers::COVER_THUMB,
        ) {}
    }
}

/// The keyboard traversal's read: the track at `index`, for Enter's
/// selection — the same window-then-seam read the row bindings take. Runs
/// under the caller's stored borrow.
#[must_use]
pub(crate) fn track_at(model: &mut UiModel, index: usize) -> Option<Arc<Track>> {
    model.list_window.row(index).or_else(|| {
        reads::track_row(
            &mut model.rt.session_views,
            "",
            model.list_window.sort(),
            index,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shrunk_total_never_produces_an_inverted_window() {
        // A report computed while the listing was 50_000 rows lands after
        // the library shrank to 100: the clamp pins the end below the
        // start, and the bounds answer None instead of underflowing the
        // refill's length arithmetic.
        let bounds = window_bounds(40000, 40028, 100);
        assert_eq!(bounds, None, "the shrunk total yields no window");
        // The healthy cases stand: the full clamp, and the last row's tail.
        assert_eq!(window_bounds(0, 28, 50_000), Some((0, 278)));
        assert_eq!(
            window_bounds(49_990, 49_999, 50_000),
            Some((49_940, 49_999))
        );
        assert_eq!(
            window_bounds(10, 20, 0),
            None,
            "an empty listing has no window"
        );
    }
}
