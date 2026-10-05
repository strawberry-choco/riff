//! Shared [`LibraryQueryStore`] test doubles for the cross-crate suites.
//!
//! Both doubles implement the whole port so a suite that needs "a seam that
//! answers" or "a seam that fails" never re-implements thirty no-op methods
//! per mock. They are test-only by convention — nothing in production
//! constructs either.

use std::path::PathBuf;

use crate::errors::StoreError;
use crate::store::{
    FullScanSummary, LibraryCounts, LibraryQueryStore, Page, SortDirection, TrackListOrder,
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
    get_track: TrackResolver,
    album_tracks: AlbumResolver,
    all_track_ids: AllIdsResolver,
}

impl StubLibraryQueryStore {
    /// A stub whose `get_track` answers with `resolve`.
    pub fn new(resolve: impl Fn(&TrackId) -> Option<Track> + Send + Sync + 'static) -> Self {
        Self {
            get_track: Box::new(resolve),
            album_tracks: Box::new(|_, _| Vec::new()),
            all_track_ids: Box::new(Vec::new),
        }
    }

    /// A stub with the default `get_track` (`None` for every id).
    pub fn empty() -> Self {
        Self::new(|_| None)
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
}

impl Default for StubLibraryQueryStore {
    fn default() -> Self {
        Self::empty()
    }
}

impl LibraryQueryStore for StubLibraryQueryStore {
    fn get_track(&self, id: &TrackId) -> Answer<Option<Track>> {
        Ok((self.get_track)(id))
    }

    fn metadata_version(&self) -> Answer<u32> {
        Ok(METADATA_VERSION)
    }

