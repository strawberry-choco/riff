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
///
/// Every level here declares itself on [`GenerationCache::level`], so this
/// projection spells out no freshness rule of its own.
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
        self.cache.level(
            &(),
            |levels| levels.counts.clone(),
            || loader().map(Arc::from),
            |levels, answer| {
                levels.counts = Some(Arc::clone(&answer));
                answer
            },
        )
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
        self.cache.level(
            &(),
            |levels| levels.artists.get(genre).cloned(),
            || loader(genre).map(Arc::from),
            |levels, answer| {
                levels
                    .artists
                    .insert(genre.to_string(), Arc::clone(&answer));
                answer
            },
        )
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
        self.cache.level(
            &(),
            |levels| levels.albums.get(&key).cloned(),
            || loader(artist, genre).map(Arc::from),
            |levels, answer| {
                levels.albums.insert(key.clone(), Arc::clone(&answer));
                answer
            },
        )
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
        self.cache.level(
            &(),
            |levels| levels.tracks.get(&key).cloned(),
            || loader(album_artist, album_title, genre).map(Arc::from),
            |levels, answer| {
                levels.tracks.insert(key.clone(), Arc::clone(&answer));
                answer
            },
        )
    }
}
