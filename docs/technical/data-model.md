# Data Model

This document describes the concrete types that make up riff's state: the pure domain entities in `src/domain/`, the application-layer `AppState`, and the port traits that define the boundary to infrastructure. The Application Store (`riff.sqlite3`) is the single authoritative — and single — implementation of collection semantics: there is no second in-memory copy of the library, and views read the store through port queries and Session Projections. See [./persistence.md](./persistence.md) for how each type is stored and [./architecture.md](./architecture.md) for where these types live and how layers may use them.

## Domain Entities (`src/domain/`)

The domain layer has no external crate dependencies — no serde, no I/O. Its types are plain data plus pure logic.

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
}
```

A single audio file in the library. `duration`, `sample_rate`, and `channels` are optional because they come from metadata probing and may be unavailable for some files. `play_count`, `last_played`, and `date_added` are the per-track play-history facts persisted in the store's `tracks` table.

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

### PlaybackState, RepeatMode, PlaybackPosition

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

### PlaybackCommand and PlaybackUpdate

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

These are the messages that cross the playback channels. `PlaybackCommand` flows from the UI and tray to the audio engine; `PlaybackUpdate` flows back from the engine to the update processor. Neither is serialized. See [./threading-model.md](./threading-model.md) for the channel directions.

### PlaybackQueue

```rust
#[derive(Debug, Clone, Default)]
pub struct PlaybackQueue {
    pub tracks: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub shuffled_indices: Vec<usize>,
    pub shuffle_history: Vec<usize>,
}
```

The playback queue and its shuffle/repeat state. `next()` and `previous()` implement ordering, shuffle (using `rand` to permute `shuffled_indices`), and repeat semantics; `shuffle_history` lets `previous()` walk back through a shuffled sequence. Mutation helpers include `append`, `insert_next`, `remove`, `set_shuffle`, `toggle_repeat`, and `clear`.

## Application State (`src/app/state.rs`)

`AppState` is the single struct that holds all runtime state. It lives behind `Arc<Mutex<_>>`, is read by the UI every frame, and is written mostly by the update processor thread. It is never persisted as a whole: the library collection lives only in the Application Store and is read through port queries and Session Projections; playlists are a Session Projection refreshed from the store after each committed change.

```rust
pub struct AppState {
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    pub muted: bool,
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub window_visible: bool,
    pub search_query: String,
    pub library_paths: Vec<PathBuf>,
    pub library_statuses: HashMap<PathBuf, LibraryStatus>,
    pub scan_status: Option<String>,
    pub browse_mode: BrowseMode,
    pub selected_folder: Option<PathBuf>,
    pub ui_flags: UiFlags,
    pub watch_states: HashMap<PathBuf, WatchState>,
    pub replaygain_enabled: bool,
    pub playlists: Vec<Playlist>,
}
```

Field by field:

| Field | Type | Meaning |
|-------|------|---------|
| `queue` | `PlaybackQueue` | Current playback queue and shuffle/repeat state. |
| `playback_state` | `PlaybackState` | Stopped, Playing, or Paused. |
| `current_position` | `PlaybackPosition` | Elapsed and total time for the current track. |
| `current_volume` | `f32` | Playback volume (default `1.0`). |
| `muted` | `bool` | Mute flag; independent of `current_volume` — the engine receives `effective_volume()`, so muting never moves the slider. |
| `selected_track` | `Option<TrackId>` | The track selected in the UI details panel (view-independent). |
| `view_mode` | `ViewMode` | Which top-level view is showing: `Library`, `NowPlaying`, or `Settings`. |
| `window_visible` | `bool` | Whether the main window is visible (toggled from the tray). |
| `search_query` | `String` | The current search bar text. |
| `library_paths` | `Vec<PathBuf>` | Registered library root folders. Persisted in the Application Store's typed settings tables. |
| `library_statuses` | `HashMap<PathBuf, LibraryStatus>` | Per-path scan status. |
| `scan_status` | `String` (optional) | A human-readable status/error line for the most recent scan or playback error. |
| `browse_mode` | `BrowseMode` | Whether the sidebar shows the metadata hierarchy (`Library`) or the folder tree (`Folders`). |
| `selected_folder` | `Option<PathBuf>` | The selected folder in Folders browse mode. |
| `ui_flags` | `UiFlags` | Library-browser and accessibility flags: `show_artists_view`, `advanced_mode`, `high_contrast`. |
| `watch_states` | `HashMap<PathBuf, WatchState>` | Per-path folder-watch state. |
| `replaygain_enabled` | `bool` | Opt-in ReplayGain loudness normalization. |
| `playlists` | `Vec<Playlist>` | Session Projection of the store's Playlists section; refreshed after each committed change, never authoritative. Playlists survive a Clear Library. |

### Supporting enums

```rust
pub enum LibraryStatus { Idle, Scanning { files_found: usize }, Scanned(usize), Unavailable }
pub enum BrowseMode { Library, Folders }                 // default: Library
pub enum ViewMode { Library, NowPlaying, Settings }
pub enum WatchState { Disabled, Enabled, Warning(String) }  // default: Disabled
```

`LibraryStatus` tracks a path through its scan lifecycle. `BrowseMode` and `ViewMode` select which UI is rendered. `WatchState` records whether folder watching is active, disabled by the user, or unavailable for a system reason (the `Warning` variant carries a human-readable reason); it persists in the Application Store's watch-state table.

## Application Store Ports (`src/app/store.rs`)

The Application Store is riff's single authoritative persistent state — Library collection, Playlists, and Settings in one SQLite database. The app layer defines one port per section, and infrastructure (`SqliteStore` in `src/infra/store.rs`) implements them over a shared connection:

- `SettingsStore` — load/save preferences: scalar settings, library paths, watch states.
- `PlaylistStore` — playlist CRUD plus `load_playlist_entries`, which returns each entry as a `PlaylistEntry { id, track, valid }` with its Library validity computed by a SQL LEFT JOIN against tracks (dangling references stay listed with `valid == false`).
- `LibraryMutationStore` — scan batches, play-history recording, targeted tag refresh, removal by root, and Clear Library. Every committed mutation bumps the session generation inside the mutation adapter, so Session Projections refetch.
- `LibraryQueryStore` — all library reads: single-track lookup, bounded flat/search windows, canonical `all_track_ids()` ordering (path ascending) for Queue Fill, artist/album browsing, folder queries, smart playlists (parameterized by the relocated `LOST_GEMS_THRESHOLD`), and search counts.

Views never query SQLite directly: per-frame reads go through the Session Projections in `src/app/projection.rs` — bounded windows for the flat list and search, browsing/folder/smart-playlist caches, and the playback-side projection holding the current Track, the Up Next window, and the details panel's selected Track. All projections invalidate on generation bumps.

## Port Traits (`src/app/traits.rs`)

The application layer defines the contracts that infrastructure implements. These traits are the seam that keeps external crates out of `app/` and `domain/`.

```rust
pub trait AudioDecoder: Send {
    fn open(&mut self, path: &PathBuf) -> Result<AudioFormatInfo, AppError>;
    fn next_frames(&mut self, samples: usize) -> Result<Option<Vec<f32>>, AppError>;
    fn seek(&mut self, position: Duration) -> Result<(), AppError>;
    fn duration(&self) -> Option<Duration>;
    fn close(&mut self) {}
}