    fn tracks_page(
        &self,
        _order: TrackListOrder,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Track>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn library_counts(&self) -> Answer<LibraryCounts> {
        Ok(LibraryCounts::default())
    }

    fn smart_list_counts(&self) -> Answer<Vec<(SmartPlaylistKind, usize)>> {
        Ok(Vec::new())
    }

    fn all_track_ids(&self) -> Answer<Vec<TrackId>> {
        Ok((self.all_track_ids)())
    }

    fn search_page(
        &self,
        _query: &str,
        _order: TrackListOrder,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Track>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn all_artists(&self) -> Answer<Vec<Artist>> {
        Ok(Vec::new())
    }

    fn artist_albums(&self, _artist: &str) -> Answer<Vec<Album>> {
        Ok(Vec::new())
    }

    fn album_tracks(&self, album_artist: &str, album_title: &str) -> Answer<Vec<Track>> {
        Ok((self.album_tracks)(album_artist, album_title))
    }

    fn folder_has_audio(&self, _folder: &std::path::Path) -> Answer<bool> {
        Ok(false)
    }

    fn folder_has_search_match(&self, _folder: &std::path::Path, _query: &str) -> Answer<bool> {
        Ok(false)
    }

    fn track_ids_in_folder_tree(&self, _folder: &std::path::Path) -> Answer<Vec<TrackId>> {
        Ok(Vec::new())
    }

    fn tracks_in_folder(&self, _folder: &std::path::Path) -> Answer<Vec<Track>> {
        Ok(Vec::new())
    }

    fn folder_track_count(&self, _folder: &std::path::Path) -> Answer<usize> {
        Ok(0)
    }

    fn last_full_scan(&self) -> Answer<Option<FullScanSummary>> {
        Ok(None)
    }

    fn subdirs_with_audio(&self, _folder: &std::path::Path) -> Answer<Vec<PathBuf>> {
        Ok(Vec::new())
    }

    fn smart_playlist(&self, _kind: SmartPlaylistKind, _limit: usize) -> Answer<Vec<Track>> {
        Ok(Vec::new())
    }

    fn genre_counts(&self) -> Answer<Vec<GenreCount>> {
        Ok(Vec::new())
    }

    fn artists_in_genre(&self, _genre: &str) -> Answer<Vec<Artist>> {
        Ok(Vec::new())
    }

    fn artist_albums_in_genre(&self, _artist: &str, _genre: &str) -> Answer<Vec<Album>> {
        Ok(Vec::new())
    }

    fn album_tracks_in_genre(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _genre: &str,
    ) -> Answer<Vec<Track>> {
        Ok(Vec::new())
    }

    fn hit_albums_page(&self, _query: &str, _offset: usize, _limit: usize) -> Answer<Page<Album>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn hit_artists_page(
        &self,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn album_hit_tracks(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _query: &str,
    ) -> Answer<Vec<Track>> {
        Ok(Vec::new())
    }

    fn album_is_name_hit(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _query: &str,
    ) -> Answer<bool> {
        Ok(false)
    }

    fn hit_albums_in_genre(
        &self,
        _genre: &str,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Vec<Album>> {
        Ok(Vec::new())
    }

    fn hit_artists_in_genre(
        &self,
        _genre: &str,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Vec<Artist>> {
        Ok(Vec::new())
    }

    fn album_hit_tracks_in_genre(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _genre: &str,
        _query: &str,
    ) -> Answer<Vec<Track>> {
        Ok(Vec::new())
    }

    fn hit_genre_counts(&self, _query: &str) -> Answer<Vec<GenreCount>> {
        Ok(Vec::new())
    }

    fn artists_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn albums_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Album>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn genres_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<GenreCount>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn artists_in_genre_page(
        &self,
        _genre: &str,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        Ok(Page::new(0, Vec::new()))
    }

    fn artist_albums_in_genre_page(
        &self,
        _artist: &str,
        _genre: &str,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Album>> {
        Ok(Page::new(0, Vec::new()))
    }
}

/// A [`LibraryQueryStore`] that fails every read with one uniform message —
/// the double for suites whose UI under test must degrade gracefully over a
/// dead store.
pub struct FailingLibraryQueryStore {
    message: String,
}

impl FailingLibraryQueryStore {
    /// A failing store whose every read reports `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn failure(&self) -> StoreError {
        StoreError::InvalidOperation(self.message.clone())
    }
}

impl Default for FailingLibraryQueryStore {
    fn default() -> Self {
        Self::new("no store behind this seam")
    }
}

impl LibraryQueryStore for FailingLibraryQueryStore {
    fn get_track(&self, _id: &TrackId) -> Answer<Option<Track>> {
        Err(self.failure())
    }

    fn metadata_version(&self) -> Answer<u32> {
        Err(self.failure())
    }

    fn tracks_page(
        &self,
        _order: TrackListOrder,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Track>> {
        Err(self.failure())
    }

    fn library_counts(&self) -> Answer<LibraryCounts> {
        Err(self.failure())
    }

    fn smart_list_counts(&self) -> Answer<Vec<(SmartPlaylistKind, usize)>> {
        Err(self.failure())
    }

    fn all_track_ids(&self) -> Answer<Vec<TrackId>> {
        Err(self.failure())
    }

    fn search_page(
        &self,
        _query: &str,
        _order: TrackListOrder,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Track>> {
        Err(self.failure())
    }

    fn all_artists(&self) -> Answer<Vec<Artist>> {
        Err(self.failure())
    }

    fn artist_albums(&self, _artist: &str) -> Answer<Vec<Album>> {
        Err(self.failure())
    }

    fn album_tracks(&self, _album_artist: &str, _album_title: &str) -> Answer<Vec<Track>> {
        Err(self.failure())
    }

    fn folder_has_audio(&self, _folder: &std::path::Path) -> Answer<bool> {
        Err(self.failure())
    }

    fn folder_has_search_match(&self, _folder: &std::path::Path, _query: &str) -> Answer<bool> {
        Err(self.failure())
    }

    fn track_ids_in_folder_tree(&self, _folder: &std::path::Path) -> Answer<Vec<TrackId>> {
        Err(self.failure())
    }

    fn tracks_in_folder(&self, _folder: &std::path::Path) -> Answer<Vec<Track>> {
        Err(self.failure())
    }

    fn folder_track_count(&self, _folder: &std::path::Path) -> Answer<usize> {
        Err(self.failure())
    }

    fn last_full_scan(&self) -> Answer<Option<FullScanSummary>> {
        Err(self.failure())
    }

    fn subdirs_with_audio(&self, _folder: &std::path::Path) -> Answer<Vec<PathBuf>> {
        Err(self.failure())
    }

    fn smart_playlist(&self, _kind: SmartPlaylistKind, _limit: usize) -> Answer<Vec<Track>> {
        Err(self.failure())
    }

    fn genre_counts(&self) -> Answer<Vec<GenreCount>> {
        Err(self.failure())
    }

    fn artists_in_genre(&self, _genre: &str) -> Answer<Vec<Artist>> {
        Err(self.failure())
    }

    fn artist_albums_in_genre(&self, _artist: &str, _genre: &str) -> Answer<Vec<Album>> {
        Err(self.failure())
    }

    fn album_tracks_in_genre(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _genre: &str,
    ) -> Answer<Vec<Track>> {
        Err(self.failure())
    }

    fn hit_albums_page(&self, _query: &str, _offset: usize, _limit: usize) -> Answer<Page<Album>> {
        Err(self.failure())
    }

    fn hit_artists_page(
        &self,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        Err(self.failure())
    }

    fn album_hit_tracks(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _query: &str,
    ) -> Answer<Vec<Track>> {
        Err(self.failure())
    }

    fn album_is_name_hit(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _query: &str,
    ) -> Answer<bool> {
        Err(self.failure())
    }

    fn hit_albums_in_genre(
        &self,
        _genre: &str,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Vec<Album>> {
        Err(self.failure())
    }

    fn hit_artists_in_genre(
        &self,
        _genre: &str,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Vec<Artist>> {
        Err(self.failure())
    }

    fn album_hit_tracks_in_genre(
        &self,
        _album_artist: &str,
        _album_title: &str,
        _genre: &str,
        _query: &str,
    ) -> Answer<Vec<Track>> {
        Err(self.failure())
    }

    fn hit_genre_counts(&self, _query: &str) -> Answer<Vec<GenreCount>> {
        Err(self.failure())
    }

    fn artists_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        Err(self.failure())
    }

    fn albums_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Album>> {
        Err(self.failure())
    }

    fn genres_page(
        &self,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<GenreCount>> {
        Err(self.failure())
    }

    fn artists_in_genre_page(
        &self,
        _genre: &str,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Artist>> {
        Err(self.failure())
    }

    fn artist_albums_in_genre_page(
        &self,
        _artist: &str,
        _genre: &str,
        _direction: SortDirection,
        _offset: usize,
        _limit: usize,
    ) -> Answer<Page<Album>> {
        Err(self.failure())
    }
}
