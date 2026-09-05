//! Bounded Session Projections over Application Store query results.
//!
//! A Session Projection is a bounded in-memory view of store query results
//! used while rendering the UI; it is never authoritative (ADR 0002). Each
//! projection caches a total count plus only the currently visible row
//! windows (`LIMIT`/`OFFSET` ranges) until invalidated by the session-local
//! Store generation counter, which bumps after every committed mutation.
//! Stale reads are possible only between a committed write and the next
//! refresh, which generation invalidation makes explicit.

use crate::app::playlist_manager;
use crate::app::store::PlaylistEntry;
use crate::app::store::{FullScanSummary, LibraryCounts, StoreError, StoreGeneration};
use crate::domain::{
    Album, Artist, GenreCount, Playlist, PlaylistId, RepeatMode, SmartPlaylistKind, Track, TrackId,
};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Minimal playback queue for projection use (avoids riff-playback dependency).
pub struct PlaybackQueue {
    pub tracks: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub shuffled_indices: VecDeque<usize>,
    pub shuffle_history: Vec<usize>,
    #[allow(dead_code, reason = "mirrors riff-playback's queue shape")]
    shuffle_dirty: bool,
}

impl PlaybackQueue {
    pub fn current_track(&self) -> Option<&TrackId> {
        self.current_index.and_then(|i| self.tracks.get(i))
    }

    pub fn upcoming(&self, limit: usize) -> Vec<&TrackId> {
        let mut out = Vec::with_capacity(limit);
        if self.tracks.is_empty() {
            return out;
        }
        if self.shuffle {
            let mut iter = self.shuffled_indices.iter();
            if let Some(_ci) = self.current_index {
                iter.next();
            }
            for idx in iter.take(limit) {
                if let Some(t) = self.tracks.get(*idx) {
                    out.push(t);
                }
            }
        } else {
            let start = self.current_index.map_or(0, |i| i + 1);
            for t in self.tracks.iter().skip(start).take(limit) {
                out.push(t);
            }
        }
        out
    }
}

/// Generation-keyed cache with freshness validation.
#[derive(Default)]
struct GenerationCache<K, V> {
    generation: StoreGeneration,
    /// The generation epoch at which `value` was loaded — staleness is a
    /// mismatch against a fresh observation of the counter.
    epoch: u64,
    key: Option<K>,
    value: Option<V>,
}

impl<K, V> GenerationCache<K, V>
where
    K: Clone + PartialEq,
    V: Clone + Default,
{
    fn new(generation: StoreGeneration) -> Self {
        Self {
            epoch: generation.current(),
            generation,
            key: None,
            value: None,
        }
    }

    fn peek(&self) -> Option<&V> {
        self.value.as_ref()
    }

    fn store(&mut self, epoch: u64, key: K, value: V) {
        self.epoch = epoch;
        self.key = Some(key);
        self.value = Some(value);
    }

    fn observe(&self) -> u64 {
        self.generation.current()
    }

    fn loaded_at(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }

    fn holds(&self, epoch: u64, key: &K) -> bool {
        self.epoch == epoch && self.key.as_ref() == Some(key)
    }

    fn take_value(&mut self) -> Option<V> {
        self.value.take()
    }

    fn invalidate(&mut self) {
        self.key = None;
        self.value = None;
    }

    /// The mutable entry slot for `key`: adopts the key and hands out a
    /// freshly initialized value when absent — the commit step for lazily
    /// filled multi-level bundles. Callers assign the loaded fields through
    /// the returned slot only after a successful fetch.
    ///
    /// A moved epoch (or changed key) starts an EMPTY bundle: re-stamping
    /// the old one would present every sibling field it still holds as
    /// fresh at the new generation, so a committed mutation could never
    /// reach the views that read through a sibling field.
    fn slot(&mut self, epoch: u64, key: &K) -> &mut V {
        if self.epoch != epoch || self.key.as_ref() != Some(key) {
            self.value = None;
        }
        self.epoch = epoch;
        self.key = Some(key.clone());
        self.value.get_or_insert_with(V::default)
    }
}
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

