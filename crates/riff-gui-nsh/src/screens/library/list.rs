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
use riff_backend::domain::Track;
use std::sync::Arc;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::track_row::{TrackRowView, row_of};
use crate::model::StageKind;
use crate::model::reads;
use crate::screens::library::search;
use crate::theme::tokens;

/// How far behind the viewport the warm window keeps rows, in rows (one
/// viewport's worth: the reverse scroll's grace).
pub const PREFETCH_LOOKBACK: usize = 50;
/// How far ahead of the viewport the warm window reaches, in rows — cover
/// decode is the limiter (P0 correction 24), so the ask goes out before the
/// row can be scrolled to. With the projection's 8-window page cache this
/// also bounds the read burst per report.
pub const PREFETCH_LOOKAHEAD: usize = 250;

/// The stage: the list beside the inspector. The list keeps its scroll
/// while mounted (the framework's own) and its element ref reaches the
/// shell's traversal, whose Enter selects and whose steps reveal through
/// `scroll_to_index`. The search query rides the row window's stamp — the
/// same listing the flat view always was, now under the stage table.
pub fn stage(
    model: Stored<UiModel>,
    state: UiState,
    list: ElementRef,
    list_focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let listing = track_listing(
        model,
        state,
        list,
        list_focus,
        TrackListingSpec {
            kind: Listing::Flat,
            stage_id: StageKind::Flat,
            plain: (
                "No tracks yet",
                "Add a folder from the sidebar to start scanning your library.",
            ),
            filtered: ("No matching tracks", "Nothing in your library matches"),
            test_id: "track-list",
        },
    );
    crate::screens::library::browse::stage_shell(model, state, listing)
}

/// Which listing a track stage renders and what it says when empty — the
/// one spec the two track stages differ in, beside the handles they share.
pub(crate) struct TrackListingSpec {
    /// The listing the window serves (the flat seam or an album's display
    /// copy).
    pub kind: Listing,
    /// The scroll memory slot the stage keeps.
    pub stage_id: StageKind,
    /// The empty state's wording when no needle stands.
    pub plain: (&'static str, &'static str),
    /// The empty state's wording when the needle matched nothing.
    pub filtered: (&'static str, &'static str),
    /// The virtual list's test id.
    pub test_id: &'static str,
}

/// The one track listing every track stage renders: the virtual list over
/// the warm row window, its query-aware empty state, and the viewability
/// handler that restores and remembers the stage's scroll position and
/// drives the prefetch window.
pub(crate) fn track_listing(
    model: Stored<UiModel>,
    state: UiState,
    list: ElementRef,
    list_focus: Signal<Option<usize>>,
    spec: TrackListingSpec,
) -> impl IntoElement {
    let palette = state.palette;
    let total = state.list_total;
    let restore_armed = stored(true);
    let empty = search::empty_state_for(
        palette,
        move || state.search.get(),
        spec.plain,
        spec.filtered,
    );
    let kind = spec.kind;
    let memory_stage = spec.stage_id;
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
                facts_of(model, at.index(), kind)
            };
            row_of(model, state, facts, list_focus)
        },
    )
    .estimated_item_size(px(tokens::ROW_H))
    .flex_1()
    .element_ref(&list)
    .test_id(spec.test_id)
    .on_viewable_items_change(move |items: &ViewableItems| {
        // The report's own pair drives the window; a None report (no
        // viewable row) skips the fill. The restore reads the slot the
        // previous visit left, the save records this report's position —
        // that order, or the save would overwrite the memory before the
        // restore read it.
        if let Some(viewable) = items.viewable {
            restore_position(model, &state, &memory_stage, list, restore_armed);
            remember_position(model, &state, &memory_stage, viewable.0);
            prefetch_as(model, &state, viewable, kind);
        }
    })
    .empty(empty)
}

/// One row's facts, out of the warm window — a miss falls back to the
/// listing's own read under the window's own stamp (the first frame before
/// the first report; the fallback runs inside the binding, the spike's
/// direct pattern, and costs no more than the pattern it replaces). The
/// stamp carries the search needle as well as the sort, so a filtered
/// listing's rows read the same way the flat one did; an album listing's
/// miss reads the stage's display copy, never the flat seam.
fn facts_of(model: Stored<UiModel>, index: usize, kind: Listing) -> TrackRowView {
    let track = model.with(|m| track_at(m, index, kind));
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
    prefetch_as(model, state, viewable, Listing::Flat);
}

