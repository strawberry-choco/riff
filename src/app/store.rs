//! Application-layer ports for the `SQLite` `Application Store`.
//!
//! The `Application Store` is the single authoritative persistent state of the
//! application (`Library`, `Playlists`, `Settings`). These ports keep the app layer
//! free of infrastructure types: infrastructure implements them over a shared
//! `rusqlite` connection, and the UI never imports `rusqlite` directly.

use crate::app::errors::AppError;
use crate::app::state::{ScalarSettings, WatchState};
use crate::domain::{Album, Artist, Playlist, PlaylistId, Track, TrackId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The user preferences the settings surface owns: single-row scalar values,
/// library paths, and per-path watch states.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub scalars: ScalarSettings,
    /// Library root paths in registration order.
    pub library_paths: Vec<PathBuf>,
    /// Per-path watch state; paths absent from this map default to
    /// [`WatchState::Disabled`].
    pub watch_states: HashMap<PathBuf, WatchState>,
}

/// Port for bringing the `Application Store` schema up to date.
///
/// Implemented by infrastructure over a real `SQLite` connection. Opening the
/// store and running migrations are fatal-startup operations: failures carry a
/// clear message and must never be silently swallowed.
pub trait StoreMigrations {
    /// Open (creating it if needed) the store at `path`, configure the
    /// connection for durability, and apply every pending migration.
    fn open_and_migrate(&self, path: &std::path::Path) -> Result<(), AppError>;
}

/// Port for reading and writing the Settings section of the `Application
/// Store`.
///
/// Every setter commits as one small durable transaction so a crash right
/// after a change cannot lose it. Implemented by infrastructure over a real
/// `SQLite` connection; app-layer orchestration tests drive this port through
/// mocks.
pub trait SettingsStore {
    /// Load every persisted setting. Missing values yield their defaults:
    /// volume `None` (the caller applies its own fallback), toggles off,
    /// empty path list, no watch states.
    fn load_settings(&self) -> Result<Settings, AppError>;

    /// Persist the scalar block as one small durable transaction.
    fn save_scalars(&mut self, scalars: &ScalarSettings) -> Result<(), AppError>;

    /// Replace the library-path list as one small durable transaction.
    fn save_library_paths(&mut self, paths: &[PathBuf]) -> Result<(), AppError>;

    /// Replace the whole watch-state map as one small durable transaction.
    fn save_watch_states(&mut self, states: &HashMap<PathBuf, WatchState>) -> Result<(), AppError>;
}

/// Port for reading and writing the Playlists section of the `Application
/// Store`.
///
/// Every mutation (create, rename, delete, add entry, remove entries)
/// commits as one immediate durable transaction, so a crash right after the
/// action cannot lose it. Entries are ordered Track references that carry no
/// enforced link to tracks: dangling references stay listed and resolve again
/// once the referenced files return. Implemented by infrastructure over a
/// real `SQLite` connection.
pub trait PlaylistStore {
    /// Load every Playlist in creation order, each with its entries in
    /// playlist order. A fresh store yields an empty `Vec`.
    fn load_playlists(&self) -> Result<Vec<Playlist>, AppError>;

    /// Create a Playlist named `name` (trimmed) with optional initial Track
    /// references (exact duplicates dropped, order preserved). The generated
    /// id is made unique against existing playlists so same-millisecond
    /// creation of same-named playlists cannot collide; duplicate names are
    /// allowed.
    fn create_playlist(
        &mut self,
        name: &str,
        initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, AppError>;

    /// Rename the Playlist with `id` to `new_name` (trimmed). Returns
    /// whether the playlist was found.
    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, AppError>;

    /// Delete the Playlist with `id` together with its entries. Returns
    /// whether anything was removed.
    fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, AppError>;

    /// Append `track` to the Playlist with `id`. Exact duplicates are
    /// ignored (returns `false`), as are unknown playlist ids.
    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Result<bool, AppError>;

    /// Remove all occurrences of `track` from the Playlist with `id`.
    /// Returns whether anything was removed.
    fn remove_playlist_entries(
        &mut self,
        id: &PlaylistId,
        track: &TrackId,
    ) -> Result<bool, AppError>;
}

/// Session-local monotonically increasing counter bumped after each committed
/// Store mutation (ADR 0002). Session Projections compare the generation they
/// were loaded at against [`Self::current`] and refetch when it moved. The
/// counter lives in memory only and resets on launch; it is never persisted.
#[derive(Clone, Debug, Default)]
pub struct StoreGeneration(Arc<AtomicU64>);

impl StoreGeneration {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    /// Record a committed mutation and return the new generation value.
    pub fn bump(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The current generation; projections reload when this differs from the
    /// generation their cached rows were loaded at.
    pub fn current(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A full snapshot of the Library collection section of the Application
/// Store, used to hydrate the transitional in-memory mirror at startup.
/// Albums are keyed by `"album artist - title"` (the same composite identity
/// the store uses); artists list their album keys in first-added order.
pub struct LibraryCollection {
    pub tracks: HashMap<TrackId, Track>,
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
}

/// Port for writing the Library collection section of the Application Store.
///
/// Implemented by infrastructure over a real `SQLite` connection. Every call
/// commits as one small durable transaction so an interrupted scan keeps all
/// previously committed batches (spec user story 3).
pub trait LibraryMutationStore {
    /// Upsert scanned Tracks as ONE immediate durable transaction — callers
    /// pass about ten tracks per scan batch. Creates missing Artist/Album
    /// parents (album identity is `(album artist, title)`; year/genre derive
    /// from the first-added track), writes the derived Rust-lowercased
    /// search text, and preserves existing play history (`play_count`,
    /// `last_played`, `date_added`) when a track already exists. Rescanning
    /// known paths therefore changes nothing observable. Returns the number
    /// of tracks written.
    ///
    /// Also serves single-track metadata persistence (e.g. a tag edit): the
    /// same upsert semantics apply.
    fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, AppError>;
}

/// Port for reading the Library collection section of the Application Store.
///
/// Flat-list and search reads are bounded windows (ADR 0003): callers fetch
/// only the visible row range plus a total count, ordered deterministically
/// by track path ascending. Search parity with the former in-memory
/// implementation is guaranteed by storing a derived Rust-lowercased
/// search-text column at write time and lowercasing the query in Rust here;
/// matching is literal substring (no wildcard semantics).
pub trait LibraryQueryStore {
    /// Resolve one `Track` by its `TrackId` (its full file path). Playback uses
    /// this instead of any in-memory copy. `None` when unknown.
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, AppError>;

    /// One bounded window of the flat library list, path-ascending.
    fn tracks_window(&self, offset: usize, limit: usize) -> Result<Vec<Track>, AppError>;

    /// Total number of stored Tracks (for the flat list projection).
    fn track_count(&self) -> Result<usize, AppError>;

    /// One bounded window of case-insensitive substring matches over title,
    /// artist, album, and album artist, path-ascending. The query is
    /// lowercased in Rust before matching.
    fn search_window(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Track>, AppError>;

    /// Total number of matches for [`Self::search_window`] semantics.
    fn search_count(&self, query: &str) -> Result<usize, AppError>;

    /// Full collection snapshot for hydrating the transitional in-memory
    /// mirror that still serves views not yet migrated to store queries
    /// (artist/album browsing, folder navigation, smart playlists land in
    /// later tickets).
    fn load_collection(&self) -> Result<LibraryCollection, AppError>;
}
