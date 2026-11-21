//! The track context menu: the egui components crate's track menu ported to
//! a native popup. The item order and labels are that renderer's — Play,
//! Play Next, Add to Queue, Add to Playlist, the Favourite flip, Remove from
//! Playlist (playlist rows), Edit Tags and Measure `ReplayGain` (resolved
//! tracks) — with a missing row reduced to what needs no Track.
//!
//! The dispatch is the egui host's: every playback item is one transport
//! call, the playlist items are one durable store call, and the Favourite
//! item writes through the same port method the row's heart does.

use nsh_core::element::desktop::{MenuItem, popup_menu_at};
use nsh_core::prelude::*;
use riff_backend::domain::{PlaylistId, TrackId};

use crate::bridge::UiModel;
use crate::components::track_row::select_track;

use super::{add_to_playlist_items, playlist_choices};

/// Where a right-click landed: the row's identity, the favourite fact its
/// label flips on, whether the row resolved a Track, and the playlist it is
/// an entry of (if any).
#[derive(Debug, Clone)]
pub struct TrackMenuSubject {
    pub id: TrackId,
    pub favorite: bool,
    /// `false` for a dangling playlist entry: no playback, no favourite, no
    /// tag or `ReplayGain` items.
    pub resolved: bool,
    /// `Some` only for a user playlist's row.
    pub playlist: Option<PlaylistId>,
}

impl TrackMenuSubject {
    /// A resolved track row outside any playlist (every Library listing).
    #[must_use]
    pub fn library(id: TrackId, favorite: bool) -> Self {
        Self {
            id,
            favorite,
            resolved: true,
            playlist: None,
        }
    }
}

/// Open the menu at the context event's page position. Opening selects the
/// row first, exactly the egui host's order.
pub fn open_track_menu(model: Stored<UiModel>, subject: TrackMenuSubject, at: Point<Points>) {
    select_track(model, subject.id.clone());
    let items = track_menu_items(model, &subject);
    drop(popup_menu_at(items, at));
}

/// The item list, built at open time so the playlist submenu and the
/// Favourite label reflect the row's live facts.
#[must_use]
pub fn track_menu_items(model: Stored<UiModel>, subject: &TrackMenuSubject) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if subject.resolved {
        let id = subject.id.clone();
        items.push(MenuItem::new("Play").on_select({
            let id = id.clone();
            move || play(model, id.clone())
        }));
        items.push(MenuItem::new("Play Next").on_select({
            let id = id.clone();
            move || play_next(model, id.clone())
        }));
        items.push(MenuItem::new("Add to Queue").on_select({
            let id = id.clone();
            move || add_to_queue(model, id.clone())
        }));
        let choices = model.with(playlist_choices);
        items.push(MenuItem::submenu(
            "Add to Playlist",
            add_to_playlist_items(model, &choices, &id),
        ));
        items.push(MenuItem::separator());
        let favorite_label = if subject.favorite {
            "Remove from Favorites"
        } else {
            "Add to Favorites"
        };
        let favorite = !subject.favorite;
        items.push(
            MenuItem::new(favorite_label)
                .on_select(move || set_favorite(model, id.clone(), favorite)),
        );
    }
    if subject.playlist.is_some() {
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Remove from Playlist").on_select({
            let playlist = subject.playlist.clone();
            let id = subject.id.clone();
            move || {
                if let Some(playlist) = playlist.clone() {
                    remove_from_playlist(model, playlist, id.clone());
                }
            }
        }));
    }
    if subject.resolved {
        items.push(MenuItem::separator());
        let edit_id = subject.id.clone();
        items.push(MenuItem::new("Edit Tags").on_select(move || {
            crate::screens::inspector::tag_editor::begin(model, edit_id.clone());
        }));
        items.push(MenuItem::new("Measure ReplayGain").on_select({
            let id = subject.id.clone();
            move || measure_replaygain(model, id.clone())
        }));
    }
    items
}

pub(crate) fn play(model: Stored<UiModel>, id: TrackId) {
    model.with(|m| m.rt.ui_transport.play(id));
}

pub(crate) fn play_next(model: Stored<UiModel>, id: TrackId) {
    model.with(|m| m.rt.ui_transport.play_next(id));
}

pub(crate) fn add_to_queue(model: Stored<UiModel>, id: TrackId) {
    model.with(|m| m.rt.ui_transport.add_to_queue(id));
}

pub(crate) fn set_favorite(model: Stored<UiModel>, id: TrackId, favorite: bool) {
    model.with(|m| {
        if let Err(e) = m.rt.library_mutations.set_track_favorite(&id, favorite) {
            tracing::warn!("Failed to commit the favorite flag for {}: {e}", id.0);
        }
    });
}

pub(crate) fn remove_from_playlist(model: Stored<UiModel>, playlist: PlaylistId, id: TrackId) {
    model.with(|m| {
        if let Err(e) = m.rt.playlists.remove_playlist_entries(&playlist, &id) {
            tracing::warn!("Failed to remove the track from the playlist: {e}");
        }
    });
}

/// One targeted `ReplayGain` measurement, submitted through the pass service
/// (the egui menu's command). The pass writes the measured pair itself.
pub(crate) fn measure_replaygain(model: Stored<UiModel>, id: TrackId) {
    model.with(|m| {
        if crate::model::reads::selected_track(&mut m.rt.session_views, &id).is_some() {
            m.rt.passes
                .submit(riff_backend::app::replaygain_pass::PassCommand::Track(id));
        }
    });
}
