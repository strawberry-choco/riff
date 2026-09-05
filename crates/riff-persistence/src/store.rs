//! Application-layer ports for the `SQLite` `Application Store`.
//!
//! The `Application Store` is the single authoritative persistent state of the
//! application (`Library`, `Playlists`, `Settings`). These ports keep the app layer
//! free of infrastructure types: infrastructure implements them over a shared
//! `rusqlite` connection, and the UI never imports `rusqlite` directly.

use crate::errors::StoreError;
use crate::playlist::{Playlist, PlaylistId};
use crate::track::{Album, Artist, GenreCount, SmartPlaylistKind, Track, TrackId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Per-root filesystem watching state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WatchState {
    #[default]
    Disabled,
    Enabled,
    Warning(String),
}

/// The audio file extensions the Library Scan indexes, lowercase, no dots —
/// the single source shared by the scanner and the Settings Library pane's
/// format chips (design-handoff issue 12).
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "opus", "ogg", "flac", "wav"];

/// The Library Scan's behavior knobs, projected from the persisted
/// [`ScalarSettings`] and handed to the filesystem walk as an explicit
/// value (design-handoff issue 12). The default mirrors the scanner's
/// historical behavior: hidden entries skipped, every [`AUDIO_EXTENSIONS`]
/// format indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    /// Leave dot-prefixed files and directories out of the walk.
    pub skip_hidden_files: bool,
    /// The enabled audio extensions, lowercase without dots.
    pub formats: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            skip_hidden_files: true,
            formats: AUDIO_EXTENSIONS
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
        }
    }
}

impl From<&ScalarSettings> for ScanOptions {
    fn from(scalars: &ScalarSettings) -> Self {
        Self {
            skip_hidden_files: scalars.skip_hidden_files,
            formats: scalars.scan_formats.clone(),
        }
    }
}

/// How artwork stands in for Tracks and Albums that have none — the
/// Settings Library pane's "Missing artwork" choice (design-handoff issue
/// 12). One strategy ships today; the enum leaves room for more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingArtworkStrategy {
    /// A solid colour derived deterministically from the item's own
    /// identity, so covers read as part of the palette rather than a
    /// fallback glyph.
    #[default]
    GeneratedColour,
}

/// One completed full Library Scan's recorded summary (design-handoff
/// issue 12): when it finished and what it saw, served back by
/// [`LibraryQueryStore::last_full_scan`] for the Settings Library pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullScanSummary {
    /// When the scan completed.
    pub at: SystemTime,
    /// Audio files the scan discovered under the scanned root(s).
    pub files: usize,
    /// Discovered files that could not be indexed (unreadable metadata);
    /// they are skipped, never committed.
    pub errors: usize,
}

/// The single-row scalar preferences persisted in the Application Store's
/// typed settings table. Volume is `None` while the user has not yet moved
/// the slider, so the caller applies its own default.
///
/// `repeat_mode` encodes the playback repeat cycle as an integer so this
/// crate stays dependency-free: 0 = off, 1 = repeat all, 2 = repeat one.
/// `browser_layout` likewise encodes the library browser column's render
/// mode: 0 = list, 1 = grid (design-handoff issue 06).
///
/// The Library scan preferences (design-handoff issue 12): `skip_hidden_files`
/// leaves dot-entries out of the scan walk, `scan_formats` lists the enabled
/// audio extensions (subsets of [`AUDIO_EXTENSIONS`]), and the artwork pair
/// drives cover resolution.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the struct mirrors the scalar settings row's typed columns"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarSettings {
    pub volume: Option<f32>,
    pub advanced_mode: bool,
    pub high_contrast: bool,
    pub replaygain_enabled: bool,
    pub shuffle: bool,
    pub repeat_mode: i64,
    pub browser_layout: i64,
    /// Skip hidden (dot-prefixed) files and directories during scans.
    pub skip_hidden_files: bool,
    /// The enabled audio extensions, lowercase without dots. Defaults to
    /// every entry of [`AUDIO_EXTENSIONS`].
    pub scan_formats: Vec<String>,
    /// Read artwork embedded in track tags before filesystem fallbacks.
    pub read_embedded_artwork: bool,
    /// What renders for Tracks and Albums with no artwork.
    pub missing_artwork_strategy: MissingArtworkStrategy,
}

