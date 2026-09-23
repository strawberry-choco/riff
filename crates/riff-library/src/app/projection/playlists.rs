//! The user-playlist Session Projection (ADR 0002): the playlist list plus
//! the per-playlist resolved views.

use crate::app::playlist_manager;
use crate::app::store::{GenerationCache, PlaylistEntry, StoreError, StoreGeneration};
use crate::domain::{Playlist, PlaylistId, Track, TrackId};
use std::collections::HashMap;
use std::sync::Arc;

/// One resolved row of an open user playlist: the entry id, its
/// store-resolved Track when the Library knows it, and whether it can play.
pub type PlaylistEntryRow = (TrackId, Option<Track>, bool);

/// The ready-to-render view of one user playlist: one row per entry in
/// playlist order, plus the playable ids (valid verdicts only) for the
/// header context menu.
#[derive(Clone, Default)]
pub struct PlaylistView {
    /// One row per entry, in playlist order. Missing tracks are included as
    /// `(id, None, false)` — dangling references stay listed (ADR 0001).
    pub rows: Arc<[PlaylistEntryRow]>,
    /// The playable ids (valid verdicts only), in playlist order.
    pub valid_ids: Arc<[TrackId]>,
}

/// Session Projection for the user playlists (ADR 0002).
///
/// Caches the playlist list plus per-playlist resolved views as TWO
/// [`GenerationCache`] instances keyed on different counters. The list is
/// pure user data: keyed on the session's dedicated playlist generation
/// alone. Resolved rows embed Track metadata resolved against the Library
/// collection, so their cache is keyed on the Library generation with the
/// playlist epoch baked into the key — a move of EITHER counter drops every
/// row while the list stays. Within matching counters levels refetch lazily
/// as their views render again.
///
/// Loader errors propagate and leave the cache untouched — the previous
/// good rows stay readable through [`Self::cached_playlists`] /
/// [`Self::cached_view`] while the next call retries.
pub struct PlaylistProjection {
    /// The playlist list, keyed on the playlist generation alone.
    playlists: GenerationCache<(), Arc<[Playlist]>>,
    /// The per-playlist resolved views: keyed on the Library generation,
    /// with the playlist generation the rows were built under as the key.
    views: GenerationCache<u64, HashMap<PlaylistId, PlaylistView>>,
}

impl Default for PlaylistProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new(), StoreGeneration::new())
    }
}

impl PlaylistProjection {
    #[must_use]
    pub fn new(playlist_generation: StoreGeneration, library_generation: StoreGeneration) -> Self {
        Self {
            playlists: GenerationCache::new(playlist_generation),
            views: GenerationCache::new(library_generation),
        }
    }

    /// Every user playlist in creation order, cached per playlist
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// This level keeps a hand-written body rather than declaring itself on
    /// [`GenerationCache::level`]: its cache holds the answer itself, not a
    /// bundle of levels filled one at a time, and `level` fills through the
    /// bundle slot its cache type can default-construct. The staleness
    /// procedure it spells out here is the one `level` implements; it is a
    /// single-stamp level, not a carve-out on policy grounds.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn playlists(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<Playlist>, StoreError>,
    ) -> Result<Arc<[Playlist]>, StoreError> {
        let epoch = self.playlists.observe();
        if self.playlists.loaded_at(epoch)
            && let Some(cached) = self.playlists.peek()
        {
            return Ok(Arc::clone(cached));
        }
        let fresh: Arc<[Playlist]> = loader()?.into();
        self.playlists.store(epoch, (), Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One playlist's resolved view, cached per playlist generation plus
    /// the Library generation the rows were resolved against. Fresh frames
    /// hand out a clone of the cached view (`Arc` row bumps, no deep copy).
    ///
    /// This level keeps a hand-written body because its freshness depends on
    /// two counters at once — the Library epoch stamps the cache while the
    /// playlist epoch rides inside its key, so a move of either drops every
    /// resolved row. [`GenerationCache::level`] is deliberately single-stamp:
    /// one cache, one counter, one generation observed per call. Expressing
    /// this level on it would mean making `level` own a rule it cannot see.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn playlist_view(
        &mut self,
        id: &PlaylistId,
        loader: &mut dyn FnMut(&PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError>,
    ) -> Result<PlaylistView, StoreError> {
        let playlist_epoch = self.playlists.observe();
        let library_epoch = self.views.observe();
        let fresh = self.views.holds(library_epoch, &playlist_epoch)
            && self
                .views
                .peek()
                .is_some_and(|views| views.contains_key(id));
        if !fresh {
            // Rows embed Library-resolved metadata: a move of either counter
            // invalidates them even though the other stood still.
            let resolved = loader(id)?;
            let views = self.views.slot(library_epoch, &playlist_epoch);
            let view = Self::resolve(resolved);
            views.insert(id.clone(), view.clone());
            return Ok(view);
        }
        Ok(self
            .views
            .peek()
            .expect("checked above")
            .get(id)
            .expect("checked above")
            .clone())
    }

    /// The stale-but-present playlist list, if any — the error fallback
    /// keeps last good data instead of blanking the sidebar.
    #[must_use]
    pub fn cached_playlists(&self) -> Option<Arc<[Playlist]>> {
        self.playlists.peek().cloned()
    }

    /// The stale-but-present view for `id`, if any — the error fallback
    /// keeps last good rows instead of blanking the open playlist.
    #[must_use]
    pub fn cached_view(&self, id: &PlaylistId) -> Option<PlaylistView> {
        self.views.peek().and_then(|views| views.get(id)).cloned()
    }

    /// Map store entries to ready-to-render rows: each entry rides its
    /// LEFT-JOIN validity plus the read-time filesystem check, and missing
    /// tracks stay listed as `(id, None, false)` (ADR 0001).
    fn resolve(entries: Vec<PlaylistEntry>) -> PlaylistView {
        let mut valid_ids = Vec::new();
        let rows: Vec<PlaylistEntryRow> = entries
            .into_iter()
            .map(|entry| {
                let valid = playlist_manager::track_is_valid(&entry);
                if valid {
                    valid_ids.push(entry.id.clone());
                }
                (entry.id, entry.track, valid)
            })
            .collect();
        PlaylistView {
            rows: rows.into(),
            valid_ids: valid_ids.into(),
        }
    }
}
