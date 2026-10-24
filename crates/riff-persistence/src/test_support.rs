//! Shared [`LibraryQueryStore`], [`LibraryMutationStore`], and
//! [`ThumbnailCache`] test doubles for the cross-crate suites:
//! [`StubLibraryQueryStore`] and [`StubLibraryMutationStore`] (whose failure
//! modes are a configuration, not a second type), and [`StubThumbnailCache`].
//!
//! Every double implements the whole port so a suite that needs "a seam that
//! answers", "a seam that fails", or "a seam that writes nothing" never
//! re-implements thirty no-op methods per mock. They are test-only by
//! convention — nothing in production constructs one. Beside the doubles sit
//! the three shared-ownership forwarding implementations for the ports a
//! deleted shared-handle adapter served (the Library read port, the
//! Playlists section port, and the mutation port), so a suite that hands a
//! double to the seam under test keeps a handle by cloning the shared cell
//! instead of re-spelling the port.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::errors::StoreError;
use crate::playlist::{Playlist, PlaylistId};
use crate::store::{
    CustomSmartPlaylist, FullScanSummary, LibraryCounts, LibraryMutationStore, LibraryQueryStore,
    Page, PlaylistEntry, PlaylistStore, RuleTree, SmartPlaylistTree, SortDirection,
    StoreGeneration, TrackListOrder,
};
use crate::thumbnail::{EncodedThumbnail, ThumbnailBox, ThumbnailCache};
use crate::track::{
    Album, Artist, GenreCount, METADATA_VERSION, SmartPlaylistKind, Track, TrackId,
};

type Answer<T> = Result<T, StoreError>;

/// The `get_track` resolver a suite hands [`StubLibraryQueryStore::new`].
type TrackResolver = Box<dyn Fn(&TrackId) -> Option<Track> + Send + Sync>;

/// The `album_tracks` resolver a suite sets via
/// [`StubLibraryQueryStore::with_album_tracks`]; the default answers empty.
type AlbumResolver = Box<dyn Fn(&str, &str) -> Vec<Track> + Send + Sync>;

/// The `all_track_ids` resolver a suite sets via
/// [`StubLibraryQueryStore::with_all_track_ids`]; the default answers empty.
type AllIdsResolver = Box<dyn Fn() -> Vec<TrackId> + Send + Sync>;

/// A [`LibraryQueryStore`] that answers every read with the empty result —
/// the double for suites exercising logic indifferent to the library's
/// contents. `get_track` is the one read a suite varies, via [`Self::new`]'s
/// resolver; [`Self::empty`] answers `None`. A suite that also needs the
/// album-membership or flat-listing reads sets them via
/// [`Self::with_album_tracks`] / [`Self::with_all_track_ids`].
///
/// Failure is a knob too, in two distinct shapes: [`Self::failing`] fails
/// every read with the configured message, and
/// [`Self::with_failing_get_track`] fails only the `get_track` read — a
/// per-read failure is a configuration on the stub, not a second type.
pub struct StubLibraryQueryStore {
    failure: Option<String>,
    get_track_failure: Option<String>,
    get_track: TrackResolver,
    album_tracks: AlbumResolver,
    all_track_ids: AllIdsResolver,
    /// The session Library generation the port answers: a fresh counter this
    /// double keeps, which nothing bumps unless the owning suite does.
    generation: StoreGeneration,
}

impl StubLibraryQueryStore {
    /// A stub whose `get_track` answers with `resolve`.
    pub fn new(resolve: impl Fn(&TrackId) -> Option<Track> + Send + Sync + 'static) -> Self {
        Self {
            failure: None,
            get_track_failure: None,
            get_track: Box::new(resolve),
            album_tracks: Box::new(|_, _| Vec::new()),
            all_track_ids: Box::new(Vec::new),
            generation: StoreGeneration::new(),
        }
    }

    /// A stub with the default `get_track` (`None` for every id).
    pub fn empty() -> Self {
        Self::new(|_| None)
    }

