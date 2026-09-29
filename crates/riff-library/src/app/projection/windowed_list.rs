//! The ONE bounded-window list projection in the library read seam.
//!
//! Every paged list read — the flat All Tracks list, the query-keyed hit
//! listings, and the browse columns — shares one generic
//! [`WindowedListProjection`]: a generation-keyed cache slot, the
//! fetch-first-swap-later page algorithm (an error leaves the previous cache
//! untouched), a FIFO window bound, and the window size. A bounded-window
//! fix is applied here once and only here (deepen-three-modules issue 01).
//!
//! This projection is where the whole paged-listing protocol lives, which is
//! why the seam's paged reads are two calls per listing and one line each:
//!
//! * [`Self::row`] answers **"give me row *i*"**. It aligns *i* down to the
//!   window size, refetches only when *i* falls outside the window already
//!   in hand, and hands back the single row asked for as an `Arc` the caller
//!   can keep while it makes other seam reads.
//! * [`Self::count`] answers **"how many rows does this listing have"** as
//!   its own read, cached beside the windows at the same generation.
//!
//! The arithmetic that used to be written once per paged read here and again
//! in every render site — the alignment, the "refetch only when the row
//! leaves the page in hand" decision, the page-start subtraction — is now
//! written here and nowhere else. An off-by-one in it was a rendering bug
//! that only a golden image could catch, six weeks later, for no visible
//! reason; it is now a seam test failure (listing-page-read 03).
//!
//! The concrete names the seam uses — [`TrackListProjection`] and
//! [`HitListProjection`] — are thin aliases over the generic, instantiated
//! with their key and row types; their former duplicated module bodies are
//! deleted.

use crate::app::store::{
    GenerationCache, Page, SortDirection, StoreError, StoreGeneration, TrackListOrder,
};
use crate::domain::Track;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// The one window size the seam's bounded-window projections share (the
/// flat list's home before deepen-three-modules issue 01): every paged read
/// fetches this many rows per window, so browse and hit lists parity with
/// All Tracks by construction.
///
/// Private to this module with the alignment that uses it: the Session
/// Views seam names row indices, never offsets, so no caller can be off by
/// a window.
const WINDOW_SIZE: usize = 50;

/// Cached-window bound before FIFO eviction kicks in. Generous for one
/// screen of scrolling; keeps memory bounded regardless of library size.
///
/// The ONE house for the window-cache bound: every bounded-window projection
/// in the seam shares these two constants, so all paged reads parity with
/// All Tracks by construction.
const MAX_CACHED_WINDOWS: usize = 8;

/// The query signature a track-list projection was created (or retargeted)
/// for. A key change invalidates cached rows even at an unchanged
/// generation — including the order changing, since the order is part of
/// what the store serves (the sort control's `ORDER BY`, not an in-memory
/// reversal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionKey {
    /// The flat all-tracks list, in the given order.
    Flat(TrackListOrder),
    /// Case-insensitive substring search over title/artist/album/album
    /// artist; the first payload is the raw query text, the second the
    /// listing order.
    Search(String, TrackListOrder),
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

/// The cached payload of one query signature: the listing's total plus the
/// bounded window map.
///
/// The windows hold **shared row handles** (`Arc<[Arc<T>]>`) rather than
/// plain rows: a render site asks for one row and then makes other seam
/// reads while it draws it, so the row it holds cannot borrow the cache.
/// Wrapping each row once per window load — not once per row per frame — is
/// what makes that free of a per-frame deep copy.
struct WindowedListRows<T> {
    /// The listing's total, as of the last successful count read at this
    /// generation. A window read never fills it: the count is its own read,
    /// so no number enters this slot that no read asked for.
    total: Option<usize>,
    /// Window start offset → that window's rows, in store order.
    windows: HashMap<usize, Arc<[Arc<T>]>>,
    eviction_order: VecDeque<usize>,
}

impl<T> Default for WindowedListRows<T> {
    fn default() -> Self {
        Self {
            total: None,
            windows: HashMap::new(),
            eviction_order: VecDeque::new(),
        }
    }
}