/// The three lazily-filled levels of the browsing hierarchy.
#[derive(Default, Clone)]
struct BrowsingLevels {
    artists: Option<Arc<[Artist]>>,
    albums: HashMap<String, Arc<[Album]>>,
    tracks: HashMap<(String, String), Arc<[Track]>>,
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
    /// Generation-keyed slot over the whole level bundle: a moved epoch
    /// drops every level together, within a generation levels fill lazily.
    cache: GenerationCache<(), BrowsingLevels>,
}

/// Loader signature for one album's tracks (factored out for readability).
type AlbumTracksLoader<'a> = &'a mut dyn FnMut(&str, &str) -> Result<Vec<Track>, StoreError>;

impl Default for BrowsingProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl BrowsingProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// Every artist name-ascending, cached per generation. Fresh frames
    /// hand out an `Arc` clone of the cached list — no per-frame copy.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artists(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<Artist>, StoreError>,
    ) -> Result<Arc<[Artist]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache.peek().and_then(|levels| levels.artists.clone())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Artist]> = loader()?.into();
        self.cache.slot(epoch, &()).artists = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One artist's albums in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artist_albums(
        &mut self,
        artist: &str,
        loader: &mut dyn FnMut(&str) -> Result<Vec<Album>, StoreError>,
    ) -> Result<Arc<[Album]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.albums.get(artist).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Album]> = loader(artist)?.into();
        let levels = self.cache.slot(epoch, &());
        levels.albums.insert(artist.to_string(), Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One album's tracks in canonical order, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn album_tracks(
        &mut self,
        album_artist: &str,
        album_title: &str,
        loader: AlbumTracksLoader<'_>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = (album_artist.to_string(), album_title.to_string());
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.tracks.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Track]> = loader(album_artist, album_title)?.into();
        eprintln!(
            "probe album_tracks fresh_favs={:?}",
            fresh.iter().map(|t| t.favorite).collect::<Vec<_>>()
        );
        let levels = self.cache.slot(epoch, &());
        levels.tracks.insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }
}

/// The five lazily-filled folder query shapes, keyed by folder.
#[derive(Default, Clone)]
struct FolderLevels {
    has_audio: HashMap<String, bool>,
    search_matches: HashMap<(String, String), bool>,
    subtree_ids: HashMap<String, Arc<[TrackId]>>,
    direct_tracks: HashMap<String, Arc<[Track]>>,
    children: HashMap<String, Arc<[PathBuf]>>,
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
    /// Generation-keyed slot over the whole level bundle: a moved epoch
    /// drops every level together, within a generation levels fill lazily.
    cache: GenerationCache<(), FolderLevels>,
}

impl Default for FolderProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl FolderProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// Whether `folder` contains any audio, cached per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn has_audio(
        &mut self,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<bool, StoreError>,
    ) -> Result<bool, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.has_audio.get(&key).copied())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh = loader(folder)?;
        self.cache.slot(epoch, &()).has_audio.insert(key, fresh);
        Ok(fresh)
    }

    /// Whether any track under `folder` matches the search query, cached
    /// per (folder, query) per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn has_search_match(
        &mut self,
        folder: &std::path::Path,
        query: &str,
        loader: &mut dyn FnMut(&std::path::Path, &str) -> Result<bool, StoreError>,
    ) -> Result<bool, StoreError> {
        let key = (folder.to_string_lossy().into_owned(), query.to_string());
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.search_matches.get(&key).copied())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh = loader(folder, query)?;
        self.cache
            .slot(epoch, &())
            .search_matches
            .insert(key, fresh);
        Ok(fresh)
    }

    /// Every track id under `folder`, path-ordered, cached per generation.
    /// Fresh frames hand out an `Arc` clone of the cached list — no
    /// per-frame copy of one id per track.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn subtree_ids(
        &mut self,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<Vec<TrackId>, StoreError>,
    ) -> Result<Arc<[TrackId]>, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.subtree_ids.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[TrackId]> = loader(folder)?.into();
        self.cache
            .slot(epoch, &())
            .subtree_ids
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }

    /// The tracks directly inside `folder`, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn direct_tracks(
        &mut self,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<Vec<Track>, StoreError>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.direct_tracks.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Track]> = loader(folder)?.into();
        self.cache
            .slot(epoch, &())
            .direct_tracks
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }

    /// The child directories of `folder` holding audio, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn children(
        &mut self,
        folder: &std::path::Path,
        loader: &mut dyn FnMut(&std::path::Path) -> Result<Vec<PathBuf>, StoreError>,
    ) -> Result<Arc<[PathBuf]>, StoreError> {
        let key = folder.to_string_lossy().into_owned();
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.children.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[PathBuf]> = loader(folder)?.into();
        self.cache
            .slot(epoch, &())
            .children
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }
}

