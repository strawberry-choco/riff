# Data Model

This document describes the concrete types that make up riff's state: the stored entities in `riff-persistence`, the playback domain types in `riff-playback`, the two session structs, and the port traits that define the boundaries infrastructure implements. The Application Store (`riff.sqlite3`) is the single authoritative — and single — implementation of collection semantics: there is no second in-memory copy of the library, and views read the store through port queries and Session Projections. See [./persistence.md](./persistence.md) for how each type is stored and [./architecture.md](./architecture.md) for where these types live and how crates may use them.

## Stored Entities (`riff-persistence`)

The persistence contract has no dependencies at all — no serde, no I/O. Its types are plain data plus pure logic.

### TrackId

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);
```

A newtype wrapper around a `String`. A track's identity is its full file path: `TrackId::from_path` builds the id from `PathBuf::to_string_lossy()`. `TrackId` is `Hash` and `Eq`, and it keys the store's `tracks` table (primary key `path`) and playlist entries.

### Track

```rust
#[derive(Debug, Clone)]
pub struct Track {
    pub id: TrackId,
    pub file_path: PathBuf,
    pub metadata: TrackMetadata,
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub play_count: u32,
    pub last_played: Option<SystemTime>,
    pub date_added: Option<SystemTime>,
    pub search_text: String,
}
```

A single audio file in the library. `duration`, `sample_rate`, and `channels` are optional because they come from metadata probing and may be unavailable for some files. `play_count`, `last_played`, and `date_added` are the per-track play-history facts persisted in the store's `tracks` table. `search_text` is the derived lowercased search corpus (title, artist, album, album artist) that the store's search queries match against.

### TrackMetadata

```rust
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub composer: Option<String>,
    pub comment: Option<String>,
}
```

Tag data read from the file. Every field is optional. Helper methods provide display fallbacks: `display_title` falls back to the file stem (underscores replaced with spaces), `display_artist` to `"Unknown Artist"`, `display_album` to `"Unknown Album"`, and `display_album_artist` to the track artist. `search_text` lowercases title, artist, album, and album artist into a single searchable string.

### Album

```rust
#[derive(Debug, Clone)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<TrackId>,
    pub year: Option<u32>,
    pub genre: Option<String>,
}
```

An aggregate of tracks grouped by album. The store keys albums by the composite `(album_artist, title)` pair, surfaced in queries as the `"album_artist - album"` string. `tracks` is kept sorted by track number (missing numbers first, path tiebreak).

### Artist

```rust
#[derive(Debug, Clone)]
pub struct Artist {
    pub name: String,
    pub albums: Vec<String>,
}
```

An artist and the list of album keys attributed to them. The name used is the album artist when present.

### CoverSource

```rust
#[derive(Debug, Clone)]
pub enum CoverSource {
    Embedded(Vec<u8>),
    Filesystem(PathBuf),
    None,
}
```

Where a track's cover art comes from: bytes embedded in the file's tags, an image file on disk, or nowhere. This type is not serialized; it is a transient value passed to the cover loader.

### PlaybackState, RepeatMode, PlaybackPosition (`riff-playback`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState { Stopped, Playing, Paused }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepeatMode { #[default] None, All, One }

#[derive(Debug, Clone, Copy, Default)]
pub struct PlaybackPosition {
    pub current: std::time::Duration,
    pub total: Option<std::time::Duration>,
}
```

`PlaybackState` is the playback state machine. `RepeatMode` drives queue repetition (`None -> All -> One -> None` via `toggle_repeat`). `PlaybackPosition` carries the current elapsed time and the optional total; it is not serialized.

### PlaybackCommand and PlaybackUpdate (`riff-playback`)

```rust
pub enum PlaybackCommand {
    Play(TrackId), Pause, Resume, Stop, Seek(Duration), SetVolume(f32),
    Next, Previous, ToggleVisibility, PlayNext(TrackId), AddToQueue(TrackId), PlayPause,
}

pub enum PlaybackUpdate {
    StateChanged(PlaybackState),
    PositionChanged(PlaybackPosition),
    TrackChanged(TrackId),
    TrackEnded,
    Error(String),
}
```

