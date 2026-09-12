//! The counts read-model Session Projection (ADR 0002): the sidebar-count
//! totals, the per-smart-list sizes, per-folder track counts, and the
//! last-scan stamp.

use crate::app::store::{
    FullScanSummary, GenerationCache, LibraryCounts, StoreError, StoreGeneration,
};
use crate::domain::SmartPlaylistKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Cached bundle for the counts read model: the library-side totals and the
/// per-smart-list sizes, each `Some` once loaded this generation.
#[derive(Default, Clone)]
struct CountsBundle {
    library: Option<Arc<LibraryCounts>>,
    smart_lists: Option<Arc<[(SmartPlaylistKind, usize)]>>,
    /// Per-folder track counts loaded this generation, keyed by folder.
    folder_counts: HashMap<PathBuf, usize>,
    /// The last-scan stamp loaded this generation: `NotLoaded` = not loaded
    /// yet, `Loaded` carrying the stamp (`None` means "never scanned").
    scan: ScanCache,
}

/// The scan-stamp slot's loaded state: distinguishes "no stamp read yet
/// this generation" from "read, and the store has never scanned".
#[derive(Default, Clone)]
enum ScanCache {
    #[default]
    NotLoaded,
    Loaded(Option<FullScanSummary>),
}

/// Session Projection for the counts read models (ADR 0002): the
/// sidebar-count totals and the per-smart-list sizes, cached per generation
/// so fresh frames cost nothing.
pub struct CountsProjection {
    cache: GenerationCache<(), CountsBundle>,
}

impl Default for CountsProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl CountsProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// The library-side totals (tracks, artists, albums, genres), cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached
    /// totals — one store query per generation, not per frame.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn library_counts(
        &mut self,
        loader: &mut dyn FnMut() -> Result<LibraryCounts, StoreError>,
    ) -> Result<Arc<LibraryCounts>, StoreError> {
        let epoch = self.cache.observe();
        if let Some(cached) = self
            .cache
            .loaded_at(epoch)
            .then(|| self.cache.peek().and_then(|bundle| bundle.library.clone()))
            .flatten()
        {
            return Ok(cached);
        }
        let fresh: Arc<LibraryCounts> = Arc::new(loader()?);
        self.cache.slot(epoch, &()).library = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// Every smart playlist's unbounded total, in `ALL` order, cached per
    /// generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn smart_list_counts(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError>,
    ) -> Result<Arc<[(SmartPlaylistKind, usize)]>, StoreError> {
        let epoch = self.cache.observe();
        if let Some(cached) = self
            .cache
            .loaded_at(epoch)
            .then(|| {
                self.cache
                    .peek()
                    .and_then(|bundle| bundle.smart_lists.clone())
            })
            .flatten()
        {
            return Ok(cached);
        }
        let fresh: Arc<[(SmartPlaylistKind, usize)]> = loader()?.into();
        self.cache.slot(epoch, &()).smart_lists = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// How many tracks live under `folder`, cached per (generation, folder)
    /// so each folder's Settings-pane count costs one query per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn folder_count(
        &mut self,
        folder: &Path,
        loader: &mut dyn FnMut(&Path) -> Result<usize, StoreError>,
    ) -> Result<usize, StoreError> {
        let epoch = self.cache.observe();
        if self.cache.loaded_at(epoch)
            && let Some(count) = self
                .cache
                .peek()
                .and_then(|bundle| bundle.folder_counts.get(folder))
        {
            return Ok(*count);
        }
        let fresh = loader(folder)?;
        self.cache
            .slot(epoch, &())
            .folder_counts
            .insert(folder.to_path_buf(), fresh);
        Ok(fresh)
    }

    /// The last completed full scan's summary (timestamp + file/error
    /// counts, design-handoff issue 12), cached per generation — including
    /// a cached absence ("never scanned"), so a cold store does not requery
    /// per frame.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn last_scan(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Option<FullScanSummary>, StoreError>,
    ) -> Result<Option<FullScanSummary>, StoreError> {
        let epoch = self.cache.observe();
        if self.cache.loaded_at(epoch)
            && let ScanCache::Loaded(scan) = self
                .cache
                .peek()
                .map_or(ScanCache::NotLoaded, |bundle| bundle.scan.clone())
        {
            return Ok(scan);
        }
        let fresh = loader()?;
        self.cache.slot(epoch, &()).scan = ScanCache::Loaded(fresh);
        Ok(fresh)
    }
}
