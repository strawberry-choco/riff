//! The folder-tree Session Projection (ADR 0002): five folder query shapes.

use crate::app::store::{GenerationCache, StoreError, StoreGeneration};
use crate::domain::{Track, TrackId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The five lazily-filled folder query shapes, keyed by folder.
#[derive(Default, Clone)]
struct FolderLevels {
    has_audio: HashMap<String, bool>,
    search_matches: HashMap<(String, String), bool>,
    subtree_ids: HashMap<String, Arc<[TrackId]>>,
    direct_tracks: HashMap<String, Arc<[Track]>>,
    children: HashMap<String, Arc<[PathBuf]>>,
}

/// Session Projection for the folder-tree views (ADR 0002).
///
/// Caches the five folder query shapes — subtree existence, subtree search
/// matches, subtree track ids, direct tracks, and child directories — keyed
/// by folder, each fetched from the store only when missing at the current
/// generation. A generation bump drops every level at once so a frame never
/// mixes rows from two generations; levels then refetch lazily as the tree
/// renders again. Loader errors propagate and leave the cache untouched.
pub struct FolderProjection {
    /// Generation-keyed slot over the whole level bundle: a moved epoch
    /// drops every level together, within a generation levels fill lazily.
    cache: GenerationCache<(), FolderLevels>,
}

impl Default for FolderProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl FolderProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// Whether `folder` contains any audio, cached per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn has_audio(
        &mut self,
        folder: &Path,
        loader: &mut dyn FnMut(&Path) -> Result<bool, StoreError>,
    ) -> Result<bool, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.has_audio.get(&key).copied())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh = loader(folder)?;
        self.cache.slot(epoch, &()).has_audio.insert(key, fresh);
        Ok(fresh)
    }

    /// Whether any track under `folder` matches the search query, cached
    /// per (folder, query) per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn has_search_match(
        &mut self,
        folder: &Path,
        query: &str,
        loader: &mut dyn FnMut(&Path, &str) -> Result<bool, StoreError>,
    ) -> Result<bool, StoreError> {
        let key = (folder.to_string_lossy().into_owned(), query.to_string());
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.search_matches.get(&key).copied())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh = loader(folder, query)?;
        self.cache
            .slot(epoch, &())
            .search_matches
            .insert(key, fresh);
        Ok(fresh)
    }

    /// Every track id under `folder`, path-ordered, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list — no
    /// per-frame copy of one id per track.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn subtree_ids(
        &mut self,
        folder: &Path,
        loader: &mut dyn FnMut(&Path) -> Result<Vec<TrackId>, StoreError>,
    ) -> Result<Arc<[TrackId]>, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.subtree_ids.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[TrackId]> = loader(folder)?.into();
        self.cache
            .slot(epoch, &())
            .subtree_ids
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }

    /// The tracks directly inside `folder`, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn direct_tracks(
        &mut self,
        folder: &Path,
        loader: &mut dyn FnMut(&Path) -> Result<Vec<Track>, StoreError>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.direct_tracks.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Track]> = loader(folder)?.into();
        self.cache
            .slot(epoch, &())
            .direct_tracks
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }

    /// The child directories of `folder` holding audio, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn children(
        &mut self,
        folder: &Path,
        loader: &mut dyn FnMut(&Path) -> Result<Vec<PathBuf>, StoreError>,
    ) -> Result<Arc<[PathBuf]>, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.children.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[PathBuf]> = loader(folder)?.into();
        self.cache
            .slot(epoch, &())
            .children
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }
}
