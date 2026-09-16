//! The query-keyed hit-listing Session Projection (ADR 0002): bounded
//! windows over the entity hit reads, keyed by the query text so a keystroke
//! retarget drops stale rows even at an unchanged generation.

use crate::app::store::{GenerationCache, StoreError, StoreGeneration};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// The one window size the seam's bounded-window projections share
/// (defined in the track-list projection).
use super::track_list::WINDOW_SIZE;

/// Cached-window bound before FIFO eviction kicks in. Generous for one
/// screen of scrolling; keeps memory bounded regardless of library size.
const MAX_CACHED_WINDOWS: usize = 8;

/// The cached payload of one query-keyed hit listing: the authoritative
/// total plus the bounded window map.
struct HitListRows<T> {
    total: usize,
    windows: HashMap<usize, Arc<[T]>>,
    eviction_order: VecDeque<usize>,
}

impl<T> Default for HitListRows<T> {
    fn default() -> Self {
        Self {
            total: 0,
            windows: HashMap::new(),
            eviction_order: VecDeque::new(),
        }
    }
}

/// Bounded window cache for one query-keyed entity hit listing (hit albums,
/// hit artists).
///
/// Per frame the UI declares which window offsets are visible
/// ([`Self::request_window`]) and calls [`Self::refresh`] with a loader
/// bound to the query port. A query change ([`Self::set_key`]) drops cached
/// rows even at an unchanged generation — retargeting the view on every
/// keystroke without waiting for a store mutation. Fresh windows serve from
/// cache; a bumped generation refetches every declared window.
pub struct HitListProjection<T> {
    /// The query text this projection serves. A change invalidates cached
    /// rows even at an unchanged generation.
    key: String,
    /// Generation-keyed slot holding [`HitListRows`]; keyed by the query
    /// text so a retarget drops rows even at an unchanged generation.
    cache: GenerationCache<String, HitListRows<T>>,
    /// Window offsets declared since the last successful refresh.
    pending_requests: Vec<usize>,
}

impl<T> HitListProjection<T> {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            key: String::new(),
            cache: GenerationCache::new(generation),
            pending_requests: Vec::new(),
        }
    }

    /// The query text this projection serves.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Retarget the projection to another query. Cached rows from the old
    /// query are dropped even at an unchanged generation.
    pub fn set_key(&mut self, key: String) {
        if key != self.key {
            self.key = key;
            self.cache.invalidate();
        }
    }

    /// Declare a visible window offset for the frame in progress. Call once
    /// per visible offset before [`Self::refresh`]; declarations accumulate
    /// until the next successful refresh consumes them.
    pub fn request_window(&mut self, offset: usize) {
        if !self.pending_requests.contains(&offset) {
            self.pending_requests.push(offset);
        }
    }

    /// Total row count as of the last successful refresh.
    #[must_use]
    pub fn total(&self) -> usize {
        self.cache.peek().map_or(0, |rows| rows.total)
    }

    /// Cached rows starting at `offset`, when present and loaded. Handed
    /// out as an `Arc` clone — a refcount bump, never a deep copy of the
    /// window's rows.
    #[must_use]
    pub fn window(&self, offset: usize) -> Option<Arc<[T]>> {
        self.cache
            .peek()
            .and_then(|rows| rows.windows.get(&offset))
            .cloned()
    }

    /// Whether cached rows reflect the session counter's current epoch.
    /// Projections reload when this returns `false`.
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        let epoch = self.cache.observe();
        self.cache.holds(epoch, &self.key)
    }

    /// The session epoch the projection currently observes — read once per
    /// frame so the torn-count guard compares two observations of the same
    /// counter.
    #[must_use]
    pub fn observe(&self) -> u64 {
        self.cache.observe()
    }

    /// Bring the projection up to date with `total`.
    ///
    /// * Invalidated (generation moved or key retargeted): every declared
    ///   window refetches and all prior rows are replaced.
    /// * Fresh: only declared-but-missing windows fetch.
    ///
    /// On a loader error the error propagates and the previous cache is left
    /// untouched — stale-but-present beats blank while the UI retries.
    pub fn refresh(
        &mut self,
        total: usize,
        loader: &mut dyn FnMut(usize, usize) -> Result<Vec<T>, StoreError>,
    ) -> Result<(), StoreError> {
        let epoch = self.cache.observe();
        let stale = !self.cache.holds(epoch, &self.key);
        let mut targets = std::mem::take(&mut self.pending_requests);
        if !stale {
            targets.retain(|offset| {
                !self
                    .cache
                    .peek()
                    .is_some_and(|rows| rows.windows.contains_key(offset))
            });
        }

        // Fetch first, swap later: a failure anywhere leaves the previous
        // cache completely untouched.
        let mut fetched: Vec<(usize, Vec<T>)> = Vec::with_capacity(targets.len());
        for offset in targets {
            let rows = loader(offset, WINDOW_SIZE)?;
            fetched.push((offset, rows));
        }

        let mut rows = if stale {
            HitListRows::default()
        } else {
            self.cache.take_value().expect("holds implied an entry")
        };
        for (offset, window) in fetched {
            if !rows.windows.contains_key(&offset) {
                rows.eviction_order.push_back(offset);
            }
            rows.windows.insert(offset, window.into());
            Self::enforce_bound(&mut rows);
        }
        rows.total = total;
        self.cache.store(epoch, self.key.clone(), rows);
        Ok(())
    }

    /// Keep at most [`MAX_CACHED_WINDOWS`] windows, evicting the oldest
    /// inserted ones first.
    fn enforce_bound(rows: &mut HitListRows<T>) {
        while rows.windows.len() > MAX_CACHED_WINDOWS {
            let oldest = rows
                .eviction_order
                .pop_front()
                .expect("eviction order tracks cached windows");
            rows.windows.remove(&oldest);
        }
    }
}
