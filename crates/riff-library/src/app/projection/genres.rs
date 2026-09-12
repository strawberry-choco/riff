//! The genre read model Session Projection (ADR 0002): the sidebar genre
//! aggregation plus the genre-filtered browsing levels.

use crate::app::store::{GenerationCache, StoreError, StoreGeneration};
use crate::domain::{Album, Artist, GenreCount, Track};
use std::collections::HashMap;
use std::sync::Arc;

/// The lazily-filled levels of the genre read model.
#[derive(Default, Clone)]
struct GenreLevels {
    counts: Option<Arc<[GenreCount]>>,
    artists: HashMap<String, Arc<[Artist]>>,
    albums: HashMap<(String, String), Arc<[Album]>>,
    tracks: HashMap<(String, String, String), Arc<[Track]>>,
}

/// Session Projection for the genre read model (ADR 0002): the genre
/// aggregation for the sidebar plus the genre-filtered browsing levels.
///
/// Caches each level fetched from the store only when missing at the
/// current generation; a generation bump (a committed store mutation) drops
/// every level at once so a frame never mixes rows from two generations.
/// Loader errors propagate and leave the cache untouched — the next call
/// retries.
pub struct GenreProjection {
    /// Generation-keyed slot over the whole level bundle: a moved epoch
    /// drops every level together, within a generation levels fill lazily.
    cache: GenerationCache<(), GenreLevels>,
}

impl Default for GenreProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

/// Loader signature for one artist's genre-filtered albums (factored out
/// for readability).
type GenreAlbumsLoader<'a> = &'a mut dyn FnMut(&str, &str) -> Result<Vec<Album>, StoreError>;

/// Loader signature for one album's genre-filtered tracks (factored out for
/// readability).
type GenreAlbumTracksLoader<'a> =
    &'a mut dyn FnMut(&str, &str, &str) -> Result<Vec<Track>, StoreError>;

impl GenreProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// Every genre with its per-track count, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list — no per-frame
    /// copy.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn counts(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<GenreCount>, StoreError>,
    ) -> Result<Arc<[GenreCount]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache.peek().and_then(|levels| levels.counts.clone())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[GenreCount]> = loader()?.into();
        self.cache.slot(epoch, &()).counts = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// Artists having at least one track with `genre`, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artists_in_genre(
        &mut self,
        genre: &str,
        loader: &mut dyn FnMut(&str) -> Result<Vec<Artist>, StoreError>,
    ) -> Result<Arc<[Artist]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.artists.get(genre).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Artist]> = loader(genre)?.into();
        self.cache
            .slot(epoch, &())
            .artists
            .insert(genre.to_string(), Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One artist's albums holding a track with `genre`, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artist_albums_in_genre(
        &mut self,
        artist: &str,
        genre: &str,
        loader: GenreAlbumsLoader<'_>,
    ) -> Result<Arc<[Album]>, StoreError> {
        let key = (artist.to_string(), genre.to_string());
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.albums.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Album]> = loader(artist, genre)?.into();
        self.cache
            .slot(epoch, &())
            .albums
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One album's tracks with `genre`, cached per generation. Fresh frames
    /// hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn album_tracks_in_genre(
        &mut self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        loader: GenreAlbumTracksLoader<'_>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = (
            album_artist.to_string(),
            album_title.to_string(),
            genre.to_string(),
        );
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.tracks.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Track]> = loader(album_artist, album_title, genre)?.into();
        self.cache
            .slot(epoch, &())
            .tracks
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }
}
