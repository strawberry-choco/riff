//! Bounded Session Projections over Application Store query results.
//!
//! A Session Projection is a bounded in-memory view of store query results
//! used while rendering the UI; it is never authoritative (ADR 0002). Each
//! projection caches a total count plus only the currently visible row
//! windows (`LIMIT`/`OFFSET` ranges) until invalidated by the session-local
//! Store generation counter, which bumps after every committed mutation.
//! Stale reads are possible only between a committed write and the next
//! refresh, which generation invalidation makes explicit.

use crate::app::errors::AppError;
use crate::domain::Track;
use std::collections::{HashMap, VecDeque};

/// Rows fetched per window. The projection owns the window-size policy so
/// callers declare visible offsets without duplicating page math.
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

/// Bounded window cache for one track-list query signature.
///
/// Per frame the UI declares which window offsets are visible
/// ([`Self::request_window`]) and calls [`Self::refresh`] with the Store's
/// current generation plus a loader bound to the query port. Fresh frames
/// serve cached rows without touching the store; invalidated frames refetch
/// every declared window.
pub struct TrackListProjection {
    key: ProjectionKey,
    /// Generation the cache was loaded at; `None` means "never loaded"
    /// (fresh construction or retargeted key).
    loaded_generation: Option<u64>,
    total: usize,
    windows: HashMap<usize, Vec<Track>>,
    eviction_order: VecDeque<usize>,
    /// Window offsets declared since the last successful refresh.
    pending_requests: Vec<usize>,
}

impl TrackListProjection {
    #[must_use]
    pub fn new(key: ProjectionKey) -> Self {
        Self {
            key,
            loaded_generation: None,
            total: 0,
            windows: HashMap::new(),
            eviction_order: VecDeque::new(),
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
            self.loaded_generation = None;
            self.windows.clear();
            self.eviction_order.clear();
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
        self.total
    }

    /// Cached rows starting at `offset`, when present and loaded.
    #[must_use]
    pub fn window(&self, offset: usize) -> Option<&[Track]> {
        self.windows.get(&offset).map(Vec::as_slice)
    }

    /// Whether cached rows reflect `generation`. Projections reload when
    /// this returns `false`.
    #[must_use]
    pub fn is_fresh(&self, generation: u64) -> bool {
        self.loaded_generation == Some(generation)
    }

    /// Bring the projection up to date with `generation` and `total`.
    ///
    /// * Invalidated (generation moved or key retargeted): every declared
    ///   window refetches and all prior rows are replaced.
    /// * Fresh: only declared-but-missing windows fetch.
    ///
    /// On a loader error the error propagates and the previous cache is left
    /// untouched — stale-but-present beats blank while the UI retries.
    pub fn refresh(
        &mut self,
        generation: u64,
        total: usize,
        loader: &mut dyn FnMut(usize, usize) -> Result<Vec<Track>, AppError>,
    ) -> Result<(), AppError> {
        let stale = !self.is_fresh(generation);
        let mut targets = std::mem::take(&mut self.pending_requests);
        if !stale {
            targets.retain(|offset| !self.windows.contains_key(offset));
        }

        // Fetch first, swap later: a failure anywhere leaves the previous
        // cache completely untouched.
        let mut fetched: Vec<(usize, Vec<Track>)> = Vec::with_capacity(targets.len());
        for offset in targets {
            let rows = loader(offset, WINDOW_SIZE)?;
            fetched.push((offset, rows));
        }

        if stale {
            self.windows.clear();
            self.eviction_order.clear();
        }
        for (offset, rows) in fetched {
            if !self.windows.contains_key(&offset) {
                self.eviction_order.push_back(offset);
            }
            self.windows.insert(offset, rows);
            self.enforce_bound();
        }
        self.total = total;
        self.loaded_generation = Some(generation);
        Ok(())
    }

    /// Keep at most [`MAX_CACHED_WINDOWS`] windows, evicting the oldest
    /// inserted ones first.
    fn enforce_bound(&mut self) {
        while self.windows.len() > MAX_CACHED_WINDOWS {
            let oldest = self
                .eviction_order
                .pop_front()
                .expect("eviction order tracks cached windows");
            self.windows.remove(&oldest);
        }
    }
}
