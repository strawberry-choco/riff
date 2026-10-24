//! The Session Views seam: the UI's single read seam over the Application
//! Store (ADR 0002).
//!
//! [`SessionViews`] owns the bounded Session Projections, the Library query
//! port, and the Playlists query port, from whose handles it derives the two
//! session-local generation counters
//! ([`crate::app::store::StoreGeneration`]) at construction. Every view
//! shape the UI renders has one method here; callers pass only intent (a
//! folder path, a search query, a queue) and receive ready-to-render data.
//! Staleness handling and store-error fallbacks all live inside this
//! module — UI code never touches a loader closure, an `is_fresh` check,
//! or a `Result`.
//!
//! ## Paged listings answer two questions, in two reads
//!
//! Every paged listing — the flat All Tracks list, the query-keyed hit
//! listings, the three browse roots, the two genre drill-downs — is read
//! through the **same** pair of methods: `*_count` for "how many rows does
//! this listing have" and `*_row` for "give me row *i*". They are two reads
//! rather than one bundled answer because bundling them is what put a page
//! cell in the caller's hands, and with it the window alignment, the
//! "refetch only when the row leaves the page in hand" decision, and the
//! page-start subtraction — an off-by-one written eight times here and
//! re-derived in eight render sites, where only a golden image could catch
//! it. All of that now lives behind the projection, so a caller names a row
//! index and receives that row.
//!
//! A count is its own honest read: it asks the listing's store page read
//! for the total alone, so no caller opens a page purely to learn a total
//! and no total is inferred from a window's length.
//!
//! Error policy: on a store error every method logs a `tracing::warn!` with
//! useful context and returns the default view (`false`, an empty list,
//! `None`, or `0`) — or the prior stale rows/total where a projection
//! already holds them, so a header never blanks over rows still on screen.
//! Projections only stamp their loaded generation after a successful fetch,
//! so the next call retries automatically.
//!
//! The one exception to "the UI never sees a `Result`" is the Custom Smart
//! Playlist reads (`custom_list_tree`, `custom_list_count`, `custom_list_row`,
//! and the draft's `custom_list_match_count`): the surfaces that draw those (a
//! listing's header, the rule editor's live count) must show *why* an answer is
//! stale or empty, so they answer through a `Result` whose `Err` carries the
//! last good value beside the reason — the typed-outcome precedent every other
//! visible failure already uses. `degrade` is unchanged everywhere else.

use crate::app::projection::{BrowseList, BrowseProjectionKey};
use crate::app::projection::{
    BrowsingProjection, CustomListDegraded, CustomPlaylistProjection, FolderProjection,
    GenreProjection, HitProjection, PlaylistProjection, ProjectionKey, SmartPlaylistsProjection,
    TrackListProjection, WindowedListProjection,
};
// The playlist view shapes are part of the seam's public surface: the
// projection module itself is private, so UI code imports these from here.
pub use crate::app::projection::{PlaylistEntryRow, PlaylistView};
use crate::app::state::TrackSort;
use crate::app::store::{
    CustomSmartPlaylist, LibraryCounts, LibraryQueryStore, PlaylistStore, SmartPlaylistTree,
    SortDirection, TrackListOrder,
};
use crate::domain::{
    Album, Artist, GenreCount, MatchableField, Playlist, PlaylistId, SmartPlaylistKind, Track,
    TrackId,
};
use riff_library::app::projection::CountsProjection;
use riff_playback::app::projection::PlaybackProjection;
use riff_playback::domain::PlaybackQueue;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Max entries in [`SessionViews`]'s per-artist first-track LRU. The cache
/// exists to keep the artists root's cover thumbnails off per-row projection
/// work; a cap keeps it bounded regardless of library size.
const ARTIST_FIRST_TRACK_CACHE_CAP: usize = 1000;

/// The number of rows a count read asks the listing's store page read for:
/// **none**. A count is its own read, so it asks for the listing's total
/// alone rather than for a window whose rows it would immediately throw
/// away. The store still takes its connection once for the whole read, and
/// the total it returns is the listing's own.
const COUNT_ROWS: usize = 0;

/// The counts read model behind every sidebar row (design-handoff issue
/// 05). Library-side fields carry the store's answer as of the latest
/// generation; `folder_roots` mirrors the session's library-path list; the
/// playlist sizes are entry counts in creation order.
pub struct SidebarCounts {
    /// Total tracks in the Library collection.
    pub tracks: usize,
    /// Distinct artists.
    pub artists: usize,
    /// Distinct `(album artist, title)` albums.
    pub albums: usize,
    /// Distinct non-empty per-track genres.
    pub genres: usize,
    /// Registered library root paths, as passed by the caller.
    pub folder_roots: usize,
    /// Each smart playlist's unbounded total, in `SmartPlaylistKind::ALL`
    /// order.
    pub smart_lists: Arc<[(SmartPlaylistKind, usize)]>,
    /// Each user playlist's entry count, in creation order.
    pub playlists: Vec<(PlaylistId, usize)>,
}

