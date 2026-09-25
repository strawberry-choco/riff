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
///
/// Every level here declares itself on [`GenerationCache::level`], so this
/// projection spells out no freshness rule of its own. `last_scan` keeps the
/// tri-state [`ScanCache`] as its slot because it must also cache an absence --
/// which `read` says, not the staleness procedure.
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
        self.cache.level(
            &(),
            |bundle| bundle.library.clone(),
            || loader().map(Arc::new),
            |bundle, answer| {
                bundle.library = Some(Arc::clone(&answer));
                answer
            },
        )
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
        self.cache.level(
            &(),
            |bundle| bundle.smart_lists.clone(),
            || loader().map(Arc::from),
            |bundle, answer| {
                bundle.smart_lists = Some(Arc::clone(&answer));
                answer
            },
        )
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
        self.cache.level(
            &(),
            |bundle| bundle.folder_counts.get(folder).copied(),
            || loader(folder),
            |bundle, answer| {
                bundle.folder_counts.insert(folder.to_path_buf(), answer);
                answer
            },
        )
    }

    /// The last completed full scan's summary (timestamp + file/error
    /// counts, design-handoff issue 12), cached per generation — including
    /// a cached absence ("never scanned"), so a cold store does not requery
    /// per frame.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    /// The last completed full scan, cached per generation -- and cached when
    /// there never was one.
    ///
    /// The cached absence is why the slot is the tri-state [`ScanCache`] and
    /// not a bare option: `read` answers `Some(None)` for "asked, and the
    /// store has never scanned" and `None` only for "not asked yet this
    /// generation", so a cold store does not requery per frame while the
    /// level still declares itself on [`GenerationCache::level`].
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn last_scan(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Option<FullScanSummary>, StoreError>,
    ) -> Result<Option<FullScanSummary>, StoreError> {
        self.cache.level(
            &(),
            |bundle| match &bundle.scan {
                ScanCache::Loaded(scan) => Some(*scan),
                ScanCache::NotLoaded => None,
            },
            loader,
            |bundle, fresh| {
                bundle.scan = ScanCache::Loaded(fresh);
                fresh
            },
        )
    }
}
