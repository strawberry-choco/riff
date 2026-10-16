//! Shared [`LibraryQueryStore`] and [`LibraryMutationStore`] test doubles
//! for the cross-crate suites: [`StubLibraryQueryStore`] (whose failure mode
//! is a configuration, not a second type) and [`StubLibraryMutationStore`].
//!
//! Every double implements the whole port so a suite that needs "a seam that
//! answers", "a seam that fails", or "a seam that writes nothing" never
//! re-implements thirty no-op methods per mock. They are test-only by
//! convention — nothing in production constructs one.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::errors::StoreError;
use crate::playlist::PlaylistId;
use crate::store::{
    FullScanSummary, LibraryCounts, LibraryMutationStore, LibraryQueryStore, Page,
    SmartPlaylistTree, SortDirection, TrackListOrder,
};
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
pub struct StubLibraryQueryStore {
    failure: Option<String>,
    get_track: TrackResolver,
    album_tracks: AlbumResolver,
    all_track_ids: AllIdsResolver,
}

impl StubLibraryQueryStore {
    /// A stub whose `get_track` answers with `resolve`.
    pub fn new(resolve: impl Fn(&TrackId) -> Option<Track> + Send + Sync + 'static) -> Self {
        Self {
            failure: None,
            get_track: Box::new(resolve),
            album_tracks: Box::new(|_, _| Vec::new()),
            all_track_ids: Box::new(Vec::new),
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
    fn get_track(&self, id: &TrackId) -> Answer<Option<Track>> {
        self.answer(|| Ok((self.get_track)(id)))
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
}

/// A [`LibraryMutationStore`] whose every write is a no-op — the double for
/// suites exercising logic that writes nothing, or whose write behavior the
/// suite owns itself and reports through its own recording.
///
/// A suite that needs the scan-batch commit to record or to fail composes it:
/// hold this stub beside the recording state and delegate every method to it
/// except the one write the suite actually exercises, so the other
/// no-op bodies are never re-implemented per mock.
#[derive(Default)]
pub struct StubLibraryMutationStore;

impl LibraryMutationStore for StubLibraryMutationStore {
    fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, StoreError> {
        Ok(0)
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
