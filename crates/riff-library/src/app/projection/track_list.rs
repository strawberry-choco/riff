//! The flat-list/search Session Projection: a bounded window cache over one
//! query signature (ADR 0002).

use crate::app::store::{GenerationCache, StoreError, StoreGeneration};
use crate::domain::Track;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub const WINDOW_SIZE: usize = 50;

/// Cached-window bound before FIFO eviction kicks in. Generous for one
/// screen of scrolling; keeps memory bounded regardless of library size.
const MAX_CACHED_WINDOWS: usize = 8;

/// The query signature a projection was created (or retargeted) for. A key
/// change invalidates cached rows even at an unchanged generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionKey {
    /// The flat all-tracks list.
    Flat,
    /// Case-insensitive substring search over title/artist/album/album
    /// artist; the payload is the raw query text.
    Search(String),
}

/// The cached payload of one track-list query signature: the authoritative
/// total plus the bounded window map.
#[derive(Default, Clone)]
struct TrackListRows {
    total: usize,
    windows: HashMap<usize, Arc<[Track]>>,
    eviction_order: VecDeque<usize>,
}

/// Bounded window cache for one track-list query signature.
///
/// Per frame the UI declares which window offsets are visible
/// ([`Self::request_window`]) and calls [`Self::refresh`] with a loader
/// bound to the query port. Fresh frames serve cached rows without touching
/// the store; invalidated frames refetch every declared window.
pub struct TrackListProjection {
    key: ProjectionKey,
    /// Generation-keyed slot holding [`TrackListRows`]; keyed by the query
    /// signature so a retarget drops rows even at an unchanged generation.
    cache: GenerationCache<ProjectionKey, TrackListRows>,
    /// Window offsets declared since the last successful refresh.
    pending_requests: Vec<usize>,
}

impl TrackListProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration, key: ProjectionKey) -> Self {
        Self {
            key,
            cache: GenerationCache::new(generation),
            pending_requests: Vec::new(),
        }
    }

    /// The query signature this projection serves.
    #[must_use]
    pub fn key(&self) -> &ProjectionKey {
        &self.key
    }

    /// Retarget the projection to another query signature (e.g. the search
    /// box changed). Cached rows from the old signature are dropped even at
    /// an unchanged generation.
    pub fn set_key(&mut self, key: ProjectionKey) {
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
    pub fn window(&self, offset: usize) -> Option<Arc<[Track]>> {
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
        loader: &mut dyn FnMut(usize, usize) -> Result<Vec<Track>, StoreError>,
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
        let mut fetched: Vec<(usize, Vec<Track>)> = Vec::with_capacity(targets.len());
        for offset in targets {
            let rows = loader(offset, WINDOW_SIZE)?;
            fetched.push((offset, rows));
        }

        let mut rows = if stale {
            TrackListRows::default()
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
    fn enforce_bound(rows: &mut TrackListRows) {
        while rows.windows.len() > MAX_CACHED_WINDOWS {
            let oldest = rows
                .eviction_order
                .pop_front()
                .expect("eviction order tracks cached windows");
            rows.windows.remove(&oldest);
        }
    }
}
