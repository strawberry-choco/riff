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
use crate::domain::{Album, Artist, PlaybackQueue, SmartPlaylistKind, Track, TrackId};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

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
    windows: HashMap<usize, Arc<[Track]>>,
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

    /// Cached rows starting at `offset`, when present and loaded. Handed
    /// out as an `Arc` clone — a refcount bump, never a deep copy of the
    /// window's rows.
    #[must_use]
    pub fn window(&self, offset: usize) -> Option<Arc<[Track]>> {
        self.windows.get(&offset).cloned()
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
            self.windows.insert(offset, rows.into());
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

/// Session Projection for the artist/album browsing views (ADR 0002).
///
/// Caches the artist list plus per-artist album lists and per-album track
/// lists, each fetched from the store only when missing at the current
/// generation. A generation bump (a committed store mutation) drops every
/// level at once so a frame never mixes rows from two generations; each
/// level then refetches lazily as its view expands again. Loader errors
/// propagate and leave the cache untouched — the next call retries.
///
/// Unlike [`TrackListProjection`] this is not windowed: browsing is
/// hierarchical, so each query returns one artist's or one album's worth of
/// rows rather than one screen's.
pub struct BrowsingProjection {
    loaded_generation: Option<u64>,
    artists: Option<Arc<[Artist]>>,
    albums: HashMap<String, Arc<[Album]>>,
    tracks: HashMap<(String, String), Arc<[Track]>>,
}

/// Loader signature for one album's tracks (factored out for readability).
type AlbumTracksLoader<'a> = &'a mut dyn FnMut(&str, &str) -> Result<Vec<Track>, AppError>;

impl Default for BrowsingProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowsingProjection {
    #[must_use]
    pub fn new() -> Self {
        Self {
            loaded_generation: None,
            artists: None,
            albums: HashMap::new(),
            tracks: HashMap::new(),
        }
    }

    /// Drop every cached level when `generation` moved since the last load.
    /// The failed-generation case stays stale: `loaded_generation` is only
    /// stamped after a successful fetch, so the next call retries.
    fn ensure_generation(&mut self, generation: u64) {
        if self.loaded_generation == Some(generation) {
            return;
        }
        self.artists = None;
        self.albums.clear();
        self.tracks.clear();
    }

    /// Every artist name-ascending, cached per generation. Fresh frames
    /// hand out an `Arc` clone of the cached list — no per-frame copy.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artists(
        &mut self,
        generation: u64,
        loader: &mut dyn FnMut() -> Result<Vec<Artist>, AppError>,
    ) -> Result<Arc<[Artist]>, AppError> {
        if self.loaded_generation != Some(generation) || self.artists.is_none() {
            let fresh: Arc<[Artist]> = loader()?.into();
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.artists = Some(Arc::clone(&fresh));
            return Ok(fresh);
        }
        Ok(Arc::clone(self.artists.as_ref().expect("just loaded")))
    }

    /// One artist's albums in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artist_albums(
        &mut self,
        generation: u64,
        artist: &str,
        loader: &mut dyn FnMut(&str) -> Result<Vec<Album>, AppError>,
    ) -> Result<Arc<[Album]>, AppError> {
        if self.loaded_generation != Some(generation) || !self.albums.contains_key(artist) {
            let fresh: Arc<[Album]> = loader(artist)?.into();
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.albums.insert(artist.to_string(), Arc::clone(&fresh));
            return Ok(fresh);
        }
        Ok(Arc::clone(self.albums.get(artist).expect("checked above")))
    }

    /// One album's tracks in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn album_tracks(
        &mut self,
        generation: u64,
        album_artist: &str,
        album_title: &str,
        loader: AlbumTracksLoader<'_>,
    ) -> Result<Arc<[Track]>, AppError> {
        let key = (album_artist.to_string(), album_title.to_string());
        if self.loaded_generation != Some(generation) || !self.tracks.contains_key(&key) {
            let fresh: Arc<[Track]> = loader(album_artist, album_title)?.into();
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.tracks.insert(key, Arc::clone(&fresh));
            return Ok(fresh);
        }
        Ok(Arc::clone(self.tracks.get(&key).expect("checked above")))
    }
}