/// The lazily-filled levels of the genre read model.
#[derive(Default, Clone)]
struct GenreLevels {
    counts: Option<Arc<[GenreCount]>>,
    artists: HashMap<String, Arc<[Artist]>>,
    albums: HashMap<(String, String), Arc<[Album]>>,
    tracks: HashMap<(String, String, String), Arc<[Track]>>,
}

/// Session Projection for the genre read model (ADR 0002): the genre
/// aggregation for the sidebar plus the genre-filtered browsing levels.
///
/// Caches each level fetched from the store only when missing at the
/// current generation; a generation bump (a committed store mutation) drops
/// every level at once so a frame never mixes rows from two generations.
/// Loader errors propagate and leave the cache untouched — the next call
/// retries.
pub struct GenreProjection {
    /// Generation-keyed slot over the whole level bundle: a moved epoch
    /// drops every level together, within a generation levels fill lazily.
    cache: GenerationCache<(), GenreLevels>,
}

impl Default for GenreProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

/// Loader signature for one artist's genre-filtered albums (factored out
/// for readability).
type GenreAlbumsLoader<'a> = &'a mut dyn FnMut(&str, &str) -> Result<Vec<Album>, StoreError>;

/// Loader signature for one album's genre-filtered tracks (factored out for
/// readability).
type GenreAlbumTracksLoader<'a> =
    &'a mut dyn FnMut(&str, &str, &str) -> Result<Vec<Track>, StoreError>;

impl GenreProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// Every genre with its per-track count, cached per generation. Fresh
    /// frames hand out an `Arc` clone of the cached list — no per-frame
    /// copy.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn counts(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<GenreCount>, StoreError>,
    ) -> Result<Arc<[GenreCount]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache.peek().and_then(|levels| levels.counts.clone())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[GenreCount]> = loader()?.into();
        self.cache.slot(epoch, &()).counts = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// Artists having at least one track with `genre`, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artists_in_genre(
        &mut self,
        genre: &str,
        loader: &mut dyn FnMut(&str) -> Result<Vec<Artist>, StoreError>,
    ) -> Result<Arc<[Artist]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.artists.get(genre).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Artist]> = loader(genre)?.into();
        self.cache
            .slot(epoch, &())
            .artists
            .insert(genre.to_string(), Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One artist's albums holding a track with `genre`, cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn artist_albums_in_genre(
        &mut self,
        artist: &str,
        genre: &str,
        loader: GenreAlbumsLoader<'_>,
    ) -> Result<Arc<[Album]>, StoreError> {
        let key = (artist.to_string(), genre.to_string());
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.albums.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Album]> = loader(artist, genre)?.into();
        self.cache
            .slot(epoch, &())
            .albums
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One album's tracks with `genre`, cached per generation. Fresh frames
    /// hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn album_tracks_in_genre(
        &mut self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        loader: GenreAlbumTracksLoader<'_>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let key = (
            album_artist.to_string(),
            album_title.to_string(),
            genre.to_string(),
        );
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|levels| levels.tracks.get(&key).cloned())
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Track]> = loader(album_artist, album_title, genre)?.into();
        self.cache
            .slot(epoch, &())
            .tracks
            .insert(key, Arc::clone(&fresh));
        Ok(fresh)
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
/// Per-kind computed lists stamped with the limit they were loaded at.
type SmartPlaylistLists = HashMap<SmartPlaylistKind, (usize, Arc<[Track]>)>;

pub struct SmartPlaylistsProjection {
    /// Generation-keyed slot over the per-kind computed lists.
    cache: GenerationCache<(), SmartPlaylistLists>,
}

