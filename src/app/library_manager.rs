use crate::app::traits::MetadataReader;
use crate::domain::{Album, Artist, SmartPlaylistKind, Track, TrackId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Age threshold for "Lost Gems": tracks whose last play is older than this
/// are considered forgotten gems worth resurfacing.
const LOST_GEMS_THRESHOLD: Duration = Duration::from_hours(2160);

/// Current on-disk library cache schema version. Bump when the serialized
/// shape changes incompatibly; caches with any other version are discarded
/// (with a logged warning) and rebuilt on the next scan.
///
/// NOTE: the first versioned release triggers a one-time rescan: caches
/// written before versioning lack `schema_version`, which serde defaults to
/// 0, so they are rejected by [`LibraryManager::deserialize_cache`]. The
/// cache is a derived, fully rebuildable view of the music files, so this is
/// safe and self-healing.
pub const CACHE_SCHEMA_VERSION: u32 = 1;

/// Manages the music library: scanning, indexing, metadata, and search.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LibraryManager {
    pub tracks: HashMap<TrackId, Track>,
    pub artists: HashMap<String, Artist>,
    pub albums: HashMap<String, Album>,
}

/// Versioned serialization envelope for the library cache. Keeps
/// `schema_version` in the JSON without polluting `LibraryManager`'s logical
/// fields.
#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEnvelope {
    /// Absent field deserializes to 0 (serde default), so pre-versioning
    /// caches are recognized as "old" and rejected deliberately.
    #[serde(default)]
    schema_version: u32,
    tracks: HashMap<TrackId, Track>,
    artists: HashMap<String, Artist>,
    albums: HashMap<String, Album>,
}

impl From<&LibraryManager> for CacheEnvelope {
    fn from(lib: &LibraryManager) -> Self {
        Self {
            schema_version: CACHE_SCHEMA_VERSION,
            tracks: lib.tracks.clone(),
            artists: lib.artists.clone(),
            albums: lib.albums.clone(),
        }
    }
}

impl From<CacheEnvelope> for LibraryManager {
    fn from(envelope: CacheEnvelope) -> Self {
        Self {
            tracks: envelope.tracks,
            artists: envelope.artists,
            albums: envelope.albums,
        }
    }
}

impl Default for LibraryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LibraryManager {
    pub fn new() -> Self {
        Self {
            tracks: HashMap::new(),
            artists: HashMap::new(),
            albums: HashMap::new(),
        }
    }

    /// Scan the given paths and add any new tracks. Per-file metadata read
    /// failures are logged and skipped — scanning never aborts on one bad
    /// file, so the result is simply the number of newly added tracks.
    pub fn scan_and_add_tracks(
        &mut self,
        paths: Vec<PathBuf>,
        reader: &dyn MetadataReader,
    ) -> usize {
        let mut added = 0;

        for path in paths {
            let id = TrackId::from_path(&path);
            if self.tracks.contains_key(&id) {
                continue;
            }

            match reader.read_all(&path) {
                Ok((metadata, duration, _cover_source, audio_format)) => {
                    let track = Track {
                        id,
                        file_path: path,
                        metadata,
                        duration,
                        sample_rate: Some(audio_format.sample_rate),
                        channels: Some(audio_format.channels),
                        play_count: 0,
                        last_played: None,
                        // Stamp first-add time once, at scan time. This (not
                        // the file mtime) drives "Recently Added".
                        date_added: Some(SystemTime::now()),
                    };
                    self.add_track(track);
                    added += 1;
                }
                Err(e) => {
                    tracing::error!("Failed to read metadata for {:?}: {}", path, e);
                }
            }
        }

        added
    }

    pub fn add_track(&mut self, track: Track) {
        let album_key = format!(
            "{} - {}",
            track.metadata.display_album_artist(),
            track.metadata.display_album()
        );
        let artist_name = track.metadata.display_album_artist();

        if !self.artists.contains_key(&artist_name) {
            self.artists.insert(
                artist_name.clone(),
                Artist {
                    name: artist_name.clone(),
                    albums: Vec::new(),
                },
            );
        }

        if let Some(artist) = self.artists.get_mut(&artist_name) {
            if !artist.albums.contains(&album_key) {
                artist.albums.push(album_key.clone());
            }
        }

        if !self.albums.contains_key(&album_key) {
            self.albums.insert(
                album_key.clone(),
                Album {
                    title: track.metadata.display_album(),
                    artist: artist_name.clone(),
                    tracks: Vec::new(),
                    year: track.metadata.year,
                    genre: track.metadata.genre.clone(),
                },
            );
        }

        if let Some(album) = self.albums.get_mut(&album_key) {
            if !album.tracks.contains(&track.id) {
                album.tracks.push(track.id.clone());
                album.tracks.sort_by_key(|id| {
                    self.tracks
                        .get(id)
                        .and_then(|t| t.metadata.track_number)
                        .unwrap_or(0)
                });
            }
        }

        self.tracks.insert(track.id.clone(), track);
    }