These are the messages that cross the playback channels. `PlaybackCommand` flows from the UI and tray (through their `FacadeTransport`s) to the audio engine; `PlaybackUpdate` flows back from the engine to the Playback Coordinator. Neither is serialized. See [./threading-model.md](./threading-model.md) for the channel directions.

### PlaybackQueue (`riff-playback`)

```rust
#[derive(Debug, Clone, Default)]
pub struct PlaybackQueue {
    pub tracks: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub shuffled_indices: VecDeque<usize>,
    pub shuffle_history: Vec<usize>,
}
```

The playback queue and its shuffle/repeat state. `next()` and `previous()` implement ordering, shuffle (using `fastrand` to permute `shuffled_indices`), and repeat semantics; `shuffle_history` lets `previous()` walk back through a shuffled sequence. Mutation helpers include `append`, `insert_next`, `remove`, `set_shuffle`, `toggle_repeat`, and `clear`.

## Session State

The former single `AppState` split into two session structs (ADR 0009), each behind its own `Arc<Mutex<_>>` and owned by the capability that mutates it. Neither is persisted as a whole: the library collection lives only in the Application Store and is read through port queries and Session Projections, and playlists are a Session Projection refreshed from the store after each committed change.

### `PlaybackSession` (`riff-playback/src/app/state.rs`)

The half the audio engine, Playback Coordinator, and transports touch:

```rust
pub struct PlaybackSession {
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    pub muted: bool,
    pub replaygain_enabled: bool,
}
```

`muted` is independent of `current_volume` — the slider keeps its value while muted, and the engine always receives `effective_volume()`, so muting never moves the slider. `replaygain_enabled` opts into loudness normalization; the engine applies each track's peak-capped gain during decoding.

### `LibrarySession` (`riff-backend/src/app/state.rs`)

Everything that is not playback — selection, views, search, library roots and their statuses, scan status, browse mode, UI flags, and per-root watch states:

```rust
pub struct LibrarySession {
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub search_query: String,
    pub library_paths: Vec<PathBuf>,
    pub library_statuses: HashMap<PathBuf, LibraryStatus>,
    pub scan_status: Option<String>,
    pub browse_mode: BrowseMode,
    pub selected_folder: Option<PathBuf>,
    pub ui_flags: UiFlags,
    pub watch_states: HashMap<PathBuf, WatchState>,
}
```

| Field | Meaning |
|-------|---------|
| `selected_track` | The track selected in the UI details panel (view-independent). |
| `view_mode` | Which top-level View is showing: `Library`, `NowPlaying`, or `Settings`. |
| `search_query` | The current search bar text. |
| `library_paths` | Registered library root folders. Persisted in the Application Store's typed settings tables. |
| `library_statuses` | Per-path scan status. |
| `scan_status` | A human-readable status/error line for the most recent scan or playback error (playback errors arrive as typed notices through the facade). |
| `browse_mode` | Whether the sidebar shows the metadata hierarchy (`Library`) or the folder tree (`Folders`). |
| `selected_folder` | The selected folder in Folders browse mode. |
| `ui_flags` | Library-browser and display flags: `show_artists_view`, `advanced_mode`, `high_contrast`, `compact_density`, and per-column toggles (track numbers, artwork, duration, play count, date added). |
| `watch_states` | Per-path folder-watch state. |

### Supporting enums

```rust
pub enum LibraryStatus { Idle, Scanning { files_found: usize }, Scanned(usize), Unavailable }
pub enum BrowseMode { Library, Folders }                 // default: Library
pub enum ViewMode { Library, NowPlaying, Settings }
pub enum WatchState { Disabled, Enabled, Warning(String) }  // default: Disabled
```

`LibraryStatus` tracks a path through its scan lifecycle. `BrowseMode` and `ViewMode` select which UI is rendered. `WatchState` records whether folder watching is active, disabled by the user, or unavailable for a system reason (the `Warning` variant carries a human-readable reason); it persists in the Application Store's watch-state table.

## Application Store Ports (`riff-persistence`)

The Application Store is riff's single authoritative persistent state — Library collection, Playlists, and Settings in one SQLite database. The persistence contract defines one port per section, and the adapter (`SqliteStore` in `riff-infra/src/store/sqlite.rs`) implements them over one shared, mutex-guarded connection:

