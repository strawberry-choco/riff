//! Context menus: the port of the egui crate's `track_menu` and
//! `collection_menu` renderers onto nsh's native popup menus. The items,
//! their order and their wording are the components crate's (one shared
//! list per surface); the dispatch is the app layer's, exactly the egui
//! `apply_track_menu` / `apply_list_menu_intent` calls.
//!
//! A menu opens through [`popup_menu_at`] at the context event's
//! `page_position`, so the platform renders and keyboard-drives it. The
//! handlers run under the caller's stored borrow, on the engine thread.

pub mod collection;
pub mod track;

use std::path::PathBuf;

use nsh_core::element::desktop::MenuItem;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::views::SessionViews;
use riff_backend::domain::{PlaylistId, SmartPlaylistKind, TrackId};

use crate::bridge::UiModel;
use crate::model::reads;

/// The set of tracks one collection menu acts on, resolved at dispatch time
/// through the read seam (the egui `entity_track_ids` walk).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionTarget {
    /// An artists-root or drill artist row.
    Artist(String),
    /// An album row, by the store's `(album artist, title)` identity.
    Album { artist: String, title: String },
    /// A genre row, genre-scoped end to end.
    Genre(String),
    /// An artist row inside a genre: only the genre-scoped albums.
    GenreArtist { artist: String, genre: String },
    /// An album row inside a genre: only the genre-scoped tracks.
    GenreAlbum {
        artist: String,
        title: String,
        genre: String,
    },
    /// A folder tree node's subtree.
    Folder(PathBuf),
    /// A user playlist's valid entries.
    Playlist(PlaylistId),
    /// A built-in smart list's tracks, under the list's own bounded limit.
    SmartList(SmartPlaylistKind),
    /// A custom smart playlist's matching tracks, read by row range.
    CustomList(PlaylistId),
}

impl CollectionTarget {
    /// The target's track ids, in listing order. A stale target (the store
    /// no longer holds it) answers empty, and an empty batch starts nothing.
    #[must_use]
    pub fn track_ids(&self, views: &mut SessionViews) -> Vec<TrackId> {
        match self {
            CollectionTarget::Artist(artist) => views
                .artist_albums(artist)
                .iter()
                .flat_map(|album| album.tracks.iter().cloned())
                .collect(),
            CollectionTarget::Album { artist, title } => views
                .album_tracks(artist, title)
                .iter()
                .map(|track| track.id.clone())
                .collect(),
            CollectionTarget::Genre(genre) => {
                let mut ids = Vec::new();
                for artist in views.artists_in_genre(genre).iter() {
                    for album in views.artist_albums_in_genre(&artist.name, genre).iter() {
                        for track in views
                            .album_tracks_in_genre(&album.artist, &album.title, genre)
                            .iter()
                        {
                            ids.push(track.id.clone());
                        }
                    }
                }
                ids
            }
            CollectionTarget::GenreArtist { artist, genre } => {
                let mut ids = Vec::new();
                for album in views.artist_albums_in_genre(artist, genre).iter() {
                    for track in views
                        .album_tracks_in_genre(&album.artist, &album.title, genre)
                        .iter()
                    {
                        ids.push(track.id.clone());
                    }
                }
                ids
            }
            CollectionTarget::GenreAlbum {
                artist,
                title,
                genre,
            } => views
                .album_tracks_in_genre(artist, title, genre)
                .iter()
                .map(|track| track.id.clone())
                .collect(),
            CollectionTarget::Folder(folder) => views.folder_subtree_ids(folder).to_vec(),
            CollectionTarget::Playlist(id) => views
                .playlist_view(id)
                .map_or_else(Vec::new, |view| view.valid_ids.to_vec()),
            CollectionTarget::SmartList(kind) => views
                .smart_list(*kind, smart_list_limit(*kind))
                .iter()
                .map(|track| track.id.clone())
                .collect(),
            CollectionTarget::CustomList(id) => {
                let total = views
                    .custom_list_count(id, riff_backend::app::state::TrackSort::NumberAsc, None)
                    .unwrap_or_default();
                (0..total)
                    .filter_map(|at| {
                        views
                            .custom_list_row(
                                id,
                                riff_backend::app::state::TrackSort::NumberAsc,
                                None,
                                at,
                            )
                            .ok()
                            .flatten()
                    })
                    .map(|track| track.id.clone())
                    .collect()
            }
        }
    }
}