    pub fn remove_track(&mut self, id: &TrackId) {
        if let Some(track) = self.tracks.remove(id) {
            let album_key = format!(
                "{} - {}",
                track.metadata.display_album_artist(),
                track.metadata.display_album()
            );
            if let Some(album) = self.albums.get_mut(&album_key) {
                album.tracks.retain(|tid| tid != id);
                if album.tracks.is_empty() {
                    self.albums.remove(&album_key);
                    if let Some(artist) =
                        self.artists.get_mut(&track.metadata.display_album_artist())
                    {
                        artist.albums.retain(|a| a != &album_key);
                    }
                }
            }
        }
    }

    /// Remove all tracks whose `file_path` starts with the given root,
    /// and clean up orphaned artists/albums.
    pub fn remove_tracks_by_root(&mut self, root: &Path) -> usize {
        let ids_to_remove: Vec<TrackId> = self
            .tracks
            .iter()
            .filter(|(_, t)| t.file_path.starts_with(root))
            .map(|(id, _)| id.clone())
            .collect();
        let count = ids_to_remove.len();
        for id in ids_to_remove {
            self.remove_track(&id);
        }
        count
    }

    pub fn tracks_in_folder(&self, folder: &Path) -> Vec<&Track> {
        let mut tracks: Vec<&Track> = self
            .tracks
            .values()
            .filter(|t| t.file_path.parent() == Some(folder))
            .collect();
        tracks.sort_by(|a, b| {
            a.metadata
                .track_number
                .unwrap_or(0)
                .cmp(&b.metadata.track_number.unwrap_or(0))
                .then_with(|| a.file_path.file_name().cmp(&b.file_path.file_name()))
        });
        tracks
    }

