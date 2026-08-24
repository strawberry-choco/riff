//! Playlist entry validity helpers. User playlists themselves are user data
//! in the Application Store: every mutation commits through the
//! [`crate::app::store::PlaylistStore`] port as one immediate durable
//! transaction, and [`crate::app::store::PlaylistStore::load_playlist_entries`]
//! resolves each entry together with its Library validity (SQL LEFT JOIN).
//!
//! These helpers decide, at read time, which entries can play: an entry is
//! valid when its track is known to the Application Store's Library AND its
//! file still exists on disk. Dangling entries stay listed (flagged invalid
//! in the UI) and resolve again once the referenced files return.

use crate::app::store::PlaylistEntry;
use crate::domain::TrackId;

// --- Invalid-path handling ----------------------------------------------------

/// A playlist entry is valid when the track is known to the Application
/// Store's Library (the store's LEFT JOIN validity flag) AND its file still
/// exists on disk. Moved or deleted files make entries invalid — they are
/// flagged in the UI and skipped for playback, never a crash.
pub fn track_is_valid(entry: &PlaylistEntry) -> bool {
    entry
        .track
        .as_ref()
        .is_some_and(|track| track.file_path.exists())
}

/// The playable entries of a resolved playlist-entry list, in playlist
/// order, ready to be loaded into the playback queue. Invalid entries are
/// skipped.
pub fn valid_tracks(entries: &[PlaylistEntry]) -> Vec<TrackId> {
    entries
        .iter()
        .filter(|entry| track_is_valid(entry))
        .map(|entry| entry.id.clone())
        .collect()
}

// --- Drag-reorder math ----------------------------------------------------------

/// The new entry order after a drag-and-drop gesture: remove the entry at
/// `from` and reinsert it at `to`, shifting everything between to close and
/// open the gaps. Returns `None` when either index is out of bounds or the
/// entry was dropped back onto its own slot (a no-op), so callers never
/// commit an empty gesture.
#[must_use]
pub fn reorder_tracks(tracks: &[TrackId], from: usize, to: usize) -> Option<Vec<TrackId>> {
    if from == to || from >= tracks.len() || to >= tracks.len() {
        return None;
    }
    let mut reordered = tracks.to_vec();
    let dragged = reordered.remove(from);
    reordered.insert(to, dragged);
    Some(reordered)
}
