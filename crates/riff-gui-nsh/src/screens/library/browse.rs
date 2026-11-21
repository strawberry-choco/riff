//! The browse stages: the drill-downs the Library sections walk (Artists →
//! albums → tracks; Albums → tracks; Genres → artists → albums → tracks)
//! and the tracks stage every one of them ends at. The egui crate's
//! `ui/app/browser_pane.rs` is the behavior contract — the stage plan over
//! the drill-down path, the entity rows' facts, the selection writes, and
//! the backend's matching rules under a query — with the egui elastic
//! column stage collapsed to one listing: the nsh stage is the fixed
//! `flex_1` region and shows the deepest active listing, the drill-down is
//! a walk, and the parent's highlight rides the path (the unit's own
//! adaptation, recorded in the trail).
//!
//! Two row shapes, one discipline. Entity listings read one batched answer
//! per stage build (the pump's read, generation-cached; a stage rebuild
//! re-reads it) and never touch the store per row. Track listings render
//! through the same warm-row-window seam as the flat list
//! ([`super::list`]): the window's refill indexes the stage's display copy
//! the pump read — no per-row store reads either way.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::BrowserSelection;
use riff_backend::domain::TrackId;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::cover;
use crate::model::{EntityView, StageKind};
use crate::screens::library::list::{self, Listing, remember_position, restore_position};
use crate::screens::library::search;
use crate::theme::tokens;

/// The stage's shell: the listing beside the inspector — the flat stage's
/// shape, shared by every Library stage (the egui inspector followed the
/// selection in all of them).
pub(crate) fn stage_shell(
    model: Stored<UiModel>,
    state: UiState,
    listing: impl IntoElement,
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
                .child(listing),
        )
        .child(crate::screens::inspector::panel(model, state))
}

/// An entity stage: the listing of the stage's batched rows, beside the
/// inspector. Sized by the browse signal; the viewability handler asks the
/// covers for the rows it shows (widened ahead of the viewport, the same
/// discipline the track window keeps) and drives the stage's scroll memory.
pub fn entity_stage(model: Stored<UiModel>, state: UiState, kind: StageKind) -> impl IntoElement {
    let listing = entity_listing(model, state, kind);
    stage_shell(model, state, listing)
}

/// The entity listing: the virtual list over the browse signal, its
/// query-aware empty state, and the handler above.
fn entity_listing(model: Stored<UiModel>, state: UiState, kind: StageKind) -> impl IntoElement {
    let palette = state.palette;
    let restore_armed = stored(true);
    let list = element_ref();
    let memory_stage = kind.clone();
    let covers_kind = kind.clone();
    let (plain, filtered) = entity_copy(&kind);
    let empty = search::empty_state_for(palette, move || state.search.get(), plain, filtered);
    virtual_list(
        move || state.browse.get().len(),
        move |at| {
            let rows = state.browse;
            let kind = kind.clone();
            let facts = move || rows.get().get(at.index()).cloned();
            entity_row(model, state, kind, facts)
        },
    )
    .estimated_item_size(px(tokens::ROW_H))
    .flex_1()
    .element_ref(&list)
    .test_id("entity-list")
    .on_viewable_items_change(move |items: &ViewableItems| {
        if let Some(viewable) = items.viewable {
            restore_position(model, &state, &memory_stage, list, restore_armed);
            remember_position(model, &state, &memory_stage, viewable.0);
            want_entity_covers(model, &state, &covers_kind, viewable);
        }
    })
    .empty(empty)
}

/// The covers the entity rows in `viewable` (widened ahead of the viewport,
/// the track window's discipline) ask for, through each row's own cover
/// track. A track id IS its file path (the store's identity), so the ask
/// carries the id's own path. Repeats are the decision half's set lookups.
/// The artists root's rows carry no cover track — their first-track
/// identity resolves here for the rows the report serves, and in the row
/// view's own binding, through the seam's per-artist cache.
fn want_entity_covers(
    model: Stored<UiModel>,
    state: &UiState,
    kind: &StageKind,
    viewable: (usize, usize),
) {
    let rows = state.browse.get();
    let len = rows.len();
    if len == 0 {
        return;
    }
    // The viewport's own range, unwidened: the artist rows' cover
    // identities resolve per row here (the first-track read is per
    // artist), so a track-window-sized lookahead would spend hundreds of
    // identity reads per report on a big library — the egui root asked for
    // its visible rows only, and so does this.
    let first = viewable.0;
    let last = viewable.1.min(len - 1);
    model.with(|m| {
        for at in first..=last {
            let Some(row) = rows.get(at) else { continue };
            let Some(track_id) = row_cover_track(m, kind, row) else {
                continue;
            };
            m.covers.want_track(
                m.rt.covers.as_ref(),
                track_id.clone(),
                std::path::PathBuf::from(track_id.0.as_str()),
                crate::bridge::covers::COVER_THUMB,
            );
        }
    });
}