    /// A stub whose every read reports `message` — the double for suites whose
    /// UI under test must degrade gracefully over a dead store.
    pub fn failing(message: impl Into<String>) -> Self {
        Self {
            failure: Some(message.into()),
            ..Self::empty()
        }
    }

    /// Also fail only the `get_track` read, with `message` — the per-read
    /// failure knob beside [`Self::failing`]'s uniform one: a suite whose
    /// flow probes the store through `get_track` and reads nothing else
    /// states one line instead of a second type. Every other read keeps
    /// answering empty; when a uniform failure is also configured, it wins
    /// here too.
    #[must_use]
    pub fn with_failing_get_track(mut self, message: impl Into<String>) -> Self {
        self.get_track_failure = Some(message.into());
        self
    }

    /// Also answer `album_tracks` with `resolve`.
    #[must_use]
    pub fn with_album_tracks(
        mut self,
        resolve: impl Fn(&str, &str) -> Vec<Track> + Send + Sync + 'static,
    ) -> Self {
        self.album_tracks = Box::new(resolve);
        self
    }

    /// Also answer `all_track_ids` with `resolve`.
    #[must_use]
    pub fn with_all_track_ids(
        mut self,
        resolve: impl Fn() -> Vec<TrackId> + Send + Sync + 'static,
    ) -> Self {
        self.all_track_ids = Box::new(resolve);
        self
    }

    /// One read against this stub: the uniform failure when one is configured,
    /// otherwise the stub's own answer.
    fn answer<T>(&self, ok: impl FnOnce() -> Answer<T>) -> Answer<T> {
        match &self.failure {
            Some(message) => Err(StoreError::InvalidOperation(message.clone())),
            None => ok(),
        }
    }
}

impl Default for StubLibraryQueryStore {
    fn default() -> Self {
        Self::empty()
    }
}

impl LibraryQueryStore for StubLibraryQueryStore {
    /// The stub's own session Library generation — fresh, and bumped by
    /// nothing unless the owning suite bumps the handle it read through the
    /// port. Never failed: a wiring read has no failure mode to degrade to.
    fn library_generation(&self) -> StoreGeneration {
        self.generation.clone()
    }

    fn get_track(&self, id: &TrackId) -> Answer<Option<Track>> {
        self.answer(|| match &self.get_track_failure {
            Some(message) => Err(StoreError::InvalidOperation(message.clone())),
            None => Ok((self.get_track)(id)),
        })
    }

    fn metadata_version(&self) -> Answer<u32> {
        self.answer(|| Ok(METADATA_VERSION))
    }

