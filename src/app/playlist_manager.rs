//! Playlist entry validity helpers. User playlists themselves are user data
//! in the Application Store: every mutation commits through the
//! [`crate::app::store::PlaylistStore`] port as one immediate durable
//! transaction.
//!
//! These helpers decide, at read time, which entries can play: an entry is
//! valid when its track is known to the library AND its file still exists.
//! Dangling entries stay listed (flagged invalid in the UI) and resolve again
//! once the referenced files return.

use crate::app::library_manager::LibraryManager;
use crate::domain::{Playlist, TrackId};

// --- Invalid-path handling ----------------------------------------------------

/// A playlist entry is valid when the track is known to the library AND its
/// file still exists on disk. Moved or deleted files make entries invalid —
/// they are flagged in the UI and skipped for playback, never a crash.
pub fn track_is_valid(library: &LibraryManager, id: &TrackId) -> bool {
    library
        .get_track(id)
        .is_some_and(|track| track.file_path.exists())
}

/// The valid entries of `playlist`, in playlist order, ready to be loaded
/// into the playback queue. Invalid entries are skipped.
pub fn valid_tracks(library: &LibraryManager, playlist: &Playlist) -> Vec<TrackId> {
    playlist
        .tracks
        .iter()
        .filter(|id| track_is_valid(library, id))
        .cloned()
        .collect()
}
