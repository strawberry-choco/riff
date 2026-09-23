//! The scoped-hit Session Projection (ADR 0002): the non-paged entity hit
//! reads — an album's hit tracks, the album name-hit boolean, hit albums and
//! artists within a genre, and the hit-scoped genre counts — cached per
//! generation like the browsing/genre projections.

use crate::app::store::{GenerationCache, StoreError, StoreGeneration};
use crate::domain::{Album, Artist, GenreCount, Track};
use std::collections::HashMap;
use std::sync::Arc;

/// The lazily-filled levels of the scoped hit read model.
#[derive(Default, Clone)]
struct HitLevels {
    album_tracks: HashMap<(String, String, String), Arc<[Track]>>,
    genre_album_tracks: HashMap<(String, String, String, String), Arc<[Track]>>,
    album_name_hits: HashMap<(String, String, String), bool>,
    genre_albums: HashMap<(String, String), Arc<[Album]>>,
    genre_artists: HashMap<(String, String), Arc<[Artist]>>,
    genre_counts: HashMap<String, Arc<[GenreCount]>>,
}

/// Loader for one album's hit tracks (factored out for readability).
type AlbumHitTracksLoader<'a> =
    &'a mut dyn FnMut(&str, &str, &str) -> Result<Vec<Track>, StoreError>;

/// Loader for one album's genre-scoped hit tracks (factored out for
/// readability).
type GenreAlbumHitTracksLoader<'a> =
    &'a mut dyn FnMut(&str, &str, &str, &str) -> Result<Vec<Track>, StoreError>;

/// Loader for the album name-hit boolean (factored out for readability).
type AlbumNameHitLoader<'a> = &'a mut dyn FnMut(&str, &str, &str) -> Result<bool, StoreError>;

/// Loader for one genre-scoped hit listing — albums or artists (factored
/// out for readability).
type GenreHitListLoader<'a, T> = &'a mut dyn FnMut(&str, &str) -> Result<Vec<T>, StoreError>;

/// Loader for the hit-scoped genre counts (factored out for readability).
type GenreCountsLoader<'a> = &'a mut dyn FnMut(&str) -> Result<Vec<GenreCount>, StoreError>;

/// Session Projection for the scoped hit reads (ADR 0002): every non-paged
/// entity hit view — what a hit album's Tracks column, the drill-through
/// boolean, the genre-scoped listing columns, and the Genres root under a
/// query render.
///
/// Caches each level fetched from the store only when missing at the
/// current generation; a generation bump (a committed store mutation) drops
/// every level at once so a frame never mixes rows from two generations.
/// Levels are keyed by their full query signature (album + query, genre +
/// query, …), so a keystroke refetches without waiting for the store. Loader
/// errors propagate and leave the cache untouched — the next call retries.
///
/// Every level here declares itself on [`GenerationCache::level`], so this
/// projection spells out no freshness rule of its own.
pub struct HitProjection {
    /// Generation-keyed slot over the whole level bundle: a moved epoch
    /// drops every level together, within a generation levels fill lazily.
    cache: GenerationCache<(), HitLevels>,
}

impl Default for HitProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl HitProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// One album's tracks that match `query`, in canonical album-track
    /// order, cached per (album, query) per generation. Fresh frames hand
    /// out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn album_hit_tracks(
        &mut self,
        album_artist: &str,
        album_title: &str,
        query: &str,
        loader: AlbumHitTracksLoader<'_>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = (
            album_artist.to_string(),
            album_title.to_string(),
            query.to_string(),
        );
        self.cache.level(
            &(),
            |levels| levels.album_tracks.get(&key).cloned(),
            || loader(album_artist, album_title, query).map(Arc::from),
            |levels, answer| {
                levels.album_tracks.insert(key.clone(), Arc::clone(&answer));
                answer
            },
        )
    }

    /// One album's tracks that match `query` among its `genre`-bearing
    /// tracks, in canonical album-track order, cached per
    /// (album, genre, query) per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn genre_album_tracks(
        &mut self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        query: &str,
        loader: GenreAlbumHitTracksLoader<'_>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = (
            album_artist.to_string(),
            album_title.to_string(),
            genre.to_string(),
            query.to_string(),
        );
        self.cache.level(
            &(),
            |levels| levels.genre_album_tracks.get(&key).cloned(),
            || loader(album_artist, album_title, genre, query).map(Arc::from),
            |levels, answer| {
                levels
                    .genre_album_tracks
                    .insert(key.clone(), Arc::clone(&answer));
                answer
            },
        )
    }

    /// Whether `album` is itself a name hit for `query`, cached per
    /// (album, query) per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn album_is_name_hit(
        &mut self,
        album_artist: &str,
        album_title: &str,
        query: &str,
        loader: AlbumNameHitLoader<'_>,
    ) -> Result<bool, StoreError> {
        let key = (
            album_artist.to_string(),
            album_title.to_string(),
            query.to_string(),
        );
        self.cache.level(
            &(),
            |levels| levels.album_name_hits.get(&key).copied(),
            || loader(album_artist, album_title, query),
            |levels, answer| {
                levels.album_name_hits.insert(key.clone(), answer);
                answer
            },
        )
    }

    /// The hit albums within `genre` for `query`, in canonical browsing
    /// order, cached per (genre, query) per generation. The loader assembles
    /// the full list; bounded store windows are the loader's concern.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn genre_albums(
        &mut self,
        genre: &str,
        query: &str,
        loader: GenreHitListLoader<'_, Album>,
    ) -> Result<Arc<[Album]>, StoreError> {
        let key = (genre.to_string(), query.to_string());
        self.cache.level(
            &(),
            |levels| levels.genre_albums.get(&key).cloned(),
            || loader(genre, query).map(Arc::from),
            |levels, answer| {
                levels.genre_albums.insert(key.clone(), Arc::clone(&answer));
                answer
            },
        )
    }

    /// The hit artists within `genre` for `query`, name-ascending, cached
    /// per (genre, query) per generation. The loader assembles the full
    /// list; bounded store windows are the loader's concern.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn genre_artists(
        &mut self,
        genre: &str,
        query: &str,
        loader: GenreHitListLoader<'_, Artist>,
    ) -> Result<Arc<[Artist]>, StoreError> {
        let key = (genre.to_string(), query.to_string());
        self.cache.level(
            &(),
            |levels| levels.genre_artists.get(&key).cloned(),
            || loader(genre, query).map(Arc::from),
            |levels, answer| {
                levels
                    .genre_artists
                    .insert(key.clone(), Arc::clone(&answer));
                answer
            },
        )
    }

    /// Every genre containing at least one hit track, with its hit-track
    /// count, cached per query per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn genre_counts(
        &mut self,
        query: &str,
        loader: GenreCountsLoader<'_>,
    ) -> Result<Arc<[GenreCount]>, StoreError> {
        let key = query.to_string();
        self.cache.level(
            &(),
            |levels| levels.genre_counts.get(&key).cloned(),
            || loader(query).map(Arc::from),
            |levels, answer| {
                levels.genre_counts.insert(key.clone(), Arc::clone(&answer));
                answer
            },
        )
    }
}