    fn tracks_page(
        &self,
        _order: TrackListOrder,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Track>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn library_counts(&self) -> Answer<LibraryCounts> {
        self.answer(|| Ok(LibraryCounts::default()))
    }

    fn smart_list_counts(&self) -> Answer<Vec<(SmartPlaylistKind, usize)>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn all_track_ids(&self) -> Answer<Vec<TrackId>> {
        self.answer(|| Ok((self.all_track_ids)()))
    }

    fn search_page(
        &self,
        _query: &str,
        _order: TrackListOrder,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Track>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn custom_smart_playlist_page(
        &self,
        _tree: &RuleTree,
        _order: TrackListOrder,
        _needle: Option<&str>,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Track>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn all_artists(&self) -> Answer<Vec<Artist>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn artist_albums(&self, _artist: &str) -> Answer<Vec<Album>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn album_tracks(&self, album_artist: &str, album_title: &str) -> Answer<Vec<Track>> {
        self.answer(|| Ok((self.album_tracks)(album_artist, album_title)))
    }

    fn folder_has_audio(&self, _folder: &std::path::Path) -> Answer<bool> {
        self.answer(|| Ok(false))
    }

    fn folder_has_search_match(&self, _folder: &std::path::Path, _query: &str) -> Answer<bool> {
        self.answer(|| Ok(false))
    }

    fn track_ids_in_folder_tree(&self, _folder: &std::path::Path) -> Answer<Vec<TrackId>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn tracks_in_folder(&self, _folder: &std::path::Path) -> Answer<Vec<Track>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn folder_track_count(&self, _folder: &std::path::Path) -> Answer<usize> {
        self.answer(|| Ok(0))
    }

    fn last_full_scan(&self) -> Answer<Option<FullScanSummary>> {
        self.answer(|| Ok(None))
    }

    fn subdirs_with_audio(&self, _folder: &std::path::Path) -> Answer<Vec<PathBuf>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn smart_playlist(&self, _kind: SmartPlaylistKind, _limit: usize) -> Answer<Vec<Track>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn genre_counts(&self) -> Answer<Vec<GenreCount>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn artists_in_genre(&self, _genre: &str) -> Answer<Vec<Artist>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn artist_albums_in_genre(&self, _artist: &str, _genre: &str) -> Answer<Vec<Album>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn album_tracks_in_genre(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _genre: &str,
    ) -> Answer<Vec<Track>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn hit_albums_page(&self, _query: &str, _offset: usize, _limit: usize) -> Answer<Page<Album>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn hit_artists_page(
        &self,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn album_hit_tracks(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _query: &str,
    ) -> Answer<Vec<Track>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn album_is_name_hit(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _query: &str,
    ) -> Answer<bool> {
        self.answer(|| Ok(false))
    }

    fn artist_is_name_hit(&self, _artist: &str, _query: &str) -> Answer<bool> {
        self.answer(|| Ok(false))
    }

    fn hit_albums_in_genre(
        &self,
        _genre: &str,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Vec<Album>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn hit_artists_in_genre(
        &self,
        _genre: &str,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Vec<Artist>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn album_hit_tracks_in_genre(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _genre: &str,
        _query: &str,
    ) -> Answer<Vec<Track>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn hit_genre_counts(&self, _query: &str) -> Answer<Vec<GenreCount>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn artists_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn albums_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Album>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn genres_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<GenreCount>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn artists_in_genre_page(
        &self,
        _genre: &str,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn artist_albums_in_genre_page(
        &self,
        _artist: &str,
        _genre: &str,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Album>> {
        self.answer(|| Ok(Page::new(0, Vec::new())))
    }

    fn picker_texts(&self, _field_id: &str, _limit: usize) -> Answer<Vec<String>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn picker_numbers(&self, _field_id: &str, _limit: usize) -> Answer<Vec<i64>> {
        self.answer(|| Ok(Vec::new()))
    }

    fn smart_playlist_tree(&self, _id: &PlaylistId) -> Answer<Option<SmartPlaylistTree>> {
        self.answer(|| Ok(None))
    }

    fn custom_smart_playlists(&self) -> Answer<Vec<CustomSmartPlaylist>> {
        self.answer(|| Ok(Vec::new()))
    }
}

/// A [`LibraryMutationStore`] whose every write is a no-op — the double for
/// suites exercising logic that writes nothing, or whose write behavior the
/// suite owns itself and reports through its own recording.
///
/// One write carries a failure knob: [`Self::with_failing_scan_batches`]
/// fails the scan-batch commit with the configured message — the double for
/// a suite exercising the scan flow's broken-store path. A suite that needs
/// a write to record composes it: hold this stub beside the recording state
/// and delegate every method to it except the one write the suite actually
/// exercises, so the other no-op bodies are never re-implemented per mock.
#[derive(Default)]
pub struct StubLibraryMutationStore {
    /// When set, `apply_scan_batch` fails with this message; every other
    /// write stays a no-op.
    scan_batch_failure: Option<String>,
}

impl StubLibraryMutationStore {
    /// A stub whose scan-batch commit fails with `message` and whose every
    /// other write stays a no-op — the double for suites exercising the scan
    /// flow's broken-store path. A failing write is a configuration on the
    /// stub, not a second type.
    #[must_use]
    pub fn with_failing_scan_batches(mut self, message: impl Into<String>) -> Self {
        self.scan_batch_failure = Some(message.into());
        self
    }
}

impl LibraryMutationStore for StubLibraryMutationStore {
    fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, StoreError> {
        match &self.scan_batch_failure {
            Some(message) => Err(StoreError::InvalidOperation(message.clone())),
            None => Ok(0),
        }
    }

    fn record_track_played(
        &mut self,
        _id: &TrackId,
        _played_at: SystemTime,
    ) -> Result<bool, StoreError> {
        Ok(false)
    }

    fn set_track_favorite(&mut self, _id: &TrackId, _favorite: bool) -> Result<bool, StoreError> {
        Ok(false)
    }

    fn apply_tag_refresh(&mut self, _track: &Track) -> Result<(), StoreError> {
        Ok(())
    }

    fn remove_library_path(&mut self, _root: &std::path::Path) -> Result<usize, StoreError> {
        Ok(0)
    }

    fn clear_library(&mut self) -> Result<usize, StoreError> {
        Ok(0)
    }

    fn record_full_scan_completed(&mut self, _summary: FullScanSummary) -> Result<(), StoreError> {
        Ok(())
    }

    fn stamp_metadata_version(&mut self, _version: u32) -> Result<(), StoreError> {
        Ok(())
    }

    // The Custom Smart Playlist writes answer the same way every other write
    // here does: nothing is stored, so nothing moved. The id a save "returns" is
    // inert for that reason — a suite that has to observe a save composes its own
    // recording double rather than reading meaning out of this one.
    fn save_smart_playlist(
        &mut self,
        _id: Option<&PlaylistId>,
        _name: &str,
        _tree: &SmartPlaylistTree,
    ) -> Result<PlaylistId, StoreError> {
        Ok(PlaylistId(String::new()))
    }

    fn rename_smart_playlist(
        &mut self,
        _id: &PlaylistId,
        _new_name: &str,
    ) -> Result<bool, StoreError> {
        Ok(false)
    }

    fn delete_smart_playlist(&mut self, _id: &PlaylistId) -> Result<bool, StoreError> {
        Ok(false)
    }

    fn duplicate_smart_playlist(
        &mut self,
        _id: &PlaylistId,
    ) -> Result<Option<PlaylistId>, StoreError> {
        Ok(None)
    }
}

/// A [`ThumbnailCache`] that is always empty — `load` misses, `store`
/// succeeds without storing, `clear` succeeds — the double for suites that
/// need the port to hold nothing and answer nothing. Its one constructor-side
/// consumer is the frontend's test twin, which builds the App Runtime bundle
/// over mock ports and must fill the one bundle member the app itself never
/// holds (in production the cache lives behind the Cover Service, so no
/// session and no UI code reads it); a suite that has to observe the cache
/// composes its own recording double rather than reading meaning out of this
/// one.
#[derive(Default)]
pub struct StubThumbnailCache;

impl ThumbnailCache for StubThumbnailCache {
    fn load(&self, _source: &Path, _box_: ThumbnailBox) -> Option<EncodedThumbnail> {
        None
    }

    fn store(
        &self,
        _source: &Path,
        _box_: ThumbnailBox,
        _thumbnail: &EncodedThumbnail,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    fn clear(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

// --- Shared-ownership forwarding implementations ------------------------------
//
// One forwarding implementation per port a deleted shared-handle adapter
// served. A test that hands a double to the seam under test loses ownership
// of it, so the seam takes the shared lock cell (boxed) and the test keeps
// the other `Arc` handle: assertions read the recordings and post-wire
// configuration mutates the canned data through the same cell the seam
// answers from. All three impls are generic over any implementor of their
// port behind a `std::sync::Mutex` cell shared by an `Arc`, and forward every
// method at the port's own full width by locking and delegating.
//
// The distinction from the narrow-port ADR (ADR 0012,
// `docs/adr/0012-narrow-port-gets-a-real-adapter.md`): those are test-support
// forwarding implementations at the port's own width for shared ownership —
// they narrow nothing, no seam goes hypothetical, and no production path
// constructs one. ADR 0012 forbids the other thing: a production blanket
// impl that coerces a wide port into a narrow one and thereby erases a real
// seam. Declared here because the orphan rule pins an implementation of this
// crate's port for `std`'s `Arc<Mutex<T>>` to this crate — the forwarding
// knowledge dies when the port dies.

/// [`LibraryQueryStore`] for any query store behind a shared lock cell: the
/// seam under test owns one handle of the cell and the test keeps the other,
/// so the mock's recordings and canned answers stay reachable after the
/// hand-over. Forwards every method at the port's own full width by locking
/// and delegating — see the section comment above for the ADR 0012
/// distinction (test support; nothing narrowed, nothing hypothetical).
impl<T: LibraryQueryStore + ?Sized> LibraryQueryStore for Arc<Mutex<T>> {
    fn library_generation(&self) -> StoreGeneration {
        self.lock().unwrap().library_generation()
    }

    fn get_track(&self, id: &TrackId) -> Answer<Option<Track>> {
        self.lock().unwrap().get_track(id)
    }

    fn metadata_version(&self) -> Answer<u32> {
        self.lock().unwrap().metadata_version()
    }

    fn tracks_page(
        &self,
        order: TrackListOrder,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<Track>> {
        self.lock().unwrap().tracks_page(order, offset, limit)
    }

    fn library_counts(&self) -> Answer<LibraryCounts> {
        self.lock().unwrap().library_counts()
    }

    fn smart_list_counts(&self) -> Answer<Vec<(SmartPlaylistKind, usize)>> {
        self.lock().unwrap().smart_list_counts()
    }

    fn all_track_ids(&self) -> Answer<Vec<TrackId>> {
        self.lock().unwrap().all_track_ids()
    }

    fn search_page(
        &self,
        query: &str,
        order: TrackListOrder,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<Track>> {
        self.lock()
            .unwrap()
            .search_page(query, order, offset, limit)
    }

    fn custom_smart_playlist_page(
        &self,
        tree: &RuleTree,
        order: TrackListOrder,
        needle: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<Track>> {
        self.lock()
            .unwrap()
            .custom_smart_playlist_page(tree, order, needle, offset, limit)
    }

    fn all_artists(&self) -> Answer<Vec<Artist>> {
        self.lock().unwrap().all_artists()
    }

    fn artist_albums(&self, artist: &str) -> Answer<Vec<Album>> {
        self.lock().unwrap().artist_albums(artist)
    }

    fn album_tracks(&self, album_artist: &str, album_title: &str) -> Answer<Vec<Track>> {
        self.lock().unwrap().album_tracks(album_artist, album_title)
    }

    fn folder_has_audio(&self, folder: &Path) -> Answer<bool> {
        self.lock().unwrap().folder_has_audio(folder)
    }

    fn folder_has_search_match(&self, folder: &Path, query: &str) -> Answer<bool> {
        self.lock().unwrap().folder_has_search_match(folder, query)
    }

    fn track_ids_in_folder_tree(&self, folder: &Path) -> Answer<Vec<TrackId>> {
        self.lock().unwrap().track_ids_in_folder_tree(folder)
    }

    fn tracks_in_folder(&self, folder: &Path) -> Answer<Vec<Track>> {
        self.lock().unwrap().tracks_in_folder(folder)
    }

    fn folder_track_count(&self, folder: &Path) -> Answer<usize> {
        self.lock().unwrap().folder_track_count(folder)
    }

    fn last_full_scan(&self) -> Answer<Option<FullScanSummary>> {
        self.lock().unwrap().last_full_scan()
    }

    fn subdirs_with_audio(&self, folder: &Path) -> Answer<Vec<PathBuf>> {
        self.lock().unwrap().subdirs_with_audio(folder)
    }

    fn smart_playlist(&self, kind: SmartPlaylistKind, limit: usize) -> Answer<Vec<Track>> {
        self.lock().unwrap().smart_playlist(kind, limit)
    }

    fn genre_counts(&self) -> Answer<Vec<GenreCount>> {
        self.lock().unwrap().genre_counts()
    }

    fn artists_in_genre(&self, genre: &str) -> Answer<Vec<Artist>> {
        self.lock().unwrap().artists_in_genre(genre)
    }

    fn artist_albums_in_genre(&self, artist: &str, genre: &str) -> Answer<Vec<Album>> {
        self.lock().unwrap().artist_albums_in_genre(artist, genre)
    }

    fn album_tracks_in_genre(
        &self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
    ) -> Answer<Vec<Track>> {
        self.lock()
            .unwrap()
            .album_tracks_in_genre(album_artist, album_title, genre)
    }

    fn hit_albums_page(&self, query: &str, offset: usize, limit: usize) -> Answer<Page<Album>> {
        self.lock().unwrap().hit_albums_page(query, offset, limit)
    }

    fn hit_artists_page(&self, query: &str, offset: usize, limit: usize) -> Answer<Page<Artist>> {
        self.lock().unwrap().hit_artists_page(query, offset, limit)
    }

    fn album_hit_tracks(
        &self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Answer<Vec<Track>> {
        self.lock()
            .unwrap()
            .album_hit_tracks(album_artist, album_title, query)
    }

    fn album_is_name_hit(
        &self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Answer<bool> {
        self.lock()
            .unwrap()
            .album_is_name_hit(album_artist, album_title, query)
    }

    fn artist_is_name_hit(&self, artist: &str, query: &str) -> Answer<bool> {
        self.lock().unwrap().artist_is_name_hit(artist, query)
    }

    fn hit_albums_in_genre(
        &self,
        genre: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Answer<Vec<Album>> {
        self.lock()
            .unwrap()
            .hit_albums_in_genre(genre, query, offset, limit)
    }

    fn hit_artists_in_genre(
        &self,
        genre: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Answer<Vec<Artist>> {
        self.lock()
            .unwrap()
            .hit_artists_in_genre(genre, query, offset, limit)
    }

    fn album_hit_tracks_in_genre(
        &self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        query: &str,
    ) -> Answer<Vec<Track>> {
        self.lock()
            .unwrap()
            .album_hit_tracks_in_genre(album_artist, album_title, genre, query)
    }

    fn hit_genre_counts(&self, query: &str) -> Answer<Vec<GenreCount>> {
        self.lock().unwrap().hit_genre_counts(query)
    }

    fn artists_page(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<Artist>> {
        self.lock().unwrap().artists_page(direction, offset, limit)
    }

    fn albums_page(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<Album>> {
        self.lock().unwrap().albums_page(direction, offset, limit)
    }

    fn genres_page(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<GenreCount>> {
        self.lock().unwrap().genres_page(direction, offset, limit)
    }

    fn artists_in_genre_page(
        &self,
        genre: &str,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<Artist>> {
        self.lock()
            .unwrap()
            .artists_in_genre_page(genre, direction, offset, limit)
    }

    fn artist_albums_in_genre_page(
        &self,
        artist: &str,
        genre: &str,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Answer<Page<Album>> {
        self.lock()
            .unwrap()
            .artist_albums_in_genre_page(artist, genre, direction, offset, limit)
    }

    fn picker_texts(&self, field_id: &str, limit: usize) -> Answer<Vec<String>> {
        self.lock().unwrap().picker_texts(field_id, limit)
    }

    fn picker_numbers(&self, field_id: &str, limit: usize) -> Answer<Vec<i64>> {
        self.lock().unwrap().picker_numbers(field_id, limit)
    }

    fn smart_playlist_tree(&self, id: &PlaylistId) -> Answer<Option<SmartPlaylistTree>> {
        self.lock().unwrap().smart_playlist_tree(id)
    }

    fn custom_smart_playlists(&self) -> Answer<Vec<CustomSmartPlaylist>> {
        self.lock().unwrap().custom_smart_playlists()
    }
}

/// [`PlaylistStore`] for any playlist store behind a shared lock cell — the
/// same shared-ownership forwarding implementation as the Library read
/// port's, for the same reason and under the same ADR 0012 distinction: the
/// port's own full width, test support only, nothing narrowed, nothing
/// hypothetical, no production path constructs one.
impl<T: PlaylistStore + ?Sized> PlaylistStore for Arc<Mutex<T>> {
    fn playlist_generation(&self) -> StoreGeneration {
        self.lock().unwrap().playlist_generation()
    }

    fn load_playlists(&self) -> Answer<Vec<Playlist>> {
        self.lock().unwrap().load_playlists()
    }

    fn load_playlist_entries(&self, id: &PlaylistId) -> Answer<Vec<PlaylistEntry>> {
        self.lock().unwrap().load_playlist_entries(id)
    }

    fn create_playlist(&mut self, name: &str, initial_tracks: &[TrackId]) -> Answer<PlaylistId> {
        self.lock().unwrap().create_playlist(name, initial_tracks)
    }

    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Answer<bool> {
        self.lock().unwrap().rename_playlist(id, new_name)
    }

    fn delete_playlist(&mut self, id: &PlaylistId) -> Answer<bool> {
        self.lock().unwrap().delete_playlist(id)
    }

    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Answer<bool> {
        self.lock().unwrap().add_playlist_entry(id, track)
    }

    fn remove_playlist_entries(&mut self, id: &PlaylistId, track: &TrackId) -> Answer<bool> {
        self.lock().unwrap().remove_playlist_entries(id, track)
    }

    fn reorder_playlist_entries(&mut self, id: &PlaylistId, ordered: &[TrackId]) -> Answer<bool> {
        self.lock().unwrap().reorder_playlist_entries(id, ordered)
    }
}

/// [`LibraryMutationStore`] for any mutation store behind a shared lock cell —
/// the same shared-ownership forwarding implementation as the Library read
/// port's, for the same reason and under the same ADR 0012 distinction: the
/// port's own full width, test support only, nothing narrowed, nothing
/// hypothetical, no production path constructs one. The mutation port's
/// writes take exclusive references, so the forward locks to the exclusive
/// borrow its methods require.
impl<T: LibraryMutationStore + ?Sized> LibraryMutationStore for Arc<Mutex<T>> {
    fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, StoreError> {
        self.lock().unwrap().apply_scan_batch(tracks)
    }

    fn record_track_played(
        &mut self,
        id: &TrackId,
        played_at: SystemTime,
    ) -> Result<bool, StoreError> {
        self.lock().unwrap().record_track_played(id, played_at)
    }

    fn set_track_favorite(&mut self, id: &TrackId, favorite: bool) -> Result<bool, StoreError> {
        self.lock().unwrap().set_track_favorite(id, favorite)
    }

    fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError> {
        self.lock().unwrap().apply_tag_refresh(track)
    }

    fn remove_library_path(&mut self, root: &std::path::Path) -> Result<usize, StoreError> {
        self.lock().unwrap().remove_library_path(root)
    }

    fn clear_library(&mut self) -> Result<usize, StoreError> {
        self.lock().unwrap().clear_library()
    }

    fn record_full_scan_completed(&mut self, summary: FullScanSummary) -> Result<(), StoreError> {
        self.lock().unwrap().record_full_scan_completed(summary)
    }

    fn stamp_metadata_version(&mut self, version: u32) -> Result<(), StoreError> {
        self.lock().unwrap().stamp_metadata_version(version)
    }

    fn save_smart_playlist(
        &mut self,
        id: Option<&PlaylistId>,
        name: &str,
        tree: &SmartPlaylistTree,
    ) -> Result<PlaylistId, StoreError> {
        self.lock().unwrap().save_smart_playlist(id, name, tree)
    }

    fn rename_smart_playlist(
        &mut self,
        id: &PlaylistId,
        new_name: &str,
    ) -> Result<bool, StoreError> {
        self.lock().unwrap().rename_smart_playlist(id, new_name)
    }

    fn delete_smart_playlist(&mut self, id: &PlaylistId) -> Result<bool, StoreError> {
        self.lock().unwrap().delete_smart_playlist(id)
    }

    fn duplicate_smart_playlist(
        &mut self,
        id: &PlaylistId,
    ) -> Result<Option<PlaylistId>, StoreError> {
        self.lock().unwrap().duplicate_smart_playlist(id)
    }
}
