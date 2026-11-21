//! The cover tile: one artwork identity's pixels from the View half, or the
//! shared placeholder well while nothing is held. The egui crate's
//! `ui/artwork.rs` split the surface into a texture map and a placeholder
//! tile; here the View half is [`crate::model::CoverViews`] and the
//! placeholder is an element (a `surface_2` well with the music glyph, the
//! same design) — nothing to cache, nothing to account, so it is exempt
//! from the View half's bounds the way the egui tile was.
//!
//! The tile renders from bindings only: the identity closure runs inside
//! them (a recycled cell re-resolves under its new row), the covers
//! revision signal is read beside the lookup (an arrival repaints the miss
//! that waited for it, P0 correction 21), and the `ImageSrc` clone per
//! binding run is the map's contract, not a per-frame copy of pixels.
//! Requests are NOT made here: rows are served by the list's prefetch
//! window, the inspector asks in its own effect — a render binding must
//! never mutate the cache's sets.

use nsh_core::prelude::*;
use riff_backend::app::traits::RequestedSize;

use crate::bridge::UiModel;
use crate::bridge::covers::cover_cache_key;
use crate::bridge::state::UiState;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// The tile for `identity` at `requested`, filling the box the caller gives
/// it. A miss paints the placeholder well; the branch flips when the
/// lookup's answer does — an arrival rebuilds the image branch, an eviction
/// the placeholder.
pub fn tile(
    model: Stored<UiModel>,
    state: UiState,
    identity: impl Fn() -> String + Clone + 'static,
    requested: RequestedSize,
) -> impl IntoElement {
    let palette = state.palette;
    let covers = state.covers;
    let resolve = move || {
        let _ = covers.get();
        let key = cover_cache_key(&identity(), requested);
        model.with(|m| m.cover_views.get(&key).cloned())
    };
    let hit = resolve.clone();
    when(move || hit().is_some(), {
        let resolve = resolve.clone();
        move || {
            let held = resolve();
            image(move || held.clone().expect("the branch only shows on a hit"))
        }
    })
    .otherwise(move || placeholder(palette))
}

/// The shared placeholder well: the `surface_2` block with the music glyph
/// tinted `ink_3` (the egui tile's design, as an element).
fn placeholder(palette: Signal<crate::theme::Palette>) -> impl IntoElement {
    view()
        .w_full()
        .h_full()
        .bg(move || palette.get().surface_2)
        .rounded(px(tokens::RADIUS_XS))
        .items_center()
        .justify_center()
        .test_id("cover-placeholder")
        .child(icon(Icon::Music, tokens::PLACEHOLDER_GLYPH, move || {
            palette.get().ink_3
        }))
}
