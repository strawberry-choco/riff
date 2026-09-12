//! The read-only smart-playlist Session Projection (ADR 0002).

use crate::app::store::{GenerationCache, StoreError, StoreGeneration};
use crate::domain::{SmartPlaylistKind, Track};
use std::collections::HashMap;
use std::sync::Arc;

/// Per-kind computed lists stamped with the limit they were loaded at.
type SmartPlaylistLists = HashMap<SmartPlaylistKind, (usize, Arc<[Track]>)>;

/// Session Projection for the read-only smart playlists (ADR 0002).
///
/// Caches one computed list per [`SmartPlaylistKind`], stamped with the
/// generation it was loaded at. A generation bump (any committed store
/// mutation — a finished play, a scan batch, a tag edit) drops every list so
/// the next frame regenerates from committed state; within a generation the
/// cache serves repeat frames without touching the store. A request whose
/// limit exceeds what the cache holds also refetches, so callers can never
/// see a truncated-as-cached list where they asked for more.
pub struct SmartPlaylistsProjection {
    /// Generation-keyed slot over the per-kind computed lists.
    cache: GenerationCache<(), SmartPlaylistLists>,
}

impl Default for SmartPlaylistsProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl SmartPlaylistsProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// The computed list for `kind`, cached per generation and limit.
    /// Fresh frames hand out an `Arc` clone of the cached list — no
    /// per-frame copy.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn list(
        &mut self,
        kind: SmartPlaylistKind,
        limit: usize,
        loader: &mut dyn FnMut(SmartPlaylistKind, usize) -> Result<Vec<Track>, StoreError>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|lists| lists.get(&kind))
                .filter(|(cached_limit, _)| *cached_limit >= limit)
                .map(|(_, list)| Arc::clone(list))
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Track]> = loader(kind, limit)?.into();
        self.cache
            .slot(epoch, &())
            .insert(kind, (limit, Arc::clone(&fresh)));
        Ok(fresh)
    }
}
