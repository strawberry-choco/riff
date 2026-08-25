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
    BrowsingProjection, FolderProjection, PlaybackProjection, ProjectionKey,
    SmartPlaylistsProjection, TrackListProjection, WINDOW_SIZE,
};
use crate::app::store::{LibraryQueryStore, StoreGeneration};
use crate::domain::{Album, Artist, PlaybackQueue, SmartPlaylistKind, Track, TrackId};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    generation: StoreGeneration,
    playlist_generation: StoreGeneration,
    tracks: TrackListProjection,
    browsing: BrowsingProjection,
    folders: FolderProjection,
    smart_playlists: SmartPlaylistsProjection,
    playback: PlaybackProjection,
}

impl SessionViews {
    /// Wire the facade to the Library query port and both session
    /// generations — the Library generation and the dedicated playlist
    /// generation their mutation adapters bump after each committed store
    /// mutation.
    #[must_use]
    pub fn new(
        queries: Box<dyn LibraryQueryStore>,
        generation: StoreGeneration,
        playlist_generation: StoreGeneration,
    ) -> Self {
        Self {
            queries,
            generation,
            playlist_generation,
            tracks: TrackListProjection::new(ProjectionKey::Flat),
            browsing: BrowsingProjection::new(),
            folders: FolderProjection::new(),
            smart_playlists: SmartPlaylistsProjection::new(),
            playback: PlaybackProjection::new(),
        }
    }

    /// The session store generation's current value — the epoch every
    /// committed Library mutation advances. UI-local caches key on it so
    /// they re-resolve when the Library moves, exactly like the projections
    /// do.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.current()
    }

    /// The playlist store generation's current value — the epoch every
    /// committed playlist mutation advances, independent of
    /// [`Self::generation`] so entry edits never invalidate Library views.
    #[must_use]
    pub fn playlist_generation(&self) -> u64 {
        self.playlist_generation.current()
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
        let outer_generation = self.generation.current();
        let total = if self.tracks.is_fresh(outer_generation) {
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
        let generation = self.generation.current();
        let effective_total = if generation == outer_generation {
            total
        } else {
            self.count_rows(query)
        };

        if let Err(e) = self
            .tracks
            .refresh(generation, effective_total, &mut |o, l| {
                if query.is_empty() {
                    self.queries.tracks_window(o, l)
                } else {
                    self.queries.search_window(query, o, l)
                }
            })
        {
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
        let generation = self.generation.current();
        self.browsing
            .artists(generation, &mut || self.queries.all_artists())
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load artists from the store: {e}");
                Arc::from([])
            })
    }

    /// One artist's albums in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    pub fn artist_albums(&mut self, artist: &str) -> Arc<[Album]> {
        let generation = self.generation.current();
        self.browsing
            .artist_albums(generation, artist, &mut |a| self.queries.artist_albums(a))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load albums for {artist}: {e}");
                Arc::from([])
            })
    }

    /// One album's tracks in canonical order, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    pub fn album_tracks(&mut self, album_artist: &str, album_title: &str) -> Arc<[Track]> {
        let generation = self.generation.current();
        self.browsing
            .album_tracks(generation, album_artist, album_title, &mut |a, t| {
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
        let generation = self.generation.current();
        self.folders
            .has_audio(generation, folder, &mut |f| {
                self.queries.folder_has_audio(f)
            })
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to probe folder {}: {e}", folder.display());
                false
            })
    }

    /// Whether any track under `folder` matches the search query, cached per
    /// (folder, query) per generation.
    pub fn folder_search_match(&mut self, folder: &Path, query: &str) -> bool {
        let generation = self.generation.current();
        self.folders
            .has_search_match(generation, folder, query, &mut |f, q| {
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
        let generation = self.generation.current();
        self.folders
            .subtree_ids(generation, folder, &mut |f| {
                self.queries.track_ids_in_folder_tree(f)
            })
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to list folder tree {}: {e}", folder.display());
                Arc::from([])
            })
    }

    /// The child directories of `folder` holding audio, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    pub fn folder_children(&mut self, folder: &Path) -> Arc<[PathBuf]> {
        let generation = self.generation.current();
        self.folders
            .children(generation, folder, &mut |f| {
                self.queries.subdirs_with_audio(f)
            })
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to list folder children {}: {e}", folder.display());
                Arc::from([])
            })
    }

    /// The tracks directly inside `folder`, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    pub fn folder_direct_tracks(&mut self, folder: &Path) -> Arc<[Track]> {
        let generation = self.generation.current();
        self.folders
            .direct_tracks(generation, folder, &mut |f| {
                self.queries.tracks_in_folder(f)
            })
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
        let generation = self.generation.current();
        self.smart_playlists
            .list(generation, kind, limit, &mut |k, l| {
                self.queries.smart_playlist(k, l)
            })
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "Failed to compute smart playlist {}: {e}",
                    kind.display_name()
                );
                Arc::from([])
            })
    }

    // --- Playback-side reads ------------------------------------------------------

    /// Bring the playback slots (current Track + Up Next window) up to date
    /// with the store generation and `queue`'s shape. Cheap when nothing
    /// moved (a stamp comparison); otherwise refetches through the store.
    /// Failures keep the previous slots so stale-but-present beats blank.
    pub fn sync_playback(&mut self, queue: &PlaybackQueue, up_next_limit: usize) {
        let generation = self.generation.current();
        if let Err(e) = self
            .playback
            .refresh(generation, queue, up_next_limit, &mut |id| {
                self.queries.get_track(id)
            })
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
        let generation = self.generation.current();
        match self
            .playback
            .selected_track(generation, id, &mut |tid| self.queries.get_track(tid))
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
