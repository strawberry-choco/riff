//! User playlist management (Task 4.2): persistence, CRUD, and invalid-path
//! handling. Playlists are user data and live in their own `playlists.json`,
//! separate from the (rebuildable) library cache — clearing the cache must
//! never destroy playlists.
//!
//! All logic is pure (operating on `Vec<Playlist>`) so it is unit-testable
//! without touching real files; only `save_playlists`/`load_playlists` do IO.

use crate::app::library_manager::LibraryManager;
use crate::domain::{Playlist, PlaylistId, TrackId};
use std::path::PathBuf;

/// Location of the playlists file, mirroring `LibraryManager::cache_path()`
/// but a SEPARATE file in the same data-local directory.
pub fn playlists_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "riff")
        .map(|d| d.data_local_dir().join("playlists.json"))
}

/// Serialize playlists to JSON. Returns an empty string if serialization
/// fails (logged); callers must not write that result.
pub fn serialize_playlists(playlists: &[Playlist]) -> String {
    match serde_json::to_string(playlists) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("Failed to serialize playlists: {e}");
            String::new()
        }
    }
}

/// Parse playlists JSON. Malformed input logs a warning and yields an empty
/// `Vec` — never panics, so a corrupt file degrades to "no playlists" rather
/// than crashing the app.
pub fn deserialize_playlists(json: &str) -> Vec<Playlist> {
    match serde_json::from_str::<Vec<Playlist>>(json) {
        Ok(playlists) => playlists,
        Err(e) => {
            tracing::warn!("Failed to deserialize playlists: {e}");
            Vec::new()
        }
    }
}

/// Persist playlists to `playlists.json` (creating the parent directory),
/// warning and continuing on IO errors like `LibraryManager::save_cache`.
pub fn save_playlists(playlists: &[Playlist]) {
    let Some(path) = playlists_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Failed to create playlists directory: {e}");
            return;
        }
    }
    let json = serialize_playlists(playlists);
    if json.is_empty() {
        // Serialization failed (already logged); don't clobber the file.
        return;
    }
    if let Err(e) = std::fs::write(&path, json) {
        tracing::warn!("Failed to write playlists file: {e}");
    }
}

/// Load playlists from `playlists.json`. A missing file is a normal
/// first-launch state (empty list); unreadable or malformed content falls
/// back to empty via [`deserialize_playlists`].
pub fn load_playlists() -> Vec<Playlist> {
    let Some(path) = playlists_path() else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    let json = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to read playlists file: {e}");
            return Vec::new();
        }
    };
    deserialize_playlists(&json)
}

// --- CRUD (pure, on `Vec<Playlist>`) -----------------------------------------

/// Create a playlist with `name` (and optional initial tracks, deduped while
/// preserving order), append it to `playlists`, and return its id. The id is
/// made unique against existing playlists so same-millisecond creation of
/// same-named playlists cannot collide.
pub fn create_playlist(
    playlists: &mut Vec<Playlist>,
    name: &str,
    initial_tracks: Vec<TrackId>,
) -> PlaylistId {
    let mut id = PlaylistId::new(name);
    let mut suffix = 2;
    while playlists.iter().any(|p| p.id == id) {
        id = PlaylistId(format!("{}-{suffix}", id.0));
        suffix += 1;
    }

    let mut playlist = Playlist::new(id.clone(), name.trim().to_string());
    for track in initial_tracks {
        if !playlist.tracks.contains(&track) {
            playlist.tracks.push(track);
        }
    }
    playlists.push(playlist);
    id
}

/// Rename the playlist with `id`. Returns whether it was found.
pub fn rename_playlist(playlists: &mut [Playlist], id: &PlaylistId, new_name: &str) -> bool {
    match playlists.iter_mut().find(|p| &p.id == id) {
        Some(playlist) => {
            playlist.name = new_name.trim().to_string();
            true
        }
        None => false,
    }
}

/// Delete the playlist with `id`. Returns whether anything was removed.
pub fn delete_playlist(playlists: &mut Vec<Playlist>, id: &PlaylistId) -> bool {
    let before = playlists.len();
    playlists.retain(|p| &p.id != id);
    playlists.len() != before
}

/// Append `track` to the playlist with `id`. Exact duplicates are ignored
/// (returns `false`), as are unknown playlist ids.
pub fn add_track_to_playlist(playlists: &mut [Playlist], id: &PlaylistId, track: TrackId) -> bool {
    match playlists.iter_mut().find(|p| &p.id == id) {
        Some(playlist) => {
            if playlist.tracks.contains(&track) {
                return false;
            }
            playlist.tracks.push(track);
            true
        }
        None => false,
    }
}

/// Remove all occurrences of `track` from the playlist with `id`. Returns
/// whether anything was removed.
pub fn remove_track_from_playlist(
    playlists: &mut [Playlist],
    id: &PlaylistId,
    track: &TrackId,
) -> bool {
    match playlists.iter_mut().find(|p| &p.id == id) {
        Some(playlist) => {
            let before = playlist.tracks.len();
            playlist.tracks.retain(|t| t != track);
            playlist.tracks.len() != before
        }
        None => false,
    }
}

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
