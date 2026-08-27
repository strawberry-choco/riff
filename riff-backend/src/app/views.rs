//! The Session Views facade: the UI's single read seam over the Application
//! Store (ADR 0002).
//!
//! [`SessionViews`] owns the five bounded Session Projections, the Library
//! query port, and the session-local [`StoreGeneration`] counter. Every view
//! shape the UI renders has one method here; callers pass only intent (a
//! folder path, a search query, a queue) and receive ready-to-render data.
//! Generation fetches, window bookkeeping, staleness handling, and
//! store-error fallbacks all live inside this module — UI code never touches
//! a loader closure, an `is_fresh` check, or a `Result`.
//!
//! Error policy: on a store error every method logs a `tracing::warn!` with
//! useful context and returns the default view (`false`, an empty list,
//! `None`, or the prior stale rows where a projection already holds them).
//! Projections only stamp their loaded generation after a successful fetch,
//! so the next call retries automatically.

use crate::app::projection::{
    BrowsingProjection, FolderProjection, PlaybackProjection, PlaylistProjection, ProjectionKey,
    SmartPlaylistsProjection, TrackListProjection, WINDOW_SIZE,
};
// The playlist view shapes are part of the seam's public surface: the
// projection module itself is private, so UI code imports these from here.
pub use crate::app::projection::{PlaylistEntryRow, PlaylistView};
use crate::app::store::{LibraryQueryStore, PlaylistStore, StoreGeneration};
use crate::domain::{
    Album, Artist, PlaybackQueue, Playlist, PlaylistId, SmartPlaylistKind, Track, TrackId,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Generic generation-keyed cache slot — the ONE implementation of "drop
/// when the epoch moved" behind every Session Projection (ADR 0002).
///
/// Holds at most one `(epoch, key, value)` entry stamped with the session
/// [`StoreGeneration`] epoch it was loaded at. Per operation a caller
/// observes the counter exactly once ([`Self::observe`]), serves the cached
/// value while [`Self::holds`] holds, and commits a fresh load through
/// [`Self::store`] or [`Self::slot`] only AFTER its loader succeeded — a
/// failed load leaves the previous entry untouched, so stale-but-present
/// data survives to the next retry.
///
/// Internal seam machinery: projections own private instances and no code
/// outside the views implementation may observe an epoch value.
pub struct GenerationCache<K, V> {
    /// The session counter this cache is keyed on.
    counter: StoreGeneration,
    /// The single cached entry, if any.
    loaded: Option<LoadedEntry<K, V>>,
}

/// One epoch-stamped cache entry.
struct LoadedEntry<K, V> {
    epoch: u64,
    key: K,
    value: V,
}

impl<K, V> GenerationCache<K, V> {
    /// Key a cache on `counter`; starts empty.
    pub fn new(counter: StoreGeneration) -> Self {
        Self {
            counter,
            loaded: None,
        }
    }

    /// The epoch currently being observed. Read ONCE per operation and
    /// reuse it for both the freshness check and the commit stamp, so a
    /// store commit racing mid-frame cannot split one logical read across
    /// two generations.
    pub fn observe(&self) -> u64 {
        self.counter.current()
    }

    /// Whether an entry is present and stamped with `epoch`.
    pub fn loaded_at(&self, epoch: u64) -> bool {
        self.loaded
            .as_ref()
            .is_some_and(|entry| entry.epoch == epoch)
    }

    /// Whether an entry is present, stamped with `epoch`, and keyed `key`.
    pub fn holds(&self, epoch: u64, key: &K) -> bool
    where
        K: PartialEq,
    {
        self.loaded
            .as_ref()
            .is_some_and(|entry| entry.epoch == epoch && entry.key == *key)
    }

    /// The cached value regardless of epoch — the stale-but-present error
    /// fallback reads through here.
    pub fn peek(&self) -> Option<&V> {
        self.loaded.as_ref().map(|entry| &entry.value)
    }

    /// Steal the cached value regardless of epoch (the fetch-then-swap
    /// merge path reuses prior-generation rows only when they still hold).
    pub fn take_value(&mut self) -> Option<V> {
        self.loaded.take().map(|entry| entry.value)
    }

    /// Drop the cached entry whatever it is stamped with.
    pub fn invalidate(&mut self) {
        self.loaded = None;
    }

    /// Commit `value` as loaded at `epoch` for `key`. The stamp happens
    /// here and nowhere else, so a failed load never advances it.
    pub fn store(&mut self, epoch: u64, key: K, value: V) {
        self.loaded = Some(LoadedEntry { epoch, key, value });
    }

    /// The mutable entry slot stamped `epoch` for `key`: drops any entry
    /// from a different epoch or key and hands out a freshly initialized
    /// default one. The commit step for lazily-filled multi-level bundles —
    /// call only after a successful load.
    pub fn slot(&mut self, epoch: u64, key: &K) -> &mut V
    where
        V: Default,
        K: Clone + PartialEq,
    {
        if !self.holds(epoch, key) {
            self.loaded = Some(LoadedEntry {
                epoch,
                key: key.clone(),
                value: V::default(),
            });
        }
        &mut self.loaded.as_mut().expect("entry just ensured").value
    }
}

/// One page of the flat/search track list: the authoritative total plus the
/// cached rows of one window.
pub struct TrackListPage {
    /// Total row count for the query as of the latest count read — the value
    /// to size row virtualization with.
    pub total: usize,
    /// Row index `rows` starts at (the projection's window-aligned offset).
    pub start: usize,
    /// The cached rows of this window, in store order. Shared with the
    /// projection's cache — handing a page out bumps a refcount instead of
    /// deep-copying the window's tracks.
    pub rows: Arc<[Track]>,
}

/// Flat facade over the Session Projections and the Library query port
/// (ADR 0002). One instance per UI session; constructed by composition root
/// injection in `main.rs`.
pub struct SessionViews {
    queries: Box<dyn LibraryQueryStore>,
    /// Query-use-only handle over the Playlists section: the projection
    /// reads through it; mutations commit through the store directly at the
    /// UI's call sites and invalidate via the playlist generation.
    playlist_queries: Box<dyn PlaylistStore>,
    tracks: TrackListProjection,
    browsing: BrowsingProjection,
    folders: FolderProjection,
    smart_playlists: SmartPlaylistsProjection,
    playback: PlaybackProjection,
    playlists: PlaylistProjection,
}

impl SessionViews {
    /// Wire the facade to the Library query port and the Playlists query
    /// port plus both session counters — the Library generation and the
    /// dedicated playlist generation the store bumps after each committed
    /// mutation. The handles are consumed here: every projection observes
    /// its counter internally, and no epoch value ever leaves this module.
    #[must_use]
    pub fn new(
        queries: Box<dyn LibraryQueryStore>,
        playlist_queries: Box<dyn PlaylistStore>,
        generation: StoreGeneration,
        playlist_generation: StoreGeneration,
    ) -> Self {
        // The projections observe the session counters internally from here
        // on: no per-call epoch crosses the seam again.
        let tracks = TrackListProjection::new(generation.clone(), ProjectionKey::Flat);
        let browsing = BrowsingProjection::new(generation.clone());
        let folders = FolderProjection::new(generation.clone());
        let smart_playlists = SmartPlaylistsProjection::new(generation.clone());
        let playback = PlaybackProjection::new(generation.clone());
        let playlists = PlaylistProjection::new(playlist_generation.clone(), generation.clone());
        Self {
            queries,
            playlist_queries,
            tracks,
            browsing,
            folders,
            smart_playlists,
            playback,
            playlists,
        }
    }

    // --- Flat list / search -------------------------------------------------

    /// One visible window of the flat track list (`query` empty) or the
    /// search results (`query` non-empty), together with the authoritative
    /// total row count.
    ///
    /// `offset` is any row index inside the wanted window; it is aligned down
    /// to the projection's window size internally. The first invalidated call
    /// refetches the window and recounts; fresh calls serve everything from
    /// cache. If a mutation commits between the count read and the refresh,
    /// the count is redone so `total` agrees with the refreshed rows.
    pub fn track_list(&mut self, query: &str, offset: usize) -> TrackListPage {
        let key = if query.is_empty() {
            ProjectionKey::Flat
        } else {
            ProjectionKey::Search(query.to_string())
        };
        if self.tracks.key() != &key {
            self.tracks.set_key(key);
        }

        // Outer count read: authoritative from the store whenever the
        // projection is invalidated; fresh frames reuse the cached count.
        let outer_generation = self.tracks.observe();
        let total = if self.tracks.is_fresh() {
            self.tracks.total()
        } else {
            self.count_rows(query)
        };

        let window_start = offset - (offset % WINDOW_SIZE);
        self.tracks.request_window(window_start);

        // Torn-count guard: if a mutation committed between the outer count
        // read and here, recount so the cached total agrees with the
        // refreshed rows; otherwise reuse the outer read (one COUNT query
        // per frame max).
        let generation = self.tracks.observe();
        let effective_total = if generation == outer_generation {
            total
        } else {
            self.count_rows(query)
        };

        if let Err(e) = self.tracks.refresh(effective_total, &mut |o, l| {
            if query.is_empty() {
                self.queries.tracks_window(o, l)
            } else {
                self.queries.search_window(query, o, l)
            }
        }) {
            tracing::warn!(
                "Failed to refresh the track list (query {query:?}) from the store: {e}"
            );
        }

        TrackListPage {
            total: effective_total,
            start: window_start,
            rows: self.tracks.window(window_start).unwrap_or_default(),
        }
    }

    /// The store's match count for `query`, defaulting to zero on error so
    /// callers can gate rendering without handling errors.
    fn count_rows(&self, query: &str) -> usize {
        let count = if query.is_empty() {
            self.queries.track_count()
        } else {
            self.queries.search_count(query)
        };
        match count {
            Ok(count) => count,
            Err(e) => {
                tracing::warn!("Failed to count tracks for query {query:?} in the store: {e}");
                0
            }
        }
    }

    /// Whether the search box's current query matches anything in the store.
    pub fn search_has_matches(&self, query: &str) -> bool {
        self.queries
            .search_count(query)
            .is_ok_and(|count| count > 0)
    }

    // --- Artist / album browsing ---------------------------------------------

    /// Every artist name-ascending, cached per generation. Fresh frames
    /// hand out an `Arc` clone of the cached list — no per-frame copy.
    pub fn artists(&mut self) -> Arc<[Artist]> {
        self.browsing
            .artists(&mut || self.queries.all_artists())
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load artists from the store: {e}");
                Arc::from([])
            })
    }

    /// One artist's albums in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    pub fn artist_albums(&mut self, artist: &str) -> Arc<[Album]> {
        self.browsing
            .artist_albums(artist, &mut |a| self.queries.artist_albums(a))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load albums for {artist}: {e}");
                Arc::from([])
            })
    }

    /// One album's tracks in canonical order, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    pub fn album_tracks(&mut self, album_artist: &str, album_title: &str) -> Arc<[Track]> {
        self.browsing
            .album_tracks(album_artist, album_title, &mut |a, t| {
                self.queries.album_tracks(a, t)
            })
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load tracks for {album_title}: {e}");
                Arc::from([])
            })
    }

    // --- Folder tree ----------------------------------------------------------

    /// Whether `folder` contains any audio, cached per generation.
    pub fn folder_has_audio(&mut self, folder: &Path) -> bool {
        self.folders
            .has_audio(folder, &mut |f| self.queries.folder_has_audio(f))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to probe folder {}: {e}", folder.display());
                false
            })
    }

    /// Whether any track under `folder` matches the search query, cached per
    /// (folder, query) per generation.
    pub fn folder_search_match(&mut self, folder: &Path, query: &str) -> bool {
        self.folders
            .has_search_match(folder, query, &mut |f, q| {
                self.queries.folder_has_search_match(f, q)
            })
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to search folder {}: {e}", folder.display());
                false
            })
    }

    /// Every track id under `folder`, path-ordered, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list — no
    /// per-frame copy of one id per track.
    pub fn folder_subtree_ids(&mut self, folder: &Path) -> Arc<[TrackId]> {
        self.folders
            .subtree_ids(folder, &mut |f| self.queries.track_ids_in_folder_tree(f))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to list folder tree {}: {e}", folder.display());
                Arc::from([])
            })
    }

    /// The child directories of `folder` holding audio, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    pub fn folder_children(&mut self, folder: &Path) -> Arc<[PathBuf]> {
        self.folders
            .children(folder, &mut |f| self.queries.subdirs_with_audio(f))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to list folder children {}: {e}", folder.display());
                Arc::from([])
            })
    }

    /// The tracks directly inside `folder`, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    pub fn folder_direct_tracks(&mut self, folder: &Path) -> Arc<[Track]> {
        self.folders
            .direct_tracks(folder, &mut |f| self.queries.tracks_in_folder(f))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to list folder tracks {}: {e}", folder.display());
                Arc::from([])
            })
    }

    // --- Smart playlists --------------------------------------------------------

    /// The computed read-only smart playlist for `kind`, cached per
    /// generation and limit. Fresh frames hand out an `Arc` clone of the
    /// cached list.
    pub fn smart_list(&mut self, kind: SmartPlaylistKind, limit: usize) -> Arc<[Track]> {
        self.smart_playlists
            .list(kind, limit, &mut |k, l| self.queries.smart_playlist(k, l))
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "Failed to compute smart playlist {}: {e}",
                    kind.display_name()
                );
                Arc::from([])
            })
    }

    // --- User playlists ---------------------------------------------------------

    /// Every user playlist in creation order, cached per playlist
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    /// On a store error the last good list is kept (a cold miss renders
    /// empty) and the next call retries.
    pub fn playlists(&mut self) -> Arc<[Playlist]> {
        self.playlists
            .playlists(&mut || self.playlist_queries.load_playlists())
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load playlists from the store: {e}");
                self.playlists.cached_playlists().unwrap_or_default()
            })
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
        match self.playlists.playlist_view(id, &mut |pid| {
            self.playlist_queries.load_playlist_entries(pid)
        }) {
            Ok(view) => Some(view),
            Err(e) => {
                tracing::warn!("Failed to load playlist entries for {id:?} from the store: {e}");
                Some(self.playlists.cached_view(id).unwrap_or_default())
            }
        }
    }

    // --- Playback-side reads ------------------------------------------------------

    /// Bring the playback slots (current Track + Up Next window) up to date
    /// with the store generation and `queue`'s shape. Cheap when nothing
    /// moved (a stamp comparison); otherwise refetches through the store.
    /// Failures keep the previous slots so stale-but-present beats blank.
    pub fn sync_playback(&mut self, queue: &PlaybackQueue, up_next_limit: usize) {
        if let Err(e) = self
            .playback
            .refresh(queue, up_next_limit, &mut |id| self.queries.get_track(id))
        {
            tracing::warn!("Failed to refresh the playback projection from the store: {e}");
        }
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
        match self
            .playback
            .selected_track(id, &mut |tid| self.queries.get_track(tid))
        {
            Ok(track) => track,
            Err(e) => {
                tracing::warn!("Failed to resolve selected track {id:?} from the store: {e}");
                None
            }
        }
    }

    // --- Uncached point reads -------------------------------------------------------

    /// Resolve one Track by id straight from the store (no projection): the
    /// tag-edit save flow and the playing-track album lookup need the live
    /// answer, not a cached view. `None` when unknown or on error.
    pub fn resolve_track(&self, id: &TrackId) -> Option<Track> {
        match self.queries.get_track(id) {
            Ok(track) => track,
            Err(e) => {
                tracing::warn!("Failed to resolve track {id:?} from the store: {e}");
                None
            }
        }
    }
}
