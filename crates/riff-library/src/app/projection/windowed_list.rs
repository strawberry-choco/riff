//! The ONE bounded-window list projection in the library read seam.
//!
//! Every paged list read — the flat All Tracks list, the query-keyed hit
//! listings, and the browse columns — shares one generic
//! [`WindowedListProjection`]: a generation-keyed cache slot, the
//! fetch-first-swap-later page algorithm (an error leaves the previous cache
//! untouched), a FIFO window bound, and the shared [`WINDOW_SIZE`] /
//! [`MAX_CACHED_WINDOWS`] constants. A bounded-window fix is applied here
//! once and only here (deepen-three-modules issue 01).
//!
//! [`WindowedListProjection::show_window`] is where a bounded read stays
//! fresh: it retargets the query signature, reads one Listing Page when the
//! asked-for window is not already held, and caches that page's total beside
//! its rows. The Session Views seam is left holding only which key and which
//! window — it observes no generation, because the epoch never leaves a
//! projection (listing-page-read 03).
//!
//! The concrete names the seam exposes — [`TrackListProjection`] and
//! [`HitListProjection`] — are thin aliases over the generic, instantiated
//! with their key and row types; their former duplicated module bodies are
//! deleted.

use crate::app::store::{GenerationCache, Page, SortDirection, StoreError, StoreGeneration};
use crate::domain::Track;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// The one window size the seam's bounded-window projections share (the
/// flat list's home before deepen-three-modules issue 01): every paged read
/// fetches this many rows per window, so browse and hit lists parity with
/// All Tracks by construction.
pub const WINDOW_SIZE: usize = 50;

/// Cached-window bound before FIFO eviction kicks in. Generous for one
/// screen of scrolling; keeps memory bounded regardless of library size.
///
/// The ONE house for the window-cache bound (beside [`WINDOW_SIZE`]): every
/// bounded-window projection in the seam shares these two constants, so all
/// paged reads parity with All Tracks by construction.
pub(crate) const MAX_CACHED_WINDOWS: usize = 8;

/// The query signature a track-list projection was created (or retargeted)
/// for. A key change invalidates cached rows even at an unchanged
/// generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionKey {
    /// The flat all-tracks list.
    Flat,
    /// Case-insensitive substring search over title/artist/album/album
    /// artist; the payload is the raw query text.
    Search(String),
}

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

/// The flat all-tracks / search track list: the bounded-window projection
/// over one query signature (ADR 0002), keyed by [`ProjectionKey`].
pub type TrackListProjection = WindowedListProjection<ProjectionKey, Track>;

/// The query-keyed hit listings (hit albums, hit artists): the
/// bounded-window projection keyed by the query text, so a keystroke
/// retarget drops stale rows even at an unchanged generation (ADR 0002).
pub type HitListProjection<T> = WindowedListProjection<String, T>;

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
/// direction, or the flat/search query) over rows `T`.
///
/// Per frame a surface asks [`Self::show_window`] for the window it is
/// displaying; the projection retargets its query signature, reads one
/// Listing Page when that window is not already held fresh, and hands the
/// rows back through [`Self::window`] with [`Self::total`] beside them. A
/// key change drops cached rows even at an unchanged generation, and a
/// bumped generation refetches.
///
/// Its one level declares itself on [`GenerationCache::level`], so this
/// projection spells out no freshness rule of its own.
pub struct WindowedListProjection<K, T> {
    /// The query signature this projection serves. A change invalidates
    /// cached rows even at an unchanged generation.
    key: K,
    /// Generation-keyed slot holding [`WindowedListRows`]; keyed by the
    /// query signature so a retarget drops rows even at an unchanged
    /// generation.
    cache: GenerationCache<K, WindowedListRows<T>>,
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
        }
    }

    /// Retarget the projection to another query signature (the sort
    /// direction flipped, a genre switch, the search box changed). Cached
    /// rows from the old signature are dropped even at an unchanged
    /// generation.
    fn set_key(&mut self, key: K) {
        if key != self.key {
            self.key = key;
            self.cache.invalidate();
        }
    }

    /// Total row count as of the last successful page read.
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

    /// Bring `offset`'s window of `key`'s listing up to date.
    ///
    /// This is the whole bounded-read procedure in one place: retarget the
    /// query signature, then declare the window as a level on
    /// [`GenerationCache::level`], so this projection spells out no
    /// freshness rule of its own. Unless that window is already held fresh
    /// it reads one [`Page`] and caches that page's total and its rows
    /// together. Because both halves come from one store read, the total a
    /// surface reports cannot disagree with the rows under it — there is no
    /// gap for a mutation to land in, and no counter for a caller to
    /// compare.
    ///
    /// `read_page` is asked for [`WINDOW_SIZE`] rows: the visible window
    /// length belongs to this seam, never to the store, which must not guess
    /// at a surface's pagination.
    ///
    /// A failed page read leaves the previous cache completely untouched: a
    /// listing keeps showing its last good total and rows, and retries on
    /// the next frame because the loaded stamp only advances on success.
    pub fn show_window(
        &mut self,
        key: K,
        offset: usize,
        read_page: &mut dyn FnMut(usize, usize) -> Result<Page<T>, StoreError>,
    ) -> Result<(), StoreError> {
        self.set_key(key);
        // The level owns the one-observation rule: the same epoch stamps both
        // the freshness check and the commit, so a store commit racing this
        // frame cannot split the read across two generations, and a failure
        // anywhere leaves the previous cache untouched.
        self.cache.level(
            &self.key,
            |rows| rows.windows.get(&offset).cloned(),
            || read_page(offset, WINDOW_SIZE),
            |rows, page| {
                let total = page.total();
                let window: Arc<[T]> = page.into_rows().into();
                if !rows.windows.contains_key(&offset) {
                    rows.eviction_order.push_back(offset);
                }
                rows.windows.insert(offset, Arc::clone(&window));
                rows.total = total;
                Self::enforce_bound(rows);
                window
            },
        )?;
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