/// The flat read seam over the Session Projections and the Library query port
/// (ADR 0002). One instance per UI session; constructed by composition root
/// injection in `main.rs`.
pub struct SessionViews {
    queries: Box<dyn LibraryQueryStore>,
    /// Query-use-only handle over the Playlists section: the projection
    /// reads through it; mutations commit through the store directly at the
    /// UI's call sites and invalidate via the playlist generation.
    playlist_queries: Box<dyn PlaylistStore>,
    tracks: TrackListProjection,
    /// The two query-keyed hit-list projections (Albums and Artists roots
    /// under a query): bounded windows keyed by the query text, so a
    /// keystroke retarget drops stale rows even at an unchanged generation.
    hit_albums: WindowedListProjection<String, Album>,
    hit_artists: WindowedListProjection<String, Artist>,
    /// The paged browse projections (paginate-browse-columns): one bounded
    /// window cache per browse root — Artists, Albums, Genres — plus the two
    /// genre drill-downs. Each is keyed by its query signature (the listing
    /// plus the sort direction), so retargeting either drops cached rows
    /// even at an unchanged generation.
    artists_pages: WindowedListProjection<BrowseProjectionKey, Artist>,
    albums_pages: WindowedListProjection<BrowseProjectionKey, Album>,
    genres_pages: WindowedListProjection<BrowseProjectionKey, GenreCount>,
    genre_artists_pages: WindowedListProjection<BrowseProjectionKey, Artist>,
    genre_albums_pages: WindowedListProjection<BrowseProjectionKey, Album>,
    /// The generation-cached scoped hit reads (an album's hit tracks, the
    /// album name-hit boolean, hit-scoped genre tracks, hit-scoped genre
    /// counts): bounded, generation-cached like the browsing/genre reads,
    /// keyed by their full query signature.
    hits: HitProjection,
    browsing: BrowsingProjection,
    /// Per-artist first-track ids (the artists-root cover thumbnails), LRU
    /// evicted beyond [`ARTIST_FIRST_TRACK_CACHE_CAP`]. The browsing
    /// projection already caches per generation; this second layer lets the
    /// artists root resolve one track per artist without per-row projection
    /// bookkeeping on every frame.
    artist_first_track_cache: HashMap<String, TrackId>,
    artist_first_track_lru: Vec<String>,
    folders: FolderProjection,
    smart_playlists: SmartPlaylistsProjection,
    /// The Custom Smart Playlist listing's bounded windows plus the
    /// generation-cached tree read they compile from (ADR 0002 — the new
    /// list is read through a Session Projection like every other listing,
    /// and through a *new* cache, never the built-ins' unbounded-Vec one).
    custom_lists: CustomPlaylistProjection,
    genres: GenreProjection,
    counts: CountsProjection,
    playback: PlaybackProjection,
    playlists: PlaylistProjection,
}

/// The Session Views seam's one store-error policy: on a store error, log a
/// `tracing::warn!` carrying `context` and degrade to `fallback`, so the UI
/// never sees a `Result`.
///
/// `context` and `fallback` are producers, not values: both run on the error
/// path only, so a read never formats a message or computes a fallback it
/// never uses. Every read site — the paged listings' two arms, the flat
/// count/row pair, the scoped and unbounded listings, the folder and sidebar
/// reads, the playlist reads, and the playback-slot reads — supplies only its
/// own context and its own fallback through this one implementation.
fn degrade<T, E: std::fmt::Display>(
    result: Result<T, E>,
    context: impl FnOnce() -> String,
    fallback: impl FnOnce() -> T,
) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            let context = context();
            tracing::warn!("{context}: {error}");
            fallback()
        }
    }
}

