use std::collections::HashMap;
use std::path::{Path, PathBuf};
use crate::app::errors::AppError;
use crate::app::traits::MetadataReader;
use crate::domain::{Track, TrackId, Artist, Album};

/// Manages the music library: scanning, indexing, metadata, and search.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LibraryManager {
    pub tracks: HashMap<TrackId, Track>,
    pub artists: HashMap<String, Artist>,
    pub albums: HashMap<String, Album>,
}

impl LibraryManager {
    pub fn new() -> Self {
        Self {
            tracks: HashMap::new(),
            artists: HashMap::new(),
            albums: HashMap::new(),
        }
    }

    pub fn scan_and_add_tracks(
        &mut self,
        paths: Vec<PathBuf>,
        reader: &dyn MetadataReader,
    ) -> Result<usize, AppError> {
        let mut added = 0;

        for path in paths {
            let id = TrackId::from_path(&path);
            if self.tracks.contains_key(&id) {
                continue;
            }

            match reader.read_metadata(&path) {
                Ok(metadata) => {
                    let duration = reader.read_duration(&path).unwrap_or(None);
                    let track = Track {
                        id,
                        file_path: path,
                        metadata,
                        duration,
                        sample_rate: None,
                        channels: None,
                    };
                    self.add_track(track);
                    added += 1;
                }
                Err(e) => {
                    tracing::error!("Failed to read metadata for {:?}: {}", path, e);
                }
            }
        }

        Ok(added)
    }

    pub fn add_track(&mut self, track: Track) {
        let album_key = format!("{} - {}", track.metadata.display_album_artist(), track.metadata.display_album());
        let artist_name = track.metadata.display_album_artist();

        if !self.artists.contains_key(&artist_name) {
            self.artists.insert(artist_name.clone(), Artist {
                name: artist_name.clone(),
                albums: Vec::new(),
            });
        }

        if let Some(artist) = self.artists.get_mut(&artist_name) {
            if !artist.albums.contains(&album_key) {
                artist.albums.push(album_key.clone());
            }
        }

        if !self.albums.contains_key(&album_key) {
            self.albums.insert(album_key.clone(), Album {
                title: track.metadata.display_album(),
                artist: artist_name.clone(),
                tracks: Vec::new(),
                year: track.metadata.year,
                genre: track.metadata.genre.clone(),
            });
        }

        if let Some(album) = self.albums.get_mut(&album_key) {
            if !album.tracks.contains(&track.id) {
                album.tracks.push(track.id.clone());
                album.tracks.sort_by_key(|id| {
                    self.tracks.get(id).and_then(|t| t.metadata.track_number).unwrap_or(0)
                });
            }
        }

        self.tracks.insert(track.id.clone(), track);
    }

    pub fn remove_track(&mut self, id: &TrackId) {
        if let Some(track) = self.tracks.remove(id) {
            let album_key = format!("{} - {}", track.metadata.display_album_artist(), track.metadata.display_album());
            if let Some(album) = self.albums.get_mut(&album_key) {
                album.tracks.retain(|tid| tid != id);
                if album.tracks.is_empty() {
                    self.albums.remove(&album_key);
                    if let Some(artist) = self.artists.get_mut(&track.metadata.display_album_artist()) {
                        artist.albums.retain(|a| a != &album_key);
                    }
                }
            }
        }
    }

    /// Remove all tracks whose file_path starts with the given root,
    /// and clean up orphaned artists/albums.
    pub fn remove_tracks_by_root(&mut self, root: &Path) -> usize {
        let ids_to_remove: Vec<TrackId> = self.tracks
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
                album.tracks.iter()
                    .filter_map(|id| self.tracks.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_artist_albums(&self, artist_name: &str) -> Vec<&Album> {
        self.artists
            .get(artist_name)
            .map(|artist| {
                artist.albums.iter()
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

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.artists.clear();
        self.albums.clear();
    }

    fn cache_path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("", "", "riff")
            .map(|d| d.data_local_dir().join("library_cache.json"))
    }

    pub fn save_cache(&self) {
        let path = match Self::cache_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!("Failed to create cache directory: {e}");
                return;
            }
        }
        let json = match serde_json::to_string(self) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize library cache: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, json) {
            tracing::warn!("Failed to write library cache: {e}");
        }
    }

    pub fn load_cache() -> Self {
        let path = match Self::cache_path() {
            Some(p) => p,
            None => return Self::new(),
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
        match serde_json::from_str(&json) {
            Ok(lib) => lib,
            Err(e) => {
                tracing::warn!("Failed to deserialize library cache: {e}");
                Self::new()
            }
        }
    }
}