/// Session Projection for the folder-tree views (ADR 0002).
///
/// Caches the five folder query shapes — subtree existence, subtree search
/// matches, subtree track ids, direct tracks, and child directories — keyed
/// by folder, each fetched from the store only when missing at the current
/// generation. A generation bump drops every level at once so a frame never
/// mixes rows from two generations; levels then refetch lazily as the tree
/// renders again. Loader errors propagate and leave the cache untouched.
pub struct FolderProjection {
    loaded_generation: Option<u64>,
    has_audio: HashMap<String, bool>,
    search_matches: HashMap<(String, String), bool>,
    subtree_ids: HashMap<String, Arc<[TrackId]>>,
    direct_tracks: HashMap<String, Arc<[Track]>>,
    children: HashMap<String, Arc<[PathBuf]>>,
}

impl Default for FolderProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl FolderProjection {
    #[must_use]
    pub fn new() -> Self {
        Self {
            loaded_generation: None,
            has_audio: HashMap::new(),
            search_matches: HashMap::new(),
            subtree_ids: HashMap::new(),
            direct_tracks: HashMap::new(),
            children: HashMap::new(),
        }
    }

    /// Drop every cached level when `generation` moved since the last load.
    /// The failed-generation case stays stale: `loaded_generation` is only
    /// stamped after a successful fetch, so the next call retries.
    fn ensure_generation(&mut self, generation: u64) {
        if self.loaded_generation == Some(generation) {
            return;
        }
        self.has_audio.clear();
        self.search_matches.clear();
        self.subtree_ids.clear();
        self.direct_tracks.clear();
        self.children.clear();
    }

    /// Whether `folder` contains any audio, cached per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn has_audio(
        &mut self,
        generation: u64,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<bool, AppError>,
    ) -> Result<bool, AppError> {
        let key = folder.to_string_lossy().into_owned();
        if self.loaded_generation != Some(generation) || !self.has_audio.contains_key(&key) {
            let fresh = loader(folder)?;
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.has_audio.insert(key.clone(), fresh);
            return Ok(fresh);
        }
        Ok(self.has_audio[&key])
    }

    /// Whether any track under `folder` matches the search query, cached
    /// per (folder, query) per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn has_search_match(
        &mut self,
        generation: u64,
        folder: &std::path::Path,
        query: &str,
        loader: &mut dyn FnMut(&std::path::Path, &str) -> Result<bool, AppError>,
    ) -> Result<bool, AppError> {
        let key = (folder.to_string_lossy().into_owned(), query.to_string());
        if self.loaded_generation != Some(generation) || !self.search_matches.contains_key(&key) {
            let fresh = loader(folder, query)?;
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.search_matches.insert(key.clone(), fresh);
            return Ok(fresh);
        }
        Ok(self.search_matches[&key])
    }

    /// Every track id under `folder`, path-ordered, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list — no
    /// per-frame copy of one id per track.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn subtree_ids(
        &mut self,
        generation: u64,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<Vec<TrackId>, AppError>,
    ) -> Result<Arc<[TrackId]>, AppError> {
        let key = folder.to_string_lossy().into_owned();
        if self.loaded_generation != Some(generation) || !self.subtree_ids.contains_key(&key) {
            let fresh: Arc<[TrackId]> = loader(folder)?.into();
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.subtree_ids.insert(key.clone(), Arc::clone(&fresh));
            return Ok(fresh);
        }
        Ok(Arc::clone(&self.subtree_ids[&key]))
    }

    /// The tracks directly inside `folder`, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn direct_tracks(
        &mut self,
        generation: u64,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<Vec<Track>, AppError>,
    ) -> Result<Arc<[Track]>, AppError> {
        let key = folder.to_string_lossy().into_owned();
        if self.loaded_generation != Some(generation) || !self.direct_tracks.contains_key(&key) {
            let fresh: Arc<[Track]> = loader(folder)?.into();
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.direct_tracks.insert(key.clone(), Arc::clone(&fresh));
            return Ok(fresh);
        }
        Ok(Arc::clone(&self.direct_tracks[&key]))
    }

    /// The child directories of `folder` holding audio, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn children(
        &mut self,
        generation: u64,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<Vec<PathBuf>, AppError>,
    ) -> Result<Arc<[PathBuf]>, AppError> {
        let key = folder.to_string_lossy().into_owned();
        if self.loaded_generation != Some(generation) || !self.children.contains_key(&key) {
            let fresh: Arc<[PathBuf]> = loader(folder)?.into();
            self.ensure_generation(generation);
            self.loaded_generation = Some(generation);
            self.children.insert(key.clone(), Arc::clone(&fresh));
            return Ok(fresh);
        }
        Ok(Arc::clone(&self.children[&key]))
    }
}

