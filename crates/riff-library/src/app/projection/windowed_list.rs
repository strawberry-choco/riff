//! The generic bounded-window list projection the browse columns are built
//! on (paginate-browse-columns issue 01): one windowed-list projection in
//! the library read seam, parameterized by a per-list query-signature key
//! that includes the sort direction. It is the flat list's proven semantics
//! (ADR 0003) made generic — bounded window map, FIFO window cap, stamped
//! authoritative total, generation-keyed staleness, and query-keyed
//! retargeting — so every paged browse read parities with All Tracks by
//! sharing the same [`WINDOW_SIZE`] and [`MAX_CACHED_WINDOWS`].

use super::track_list::{MAX_CACHED_WINDOWS, WINDOW_SIZE};
use crate::app::store::{GenerationCache, SortDirection, StoreError, StoreGeneration};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// Which bounded browse listing a projection serves, plus the sort
/// direction that scopes it. Every mutable element of the read — the list
/// identity *and* the A–Z / Z–A direction — is part of the query
/// signature, so retargeting either (switching genre, reversing the sort)
/// drops cached rows even at an unchanged generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BrowseList {
    /// The Artists root list (issue 02).
    Artists,
    /// The Albums root list (issue 03).
    Albums,
    /// The Genres root list (issue 04).
    Genres,
    /// The artists-within-a-genre drill list (issue 05); the payload is the
    /// genre.
    ArtistsInGenre(String),
    /// The albums-within-an-artist-and-genre drill list (issue 05).
    ArtistAlbumsInGenre { artist: String, genre: String },
}

/// The per-list query-signature key of one paged browse read: which listing
/// plus the direction, so retargeting either drops cached rows even at an
/// unchanged generation (paginate-browse-columns issue 01, checkbox 2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BrowseProjectionKey {
    /// The browse listing this projection serves.
    pub list: BrowseList,
    /// The A–Z / Z–A direction baked into the query signature: reversing
    /// the sort retargets the projection exactly like switching lists.
    pub direction: SortDirection,
}

/// The cached payload of one query signature: the authoritative total plus
/// the bounded window map.
struct WindowedListRows<T> {
    total: usize,
    windows: HashMap<usize, Arc<[T]>>,
    eviction_order: VecDeque<usize>,
}

impl<T> Default for WindowedListRows<T> {
    fn default() -> Self {
        Self {
            total: 0,
            windows: HashMap::new(),
            eviction_order: VecDeque::new(),
        }
    }
}

/// Generic bounded window cache for one store list read, keyed by a
/// caller-shaped query signature `K` (e.g. the browse list plus sort
/// direction) over rows `T`.
///
/// Per frame the UI declares which window offsets are visible
/// ([`Self::request_window`]) and calls [`Self::refresh`] with a loader
/// bound to the store port. A key change ([`Self::set_key`]) drops cached
/// rows even at an unchanged generation; a bumped generation refetches
/// every declared window. Fresh windows serve from cache; invalidated
/// frames refetch.
pub struct WindowedListProjection<K, T> {
    /// The query signature this projection serves. A change invalidates
    /// cached rows even at an unchanged generation.
    key: K,
    /// Generation-keyed slot holding [`WindowedListRows`]; keyed by the
    /// query signature so a retarget drops rows even at an unchanged
    /// generation.
    cache: GenerationCache<K, WindowedListRows<T>>,
    /// Window offsets declared since the last successful refresh.
    pending_requests: Vec<usize>,
}

impl<K, T> WindowedListProjection<K, T>
where
    K: Clone + PartialEq,
{
    /// Start a projection observing `generation`, serving `key`'s list.
    #[must_use]
    pub fn new(generation: StoreGeneration, key: K) -> Self {
        Self {
            key,
            cache: GenerationCache::new(generation),
            pending_requests: Vec::new(),
        }
    }

    /// The query signature this projection serves.
    #[must_use]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Retarget the projection to another query signature (e.g. the sort
    /// direction flipped or a genre switch). Cached rows from the old
    /// signature are dropped even at an unchanged generation.
    pub fn set_key(&mut self, key: K) {
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

    /// Whether cached rows reflect the session counter's current epoch AND
    /// the current query signature. Projections reload when this returns
    /// `false`.
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
            WindowedListRows::default()
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
    fn enforce_bound(rows: &mut WindowedListRows<T>) {
        while rows.windows.len() > MAX_CACHED_WINDOWS {
            let oldest = rows
                .eviction_order
                .pop_front()
                .expect("eviction order tracks cached windows");
            rows.windows.remove(&oldest);
        }
    }
}