    pub fn subdirs_with_audio(&self, folder: &Path) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for track in self.tracks.values() {
            let track_path = &track.file_path;
            if !track_path.starts_with(folder) {
                continue;
            }
            let Ok(relative) = track_path.strip_prefix(folder) else {
                continue;
            };
            if let Some(first_component) = relative.iter().next() {
                let child_dir = folder.join(first_component);
                if child_dir.is_dir() && seen.insert(child_dir.clone()) {
                    dirs.push(child_dir);
                }
            }
        }
        dirs.sort();
        dirs
    }

    pub fn folder_has_audio(&self, folder: &Path) -> bool {
        self.tracks
            .values()
            .any(|t| t.file_path.starts_with(folder))
    }

    pub fn track_ids_in_folder_tree(&self, folder: &Path) -> Vec<TrackId> {
        let mut ids: Vec<TrackId> = self
            .tracks
            .values()
            .filter(|t| t.file_path.starts_with(folder))
            .map(|t| t.id.clone())
            .collect();
        ids.sort_by(|a, b| {
            let path_a = self.tracks.get(a).map(|t| &t.file_path);
            let path_b = self.tracks.get(b).map(|t| &t.file_path);
            path_a.cmp(&path_b)
        });
        ids
    }

    pub fn search(&self, query: &str) -> Vec<&Track> {
        let query_lower = query.to_lowercase();
        self.tracks
            .values()
            .filter(|track| track.metadata.search_text().contains(&query_lower))
            .collect()
    }

    pub fn get_track(&self, id: &TrackId) -> Option<&Track> {
        self.tracks.get(id)
    }

    pub fn get_album_tracks(&self, album_key: &str) -> Vec<&Track> {
        self.albums
            .get(album_key)
            .map(|album| {
                album
                    .tracks
                    .iter()
                    .filter_map(|id| self.tracks.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_artist_albums(&self, artist_name: &str) -> Vec<&Album> {
        self.artists
            .get(artist_name)
            .map(|artist| {
                artist
                    .albums
                    .iter()
                    .filter_map(|key| self.albums.get(key))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn all_artists(&self) -> Vec<&Artist> {
        self.artists.values().collect()
    }

    pub fn all_albums(&self) -> Vec<&Album> {
        self.albums.values().collect()
    }

    pub fn all_tracks(&self) -> Vec<&Track> {
        self.tracks.values().collect()
    }

    /// Record one completed play for a track: bumps `play_count` and stamps
    /// `last_played` with the current time. Unknown ids are a no-op.
    pub fn increment_play_count(&mut self, id: &TrackId) {
        if let Some(track) = self.tracks.get_mut(id) {
            track.play_count += 1;
            track.last_played = Some(SystemTime::now());
        }
    }

    /// Compute a read-only smart playlist on demand from local library data.
    ///
    /// Smart playlists are virtual: they are never stored, never editable,
    /// and re-derived on every call so they always reflect current play
    /// history. `limit` caps the number of returned tracks.
    pub fn smart_playlist(&self, kind: SmartPlaylistKind, limit: usize) -> Vec<TrackId> {
        let mut selected: Vec<&Track> = match kind {
            SmartPlaylistKind::RecentlyAdded => {
                // NOTE: sorted by the stored `date_added`, deliberately NOT
                // the filesystem mtime — mtime changes whenever tags are
                // edited, which would corrupt "recently added". `date_added`
                // is stamped once when the track is first scanned.
                let mut dated: Vec<(SystemTime, &Track)> = self
                    .tracks
                    .values()
                    .filter_map(|t| t.date_added.map(|added| (added, t)))
                    .collect();
                dated.sort_by(|(added_a, a), (added_b, b)| {
                    added_b
                        .cmp(added_a) // newest first
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                dated.into_iter().map(|(_, t)| t).collect()
            }
            SmartPlaylistKind::MostPlayed => {
                let mut played: Vec<&Track> =
                    self.tracks.values().filter(|t| t.play_count > 0).collect();
                played.sort_by(|a, b| {
                    b.play_count
                        .cmp(&a.play_count) // highest first
                        .then_with(|| {
                            a.metadata
                                .display_title(&a.file_path)
                                .cmp(&b.metadata.display_title(&b.file_path))
                        })
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                played
            }
            SmartPlaylistKind::NeverPlayed => {
                let mut unplayed: Vec<&Track> =
                    self.tracks.values().filter(|t| t.play_count == 0).collect();
                unplayed.sort_by(|a, b| a.file_path.cmp(&b.file_path));
                unplayed
            }
            SmartPlaylistKind::LostGems => {
                // Played before, but not within the last 90 days. Tracks that
                // were never played are excluded (they belong to NeverPlayed).
                let mut gems: Vec<(SystemTime, &Track)> = self
                    .tracks
                    .values()
                    .filter_map(|t| t.last_played.map(|last| (last, t)))
                    .filter(|(last, _)| match last.elapsed() {
                        Ok(age) => age > LOST_GEMS_THRESHOLD,
                        // A clock anomaly (last_played in the future) counts
                        // as "very old" rather than excluding the track.
                        Err(_) => true,
                    })
                    .collect();
                gems.sort_by(|(last_a, a), (last_b, b)| {
                    last_a
                        .cmp(last_b) // longest-unplayed first
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                gems.into_iter().map(|(_, t)| t).collect()
            }
        };
        selected.truncate(limit);
        selected.into_iter().map(|t| t.id.clone()).collect()
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.artists.clear();
        self.albums.clear();
    }

    /// Location of the library cache file, if the platform provides a local
    /// data directory.
    pub fn cache_path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("", "", "riff")
            .map(|d| d.data_local_dir().join("library_cache.json"))
    }

    /// Serialize this library into the versioned cache JSON format (an
    /// envelope carrying `schema_version`). Returns an empty string if
    /// serialization fails (logged); callers must not write that result.
    pub fn serialize_cache(&self) -> String {
        match serde_json::to_string(&CacheEnvelope::from(self)) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("Failed to serialize library cache: {e}");
                String::new()
            }
        }
    }

    /// Parse versioned cache JSON into a `LibraryManager`. Falls back to an
    /// empty library on malformed JSON or on a schema version mismatch —
    /// including pre-versioning caches, whose absent `schema_version` serde
    /// defaults to 0. Both fallbacks are logged so they are explainable to
    /// users rather than silent. Never panics on malformed/old caches.
    pub fn deserialize_cache(json: &str) -> LibraryManager {
        match serde_json::from_str::<CacheEnvelope>(json) {
            Ok(envelope) if envelope.schema_version == CACHE_SCHEMA_VERSION => envelope.into(),
            Ok(envelope) => {
                tracing::warn!(
                    "Library cache schema version mismatch: found {}, expected {}; \
                     discarding the cache (it will be rebuilt on the next scan)",
                    envelope.schema_version,
                    CACHE_SCHEMA_VERSION
                );
                LibraryManager::new()
            }
            Err(e) => {
                tracing::warn!("Failed to deserialize library cache: {e}");
                LibraryManager::new()
            }
        }
    }

    pub fn save_cache(&self) {
        let Some(path) = Self::cache_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!("Failed to create cache directory: {e}");
                return;
            }
        }
        let json = self.serialize_cache();
        if json.is_empty() {
            // Serialization failed (already logged); don't clobber the cache.
            return;
        }
        if let Err(e) = std::fs::write(&path, json) {
            tracing::warn!("Failed to write library cache: {e}");
        }
    }

    pub fn load_cache() -> Self {
        let Some(path) = Self::cache_path() else {
            return Self::new();
        };
        if !path.exists() {
            return Self::new();
        }
        let json = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to read library cache: {e}");
                return Self::new();
            }
        };
        Self::deserialize_cache(&json)
    }

    /// Delete the library cache file if present. Returns whether a file was
    /// removed; a missing file is not an error. The cache is rebuildable, so
    /// deletion is always safe.
    pub fn delete_cache_file() -> bool {
        let Some(path) = Self::cache_path() else {
            return false;
        };
        match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                tracing::warn!("Failed to delete library cache {:?}: {e}", path);
                false
            }
        }
    }
}