impl Default for ScalarSettings {
    /// The scanner's historical behavior: hidden files skipped, every
    /// supported format indexed, embedded artwork read.
    fn default() -> Self {
        Self {
            volume: None,
            advanced_mode: false,
            high_contrast: false,
            replaygain_enabled: false,
            shuffle: false,
            repeat_mode: 0,
            browser_layout: 0,
            skip_hidden_files: true,
            scan_formats: AUDIO_EXTENSIONS
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
            read_embedded_artwork: true,
            missing_artwork_strategy: MissingArtworkStrategy::default(),
        }
    }
}

/// Age threshold for the Lost Gems smart playlist: tracks whose last play is
/// older than this are considered forgotten gems worth resurfacing. Tracks
/// that were never played qualify unconditionally ("unheard" includes
/// never-heard). Lives beside the [`LibraryQueryStore::smart_playlist`] port
/// because it parameterizes that query's semantics; the SQL implementation
/// in infrastructure reads it from here.
pub const LOST_GEMS_THRESHOLD: Duration = Duration::from_hours(2160);

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
    fn open_and_migrate(&self, path: &std::path::Path) -> Result<(), StoreError>;
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
    fn load_settings(&self) -> Result<Settings, StoreError>;

    /// Persist the scalar block as one small durable transaction.
    fn save_scalars(&mut self, scalars: &ScalarSettings) -> Result<(), StoreError>;

    /// Replace the library-path list as one small durable transaction.
    fn save_library_paths(&mut self, paths: &[PathBuf]) -> Result<(), StoreError>;

    /// Replace the whole watch-state map as one small durable transaction.
    fn save_watch_states(
        &mut self,
        states: &HashMap<PathBuf, WatchState>,
    ) -> Result<(), StoreError>;
}

/// One user-playlist entry together with its Library validity: `valid` is
/// true exactly when the referenced Track is still present in the
/// Application Store's Library collection, computed by a SQL LEFT JOIN
/// against tracks. A dangling reference — an entry whose file left the
/// library — stays listed with `valid == false` and its `track` unset;
/// that is product behavior per ADR 0001, not corruption, and it resolves
/// again once the file returns via a rescan. Whether an otherwise known
/// track's file still exists on disk is a filesystem concern no store query
/// can answer: callers apply that check on top of `valid` where playback
/// semantics require it (see `playlist_manager`).
#[derive(Debug, Clone)]
pub struct PlaylistEntry {
    /// The referenced [`TrackId`](crate::track::TrackId) (its full file path).
    pub id: TrackId,
    /// The resolved Track when the Library knows it, `None` when dangling.
    pub track: Option<Track>,
    /// Whether the Library collection still contains this entry's track.
    pub valid: bool,
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
    fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError>;