- `SettingsStore` — load/save preferences: scalar settings, library paths, watch states.
- `PlaylistStore` — playlist CRUD plus `load_playlist_entries`, which returns each entry as a `PlaylistEntry { id, track, valid }` with its Library validity computed by a SQL LEFT JOIN against tracks (dangling references stay listed with `valid == false`).
- `LibraryMutationStore` — scan batches, play-history recording, targeted tag refresh, removal by root, and Clear Library. Every committed mutation bumps the session generations (library and playlist) inside the store, so Session Projections refetch.
- `LibraryQueryStore` — all library reads: single-track lookup, bounded flat/search windows, canonical `all_track_ids()` ordering (path ascending) for Queue Fill, artist/album browsing, folder queries, smart playlists (parameterized by the relocated `LOST_GEMS_THRESHOLD`), and search counts.

Views never query SQLite directly: per-frame reads go through the Session Projections — the library-side projections in `riff-library/src/app/projection.rs` (bounded windows for the flat list and search, browsing/folder/smart-playlist caches) and the playback-side projection in `riff-playback/src/app/projection.rs` (current Track, Up Next window, details-panel selection). All projections invalidate on generation bumps; the frontend reaches them through the Session Views facade in `riff-backend`.

## Port Traits (`riff-playback`, `riff-library`, `riff-persistence`)

Each crate defines the contracts that `riff-infra` implements. These traits are the seam that keeps external crates out of the slices.

```rust
// riff-playback/src/infra/ports.rs
pub type DecoderFactory = Box<dyn Fn() -> Box<dyn AudioDecoder> + Send>;

pub trait AudioDecoder: Send {
    fn source_path(&self) -> &Path;
    fn init(&mut self, path: &Path) -> Result<AudioFormatInfo, PlaybackError>;
    fn next_frames(&mut self, buf: &mut [f32]) -> Option<usize>; // None at EOF
    fn seek(&mut self, position: Duration) -> Duration;          // returns actual position
    fn duration(&self) -> Option<Duration>;
}

pub trait AudioOutput: Send {
    fn start(&mut self, format: AudioFormatInfo) -> Result<(), PlaybackError>;
    fn write(&mut self, samples: &[f32]) -> usize;
    fn stop(&mut self);
    fn set_volume(&mut self, volume: f32);
    fn latency(&self) -> u32;
}
```

```rust
// riff-library/src/infra/ports.rs (re-exported through app/traits.rs)
pub trait MetadataReader: Send + Sync {
    fn read_all(
        &self,
        path: &Path,
    ) -> Result<(TrackMetadata, Duration, CoverSource, AudioFormatInfo), LibraryError>;
    fn read_cover_source(&self, path: &Path) -> Result<CoverSource, LibraryError>;
}

pub trait MetadataWriter: Send + Sync {
    fn write_tags(&self, path: &Path, edit: &TagEdit) -> Result<(), LibraryError>;
}

pub trait CoverLoader: Send + Sync {
    fn load_cover(&self, source: &CoverSource) -> Result<Option<CoverImage>, LibraryError>;
}

pub trait FilesystemWatch: Send {
    fn watch(&mut self, path: &Path) -> Result<(), LibraryError>;
    fn unwatch(&mut self, path: &Path) -> Result<(), LibraryError>;
    // ...
}
```

Two supporting data types cross these traits — `AudioFormatInfo` (`sample_rate`, `channels`) and `CoverImage` (`width`, `height`, `rgba`).

The concrete implementations — `SymphoniaDecoder`, `CpalAudioOutput`, `LoftyMetadataReader`, `LoftyMetadataWriter`, `ImageCoverLoader`, `AudioFileScanner`, `FilesystemWatcher`, and `SqliteStore` — all live in `riff-infra`. `CpalAudioOutput` additionally exposes the richer surface the engine port maps onto (`initialize`/`start`, which owns the device-default-rate fallback, `clear_buffer`, `set_replaygain`, `effective_sample_rate`). See [./dependencies.md](./dependencies.md) for the crates behind them.

## See also

- [./architecture.md](./architecture.md) — the layer rules that constrain these types.
- [./persistence.md](./persistence.md) — how these types persist in the Application Store.