impl Default for SmartPlaylistsProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl SmartPlaylistsProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
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
        kind: SmartPlaylistKind,
        limit: usize,
        loader: &mut dyn FnMut(SmartPlaylistKind, usize) -> Result<Vec<Track>, StoreError>,
    ) -> Result<Arc<[Track]>, StoreError> {
        let epoch = self.cache.observe();
        let cached = if self.cache.loaded_at(epoch) {
            self.cache
                .peek()
                .and_then(|lists| lists.get(&kind))
                .filter(|(cached_limit, _)| *cached_limit >= limit)
                .map(|(_, list)| Arc::clone(list))
        } else {
            None
        };
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let fresh: Arc<[Track]> = loader(kind, limit)?.into();
        self.cache
            .slot(epoch, &())
            .insert(kind, (limit, Arc::clone(&fresh)));
        Ok(fresh)
    }
}

#[derive(Clone, Default)]
struct PlaybackSlots {
    /// Queue shape the slots were loaded for: the (current index, upcoming
    /// ids at the window limit) pair. Recomputing this cheap stamp detects
    /// every queue mutation (advance, previous, insert-next, append,
    /// shuffle regeneration) without hooking each mutator.
    stamp: (Option<usize>, Vec<TrackId>),
    current: Option<Track>,
    up_next: Vec<Track>,
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
    /// Generation-keyed slot over the playback slots; the queue shape rides
    /// inside as part of the loaded state.
    slots: GenerationCache<(), PlaybackSlots>,
    /// Generation-keyed single-selection slot: a cached `None` means the id
    /// is known absent from the store, so a dangling selection does not
    /// requery per frame.
    selected: GenerationCache<TrackId, Option<Track>>,
}

impl Default for PlaybackProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl PlaybackProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            slots: GenerationCache::new(generation.clone()),
            selected: GenerationCache::new(generation),
        }
    }

    /// The resolved current Track, when one is playing and it still resolves.
    #[must_use]
    pub fn current(&self) -> Option<&Track> {
        self.slots.peek().and_then(|slots| slots.current.as_ref())
    }

    /// The resolved Up Next window in Playback Queue order. Ids whose files
    /// left the library are skipped (the former mirror-reader behavior), so
    /// this can be shorter than the requested window.
    #[must_use]
    pub fn up_next(&self) -> &[Track] {
        self.slots
            .peek()
            .map_or(&[], |slots| slots.up_next.as_slice())
    }

    /// Bring the playback slots up to date with `queue`.
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
        queue: &PlaybackQueue,
        limit: usize,
        loader: &mut dyn FnMut(&TrackId) -> Result<Option<Track>, StoreError>,
    ) -> Result<(), StoreError> {
        let epoch = self.slots.observe();

        // Fresh-frame fast path: compare the queue's shape lazily, by
        // reference — the per-frame check materializes nothing (the stamp's
        // `Vec` is only built below, when the inputs actually moved).
        if let Some(slots) = self.slots.peek()
            && self.slots.loaded_at(epoch)
            && slots.stamp.0 == queue.current_index
            && upcoming_matches(&slots.stamp.1, queue, limit)
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

        self.slots.store(
            epoch,
            (),
            PlaybackSlots {
                stamp,
                current: fetched_current,
                up_next: fetched_up_next,
            },
        );
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
        id: &TrackId,
        loader: &mut dyn FnMut(&TrackId) -> Result<Option<Track>, StoreError>,
    ) -> Result<Option<Track>, StoreError> {
        let epoch = self.selected.observe();
        if self.selected.holds(epoch, id) {
            return Ok(self
                .selected
                .peek()
                .expect("holds implies an entry")
                .clone());
        }
        let fresh = loader(id)?;
        self.selected.store(epoch, id.clone(), fresh.clone());
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

/// One resolved row of an open user playlist: the entry id, its
/// store-resolved Track when the Library knows it, and whether it can play.
pub type PlaylistEntryRow = (TrackId, Option<Track>, bool);

/// The ready-to-render view of one user playlist: one row per entry in
/// playlist order, plus the playable ids (valid verdicts only) for the
/// header context menu.
#[derive(Clone, Default)]
pub struct PlaylistView {
    /// One row per entry, in playlist order. Missing tracks are included as
    /// `(id, None, false)` — dangling references stay listed (ADR 0001).
    pub rows: Arc<[PlaylistEntryRow]>,
    /// The playable ids (valid verdicts only), in playlist order.
    pub valid_ids: Arc<[TrackId]>,
}

/// Session Projection for the user playlists (ADR 0002).
///
/// Caches the playlist list plus per-playlist resolved views as TWO
/// [`GenerationCache`] instances keyed on different counters. The list is
/// pure user data: keyed on the session's dedicated playlist generation
/// alone. Resolved rows embed Track metadata resolved against the Library
/// collection, so their cache is keyed on the Library generation with the
/// playlist epoch baked into the key — a move of EITHER counter drops every
/// row while the list stays. Within matching counters levels refetch lazily
/// as their views render again.
///
/// Loader errors propagate and leave the cache untouched — the previous
/// good rows stay readable through [`Self::cached_playlists`] /
/// [`Self::cached_view`] while the next call retries.
pub struct PlaylistProjection {
    /// The playlist list, keyed on the playlist generation alone.
    playlists: GenerationCache<(), Arc<[Playlist]>>,
    /// The per-playlist resolved views: keyed on the Library generation,
    /// with the playlist generation the rows were built under as the key.
    views: GenerationCache<u64, HashMap<PlaylistId, PlaylistView>>,
}

impl Default for PlaylistProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new(), StoreGeneration::new())
    }
}