/// The one window the two track stages share, over the listing `kind`
/// names: the flat list reads the seam by stamp, an album's tracks index
/// the display copy the pump read for the stage. The stamp is re-checked
/// here (the same values the pump stamps with), so a stale window never
/// serves.
pub(crate) fn prefetch_as(
    model: Stored<UiModel>,
    state: &UiState,
    viewable: (usize, usize),
    kind: Listing,
) {
    let (first, last) = viewable;
    let total = state.list_total.get();
    let Some((start, end)) = window_bounds(first, last, total) else {
        return;
    };
    model.with(|m| {
        refill_window(m, start, end, kind);
        want_covers(m);
    });
}

/// Which listing a track stage's window serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Listing {
    /// The flat All Tracks list (the search query included): rows read the
    /// seam by the window's stamp.
    Flat,
    /// An album's tracks: rows index the stage's display copy.
    Album,
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
pub(crate) fn refill_window(model: &mut UiModel, start: usize, end: usize, kind: Listing) {
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
    // and the FIFO window cache never thrashes (the span is bounded). The
    // album listing's rows are already in hand (one read per stage build),
    // so its refill copies with no store read at all.
    for index in &missing {
        rows[*index - start] = match kind {
            Listing::Flat => reads::track_row(
                &mut model.rt.session_views,
                model.list_window.query(),
                model.list_window.sort(),
                *index,
            ),
            Listing::Album => model.album_rows.get(*index).cloned(),
        };
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
pub(crate) fn track_at(model: &mut UiModel, index: usize, kind: Listing) -> Option<Arc<Track>> {
    model.list_window.row(index).or_else(|| match kind {
        Listing::Flat => reads::track_row(
            &mut model.rt.session_views,
            model.list_window.query(),
            model.list_window.sort(),
            index,
        ),
        Listing::Album => model.album_rows.get(index).cloned(),
    })
}

/// The stage's content fingerprint for the scroll memory: the needle the
/// search signal carries, the listing sort the window is stamped with, and
/// the Library generation the UI last saw — the egui fingerprint's three
/// parts, read where the handler already holds the model.
fn fingerprint(model: &mut UiModel, state: &UiState) -> (String, u8, u64) {
    (
        state.search.get(),
        model.list_window.sort().as_sort_key(),
        state.library_revision.get(),
    )
}

/// Save the stage's position: every viewability report records the stage's
/// first viewable row — the egui protocol's per-frame end-section, adapted
/// to the stage table. Cheap, and the restore below reads it back.
pub(crate) fn remember_position(
    model: Stored<UiModel>,
    state: &UiState,
    slot: &StageKind,
    first_viewable: usize,
) {
    model.with(|m| {
        let (query, sort_key, generation) = fingerprint(m, state);
        m.scroll_memory
            .remember(slot, &query, sort_key, generation, first_viewable);
    });
}

/// Restore the stage's position once per mount: the first viewability
/// report after the list remounts scrolls it back to the remembered row —
/// `scroll_to_index`, the plan's restoration — when the memory's slot
/// still matches the stage's content identity. A stale fingerprint (the
/// query, the sort or the Library moved behind the position) leaves the
/// list at the top, the egui reset contract. `armed` is the mount's
/// one-shot flag, created by the stage component so it dies with it.
pub(crate) fn restore_position(
    model: Stored<UiModel>,
    state: &UiState,
    slot: &StageKind,
    list: ElementRef,
    armed: Stored<bool>,
) {
    if !armed.get() {
        return;
    }
    armed.replace(false);
    let (query, sort_key, generation) = model.with(|m| fingerprint(m, state));
    let Some(row) = model.with(|m| m.scroll_memory.restore(slot, &query, sort_key, generation))
    else {
        return;
    };
    list.scroll_to_index(
        u32::try_from(row).unwrap_or(u32::MAX),
        percentage(0.0),
        px(0.0),
        false,
    );
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
