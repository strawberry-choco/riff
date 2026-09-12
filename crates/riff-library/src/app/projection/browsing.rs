//! The artist/album browsing Session Projection (ADR 0002).

use crate::app::store::{GenerationCache, StoreError, StoreGeneration};
use crate::domain::{Album, Artist, Track};
use std::collections::HashMap;
use std::sync::Arc;

/// The three lazily-filled levels of the browsing hierarchy.
#[derive(Default, Clone)]
struct BrowsingLevels {
    artists: Option<Arc<[Artist]>>,
    albums: HashMap<String, Arc<[Album]>>,
    tracks: HashMap<(String, String), Arc<[Track]>>,
}

/// Session Projection for the artist/album browsing views (ADR 0002).
///
/// Caches the artist list plus per-artist album lists and per-album track
/// lists, each fetched from the store only when missing at the current
/// generation. A generation bump (a committed store mutation) drops every
/// level at once so a frame never mixes rows from two generations; each
/// level then refetches lazily as its view expands again. Loader errors
/// propagate and leave the cache untouched — the next call retries.
///
/// Unlike the windowed `TrackListProjection` this is not windowed: browsing
/// is hierarchical, so each query returns one artist's or one album's worth
/// of rows rather than one screen's.
pub struct BrowsingProjection {
    /// Generation-keyed slot over the whole level bundle: a moved epoch
    /// drops every level together, within a generation levels fill lazily.
    cache: GenerationCache<(), BrowsingLevels>,
}

/// Loader signature for one album's tracks (factored out for readability).
type AlbumTracksLoader<'a> = &'a mut dyn FnMut(&str, &str) -> Result<Vec<Track>, StoreError>;

impl Default for BrowsingProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl BrowsingProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// Every artist name-ascending, cached per generation. Fresh frames
    /// hand out an `Arc` clone of the cached list — no per-frame copy.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artists(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<Artist>, StoreError>,
    ) -> Result<Arc<[Artist]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache.peek().and_then(|levels| levels.artists.clone())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Artist]> = loader()?.into();
        self.cache.slot(epoch, &()).artists = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One artist's albums in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artist_albums(
        &mut self,
        artist: &str,
        loader: &mut dyn FnMut(&str) -> Result<Vec<Album>, StoreError>,
    ) -> Result<Arc<[Album]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.albums.get(artist).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Album]> = loader(artist)?.into();
        let levels = self.cache.slot(epoch, &());
        levels.albums.insert(artist.to_string(), Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One album's tracks in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn album_tracks(
        &mut self,
        album_artist: &str,
        album_title: &str,
        loader: AlbumTracksLoader<'_>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = (album_artist.to_string(), album_title.to_string());
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
        let fresh: Arc<[Track]> = loader(album_artist, album_title)?.into();
        let levels = self.cache.slot(epoch, &());
        levels.tracks.insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }
}