/// Session Projection for the read-only smart playlists (ADR 0002).
///
/// Caches one computed list per [`SmartPlaylistKind`], stamped with the
/// generation it was loaded at. A generation bump (any committed store
/// mutation — a finished play, a scan batch, a tag edit) drops every list so
/// the next frame regenerates from committed state; within a generation the
/// cache serves repeat frames without touching the store. A request whose
/// limit exceeds what the cache holds also refetches, so callers can never
/// see a truncated-as-cached list where they asked for more.
pub struct SmartPlaylistsProjection {
    loaded_generation: Option<u64>,
    lists: HashMap<SmartPlaylistKind, (usize, Arc<[Track]>)>,
}

impl Default for SmartPlaylistsProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl SmartPlaylistsProjection {
    #[must_use]
    pub fn new() -> Self {
        Self {
            loaded_generation: None,
            lists: HashMap::new(),
        }
    }

    /// The computed list for `kind`, cached per generation and limit.
    /// Fresh frames hand out an `Arc` clone of the cached list — no
    /// per-frame copy.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn list(
        &mut self,
        generation: u64,
        kind: SmartPlaylistKind,
        limit: usize,
        loader: &mut dyn FnMut(SmartPlaylistKind, usize) -> Result<Vec<Track>, AppError>,
    ) -> Result<Arc<[Track]>, AppError> {
        if self.loaded_generation != Some(generation)
            || self
                .lists
                .get(&kind)
                .is_none_or(|(cached_limit, _)| *cached_limit < limit)
        {
            let fresh: Arc<[Track]> = loader(kind, limit)?.into();
            if self.loaded_generation != Some(generation) {
                self.lists.clear();
                self.loaded_generation = Some(generation);
            }
            self.lists.insert(kind, (limit, Arc::clone(&fresh)));
            return Ok(fresh);
        }
        Ok(Arc::clone(&self.lists[&kind].1))
    }
}

/// Session Projection for the playback-side reads (ADR 0002): the current
/// Track, the Up Next window, and the track-details panel's selected Track.
///
/// These are per-frame UI reads that resolve through the Application Store's
/// `get_track` query only when something they depend on moved — the Store
/// generation (a committed mutation) or the Playback Queue's shape (a
/// `TrackChanged` advance, Next/Previous/PlayNext/AddToQueue). Between such
/// moves every frame is served from cache without touching the store. Loader
/// errors propagate and leave the previous cache untouched — the next call
/// retries.
pub struct PlaybackProjection {
    loaded_generation: Option<u64>,
    /// Queue shape the playback slots were loaded for: `(current_index,
    /// upcoming ids at the window limit)`. Recomputing this cheap stamp per
    /// frame detects every queue mutation (advance, previous, insert-next,
    /// append, shuffle regeneration) without hooking each mutator.
    queue_stamp: Option<(Option<usize>, Vec<TrackId>)>,
    current: Option<Track>,
    up_next: Vec<Track>,
    /// The details-panel slot: generation, selected id, and the resolution
    /// (`None` = known absent from the store). Stamped separately from the
    /// playback slots because selection is independent of the queue.
    selected: Option<(u64, TrackId, Option<Track>)>,
}

impl Default for PlaybackProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackProjection {
    #[must_use]
    pub fn new() -> Self {
        Self {
            loaded_generation: None,
            queue_stamp: None,
            current: None,
            up_next: Vec::new(),
            selected: None,
        }
    }