/// The bounded lists cap at 50 entries; open-ended ones list all (the egui
/// smart-list listing's own limit choice).
#[must_use]
pub fn smart_list_limit(kind: SmartPlaylistKind) -> usize {
    match kind {
        SmartPlaylistKind::RecentlyAdded
        | SmartPlaylistKind::MostPlayed
        | SmartPlaylistKind::RecentlyPlayed => 50,
        SmartPlaylistKind::Favorites
        | SmartPlaylistKind::NeverPlayed
        | SmartPlaylistKind::LostGems => usize::MAX,
    }
}

/// The shared set-of-tracks batch dispatch, the egui `apply_list_menu_intent`
/// ported: Play is the one `play_many`, Play Next queues in reverse so the
/// first track ends up next, Add to Queue is one `AddMany`, and Shuffle
/// engages shuffle then plays the same batch.
pub(crate) fn play_batch(model: Stored<UiModel>, ids: Vec<TrackId>) {
    let Some(first) = ids.first().cloned() else {
        return;
    };
    let rest = ids[1..].to_vec();
    model.with(|m| m.rt.ui_transport.play_many(first, rest));
}

pub(crate) fn play_next_batch(model: Stored<UiModel>, ids: &[TrackId]) {
    model.with(|m| {
        for id in ids.iter().rev() {
            m.rt.ui_transport.play_next(id.clone());
        }
    });
}

pub(crate) fn add_batch_to_queue(model: Stored<UiModel>, ids: Vec<TrackId>) {
    if ids.is_empty() {
        return;
    }
    model.with(|m| m.rt.ui_transport.add_many(ids));
}

pub(crate) fn shuffle_batch(model: Stored<UiModel>, ids: Vec<TrackId>) {
    if ids.is_empty() {
        return;
    }
    model.with(|m| {
        m.rt.playback.lock_or_recover().queue.set_shuffle(true);
    });
    play_batch(model, ids);
}

/// The batch a target names, read at dispatch time.
pub(crate) fn target_ids(model: &mut UiModel, target: &CollectionTarget) -> Vec<TrackId> {
    target.track_ids(&mut model.rt.session_views)
}

/// One playlist row's summary for the Add-to-Playlist submenu.
#[derive(Debug, Clone)]
pub(crate) struct PlaylistChoice {
    pub id: PlaylistId,
    pub name: String,
}

/// Every user playlist, in creation order — read through the seam so a menu
/// opened between pump ticks sees a fresh list.
pub(crate) fn playlist_choices(model: &mut UiModel) -> Vec<PlaylistChoice> {
    reads::playlists(&mut model.rt.session_views)
        .iter()
        .map(|playlist| PlaylistChoice {
            id: playlist.id.clone(),
            name: playlist.name.clone(),
        })
        .collect()
}

/// The "Add to Playlist" submenu: one row per playlist, or the inert line
/// that says there are none to pick.
pub(crate) fn add_to_playlist_items(
    model: Stored<UiModel>,
    choices: &[PlaylistChoice],
    track: &TrackId,
) -> Vec<MenuItem> {
    if choices.is_empty() {
        return vec![MenuItem::new("No playlists yet").disabled(true)];
    }
    choices
        .iter()
        .map(|choice| {
            let playlist = choice.id.clone();
            let track = track.clone();
            MenuItem::new(choice.name.clone()).on_select(move || {
                model.with(|m| {
                    if let Err(e) = m
                        .rt
                        .playlists
                        .add_playlist_entry(&playlist, &track)
                    {
                        tracing::warn!("Failed to add the track to the playlist: {e}");
                    }
                });
            })
        })
        .collect()
}
