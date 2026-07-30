# Data Model

This document describes the concrete types that make up riff's state: the pure domain entities in `src/domain/`, the application-layer `AppState` and `LibraryManager`, and the port traits that define the boundary to infrastructure. Types that are serialized to the library cache are marked; see [./persistence.md](./persistence.md) for how that cache is written and read. For where these types live and how layers may use them, see [./architecture.md](./architecture.md).

## Domain Entities (`src/domain/`)

The domain layer has no external crate dependencies. Its types are plain data plus pure logic. Several derive `serde::Serialize`/`serde::Deserialize` so the whole library can be cached as JSON; these are marked **(serde)** below.

### TrackId

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TrackId(pub String);
```

A newtype wrapper around a `String`. A track's identity is its full file path: `TrackId::from_path` builds the id from `PathBuf::to_string_lossy()`. `TrackId` is `Hash` and `Eq` so it can key the library's `HashMap`s. **(serde)**

### Track

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub file_path: PathBuf,
    pub metadata: TrackMetadata,
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
}
```

A single audio file in the library. `duration`, `sample_rate`, and `channels` are optional because they come from metadata probing and may be unavailable for some files. **(serde)**

### TrackMetadata

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

Tag data read from the file. Every field is optional. Helper methods provide display fallbacks: `display_title` falls back to the file stem (underscores replaced with spaces), `display_artist` to `"Unknown Artist"`, `display_album` to `"Unknown Album"`, and `display_album_artist` to the track artist. `search_text` lowercases title, artist, album, and album artist into a single searchable string. **(serde)**

### Album

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<TrackId>,
    pub year: Option<u32>,
    pub genre: Option<String>,
}
```

An aggregate of tracks grouped by album. In the library index it is keyed by a composite string of the form `"album_artist - album"`. `tracks` is kept sorted by track number. **(serde)**

### Artist

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Artist {
    pub name: String,
    pub albums: Vec<String>,
}
```

An artist and the list of album keys attributed to them. The name used is the album artist when present. **(serde)**

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlaybackState { Stopped, Playing, Paused }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PlaybackQueue {
    pub tracks: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub shuffled_indices: Vec<usize>,
    pub shuffle_history: Vec<usize>,
}
```

The playback queue and its shuffle/repeat state. `next()` and `previous()` implement ordering, shuffle (using `rand` to permute `shuffled_indices`), and repeat semantics; `shuffle_history` lets `previous()` walk back through a shuffled sequence. Mutation helpers include `append`, `insert_next`, `remove`, `set_shuffle`, `toggle_repeat`, and `clear`. **(serde)**

## Application State (`src/app/state.rs`)

`AppState` is the single struct that holds all runtime state. It lives behind `Arc<Mutex<_>>`, is read by the UI every frame, and is written mostly by the update processor thread. It is intentionally not serialized as a whole; only its `library` field is cached (via `LibraryManager`).

```rust
pub struct AppState {
    pub library: LibraryManager,
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub window_visible: bool,
    pub search_query: String,
    pub library_paths: Vec<PathBuf>,
    pub library_statuses: HashMap<PathBuf, LibraryStatus>,
    pub scan_status: Option<String>,
    pub browse_mode: BrowseMode,
    pub selected_folder: Option<PathBuf>,
    pub show_artists_view: bool,
    pub watch_states: HashMap<PathBuf, WatchState>,
}
```

Field by field:

| Field | Type | Meaning |
|-------|------|---------|
| `library` | `LibraryManager` | The indexed music library (tracks, artists, albums). Cached to disk. |
| `queue` | `PlaybackQueue` | Current playback queue and shuffle/repeat state. |
| `playback_state` | `PlaybackState` | Stopped, Playing, or Paused. |
| `current_position` | `PlaybackPosition` | Elapsed and total time for the current track. |
| `current_volume` | `f32` | Playback volume (default `1.0`). |
| `selected_track` | `Option<TrackId>` | The track selected in the UI details panel (view-independent). |
| `view_mode` | `ViewMode` | Which top-level view is showing: `Library`, `NowPlaying`, or `Settings`. |
| `window_visible` | `bool` | Whether the main window is visible (toggled from the tray). |
| `search_query` | `String` | The current search bar text. |
| `library_paths` | `Vec<PathBuf>` | Registered library root folders. Persisted via `eframe::Storage`. |
| `library_statuses` | `HashMap<PathBuf, LibraryStatus>` | Per-path scan status. |
| `scan_status` | `String` (optional) | A human-readable status/error line for the most recent scan or playback error. |
| `browse_mode` | `BrowseMode` | Whether the sidebar shows the metadata hierarchy (`Library`) or the folder tree (`Folders`). |
| `selected_folder` | `Option<PathBuf>` | The selected folder in Folders browse mode. |
| `show_artists_view` | `bool` | Within Library browse mode, whether to show the artist/album hierarchy or a flat track list. |
| `watch_states` | `HashMap<PathBuf, WatchState>` | Per-path folder-watch state. |

### Supporting enums

```rust
pub enum LibraryStatus { Idle, Scanning { files_found: usize }, Scanned(usize), Unavailable }
pub enum BrowseMode { Library, Folders }                 // default: Library
pub enum ViewMode { Library, NowPlaying, Settings }
pub enum WatchState { Disabled, Enabled, Warning(String) }  // (serde), default: Disabled
```

`LibraryStatus` tracks a path through its scan lifecycle. `BrowseMode` and `ViewMode` select which UI is rendered. `WatchState` records whether folder watching is active, disabled by the user, or unavailable for a system reason (the `Warning` variant carries a human-readable reason). Only `WatchState` derives serde, for persistence alongside the library paths.

## LibraryManager (`src/app/library_manager.rs`)

`LibraryManager` is the application aggregate that owns the library index. It derives serde so the entire library can be written to and read from the JSON cache as one object.

```rust
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LibraryManager {
    pub tracks: HashMap<TrackId, Track>,
    pub artists: HashMap<String, Artist>,
    pub albums: HashMap<String, Album>,
}
```

Beyond indexing, it provides search (`search`), lookup (`get_track`, `get_album_tracks`, `get_artist_albums`, `all_tracks`, `all_artists`, `all_albums`), folder-oriented projections used by the Folders view (`tracks_in_folder`, `subdirs_with_audio`, `folder_has_audio`, `track_ids_in_folder_tree`), removal (`remove_track`, `remove_tracks_by_root`), and the cache methods `save_cache` / `load_cache`. Album keys are the composite `"album_artist - album"` string; artist keys are the album-artist name.

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
- [./persistence.md](./persistence.md) — which of these types are serialized and where.