    /// The resolved current Track, when one is playing and it still resolves.
    #[must_use]
    pub fn current(&self) -> Option<&Track> {
        self.current.as_ref()
    }

    /// The resolved Up Next window in Playback Queue order. Ids whose files
    /// left the library are skipped (the former mirror-reader behavior), so
    /// this can be shorter than the requested window.
    #[must_use]
    pub fn up_next(&self) -> &[Track] {
        &self.up_next
    }

    /// Bring the playback slots up to date with `generation` and `queue`.
    ///
    /// Fresh inputs (same generation, same queue shape) are served entirely
    /// from cache; moved inputs refetch the current Track plus the first
    /// `limit` upcoming ids through `loader`. On a loader error the error
    /// propagates and the previous cache is left untouched — stale-but-present
    /// beats blank while the UI retries.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn refresh(
        &mut self,
        generation: u64,
        queue: &PlaybackQueue,
        limit: usize,
        loader: &mut dyn FnMut(&TrackId) -> Result<Option<Track>, AppError>,
    ) -> Result<(), AppError> {
        // Fresh-frame fast path: compare the queue's shape lazily, by
        // reference — the per-frame check materializes nothing (the stamp's
        // `Vec` is only built below, when the inputs actually moved).
        if self.loaded_generation == Some(generation)
            && self.queue_stamp.as_ref().is_some_and(|(index, ids)| {
                *index == queue.current_index && upcoming_matches(ids, queue, limit)
            })
        {
            return Ok(());
        }

        let stamp = (
            queue.current_index,
            queue
                .upcoming(limit)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
        );

        // Fetch first, swap later: a failure anywhere leaves the previous
        // cache completely untouched.
        let fetched_current = match queue.current_track() {
            Some(id) => loader(id)?,
            None => None,
        };
        let mut fetched_up_next = Vec::with_capacity(stamp.1.len());
        for id in &stamp.1 {
            if let Some(track) = loader(id)? {
                fetched_up_next.push(track);
            }
        }

        self.current = fetched_current;
        self.up_next = fetched_up_next;
        self.loaded_generation = Some(generation);
        self.queue_stamp = Some(stamp);
        Ok(())
    }

    /// The track-details panel's selected Track, cached until the selection
    /// or the generation moves. A cached `None` means the id is known absent
    /// from the store, so a dangling selection does not requery per frame.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn selected_track(
        &mut self,
        generation: u64,
        id: &TrackId,
        loader: &mut dyn FnMut(&TrackId) -> Result<Option<Track>, AppError>,
    ) -> Result<Option<Track>, AppError> {
        if let Some((cached_generation, cached_id, track)) = &self.selected
            && *cached_generation == generation
            && cached_id == id
        {
            return Ok(track.clone());
        }
        let fresh = loader(id)?;
        self.selected = Some((generation, id.clone(), fresh.clone()));
        Ok(fresh)
    }
}

/// Whether `cached` — the Up Next ids recorded in the playback stamp —
/// still equals what [`PlaybackQueue::upcoming`] would return for `limit`.
///
/// The comparison walks the queue **by reference**, mirroring the domain
/// traversal exactly (shuffle order when active, otherwise linear order
/// from the slot after the current one; out-of-range shuffle indices are
/// skipped just as the domain skips them), so the per-frame freshness check
/// materializes nothing. Keep in sync with `PlaybackQueue::upcoming`; the
/// app-layer projection tests pin both orders.
fn upcoming_matches(cached: &[TrackId], queue: &PlaybackQueue, limit: usize) -> bool {
    if queue.shuffle && !queue.shuffled_indices.is_empty() {
        let mut matched = 0;
        for &idx in queue.shuffled_indices.iter().take(limit) {
            if let Some(track) = queue.tracks.get(idx) {
                if cached.get(matched) != Some(track) {
                    return false;
                }
                matched += 1;
            }
        }
        matched == cached.len()
    } else {
        let start = queue.current_index.map_or(0, |i| i + 1);
        let end = (start + limit).min(queue.tracks.len());
        let window = queue.tracks.get(start..end).unwrap_or(&[]);
        window == cached
    }
}