/// The track whose cover a row shows: the row's own where the stage build
/// carried one, the artist's first track where the row is an artists-root
/// row (resolved at serve time, not build time — the build stays cheap on
/// a big library). Runs under the caller's stored borrow.
fn row_cover_track(model: &mut UiModel, kind: &StageKind, row: &EntityView) -> Option<TrackId> {
    row.cover_track.clone().or_else(|| match kind {
        StageKind::ArtistsRoot => {
            crate::model::reads::artist_first_track(&mut model.rt.session_views, &row.key)
        }
        _ => None,
    })
}

/// The tracks stage: the selected album's track listing — the same warm
/// row window the flat list renders through, refilled from the stage's
/// display copy (no store read per row), its element ref reaching the
/// shell's traversal like the flat stage's.
pub fn tracks_stage(
    model: Stored<UiModel>,
    state: UiState,
    list: ElementRef,
    list_focus: Signal<Option<usize>>,
    kind: StageKind,
) -> impl IntoElement {
    let listing = list::track_listing(
        model,
        state,
        list,
        list_focus,
        list::TrackListingSpec {
            kind: Listing::Album,
            stage_id: kind,
            plain: ("No tracks here yet", "This selection has nothing to show."),
            filtered: ("No matching tracks", "Nothing in this view matches"),
            test_id: "entity-track-list",
        },
    );
    stage_shell(model, state, listing)
}

/// One entity row: the egui `browser_row`'s anatomy — the cover tile (or
/// its placeholder), the label over the muted detail line, the selection
/// band while the drill-down path names it, the hover wash. A press writes
/// the selection at the row's own depth (the egui
/// `apply_entity_selection`), clearing the track selection with it —
/// `LibrarySession::select_at`'s mutual-exclusion rule, mirrored to the
/// selection signal the same way the nav mirror rides the path.
fn entity_row(
    model: Stored<UiModel>,
    state: UiState,
    kind: StageKind,
    facts: impl Fn() -> Option<EntityView> + Clone + 'static,
) -> impl IntoElement {
    let palette = state.palette;
    let nav = state.nav;
    let selected = {
        let facts = facts.clone();
        let kind = kind.clone();
        move || {
            facts().is_some_and(|row| {
                selection_at(&kind, &row.key).is_some_and(|(level, selection)| {
                    nav.get().browser_path.get(level) == Some(&selection)
                })
            })
        }
    };
    let selected_a11y = selected.clone();
    let press_facts = facts.clone();
    let press_kind = kind.clone();
    let menu_facts = facts.clone();
    let menu_kind = kind.clone();
    let cover_facts = facts.clone();
    let cover_kind = kind.clone();
    let label_facts = facts.clone();
    let detail_facts = facts;
    view()
        .h(px(tokens::ROW_H))
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .px(px(tokens::SPACE_SM))
        .rounded(px(tokens::RADIUS_SM))
        .test_id("entity-row")
        .label({
            let label_facts = label_facts.clone();
            move || label_facts().map_or_else(String::new, |row| row.label)
        })
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
        .on_press(move || {
            if let Some((level, selection)) =
                press_facts().and_then(|row| selection_at(&press_kind, &row.key))
            {
                select_entity(model, level, selection);
            }
        })
        .on_context_menu(move |event: &ContextMenuEvent| {
            if let Some(row) = menu_facts()
                && let Some(target) = collection_target(&menu_kind, &row.key)
            {
                if let Some((level, selection)) = selection_at(&menu_kind, &row.key) {
                    select_entity(model, level, selection);
                }
                crate::menus::collection::open_collection_menu(
                    model,
                    target,
                    event.page_position,
                );
            }
        })
        .child(
            view()
                .w(px(tokens::ROW_COVER))
                .h(px(tokens::ROW_COVER))
                .rounded(px(tokens::RADIUS_XS))
                .test_id("entity-cover")
                .child(cover::tile(
                    model,
                    state,
                    move || {
                        cover_facts()
                            .and_then(|row| model.with(|m| row_cover_track(m, &cover_kind, &row)))
                            .map_or_else(String::new, |id| id.0)
                    },
                    crate::bridge::covers::COVER_THUMB,
                )),
        )
        .child(
            view()
                .flex_1()
                .flex_col()
                .child(
                    text(move || label_facts().map_or_else(String::new, |row| row.label))
                        .text_sm()
                        .truncate()
                        .text_color(move || palette.get().ink)
                        .test_id("entity-label"),
                )
                .child(
                    text(move || detail_facts().map_or_else(String::new, |row| row.detail))
                        .text_xs()
                        .truncate()
                        .text_color(move || palette.get().ink_3)
                        .test_id("entity-detail"),
                ),
        )
}

/// One selection: the egui frame's write set for an entity row, applied
/// through the library guard, mirrored to the nav signal (path included —
/// the stage table and the row highlights read it) and to the selection
/// signal outside the borrow. `select_at` clears the track selection: the
/// two selections are mutually exclusive, and the signal follows the
/// session the same way the row presses' mirror does.
pub fn select_entity(model: Stored<UiModel>, level: usize, selection: BrowserSelection) {
    let nav_signal = model.with(|m| m.state.nav);
    let selection_signal = model.with(|m| m.state.selection);
    let mut nav = nav_signal.get();
    nav = model.with(|m| {
        let mut library = m.rt.library.lock_or_recover();
        library.select_at(level, selection);
        nav.view_mode = library.view_mode;
        nav.browse_mode = library.browse_mode;
        nav.library_section = library.library_section;
        library.browser_path.clone_into(&mut nav.browser_path);
        nav
    });
    nav_signal.set(nav);
    selection_signal.set(None);
}