/// Generate one paged listing's `*_count` / `*_row` method pair — the shape
/// the "Paged listings" section comment inside [`SessionViews`] describes.
/// Both arms delegate their store-error policy to the module's one
/// [`degrade`] combinator, so the paged and hand-written reads cannot drift.
///
/// - `$count` / `$row`: the two method names.
/// - `args`: the parameters both take; `$row` takes the same list plus the
///   row `index` last.
/// - `projection`: the projection field the pair reads through.
/// - `key`: the projection key expression, built from the args.
/// - `page`: the store query method and its leading arguments; the macro
///   appends the `(offset, limit)` window and routes it through
///   `self.queries`.
/// - `listing`: the human label interpolated into the warn logs — a
///   `format!` over the args, evaluated on the error path only.
macro_rules! paged_listing {
    (
        $(#[$count_meta:meta])* ; $(#[$row_meta:meta])* ;
        $count:ident, $row:ident, $ret:ty,
        args ($($arg:ident: $arg_ty:ty),*),
        projection $proj:ident,
        key $key:expr,
        page $page:ident ($($parg:expr),* $(,)?),
        listing $listing:expr $(,)?
    ) => {
        $(#[$count_meta])*
        pub fn $count(&mut self, $($arg: $arg_ty),*) -> usize {
            let result = self.$proj.count($key, &mut || {
                self.queries
                    .$page($($parg,)* 0, COUNT_ROWS)
                    .map(|page| page.total())
            });
            degrade(
                result,
                || format!("Failed to count {} in the store", $listing),
                || self.$proj.cached_count().unwrap_or(0),
            )
        }

        $(#[$row_meta])*
        pub fn $row(&mut self, $($arg: $arg_ty,)* index: usize) -> Option<$ret> {
            let result = self.$proj.row($key, index, &mut |offset, limit| {
                self.queries.$page($($parg,)* offset, limit)
            });
            degrade(
                result,
                || format!("Failed to refresh {} from the store", $listing),
                || self.$proj.cached_row(index),
            )
        }
    };
}

impl SessionViews {
    /// Wire the seam to the Library query port and the Playlists query port —
    /// and to nothing else. The seam derives both session counters itself,
    /// at one instant, from the handles it is about to keep: the Library
    /// generation read once from the Library query port and the dedicated
    /// playlist generation (the one the store bumps after each committed
    /// playlist mutation) read once from the Playlists query port. The
    /// same-store-same-instant rule — both counters belong to the same store
    /// the seam reads through, taken together — is enforced by this shape: a
    /// stale counter, or one from a different store handle, is
    /// unrepresentable. The handles are consumed here: every projection
    /// observes its counter internally, and no epoch value ever leaves this
    /// module.
    #[must_use]
    pub fn new(
        queries: Box<dyn LibraryQueryStore>,
        playlist_queries: Box<dyn PlaylistStore>,
    ) -> Self {
        // The wiring read, one per port, both at this instant: the counters
        // the projections are about to observe come from the ports this
        // constructor keeps, and from here on the projections observe the
        // session counters internally — no per-call epoch crosses the seam.
        let generation = queries.library_generation();
        let playlist_generation = playlist_queries.playlist_generation();
        let tracks = TrackListProjection::new(
            generation.clone(),
            ProjectionKey::Flat(TrackListOrder::default()),
        );
        let hit_albums = WindowedListProjection::new(generation.clone(), String::new());
        let hit_artists = WindowedListProjection::new(generation.clone(), String::new());
        let artists_pages = WindowedListProjection::new(
            generation.clone(),
            BrowseProjectionKey {
                list: BrowseList::Artists,
                direction: SortDirection::Ascending,
            },
        );
        let albums_pages = WindowedListProjection::new(
            generation.clone(),
            BrowseProjectionKey {
                list: BrowseList::Albums,
                direction: SortDirection::Ascending,
            },
        );
        let genres_pages = WindowedListProjection::new(
            generation.clone(),
            BrowseProjectionKey {
                list: BrowseList::Genres,
                direction: SortDirection::Ascending,
            },
        );
        let genre_artists_pages = WindowedListProjection::new(
            generation.clone(),
            BrowseProjectionKey {
                list: BrowseList::ArtistsInGenre(String::new()),
                direction: SortDirection::Ascending,
            },
        );
        let genre_albums_pages = WindowedListProjection::new(
            generation.clone(),
            BrowseProjectionKey {
                list: BrowseList::ArtistAlbumsInGenre {
                    artist: String::new(),
                    genre: String::new(),
                },
                direction: SortDirection::Ascending,
            },
        );
        let hits = HitProjection::new(generation.clone());
        let browsing = BrowsingProjection::new(generation.clone());
        let folders = FolderProjection::new(generation.clone());
        let smart_playlists = SmartPlaylistsProjection::new(generation.clone());
        let custom_lists =
            CustomPlaylistProjection::new(playlist_generation.clone(), generation.clone());
        let genres = GenreProjection::new(generation.clone());
        let counts = CountsProjection::new(generation.clone());
        let playback = PlaybackProjection::new(generation.clone());
        let playlists = PlaylistProjection::new(playlist_generation.clone(), generation.clone());
        Self {
            queries,
            playlist_queries,
            tracks,
            hit_albums,
            hit_artists,
            artists_pages,
            albums_pages,
            genres_pages,
            genre_artists_pages,
            genre_albums_pages,
            hits,
            browsing,
            artist_first_track_cache: HashMap::new(),
            artist_first_track_lru: Vec::new(),
            folders,
            smart_playlists,
            custom_lists,
            genres,
            counts,
            playback,
            playlists,
        }
    }

    // --- Paged listings: the count read and the row read --------------------
    //
    // Every paged listing in the app — the flat All Tracks list, the search
    // results, the hit-album and hit-artist roots, the three browse roots,
    // and the two genre drill-downs — is served by the same pair of methods
    // in the same shape, and every one of them is two reads rather than one
    // bundled answer. `*_count` is the listing's own total; `*_row` is one
    // row of it. The window alignment, the refetch decision, and the
    // window-start subtraction are the projection's, so a caller passes a
    // row index and receives that row.
    //
    // `sort`/`direction` is part of each query signature, so reversing
    // A–Z / Z–A retargets the projection exactly like a keystroke does: the
    // direction lands in the store's `ORDER BY`, not an in-memory reversal
    // of an ascending copy, so a row index names the same row before and
    // after the flip.
    //
    // Error policy, once, for every one of them: a failed read logs a
    // `tracing::warn!` with its context and degrades to the projection's
    // last good answer — the last good row, the last good total — or to the
    // default (`None`, `0`) when there is none. A header never blanks over
    // rows still on screen. (The Custom Smart Playlist trio below is the
    // documented exception: it answers as a typed outcome carrying the
    // reason beside the last good value.)
    // --- Flat list / search -------------------------------------------------

    /// The store order the listing's shared sort spells — the one
    /// `TrackSort` → `TrackListOrder` translation the seam uses (the flat
    /// list, the search, and the Custom Smart Playlist listing alike).
    fn track_list_order(sort: TrackSort) -> TrackListOrder {
        match sort {
            TrackSort::NumberAsc => TrackListOrder::PathAsc,
            TrackSort::NumberDesc => TrackListOrder::PathDesc,
            TrackSort::TitleAsc => TrackListOrder::TitleAsc,
            TrackSort::TitleDesc => TrackListOrder::TitleDesc,
        }
    }

    /// The query signature of the flat track list (`query` empty) or the
    /// search results (`query` non-empty) in `sort`.
    fn track_key(query: &str, sort: TrackSort) -> (ProjectionKey, TrackListOrder) {
        let order = Self::track_list_order(sort);
        let key = if query.is_empty() {
            ProjectionKey::Flat(order)
        } else {
            ProjectionKey::Search(query.to_string(), order)
        };
        (key, order)
    }

    /// How many rows the flat track list (`query` empty) or the search
    /// results (`query` non-empty) have, in `sort` — the value a surface
    /// sizes row virtualization with.
    ///
    /// Its own read: the store is asked for the listing's total and no rows
    /// at all, so a caller that only needs a total never opens a window to
    /// throw away. Cached per query signature per generation, so a frame
    /// costs no store read; a failed read keeps the last good total.
    pub fn track_count(&mut self, query: &str, sort: TrackSort) -> usize {
        let (key, order) = Self::track_key(query, sort);
        let result = self.tracks.count(key, &mut || {
            let page = if query.is_empty() {
                self.queries.tracks_page(order, 0, COUNT_ROWS)
            } else {
                self.queries.search_page(query, order, 0, COUNT_ROWS)
            };
            page.map(|page| page.total())
        });
        degrade(
            result,
            || {
                let listing = if query.is_empty() {
                    "the flat list"
                } else {
                    "the search"
                };
                format!("Failed to count {listing} (query {query:?}) in the store")
            },
            || self.tracks.cached_count().unwrap_or(0),
        )
    }

    /// Row `index` of the flat track list (`query` empty) or the search
    /// results (`query` non-empty), in `sort` — the Track a surface is
    /// about to draw. `None` when the listing has no row at that index.
    ///
    /// The row is a shared handle, so the caller may hold it while it makes
    /// other seam reads. A row inside a window the projection already has
    /// costs no store read; a row past the window in hand fetches it. The
    /// returned row is the listing's own at `index` — a caller never sees an
    /// offset, a window start, or a window boundary.
    pub fn track_row(&mut self, query: &str, sort: TrackSort, index: usize) -> Option<Arc<Track>> {
        let (key, order) = Self::track_key(query, sort);
        let result = self.tracks.row(key, index, &mut |offset, limit| {
            if query.is_empty() {
                self.queries.tracks_page(order, offset, limit)
            } else {
                self.queries.search_page(query, order, offset, limit)
            }
        });
        degrade(
            result,
            || {
                let listing = if query.is_empty() {
                    "the flat list"
                } else {
                    "the search"
                };
                format!("Failed to refresh {listing} (query {query:?}) from the store")
            },
            || self.tracks.cached_row(index),
        )
    }

    // --- Entity hit roots (search across Library sections) ------------------

    paged_listing! {
        /// How many rows the hit-albums listing for `query` has — what the
        /// Albums root renders under a query. Its own read, cached per query
        /// per generation; a failed read keeps the last good total.
        ;
        /// Row `index` of the hit-albums listing for `query`. The listing is a
        /// bounded-window projection keyed by the query text: a keystroke
        /// retarget drops stale rows even at an unchanged generation, and a row
        /// the projection already has costs no store read. `None` when the
        /// listing has no row at that index.
        ;
        hit_album_count, hit_album_row, Arc<Album>,
        args (query: &str),
        projection hit_albums,
        key query.to_string(),
        page hit_albums_page(query),
        listing format!("the hit-albums list (query {query:?})"),
    }

    paged_listing! {
        /// How many rows the hit-artists listing for `query` has — what the
        /// Artists root renders under a query. Its own read, cached per query
        /// per generation; a failed read keeps the last good total.
        ;
        /// Row `index` of the hit-artists listing for `query`,
        /// name-ascending. `None` when the listing has no row at that index.
        ;
        hit_artist_count, hit_artist_row, Arc<Artist>,
        args (query: &str),
        projection hit_artists,
        key query.to_string(),
        page hit_artists_page(query),
        listing format!("the hit-artists list (query {query:?})"),
    }

    // --- Browse roots --------------------------------------------------------

    paged_listing! {
        /// How many rows the Artists root has, name-ascending or
        /// name-descending per `direction`. Its own read, cached per query
        /// signature per generation; a failed read keeps the last good total.
        ;
        /// Row `index` of the Artists root, name-ascending or name-descending
        /// per `direction` — the Artist a surface is about to draw. `None` when
        /// the listing has no row at that index.
        ;
        artist_count, artist_row, Arc<Artist>,
        args (direction: SortDirection),
        projection artists_pages,
        key BrowseProjectionKey { list: BrowseList::Artists, direction },
        page artists_page(direction),
        listing format!("the artists list (direction {direction:?})"),
    }

    paged_listing! {
        /// How many rows the Albums root has over the flat browsing order (or
        /// its exact reversal per `direction`). Its own read, cached per query
        /// signature per generation; a failed read keeps the last good total.
        ;
        /// Row `index` of the Albums root, in the flat browsing order or its
        /// exact reversal per `direction`. Each album carries its full track
        /// ids, so the cover thumbnail and detail line need no extra queries.
        /// `None` when the listing has no row at that index.
        ;
        album_count, album_row, Arc<Album>,
        args (direction: SortDirection),
        projection albums_pages,
        key BrowseProjectionKey { list: BrowseList::Albums, direction },
        page albums_page(direction),
        listing format!("the albums list (direction {direction:?})"),
    }

    paged_listing! {
        /// How many rows the Genres root has, name-ascending or name-descending
        /// per `direction`. Its own read, cached per query signature per
        /// generation; a failed read keeps the last good total.
        ;
        /// Row `index` of the Genres root, name-ascending or name-descending
        /// per `direction`. Each row carries its per-track count. `None` when
        /// the listing has no row at that index.
        ;
        genre_count, genre_row, Arc<GenreCount>,
        args (direction: SortDirection),
        projection genres_pages,
        key BrowseProjectionKey { list: BrowseList::Genres, direction },
        page genres_page(direction),
        listing format!("the genres list (direction {direction:?})"),
    }

    // --- Genre drill-downs ---------------------------------------------------

    paged_listing! {
        /// How many artists `genre` has, name-ascending or name-descending per
        /// `direction`. Its own read; a genre change retargets the projection,
        /// dropping the previous genre's total even at an unchanged generation.
        ;
        /// Row `index` of a genre's artists, name-ascending or name-descending
        /// per `direction` — what the genre drill's artists column renders. A
        /// genre change retargets the projection, dropping stale rows even at an
        /// unchanged generation. `None` when the listing has no row at that
        /// index.
        ;
        genre_artist_count, genre_artist_row, Arc<Artist>,
        args (genre: &str, direction: SortDirection),
        projection genre_artists_pages,
        key BrowseProjectionKey {
            list: BrowseList::ArtistsInGenre(genre.to_string()),
            direction,
        },
        page artists_in_genre_page(genre, direction),
        listing format!("the artists list in genre {genre:?} (direction {direction:?})"),
    }

    paged_listing! {
        /// How many albums `artist` has within `genre`, in canonical browsing
        /// order or its exact reversal per `direction`. Its own read; a genre or
        /// artist change retargets the projection.
        ;
        /// Row `index` of an artist's albums within `genre`, in canonical
        /// browsing order or its exact reversal per `direction` — what the genre
        /// drill's album column renders. A genre or artist change retargets the
        /// projection, dropping stale rows even at an unchanged generation.
        /// `None` when the listing has no row at that index.
        ;
        genre_album_count, genre_album_row, Arc<Album>,
        args (artist: &str, genre: &str, direction: SortDirection),
        projection genre_albums_pages,
        key BrowseProjectionKey {
            list: BrowseList::ArtistAlbumsInGenre {
                artist: artist.to_string(),
                genre: genre.to_string(),
            },
            direction,
        },
        page artist_albums_in_genre_page(artist, genre, direction),
        listing format!("the albums list for {artist} in genre {genre:?} (direction {direction:?})"),
    }

    // --- Scoped entity hits --------------------------------------------------

    /// One album's tracks that match `query`, in canonical album-track
    /// order, cached per (album, query) per generation. This is a
    /// *scoped* read, not a listing: it answers about one album's rows, so
    /// it is bounded by that album and takes no row index and no window.
    /// Empty on a store error (a `tracing::warn!` carries the context).
    pub fn album_hit_tracks(
        &mut self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Arc<[Track]> {
        let result =
            self.hits
                .album_hit_tracks(album_artist, album_title, query, &mut |a, t, q| {
                    self.queries.album_hit_tracks(a, t, q)
                });
        degrade(
            result,
            || {
                format!(
                    "Failed to load hit tracks for {album_title} (query {query:?}) from the store"
                )
            },
            || Arc::from([]),
        )
    }

    /// Whether `album` is itself a name hit for `query` (its album artist or
    /// title matched), cached per (album, query) per generation. `false` on
    /// a store error or for unknown albums.
    pub fn album_is_name_hit(
        &mut self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> bool {
        let result =
            self.hits
                .album_is_name_hit(album_artist, album_title, query, &mut |a, t, q| {
                    self.queries.album_is_name_hit(a, t, q)
                });
        degrade(
            result,
            || {
                format!(
                    "Failed to check whether {album_title} is a name hit (query {query:?}) in the store"
                )
            },
            || false,
        )
    }

    /// Whether `artist` is itself a name hit for `query` (its own name
    /// matched), cached per (artist, query) per generation. `false` on a
    /// store error or for unknown artists.
    pub fn artist_is_name_hit(&mut self, artist: &str, query: &str) -> bool {
        let result = self.hits.artist_is_name_hit(artist, query, &mut |a, q| {
            self.queries.artist_is_name_hit(a, q)
        });
        degrade(
            result,
            || {
                format!(
                    "Failed to check whether {artist} is a name hit (query {query:?}) in the store"
                )
            },
            || false,
        )
    }

    /// One album's tracks that match `query` among its `genre`-bearing
    /// tracks, in canonical album-track order, cached per
    /// (album, genre, query) per generation. Scoped like
    /// [`Self::album_hit_tracks`]: it answers about one album's rows, so it
    /// takes no row index and no window. Empty on a store error.
    pub fn album_hit_tracks_in_genre(
        &mut self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        query: &str,
    ) -> Arc<[Track]> {
        let result = self.hits.genre_album_tracks(
            album_artist,
            album_title,
            genre,
            query,
            &mut |a, t, g, q| self.queries.album_hit_tracks_in_genre(a, t, g, q),
        );
        degrade(
            result,
            || {
                format!(
                    "Failed to load hit tracks for {album_title} in genre {genre:?} (query {query:?}) from the store"
                )
            },
            || Arc::from([]),
        )
    }

    // --- Artist / album browsing ---------------------------------------------
    //
    // The Artists *listing* is a paged browse root and reads through
    // `artist_count` / `artist_row` like every other listing. What browsing
    // reaches *past* a listing — one artist's albums, one album's tracks —
    // is unbounded by anything but that artist or album, so it is a
    // generation-cached list rather than a paged read.

    /// One artist's albums in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    pub fn artist_albums(&mut self, artist: &str) -> Arc<[Album]> {
        let result = self
            .browsing
            .artist_albums(artist, &mut |a| self.queries.artist_albums(a));
        degrade(
            result,
            || format!("Failed to load albums for {artist}"),
            || Arc::from([]),
        )
    }

    /// One album's tracks in canonical order, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    pub fn album_tracks(&mut self, album_artist: &str, album_title: &str) -> Arc<[Track]> {
        let result = self
            .browsing
            .album_tracks(album_artist, album_title, &mut |a, t| {
                self.queries.album_tracks(a, t)
            });
        degrade(
            result,
            || format!("Failed to load tracks for {album_title}"),
            || Arc::from([]),
        )
    }

    /// The first track (by file path) of an artist's first album, used for
    /// the artists-root cover thumbnails. Served from a bounded per-artist
    /// LRU so the artists root never repeats the projection reads per row
    /// (the root renders the whole visible window through this seam).
    pub fn artist_first_track(&mut self, artist: &str) -> Option<TrackId> {
        if let Some(track_id) = self.artist_first_track_cache.get(artist) {
            let _ = crate::app::lru_insert(
                &mut self.artist_first_track_lru,
                artist.to_string(),
                ARTIST_FIRST_TRACK_CACHE_CAP,
            );
            return Some(track_id.clone());
        }
        let albums = self.artist_albums(artist);
        let first_album = albums.first()?;
        let first_track = self
            .album_tracks(&first_album.artist, &first_album.title)
            .first()
            .map(|track| track.id.clone())?;
        for old in crate::app::lru_insert(
            &mut self.artist_first_track_lru,
            artist.to_string(),
            ARTIST_FIRST_TRACK_CACHE_CAP,
        ) {
            self.artist_first_track_cache.remove(&old);
        }
        self.artist_first_track_cache
            .insert(artist.to_string(), first_track.clone());
        Some(first_track)
    }

    // --- Genre read model -------------------------------------------------------

    /// Every genre with its per-track count, name-ascending, cached per
    /// generation. The sidebar's total-genres count is the list's length.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    pub fn genres(&mut self) -> Arc<[GenreCount]> {
        let result = self.genres.counts(&mut || self.queries.genre_counts());
        degrade(
            result,
            || "Failed to load genre counts from the store".to_string(),
            || Arc::from([]),
        )
    }

    /// Artists having at least one track with `genre`, name-ascending, each
    /// with their genre-matching album keys, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    pub fn artists_in_genre(&mut self, genre: &str) -> Arc<[Artist]> {
        let result = self
            .genres
            .artists_in_genre(genre, &mut |g| self.queries.artists_in_genre(g));
        degrade(
            result,
            || format!("Failed to load artists for genre {genre:?}"),
            || Arc::from([]),
        )
    }

    /// One artist's albums holding a track with `genre`, in canonical
    /// browsing order with genre-matching track membership, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    pub fn artist_albums_in_genre(&mut self, artist: &str, genre: &str) -> Arc<[Album]> {
        let result = self
            .genres
            .artist_albums_in_genre(artist, genre, &mut |a, g| {
                self.queries.artist_albums_in_genre(a, g)
            });
        degrade(
            result,
            || format!("Failed to load albums for {artist} in genre {genre:?}"),
            || Arc::from([]),
        )
    }

    /// One album's tracks with `genre`, in canonical album-track order,
    /// cached per generation. Fresh frames hand out an `Arc` clone of the
    /// cached list.
    pub fn album_tracks_in_genre(
        &mut self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
    ) -> Arc<[Track]> {
        let result =
            self.genres
                .album_tracks_in_genre(album_artist, album_title, genre, &mut |a, t, g| {
                    self.queries.album_tracks_in_genre(a, t, g)
                });
        degrade(
            result,
            || format!("Failed to load tracks for {album_title} in genre {genre:?}"),
            || Arc::from([]),
        )
    }

    // --- Folder tree ----------------------------------------------------------

    /// Whether `folder` contains any audio, cached per generation.
    pub fn folder_has_audio(&mut self, folder: &Path) -> bool {
        let result = self
            .folders
            .has_audio(folder, &mut |f| self.queries.folder_has_audio(f));
        degrade(
            result,
            || format!("Failed to probe folder {}", folder.display()),
            || false,
        )
    }

    /// Whether any track under `folder` matches the search query, cached per
    /// (folder, query) per generation.
    pub fn folder_search_match(&mut self, folder: &Path, query: &str) -> bool {
        let result = self.folders.has_search_match(folder, query, &mut |f, q| {
            self.queries.folder_has_search_match(f, q)
        });
        degrade(
            result,
            || format!("Failed to search folder {}", folder.display()),
            || false,
        )
    }

    /// Every track id under `folder`, path-ordered, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list — no
    /// per-frame copy of one id per track.
    pub fn folder_subtree_ids(&mut self, folder: &Path) -> Arc<[TrackId]> {
        let result = self
            .folders
            .subtree_ids(folder, &mut |f| self.queries.track_ids_in_folder_tree(f));
        degrade(
            result,
            || format!("Failed to list folder tree {}", folder.display()),
            || Arc::from([]),
        )
    }

    /// The child directories of `folder` holding audio, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    pub fn folder_children(&mut self, folder: &Path) -> Arc<[PathBuf]> {
        let result = self
            .folders
            .children(folder, &mut |f| self.queries.subdirs_with_audio(f));
        degrade(
            result,
            || format!("Failed to list folder children {}", folder.display()),
            || Arc::from([]),
        )
    }

    /// The tracks directly inside `folder`, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    pub fn folder_direct_tracks(&mut self, folder: &Path) -> Arc<[Track]> {
        let result = self
            .folders
            .direct_tracks(folder, &mut |f| self.queries.tracks_in_folder(f));
        degrade(
            result,
            || format!("Failed to list folder tracks {}", folder.display()),
            || Arc::from([]),
        )
    }

    // --- Smart playlists --------------------------------------------------------

    /// The computed read-only smart playlist for `kind`, cached per
    /// generation and limit. Fresh frames hand out an `Arc` clone of the
    /// cached list.
    pub fn smart_list(&mut self, kind: SmartPlaylistKind, limit: usize) -> Arc<[Track]> {
        let result = self
            .smart_playlists
            .list(kind, limit, &mut |k, l| self.queries.smart_playlist(k, l));
        degrade(
            result,
            || format!("Failed to compute smart playlist {}", kind.display_name()),
            || Arc::from([]),
        )
    }

    // --- Custom Smart Playlists ---------------------------------------------
    //
    // A Custom list is read through the same bounded discipline as every
    // other listing — windows plus a total, cached per query signature per
    // generation — but its three reads answer as a typed outcome rather
    // than a bare value: the surfaces that draw them (the listing's header,
    // the rule editor's count line) must be able to show *why* an answer is
    // stale or empty, which is exactly what no `degrade`d read can carry.
    // The store error still warns and the last good value is still served;
    // it now arrives beside the reason instead of alone.

    /// The Rule tree of the Custom Smart Playlist with `id`, cached per
    /// playlist generation (a cached absence included). `Ok(None)` means the
    /// list is gone; `Err` carries the last good tree beside the reason when
    /// the store read failed.
    pub fn custom_list_tree(
        &mut self,
        id: &PlaylistId,
    ) -> Result<Option<SmartPlaylistTree>, CustomListDegraded<Option<SmartPlaylistTree>>> {
        self.custom_lists
            .tree(id, &mut |id| self.queries.smart_playlist_tree(id))
    }

    /// How many Tracks the Custom Smart Playlist with `id` currently matches
    /// under `sort` and the listing's current `needle` (`None` for no
    /// needle). Its own read: the store's Listing Page is asked for the
    /// total alone (zero rows).
    ///
    /// # Errors
    /// `Err` carries the value to draw beside the reason: the last good
    /// total for this query when a store failure interrupted a listing in
    /// hand, and `0` — matching nothing, never a silent "no rows" — when the
    /// list is gone or its tree holds an Unreadable Rule, for which no query
    /// is ever issued.
    pub fn custom_list_count(
        &mut self,
        id: &PlaylistId,
        sort: TrackSort,
        needle: Option<&str>,
    ) -> Result<usize, CustomListDegraded<usize>> {
        self.custom_lists.count(
            id,
            Self::track_list_order(sort),
            needle,
            &mut |id| self.queries.smart_playlist_tree(id),
            &mut |tree, order, needle, offset, limit| {
                self.queries
                    .custom_smart_playlist_page(tree, order, needle, offset, limit)
            },
        )
    }

    /// Row `index` of the Custom Smart Playlist with `id` under `sort` and
    /// the listing's current `needle` — the Track a surface is about to
    /// draw, from the bounded window cache (the window size and cache bound
    /// are the shared house's, never this listing's). `Ok(None)` means the
    /// listing has no row at that index.
    ///
    /// # Errors
    /// `Err` carries the value to draw beside the reason: the last good row
    /// for this query when a store failure interrupted a listing in hand,
    /// and `None` when the list is gone or its tree holds an Unreadable
    /// Rule, for which no query is ever issued.
    pub fn custom_list_row(
        &mut self,
        id: &PlaylistId,
        sort: TrackSort,
        needle: Option<&str>,
        index: usize,
    ) -> Result<Option<Arc<Track>>, CustomListDegraded<Option<Arc<Track>>>> {
        self.custom_lists.row(
            id,
            Self::track_list_order(sort),
            needle,
            index,
            &mut |id| self.queries.smart_playlist_tree(id),
            &mut |tree, order, needle, offset, limit| {
                self.queries
                    .custom_smart_playlist_page(tree, order, needle, offset, limit)
            },
        )
    }

    /// How many Tracks the draft tree `tree` matches right now — the count
    /// behind the Smart Playlists editor's live "matches N songs" line.
    ///
    /// It is the same bounded Listing Page read as [`Self::custom_list_count`]
    /// (the total alone, zero rows), taken for a **tree** rather than a list
    /// id: a draft has no id, and the tree it spells has never been saved, so
    /// the id-keyed accessors above cannot serve it. The seam holds the
    /// answer against the tree's compiled shape for the current library
    /// generation, so a repaint of an unchanged tree and an edit back to an
    /// earlier shape are both served without a store read — the read
    /// discipline ADR 0002 asks for, kept at the seam where its read counts
    /// are pinned.
    ///
    /// # Errors
    /// `Err` carries the value to draw beside the reason: the last good
    /// count for this shape (or `0` when none is held), and the reason a
    /// tree holding an Unreadable Rule is never counted — no query is ever
    /// issued for it, the same rule and the same words the id-keyed count
    /// answers with.
    pub fn custom_list_match_count(
        &mut self,
        tree: &SmartPlaylistTree,
    ) -> Result<usize, CustomListDegraded<usize>> {
        self.custom_lists
            .count_tree(tree, &mut |tree, order, needle, offset, limit| {
                self.queries
                    .custom_smart_playlist_page(tree, order, needle, offset, limit)
            })
    }

    // --- Rule value pickers ------------------------------------------------------

    /// At most `limit` distinct values the Library holds for a pickable
    /// **text** Matchable Field — the list a Rule row's picker draws beside
    /// the listener's own typing.
    ///
    /// One uncached pass-through to the Library query port: a picker is read
    /// **once per open**, never once per frame, so the projection machinery
    /// the listings use would only add a staleness question this read does
    /// not have. The caller is the editor pane, which asks on the frame a row
    /// reports `PickerOpened` (ticket 14's rule row). Absent values are the
    /// store's to exclude, and a field the catalog does not make pickable is
    /// a store error, which answers here as an empty list behind a
    /// `tracing::warn!` like every other degraded read.
    pub fn picker_texts(&mut self, field: MatchableField, limit: usize) -> Vec<String> {
        degrade(
            self.queries.picker_texts(field.stored_id(), limit),
            || format!("Failed to read the value picker for {}", field.stored_id()),
            Vec::new,
        )
    }

    /// At most `limit` distinct values the Library holds for a pickable
    /// **numeric** Matchable Field, ascending. The numeric half of
    /// [`Self::picker_texts`], with the same one-read-per-open discipline; a
    /// stored `0` is a present value and is offered like any other.
    pub fn picker_numbers(&mut self, field: MatchableField, limit: usize) -> Vec<i64> {
        degrade(
            self.queries.picker_numbers(field.stored_id(), limit),
            || {
                format!(
                    "Failed to read the numeric picker for {}",
                    field.stored_id()
                )
            },
            Vec::new,
        )
    }

    // --- Custom Smart Playlists --------------------------------------------------

    /// Every Custom Smart Playlist the listener has saved — id, name and
    /// creation order — which is the D14 enumeration read (ticket 15's
    /// Settings page lists them, ticket 12's sidebar rows wear them).
    ///
    /// Deliberately uncached at this seam: the read answers the *which lists
    /// exist* question only (no Tracks, no Rule tree, no counts), the surfaces
    /// that ask are the Smart Playlists pane — only while that section is the
    /// current one — and the sidebar section rather than a paged listing, and
    /// a new projection would need the generation story ticket 11's window
    /// cache owns. On a store error the list answers empty behind a
    /// `tracing::warn!`, like every other degraded read — a caller that must
    /// tell a failed read from an answered one asks
    /// [`Self::try_custom_smart_playlists`], the fallible half below.
    pub fn custom_smart_playlists(&mut self) -> Vec<CustomSmartPlaylist> {
        degrade(
            self.queries.custom_smart_playlists(),
            || "Failed to enumerate the Custom Smart Playlists from the store".to_string(),
            Vec::new,
        )
    }

    /// The fallible half of [`Self::custom_smart_playlists`]: every saved
    /// Custom Smart Playlist, or the store's own reason it could not answer.
    ///
    /// This is the reason channel (D6) applied to the one enumeration read
    /// whose *failure* must not be mistaken for its answer. The editor's
    /// reconcile asks whether the list a draft was written against still
    /// exists, and a degraded empty vector would read as "it was deleted" —
    /// destroying the listener's unsaved work on a transient store error. So
    /// the failure arrives as a reason instead, and the reconcile keeps the
    /// draft. The reason travels alone, with no value beside it: unlike the
    /// tree, total and row reads above, this read has no last-good answer to
    /// hand over (it is deliberately uncached), and the empty list a degraded
    /// read would answer with is exactly the value that must not be invented.
    ///
    /// The read is the same one [`Self::custom_smart_playlists`] makes, which
    /// stays on `degrade` for every caller that only draws the answer.
    pub fn try_custom_smart_playlists(&mut self) -> Result<Vec<CustomSmartPlaylist>, String> {
        self.queries.custom_smart_playlists().map_err(|error| {
            tracing::warn!(
                "Failed to enumerate the Custom Smart Playlists from the store: {error}"
            );
            error.to_string()
        })
    }

    // --- Sidebar counts ----------------------------------------------------------

    /// Every count the sidebar renders (design-handoff issue 05), in one
    /// ready-to-render shape: the library totals (tracks, artists, albums,
    /// genres), the registered folder roots, each smart playlist's total,
    /// and each user playlist's size. The library-side counts cache per
    /// generation (one store query per generation, not per frame); the
    /// playlist sizes ride the playlists read model; `folder_roots`
    /// passes through from the caller, which owns the session's library
    /// paths. On a store error the affected counts answer their defaults
    /// (`0`, an empty list) so the sidebar still renders.
    pub fn sidebar_counts(&mut self, folder_roots: usize) -> SidebarCounts {
        let library = degrade(
            self.counts
                .library_counts(&mut || self.queries.library_counts()),
            || "Failed to load the library counts from the store".to_string(),
            || Arc::new(LibraryCounts::default()),
        );
        let smart_lists = degrade(
            self.counts
                .smart_list_counts(&mut || self.queries.smart_list_counts()),
            || "Failed to load the smart-list counts from the store".to_string(),
            || Arc::from([]),
        );
        let playlists = self
            .playlists()
            .iter()
            .map(|playlist| (playlist.id.clone(), playlist.tracks.len()))
            .collect();
        SidebarCounts {
            tracks: library.tracks,
            artists: library.artists,
            albums: library.albums,
            genres: library.genres,
            folder_roots,
            smart_lists,
            playlists,
        }
    }

    /// How many tracks live under `folder` (component-wise subtree), cached
    /// per (generation, folder) — the per-music-folder count the Settings
    /// Library pane shows. `0` on a store error so the pane still renders.
    pub fn folder_track_count(&mut self, folder: &Path) -> usize {
        degrade(
            self.counts
                .folder_count(folder, &mut |f| self.queries.folder_track_count(f)),
            || format!("Failed to count tracks under {}", folder.display()),
            || 0,
        )
    }

    /// The timestamp of the last completed full library scan, cached per
    /// generation (a cached absence included). `None` when no scan has ever
    /// completed or the read failed — the footer just shows no stamp.
    pub fn last_scan(&mut self) -> Option<SystemTime> {
        self.last_full_scan_summary().map(|summary| summary.at)
    }

    /// The last completed full scan's summary — when it finished plus its
    /// file/error counts — cached per generation (a cached absence
    /// included). `None` when no scan has ever completed or the read failed
    /// (design-handoff issue 12).
    pub fn last_full_scan_summary(&mut self) -> Option<crate::app::store::FullScanSummary> {
        degrade(
            self.counts.last_scan(&mut || self.queries.last_full_scan()),
            || "Failed to read the last-scan summary from the store".to_string(),
            || None,
        )
    }

    // --- User playlists ---------------------------------------------------------

    /// Every user playlist in creation order, cached per playlist
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    /// On a store error the last good list is kept (a cold miss renders
    /// empty) and the next call retries.
    pub fn playlists(&mut self) -> Arc<[Playlist]> {
        let result = self
            .playlists
            .playlists(&mut || self.playlist_queries.load_playlists());
        degrade(
            result,
            || "Failed to load playlists from the store".to_string(),
            || self.playlists.cached_playlists().unwrap_or_default(),
        )
    }

    /// One user playlist's ready-to-render rows: the entry id, its
    /// Library-resolved Track when known, and the playability verdict, plus
    /// the playable ids for the header context menu. Cached per playlist
    /// generation and Library generation; on a store error the last good
    /// rows are kept (a cold miss renders empty). Unknown ids yield `None`
    /// without a third method.
    pub fn playlist_view(&mut self, id: &PlaylistId) -> Option<PlaylistView> {
        // Unknown ids answer None without touching the entry loader.
        if !self.playlists().iter().any(|playlist| &playlist.id == id) {
            return None;
        }
        let result = self.playlists.playlist_view(id, &mut |pid| {
            self.playlist_queries.load_playlist_entries(pid)
        });
        Some(degrade(
            result,
            || format!("Failed to load playlist entries for {id:?} from the store"),
            || self.playlists.cached_view(id).unwrap_or_default(),
        ))
    }

    // --- Playback-side reads ------------------------------------------------------

    /// Bring the playback slots (current Track + Up Next window) up to date
    /// with the store generation and `queue`'s shape. Cheap when nothing
    /// moved (a stamp comparison); otherwise refetches through the store.
    /// Failures keep the previous slots so stale-but-present beats blank.
    pub fn sync_playback(&mut self, queue: &PlaybackQueue, up_next_limit: usize) {
        degrade(
            self.playback
                .refresh(queue, up_next_limit, &mut |id| self.queries.get_track(id)),
            || "Failed to refresh the playback projection from the store".to_string(),
            || (),
        );
    }

    /// The resolved current Track, when one is playing and it still resolves.
    /// Read after [`Self::sync_playback`].
    #[must_use]
    pub fn playback_current(&self) -> Option<&Track> {
        self.playback.current()
    }

    /// The resolved Up Next window in Playback Queue order. Ids whose files
    /// left the library are skipped, so this can be shorter than the
    /// requested window. Read after [`Self::sync_playback`].
    #[must_use]
    pub fn playback_up_next(&self) -> &[Track] {
        self.playback.up_next()
    }

    /// The track-details panel's selected Track, cached until the selection
    /// or the generation moves. A cached absence (id unknown to the store)
    /// yields `None` without requerying per frame.
    pub fn selected_track(&mut self, id: &TrackId) -> Option<Track> {
        degrade(
            self.playback
                .selected_track(id, &mut |tid| self.queries.get_track(tid)),
            || format!("Failed to resolve selected track {id:?} from the store"),
            || None,
        )
    }
}