pub trait AudioOutput: Send {
    fn initialize(&mut self, sample_rate: u32, channels: u16) -> Result<(), AppError>;
    fn start(&mut self) -> Result<(), AppError>;
    fn stop(&mut self) -> Result<(), AppError>;
    fn write_samples(&mut self, samples: &[f32]) -> Result<usize, AppError>;
    fn set_volume(&mut self, volume: f32);
    fn buffer_len(&self) -> usize;
    fn clear_buffer(&mut self);
}

pub trait MetadataReader: Send + Sync {
    fn read_metadata(&self, path: &PathBuf) -> Result<TrackMetadata, AppError>;
    fn read_duration(&self, path: &PathBuf) -> Result<Option<Duration>, AppError>;
    fn read_cover_source(&self, path: &PathBuf) -> Result<CoverSource, AppError>;
    fn read_audio_format(&self, path: &PathBuf) -> Result<AudioFormatInfo, AppError>;
    fn read_all(&self, path: &PathBuf)
        -> Result<(TrackMetadata, Option<Duration>, CoverSource, AudioFormatInfo), AppError>;
}

pub trait CoverLoader: Send + Sync {
    fn load_cover(&self, source: &CoverSource) -> Result<Option<CoverImage>, AppError>;
}
```

Two supporting data types cross these traits:

```rust
pub struct AudioFormatInfo { pub sample_rate: u32, pub channels: u16, pub duration: Option<Duration> }
pub struct CoverImage { pub width: u32, pub height: u32, pub rgba: Vec<u8> }
```

The concrete implementations are `SymphoniaDecoder`, `CpalAudioOutput`, `LoftyMetadataReader`, and `ImageCoverLoader` in `src/infra/`. See [./dependencies.md](./dependencies.md) for the crates behind them.

## See also

- [./architecture.md](./architecture.md) — the layer rules that constrain these types.
- [./persistence.md](./persistence.md) — how these types persist in the Application Store.