impl PlaylistProjection {
    #[must_use]
    pub fn new(playlist_generation: StoreGeneration, library_generation: StoreGeneration) -> Self {
        Self {
            playlists: GenerationCache::new(playlist_generation),
            views: GenerationCache::new(library_generation),
        }
    }

    /// Every user playlist in creation order, cached per playlist
    /// generation. Fresh frames hand out an `Arc` clone of the cached list.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn playlists(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<Playlist>, StoreError>,
    ) -> Result<Arc<[Playlist]>, StoreError> {
        let epoch = self.playlists.observe();
        if self.playlists.loaded_at(epoch)
            && let Some(cached) = self.playlists.peek()
        {
            return Ok(Arc::clone(cached));
        }
        let fresh: Arc<[Playlist]> = loader()?.into();
        self.playlists.store(epoch, (), Arc::clone(&fresh));
        Ok(fresh)
    }

    /// One playlist's resolved view, cached per playlist generation plus
    /// the Library generation the rows were resolved against. Fresh frames
    /// hand out a clone of the cached view (`Arc` row bumps, no deep copy).
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn playlist_view(
        &mut self,
        id: &PlaylistId,
        loader: &mut dyn FnMut(&PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError>,
    ) -> Result<PlaylistView, StoreError> {
        let playlist_epoch = self.playlists.observe();
        let library_epoch = self.views.observe();
        let fresh = self.views.holds(library_epoch, &playlist_epoch)
            && self
                .views
                .peek()
                .is_some_and(|views| views.contains_key(id));
        if !fresh {
            // Rows embed Library-resolved metadata: a move of either counter
            // invalidates them even though the other stood still.
            let resolved = loader(id)?;
            let views = self.views.slot(library_epoch, &playlist_epoch);
            let view = Self::resolve(resolved);
            views.insert(id.clone(), view.clone());
            return Ok(view);
        }
        Ok(self
            .views
            .peek()
            .expect("checked above")
            .get(id)
            .expect("checked above")
            .clone())
    }

    /// The stale-but-present playlist list, if any — the error fallback
    /// keeps last good data instead of blanking the sidebar.
    #[must_use]
    pub fn cached_playlists(&self) -> Option<Arc<[Playlist]>> {
        self.playlists.peek().cloned()
    }

    /// The stale-but-present view for `id`, if any — the error fallback
    /// keeps last good rows instead of blanking the open playlist.
    #[must_use]
    pub fn cached_view(&self, id: &PlaylistId) -> Option<PlaylistView> {
        self.views.peek().and_then(|views| views.get(id)).cloned()
    }

    /// Map store entries to ready-to-render rows: each entry rides its
    /// LEFT-JOIN validity plus the read-time filesystem check, and missing
    /// tracks stay listed as `(id, None, false)` (ADR 0001).
    fn resolve(entries: Vec<PlaylistEntry>) -> PlaylistView {
        let mut valid_ids = Vec::new();
        let rows: Vec<PlaylistEntryRow> = entries
            .into_iter()
            .map(|entry| {
                let valid = playlist_manager::track_is_valid(&entry);
                if valid {
                    valid_ids.push(entry.id.clone());
                }
                (entry.id, entry.track, valid)
            })
            .collect();
        PlaylistView {
            rows: rows.into(),
            valid_ids: valid_ids.into(),
        }
    }
}