    /// Load one Playlist's entries in playlist order, each with its Library
    /// validity flag computed by a SQL LEFT JOIN against tracks (see
    /// [`PlaylistEntry`]). Dangling references stay listed with
    /// `valid == false` — ADR 0001 product behavior, never silently
    /// dropped. Unknown playlist ids yield an empty `Vec`.
    fn load_playlist_entries(&self, id: &PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError>;

    /// Create a Playlist named `name` (trimmed) with optional initial Track
    /// references (exact duplicates dropped, order preserved). The generated
    /// id is made unique against existing playlists so same-millisecond
    /// creation of same-named playlists cannot collide; duplicate names are
    /// allowed.
    fn create_playlist(
        &mut self,
        name: &str,
        initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, StoreError>;

    /// Rename the Playlist with `id` to `new_name` (trimmed). Returns
    /// whether the playlist was found.
    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, StoreError>;

    /// Delete the Playlist with `id` together with its entries. Returns
    /// whether anything was removed.
    fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, StoreError>;

    /// Append `track` to the Playlist with `id`. Exact duplicates are
    /// ignored (returns `false`), as are unknown playlist ids.
    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Result<bool, StoreError>;

    /// Remove all occurrences of `track` from the Playlist with `id`.
    /// Returns whether anything was removed.
    fn remove_playlist_entries(
        &mut self,
        id: &PlaylistId,
        track: &TrackId,
    ) -> Result<bool, StoreError>;

    /// Persist a new entry order for the Playlist with `id` as ONE immediate
    /// durable transaction: the entries are rewritten to exactly `ordered`
    /// (position 0..n). `ordered` must be a permutation of the playlist's
    /// current entries — callers derive it from loaded state (e.g. the UI's
    /// drag-reorder math), never invent ids. Returns whether the playlist
    /// was found; unknown ids change nothing.
    fn reorder_playlist_entries(
        &mut self,
        id: &PlaylistId,
        ordered: &[TrackId],
    ) -> Result<bool, StoreError>;
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

    /// Freshness guard alias for [`Self::current`] — used by projections to
    /// check if their cached generation is still current (issue 10).
    #[must_use]
    pub fn generation_at(&self) -> u64 {
        self.current()
    }
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
    fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, StoreError>;

    /// Record one completed play for `id` as ONE immediate durable
    /// transaction: `play_count` increments and `last_played` moves to
    /// `played_at` together or not at all, so a crash right after a finished
    /// track cannot lose the play. Returns whether the track was known;
    /// unknown ids change nothing.
    fn record_track_played(
        &mut self,
        id: &TrackId,
        played_at: SystemTime,
    ) -> Result<bool, StoreError>;

    /// Set or clear `id`'s favorite flag as ONE immediate durable
    /// transaction, so a crash right after a toggle cannot lose it. Returns
    /// whether a change was committed: the track was known AND its flag
    /// state actually moved — unknown ids and redundant sets change nothing
    /// (and invalidate no projections).
    fn set_track_favorite(&mut self, id: &TrackId, favorite: bool) -> Result<bool, StoreError>;

    /// The targeted tag-edit refresh: upsert `track`'s metadata as ONE
    /// immediate durable transaction without ever touching its play history
    /// (`play_count`, `last_played`, `date_added`). Unlike a plain rescan,
    /// this flow re-derives every affected album's year/genre from its
    /// first-added remaining track and cleans up albums left empty (and
    /// artists left with no albums) when the edit moves the track to another
    /// album.
    fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError>;

    /// Remove everything belonging to the library root `root` as ONE
    /// immediate durable transaction: exactly that root's tracks (byte-prefix
    /// match on the track path), albums left without tracks, artists left
    /// without albums, and the root's own `library_paths` record. Playlist
    /// entries referencing removed tracks survive as dangling references and
    /// stay listed until the files return. Returns the number of tracks
    /// removed; an unknown root removes nothing.
    fn remove_library_path(&mut self, root: &std::path::Path) -> Result<usize, StoreError>;

    /// The maintenance wipe: delete the Library collection section's
    /// contents — every track (with its play history), album, and artist —
    /// as ONE immediate durable transaction, so a failure mid-clear cannot
    /// leave partial deletion. Playlists and Settings tables are untouched:
    /// playlist entries referencing wiped tracks survive dangling and stay
    /// listed until the files return via a rescan. Returns the number of
    /// tracks removed.
    fn clear_library(&mut self) -> Result<usize, StoreError>;

    /// Record the completion of one full library scan as ONE immediate
    /// durable transaction: the summary overwrites any previous one in the
    /// store's metadata table, and the committed write bumps + notifies the
    /// session Library generation like every Library mutation, so the read
    /// models refetch (design-handoff issues 05 and 12).
    fn record_full_scan_completed(&mut self, summary: FullScanSummary) -> Result<(), StoreError>;
}

/// The sidebar-count totals for the Library collection, answered by ONE
/// store query (design-handoff issue 05): total tracks, distinct artists,
/// distinct `(album artist, title)` albums, and distinct non-empty per-track
/// genres. A fresh store answers all zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LibraryCounts {
    /// Total number of stored Tracks.
    pub tracks: usize,
    /// Distinct album artists (the sidebar's Artists rows).
    pub artists: usize,
    /// Distinct `(album artist, title)` albums (the sidebar's Albums rows).
    pub albums: usize,
    /// Distinct non-empty per-track genres (the sidebar's Genres rows).
    pub genres: usize,
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
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError>;

    /// One bounded window of the flat library list, path-ascending.
    fn tracks_window(&self, offset: usize, limit: usize) -> Result<Vec<Track>, StoreError>;

    /// Total number of stored Tracks (for the flat list projection).
    fn track_count(&self) -> Result<usize, StoreError>;

    /// The Library-count totals — tracks, artists, albums, genres — in ONE
    /// query (design-handoff issue 05), so the sidebar-counts read model
    /// costs one store round trip per generation instead of four queries.
    /// Genre semantics match [`Self::genre_counts`]: distinct non-empty
    /// per-track genres.
    fn library_counts(&self) -> Result<LibraryCounts, StoreError>;

    /// Every smart playlist's unbounded total in [`SmartPlaylistKind::ALL`]
    /// order — how many tracks each list would contain with no limit
    /// (design-handoff issue 05). Membership semantics are exactly the
    /// [`Self::smart_playlist`] queries' filters; the `Lost Gems` count is
    /// relative to the current clock like the list itself.
    fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError>;

    /// Every Track id in the collection, ordered by full path ascending —
    /// the canonical flat ordering (ADR 0003). Serves Queue Fill: the whole
    /// Library loads into the Playback Queue in this order when playback
    /// starts from an empty queue.
    fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError>;

    /// One bounded window of case-insensitive substring matches over title,
    /// artist, album, and album artist, path-ascending. The query is
    /// lowercased in Rust before matching.
    fn search_window(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Track>, StoreError>;

    /// Total number of matches for [`Self::search_window`] semantics.
    fn search_count(&self, query: &str) -> Result<usize, StoreError>;

    /// Every artist in the collection, name-ascending (byte-wise, matching
    /// the former UI sort). Each artist's `albums` lists its composite keys
    /// (`"album artist - title"`) in the canonical browsing order — year
    /// descending with missing years last, then title ascending — so callers
    /// never re-sort.
    fn all_artists(&self) -> Result<Vec<Artist>, StoreError>;

    /// One artist's albums in canonical browsing order: year descending with
    /// missing years last (treated as 0), then title ascending byte-wise.
    /// Each album carries its track ids in album-track order (track number
    /// ascending with missing numbers first, path tiebreak). Unknown artists
    /// yield an empty `Vec`.
    fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError>;

    /// One album's tracks in full, in canonical album-track order: track
    /// number ascending with missing numbers first (the legacy
    /// `unwrap_or(0)` slot), then path tiebreak. Unknown albums yield an
    /// empty `Vec`.
    fn album_tracks(&self, album_artist: &str, album_title: &str)
    -> Result<Vec<Track>, StoreError>;

    /// Whether any stored Track lives under `folder` (component-wise path
    /// prefix, exactly like the former in-memory `Path::starts_with` checks:
    /// case-sensitive, and a sibling sharing a byte-prefix — `a` vs `ab` —
    /// never matches). Backed by escaped SQL prefix matching over stored
    /// track paths; there is no folder table. Separators are matched
    /// literally, so callers pass consistently separated paths — what scan
    /// joins and `Path::join` always produce.
    fn folder_has_audio(&self, folder: &std::path::Path) -> Result<bool, StoreError>;

    /// Whether any stored Track under `folder` matches the search query
    /// (same substring semantics as [`Self::search_window`]: the query is
    /// lowercased in Rust and matched literally against the derived
    /// Rust-lowercased search text).
    fn folder_has_search_match(
        &self,
        folder: &std::path::Path,
        query: &str,
    ) -> Result<bool, StoreError>;

    /// Every Track id under `folder`, ordered by full path exactly like the
    /// former in-memory tree listing (component-wise `Path` order).
    fn track_ids_in_folder_tree(
        &self,
        folder: &std::path::Path,
    ) -> Result<Vec<TrackId>, StoreError>;

    /// The Tracks directly inside `folder` (their parent is exactly
    /// `folder`), ordered by track number ascending with missing numbers
    /// first, then filename — the former in-memory direct-listing order.
    fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, StoreError>;

    /// How many tracks live under `folder` (component-wise path prefix, the
    /// same membership as [`Self::track_ids_in_folder_tree`]) — the
    /// per-music-folder count the Settings Library pane shows
    /// (design-handoff issue 05).
    fn folder_track_count(&self, folder: &std::path::Path) -> Result<usize, StoreError>;

    /// The last completed full library scan's summary — when it finished and
    /// what it saw — as recorded by
    /// [`LibraryMutationStore::record_full_scan_completed`]. `None` when no
    /// scan has ever completed in this store (design-handoff issue 12).
    fn last_full_scan(&self) -> Result<Option<FullScanSummary>, StoreError>;

    /// The direct child directories of `folder` that contain at least one
    /// stored Track anywhere beneath them, name-ascending (`PathBuf` order,
    /// like the former in-memory tree walk). Derived purely from stored
    /// track paths: a child counts as a directory when some stored track
    /// lives deeper inside it.
    fn subdirs_with_audio(&self, folder: &std::path::Path) -> Result<Vec<PathBuf>, StoreError>;

    /// Compute one read-only smart playlist as a store query, reproducing
    /// the former in-memory semantics precisely: the same filters, ordering
    /// tie-breaks (including display-title fallbacks), limits, and the
    /// ninety-day Lost Gems threshold. Tracks arrive ready to render — the
    /// former id list plus per-id resolution composed into one result.
    fn smart_playlist(
        &self,
        kind: SmartPlaylistKind,
        limit: usize,
    ) -> Result<Vec<Track>, StoreError>;

    /// Every genre carrying at least one stored Track, name-ascending
    /// (byte-wise), each with its per-track count aggregated from the
    /// tracks' own genre metadata — not the albums' derived genre column.
    /// Tracks whose genre is missing or empty aggregate into nothing: there
    /// is no row for them. The sidebar's total-genres count is this list's
    /// length.
    fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError>;

    /// Every artist having at least one Track with genre `genre`,
    /// name-ascending, each with only the album keys of albums holding at
    /// least one matching track, in canonical browsing order. Matching is an
    /// exact comparison against the stored per-track genre. Unknown genres
    /// yield an empty `Vec`.
    fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError>;

    /// One artist's albums holding at least one Track with genre `genre`,
    /// in canonical browsing order, each carrying only its matching track
    /// ids in album-track order. Unknown artists or genres yield an empty
    /// `Vec`.
    fn artist_albums_in_genre(&self, artist: &str, genre: &str) -> Result<Vec<Album>, StoreError>;

    /// One album's Tracks with genre `genre`, in canonical album-track
    /// order: track number ascending with missing numbers first, then path
    /// tiebreak. Unknown albums or genres yield an empty `Vec`.
    fn album_tracks_in_genre(
        &self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
    ) -> Result<Vec<Track>, StoreError>;
}

/// Notification the `Application Store` emits (best-effort) over a
/// crossbeam channel sender whenever a committed mutation bumps one of
/// its two session generations (ADR 0002). The `BackendFacade` drains
/// this channel on the frontend frame path, coalesces rapid Library bumps,
/// and surfaces them as `BackendEvents`.
///
/// Invariant (emit-beside-bump): every committed mutation in
/// the `SqliteStore` adapter sends exactly one of these over the
/// sender, immediately after bumping the corresponding generation. A
/// mutation that fails to commit sends nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum StoreChanged {
    /// The session Library generation moved to `gen` on a committed
    /// Library mutation (scan batch, play recording, tag edit, remove,
    /// clear). The facade coalesces these to roughly four per second.
    Library(u64),

    /// The session playlist generation moved to `gen` on a committed
    /// playlist mutation (create, rename, delete, add, remove, reorder).
    /// These are infrequent user actions and are forwarded without
    /// coalescing.
    Playlists(u64),
}

/// The [`StoreChanged`] variant produced by the given `StoreChanged`.
#[cfg(test)]
mod issue04_store_events {
    use super::{StoreChanged, StoreGeneration};

    #[test]
    fn store_generation_bumps_are_independent() {
        let lib = StoreGeneration::new();
        let pl = StoreGeneration::new();
        lib.bump();
        lib.bump();
        pl.bump();
        assert_eq!(lib.current(), 2);
        assert_eq!(pl.current(), 1);
    }

    #[test]
    fn store_changed_carries_generation_value() {
        let generation = 5;
        let e = StoreChanged::Library(generation);
        assert_eq!(e, StoreChanged::Library(generation));
        assert_ne!(e, StoreChanged::Library(generation + 1));
        assert_ne!(e, StoreChanged::Playlists(generation));
    }
}