/// Generic bounded window cache for one store list read, keyed by a
/// caller-shaped query signature `K` (e.g. the browse list plus sort
/// direction, or the flat/search query) over rows `T`.
///
/// A surface asks [`Self::row`] for the row it is about to draw and
/// [`Self::count`] for the total it draws above them — two reads, never one
/// bundled answer, so neither has to carry the other's data. The projection
/// retargets its query signature, reads a window when *i* is not already
/// covered by one in hand, and caches windows and the total together at one
/// generation. A key change drops cached rows even at an unchanged
/// generation, and a bumped generation refetches.
///
/// Both reads declare themselves on [`GenerationCache::level`], so this
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

    /// The window start that covers row `index` — the alignment, in one
    /// place, so no caller derives an offset from an index.
    fn window_start(index: usize) -> usize {
        index - (index % WINDOW_SIZE)
    }

    /// Row `index` of `key`'s listing, fetched if the window in hand does
    /// not already cover it.
    ///
    /// This is the whole row-at-*i* procedure: retarget the query
    /// signature, align `index` down to a window, then declare that window
    /// as a level on [`GenerationCache::level`] so this projection spells
    /// out no freshness rule of its own. Unless the window is already held
    /// at the current generation it reads one [`Page`] and caches its rows;
    /// a row inside a window already in hand costs no store read, which is
    /// what "refetch only when the row leaves the page in hand" means.
    ///
    /// `read_page` is asked for [`WINDOW_SIZE`] rows: the visible window
    /// length belongs to this seam, never to the store, which must not guess
    /// at a surface's pagination.
    ///
    /// The row is handed out as a shared handle, so a caller may hold it
    /// while it makes other seam reads. `Ok(None)` means the listing has no
    /// row at that index — a caller asking past its end, or a partially
    /// filled final window.
    ///
    /// # Errors
    /// Propagates a failed window read. The previous cache is untouched, so
    /// [`Self::cached_row`] still answers the last good row.
    pub fn row(
        &mut self,
        key: K,
        index: usize,
        read_page: &mut dyn FnMut(usize, usize) -> Result<Page<T>, StoreError>,
    ) -> Result<Option<Arc<T>>, StoreError> {
        let start = Self::window_start(index);
        self.set_key(key);
        let window = self.cache.level(
            &self.key,
            |rows| rows.windows.get(&start).cloned(),
            || read_page(start, WINDOW_SIZE).map(|page| Self::into_handles(page)),
            |rows, window| Self::commit_window(rows, start, window),
        )?;
        Ok(window.get(index - start).cloned())
    }

    /// Row `index` of the listing in hand, from the last good window,
    /// regardless of generation — the error fallback a caller takes after
    /// [`Self::row`] failed, so a listing keeps showing its last good rows
    /// rather than blanking. `None` when no window covers it.
    #[must_use]
    pub fn cached_row(&self, index: usize) -> Option<Arc<T>> {
        let start = Self::window_start(index);
        self.cache
            .peek()?
            .windows
            .get(&start)?
            .get(index - start)
            .cloned()
    }

    /// How many rows `key`'s listing has, as its own read.
    ///
    /// Cached at the same generation as the windows, so a frame that reads
    /// the total and then walks rows pays for one count read per listing per
    /// generation. `read_count` is the listing's own store read for the
    /// total alone: no window is fetched to be thrown away, and a caller
    /// that never asked for a row never pays for one.
    ///
    /// # Errors
    /// Propagates a failed count read. The previous cache is untouched, so
    /// [`Self::cached_count`] still answers the last good total — which is
    /// what keeps a header from blanking over rows it is still drawing.
    pub fn count(
        &mut self,
        key: K,
        read_count: &mut dyn FnMut() -> Result<usize, StoreError>,
    ) -> Result<usize, StoreError> {
        self.set_key(key);
        self.cache.level(
            &self.key,
            |rows| rows.total,
            || (*read_count)(),
            |rows, total| {
                rows.total = Some(total);
                total
            },
        )
    }

    /// The last good total of the listing in hand, regardless of
    /// generation — the error fallback a caller takes after [`Self::count`]
    /// failed. `None` when the listing never loaded a total.
    #[must_use]
    pub fn cached_count(&self) -> Option<usize> {
        self.cache.peek().and_then(|rows| rows.total)
    }

    /// Wrap one page's rows in shared handles — once per window load, so a
    /// caller holding a row pays a refcount bump rather than a deep copy.
    fn into_handles(page: Page<T>) -> Vec<Arc<T>> {
        page.into_rows().into_iter().map(Arc::new).collect()
    }

    /// Cache `window` under its start offset, evicting the oldest if the
    /// bound is exceeded.
    fn commit_window(
        rows: &mut WindowedListRows<T>,
        start: usize,
        window: Vec<Arc<T>>,
    ) -> Arc<[Arc<T>]> {
        let window: Arc<[Arc<T>]> = window.into();
        if !rows.windows.contains_key(&start) {
            rows.eviction_order.push_back(start);
        }
        rows.windows.insert(start, Arc::clone(&window));
        Self::enforce_bound(rows);
        window
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