/// Cached bundle for the counts read model: the library-side totals and the
/// per-smart-list sizes, each `Some` once loaded this generation.
#[derive(Default, Clone)]
struct CountsBundle {
    library: Option<Arc<LibraryCounts>>,
    smart_lists: Option<Arc<[(SmartPlaylistKind, usize)]>>,
    /// Per-folder track counts loaded this generation, keyed by folder.
    folder_counts: HashMap<PathBuf, usize>,
    /// The last-scan stamp loaded this generation: `NotLoaded` = not loaded
    /// yet, `Loaded` carrying the stamp (`None` means "never scanned").
    scan: ScanCache,
}

/// The scan-stamp slot's loaded state: distinguishes "no stamp read yet
/// this generation" from "read, and the store has never scanned".
#[derive(Default, Clone)]
enum ScanCache {
    #[default]
    NotLoaded,
    Loaded(Option<FullScanSummary>),
}

/// Session Projection for the counts read models (ADR 0002): the
/// sidebar-count totals and the per-smart-list sizes, cached per generation
/// so fresh frames cost nothing.
pub struct CountsProjection {
    cache: GenerationCache<(), CountsBundle>,
}

impl Default for CountsProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl CountsProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            cache: GenerationCache::new(generation),
        }
    }

    /// The library-side totals (tracks, artists, albums, genres), cached per
    /// generation. Fresh frames hand out an `Arc` clone of the cached
    /// totals — one store query per generation, not per frame.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn library_counts(
        &mut self,
        loader: &mut dyn FnMut() -> Result<LibraryCounts, StoreError>,
    ) -> Result<Arc<LibraryCounts>, StoreError> {
        let epoch = self.cache.observe();
        if let Some(cached) = self
            .cache
            .loaded_at(epoch)
            .then(|| self.cache.peek().and_then(|bundle| bundle.library.clone()))
            .flatten()
        {
            return Ok(cached);
        }
        let fresh: Arc<LibraryCounts> = Arc::new(loader()?);
        self.cache.slot(epoch, &()).library = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// Every smart playlist's unbounded total, in `ALL` order, cached per
    /// generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn smart_list_counts(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError>,
    ) -> Result<Arc<[(SmartPlaylistKind, usize)]>, StoreError> {
        let epoch = self.cache.observe();
        if let Some(cached) = self
            .cache
            .loaded_at(epoch)
            .then(|| {
                self.cache
                    .peek()
                    .and_then(|bundle| bundle.smart_lists.clone())
            })
            .flatten()
        {
            return Ok(cached);
        }
        let fresh: Arc<[(SmartPlaylistKind, usize)]> = loader()?.into();
        self.cache.slot(epoch, &()).smart_lists = Some(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// How many tracks live under `folder`, cached per (generation, folder)
    /// so each folder's Settings-pane count costs one query per generation.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn folder_count(
        &mut self,
        folder: &Path,
        loader: &mut dyn FnMut(&Path) -> Result<usize, StoreError>,
    ) -> Result<usize, StoreError> {
        let epoch = self.cache.observe();
        if self.cache.loaded_at(epoch)
            && let Some(count) = self
                .cache
                .peek()
                .and_then(|bundle| bundle.folder_counts.get(folder))
        {
            return Ok(*count);
        }
        let fresh = loader(folder)?;
        self.cache
            .slot(epoch, &())
            .folder_counts
            .insert(folder.to_path_buf(), fresh);
        Ok(fresh)
    }

    /// The last completed full scan's summary (timestamp + file/error
    /// counts, design-handoff issue 12), cached per generation — including
    /// a cached absence ("never scanned"), so a cold store does not requery
    /// per frame.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn last_scan(
        &mut self,
        loader: &mut dyn FnMut() -> Result<Option<FullScanSummary>, StoreError>,
    ) -> Result<Option<FullScanSummary>, StoreError> {
        let epoch = self.cache.observe();
        if self.cache.loaded_at(epoch)
            && let ScanCache::Loaded(scan) = self
                .cache
                .peek()
                .map_or(ScanCache::NotLoaded, |bundle| bundle.scan.clone())
        {
            return Ok(scan);
        }
        let fresh = loader()?;
        self.cache.slot(epoch, &()).scan = ScanCache::Loaded(fresh);
        Ok(fresh)
    }
}