/// The selection a row's key denotes, at the depth its stage renders: the
/// egui `entity_row`'s classification, keyed off the stage instead of a
/// `(section, level)` pair. An album key carries the `artist\u{1f}title`
/// separator — the store's album identity joined — so the row a click
/// selects and the selection the session stores are the same bytes.
#[must_use]
fn selection_at(stage: &StageKind, key: &str) -> Option<(usize, BrowserSelection)> {
    match stage {
        StageKind::ArtistsRoot => Some((0, BrowserSelection::Artist(key.to_owned()))),
        StageKind::AlbumsRoot => {
            let (artist, title) = split_album_key(key)?;
            Some((0, BrowserSelection::Album { artist, title }))
        }
        StageKind::GenresRoot => Some((0, BrowserSelection::Genre(key.to_owned()))),
        StageKind::ArtistAlbums { .. } => {
            let (artist, title) = split_album_key(key)?;
            Some((1, BrowserSelection::Album { artist, title }))
        }
        StageKind::GenreArtists { .. } => Some((1, BrowserSelection::Artist(key.to_owned()))),
        StageKind::GenreArtistAlbums { .. } => {
            let (artist, title) = split_album_key(key)?;
            Some((2, BrowserSelection::Album { artist, title }))
        }
        StageKind::Flat | StageKind::Tracks { .. } | StageKind::Folders => None,
    }
}

/// The store's album identity, joined in the row key, split back apart.
#[must_use]
fn split_album_key(key: &str) -> Option<(String, String)> {
    let (artist, title) = key.split_once('\u{1f}')?;
    Some((artist.to_owned(), title.to_owned()))
}

/// The collection a row's context menu acts on, keyed off the stage: the
/// entity the row denotes, resolved to a track batch at dispatch time.
#[must_use]
fn collection_target(stage: &StageKind, key: &str) -> Option<crate::menus::CollectionTarget> {
    use crate::menus::CollectionTarget;
    match stage {
        StageKind::ArtistsRoot => Some(CollectionTarget::Artist(key.to_owned())),
        StageKind::AlbumsRoot => {
            let (artist, title) = split_album_key(key)?;
            Some(CollectionTarget::Album { artist, title })
        }
        StageKind::GenresRoot => Some(CollectionTarget::Genre(key.to_owned())),
        StageKind::ArtistAlbums { .. } => {
            let (artist, title) = split_album_key(key)?;
            Some(CollectionTarget::Album { artist, title })
        }
        StageKind::GenreArtists { genre } => Some(CollectionTarget::GenreArtist {
            artist: key.to_owned(),
            genre: genre.clone(),
        }),
        StageKind::GenreArtistAlbums { genre, .. } => {
            let (artist, title) = split_album_key(key)?;
            Some(CollectionTarget::GenreAlbum {
                artist,
                title,
                genre: genre.clone(),
            })
        }
        StageKind::Flat | StageKind::Tracks { .. } | StageKind::Folders => None,
    }
}

/// The stage's empty copy: the egui columns' wording, the plain pair for a
/// library that holds none of the entity, the filtered pair for a needle
/// that matched nothing. The stages whose reads ignore the query (the
/// Genres root and the genre drills — hit-scoped genre counts are the egui
/// spec's isolated cut) carry the plain pair for both.
fn entity_copy(stage: &StageKind) -> ((&'static str, &'static str), (&'static str, &'static str)) {
    match stage {
        StageKind::ArtistsRoot => (
            (
                "No artists yet",
                "Add a folder from the sidebar to start scanning your library.",
            ),
            ("No matching artists", "Nothing in your library matches"),
        ),
        StageKind::AlbumsRoot | StageKind::ArtistAlbums { .. } => (
            (
                "No albums yet",
                "Add a folder from the sidebar to start scanning your library.",
            ),
            ("No matching albums", "Nothing in your library matches"),
        ),
        StageKind::GenresRoot => (
            (
                "No genres yet",
                "Genres come from your tracks' tags \u{2014} add music and rescan.",
            ),
            (
                "No genres yet",
                "Genres come from your tracks' tags \u{2014} add music and rescan.",
            ),
        ),
        StageKind::GenreArtists { .. } => (
            (
                "No artists in this genre",
                "This genre has no artists in your library.",
            ),
            (
                "No artists in this genre",
                "This genre has no artists in your library.",
            ),
        ),
        StageKind::GenreArtistAlbums { .. } => (
            (
                "No albums in this genre",
                "This artist has no albums carrying this genre.",
            ),
            (
                "No albums in this genre",
                "This artist has no albums carrying this genre.",
            ),
        ),
        StageKind::Flat | StageKind::Tracks { .. } | StageKind::Folders => (
            ("Nothing here yet", "This selection has nothing to show."),
            ("Nothing here yet", "This selection has nothing to show."),
        ),
    }
}
