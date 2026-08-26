//! riff Music Player - Test Suite
//!
//! This is the single integration-test crate root (declared in `Cargo.toml`
//! via `[[test]] path = "tests/mod.rs"` with `autotests = false`). It wires the
//! individual test modules together and re-exports the library so the test
//! bodies can refer to types by their short names (via `use super::*`).
//!
//! # Test Organization
//!
//! - `domain_tests.rs`: Tests for domain objects like Track, `TrackId`, `PlaybackState`, etc.
//! - `app_tests.rs`: Tests for application logic like `AppState`, the Session
//!   Projections, and scan-side Track construction.
//! - `infra_tests.rs`: Tests for infrastructure components like audio decoders, metadata readers, etc.
//! - `ui_tests.rs`: Tests for UI-related functionality like settings storage, etc.
//! - `golden_tests.rs`: Golden-image snapshot tests rendering real egui frames headlessly.
//! - `integration_tests.rs`: End-to-end integration tests that test multiple components together.
//!
//! # Running Tests
//!
//! ```bash
//! cargo test                # Run all tests
//! cargo test domain_tests   # Run specific test module
//! cargo test -- --nocapture  # Run tests with output
//! ```

pub mod app_tests;
pub mod domain_tests;
pub mod golden_tests;
pub mod infra_tests;
pub mod integration_tests;
pub mod ui_tests;

// --- Library re-exports ---------------------------------------------------
//
// Bring the library modules into the test crate root so qualified paths such as
// `crate::domain::TrackMetadata`, `crate::app::state::AppState` and
// `crate::app::commands::LibraryCommand` resolve from inside the test modules.
pub use riff::app;
pub use riff::domain;
pub use riff::infra;
pub use riff::ui;

// Prelude of bare names used inside the test bodies through `use super::*`.
// Kept explicit (rather than glob re-exports) to avoid name collisions.
pub use riff::app::MutexExt;
pub use riff::app::gapless::{
    GaplessConditions, QueueConditions, duration_from_frames, elapsed_from_samples,
    formats_gapless_compatible, frames_from_duration, is_gapless_eligible, pre_buffer_cap,
    repeat_one_handoff_eligible, samples_from_duration,
};
pub use riff::app::state::{AppState, LibraryStatus, WatchState, replaygain_factor};
pub use riff::domain::{
    Album, Artist, PlaybackCommand, PlaybackPosition, PlaybackQueue, PlaybackState, PlaybackUpdate,
    Playlist, PlaylistId, RepeatMode, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};
pub use riff::infra::metadata_reader::parse_replaygain_gain;
pub use riff::infra::{
    AudioFileScanner, CpalAudioOutput, FilesystemWatcher, ImageCoverLoader, LoftyMetadataReader,
    LoftyMetadataWriter, SymphoniaDecoder,
};
pub use riff::ui::app::{TagEditState, clamp_seek, format_duration, lru_insert};
pub use riff::ui::settings::{expand_tilde, suggest_directories};

// Standard-library names referenced unqualified in some suites.
pub use std::sync::atomic::AtomicBool;
pub use std::sync::{Arc, Mutex};

// Test utilities that can be used across test modules
pub mod test_utils {
    use crate::domain::{TrackId, TrackMetadata};
    use std::path::PathBuf;

    /// Approximate equality for `f32` values. Tests compare audio parameters
    /// (volume, sample values) with this instead of exact `==` so assertions
    /// stay robust to float representation (`clippy::float_cmp`).
    #[must_use]
    pub fn float_close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-6
    }

    /// Create a test track with the given ID and file path
    pub fn create_test_track(id: &str, file_path: &str) -> crate::domain::Track {
        crate::domain::Track {
            id: TrackId(id.to_string()),
            file_path: PathBuf::from(file_path),
            metadata: TrackMetadata::default(),
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
            search_text: String::new(),
        }
    }

    /// Create a test track with custom metadata
    pub fn create_test_track_with_metadata(
        id: &str,
        file_path: &str,
        artist: &str,
        title: &str,
        album: &str,
    ) -> crate::domain::Track {
        crate::domain::Track {
            id: TrackId(id.to_string()),
            file_path: PathBuf::from(file_path),
            metadata: TrackMetadata {
                artist: Some(artist.to_string()),
                title: Some(title.to_string()),
                album: Some(album.to_string()),
                ..Default::default()
            },
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
            search_text: String::new(),
        }
    }
}

/// Shared trait-based mocks for exercising the port boundaries
/// (`src/app/traits.rs`) without real audio hardware or files on disk.
/// These are intentionally reusable: later suites (e.g. gapless-playback
/// tests) build on the same scripted decoder/output behavior.
pub mod mocks {
    use riff::app::errors::AppError;
    use riff::app::store::{
        LibraryMutationStore, LibraryQueryStore, PlaylistStore, Settings, SettingsStore,
    };
    use riff::app::traits::{
        AudioDecoder, AudioFormatInfo, AudioOutput, CoverImage, CoverLoader, MetadataReader,
        MetadataWriter, TagEdit,
    };
    use riff::domain::{
        Album, Artist, CoverSource, Playlist, PlaylistId, SmartPlaylistKind, Track, TrackId,
        TrackMetadata,
    };
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::Duration;

    /// Scripted [`AudioDecoder`]: `open` returns a configured format (or an
    /// injected error), `next_frames` drains a queue of sample batches and
    /// then reports EOF (`Ok(0)`), and every `seek` is recorded and resets the
    /// stream to the start of the script.
    pub struct MockAudioDecoder {
        pub open_error: Option<String>,
        pub decode_error: Option<String>,
        pub format: AudioFormatInfo,
        pub duration: Option<Duration>,
        /// Full sample script; `queue` is refilled from this on `open` and
        /// reset on `seek`.
        scripted: Vec<Vec<f32>>,
        queue: Vec<Vec<f32>>,
        pub seeks: Vec<Duration>,
        pub opened: Vec<PathBuf>,
        pub closed: bool,
    }

    impl MockAudioDecoder {
        pub fn new(format: AudioFormatInfo) -> Self {
            Self {
                open_error: None,
                decode_error: None,
                format,
                duration: None,
                scripted: Vec::new(),
                queue: Vec::new(),
                seeks: Vec::new(),
                opened: Vec::new(),
                closed: false,
            }
        }

        /// Script the sample batches that `next_frames` will yield, in order.
        #[must_use]
        pub fn with_batches(mut self, batches: Vec<Vec<f32>>) -> Self {
            self.scripted = batches;
            self
        }
    }

    impl AudioDecoder for MockAudioDecoder {
        fn open(&mut self, path: &std::path::Path) -> Result<AudioFormatInfo, AppError> {
            if let Some(ref msg) = self.open_error {
                return Err(AppError::Decode(msg.clone()));
            }
            self.opened.push(path.to_path_buf());
            self.closed = false;
            self.queue = self.scripted.clone();
            Ok(self.format.clone())
        }

        fn next_frames(&mut self, out: &mut [f32]) -> Result<usize, AppError> {
            if let Some(ref msg) = self.decode_error {
                return Err(AppError::Decode(msg.clone()));
            }
            let Some(batch) = self.queue.first_mut() else {
                return Ok(0);
            };
            // Fill as much of `out` as the current scripted batch holds; a
            // batch larger than `out` keeps its remainder queued for the next
            // call, mirroring how the real decoder spills oversized packets
            // into `pending_samples` (nothing is ever dropped).
            let n = out.len().min(batch.len());
            out[..n].copy_from_slice(&batch[..n]);
            batch.drain(..n);
            if batch.is_empty() {
                self.queue.remove(0);
            }
            Ok(n)
        }

        fn seek(&mut self, position: Duration) -> Result<(), AppError> {
            self.seeks.push(position);
            self.queue = self.scripted.clone();
            Ok(())
        }

        fn duration(&self) -> Option<Duration> {
            self.duration
        }

        fn close(&mut self) {
            self.closed = true;
        }
    }

    /// Recording [`AudioOutput`]: tracks every invocation and maintains an
    /// internal buffer so `buffer_len` is meaningful. Errors are injectable
    /// per method.
    pub struct MockAudioOutput {
        pub initialize_error: Option<String>,
        pub write_error: Option<String>,
        pub initialized: Vec<(u32, u16)>,
        pub start_count: usize,
        pub stop_count: usize,
        pub written: Vec<Vec<f32>>,
        pub volumes: Vec<f32>,
        pub clear_count: usize,
        /// The rate reported by the `AudioOutput::effective_sample_rate`
        /// trait method — what a real output's stream was actually built
        /// with. Defaults to 44.1 kHz like `CpalAudioOutput::new`.
        effective_sample_rate: u32,
        buffer: Vec<f32>,
    }

    impl MockAudioOutput {
        pub fn new() -> Self {
            Self {
                initialize_error: None,
                write_error: None,
                initialized: Vec::new(),
                start_count: 0,
                stop_count: 0,
                written: Vec::new(),
                volumes: Vec::new(),
                clear_count: 0,
                effective_sample_rate: 44_100,
                buffer: Vec::new(),
            }
        }

        /// Set the value reported by
        /// [`effective_sample_rate`](riff::app::traits::AudioOutput::effective_sample_rate).
        pub fn set_effective_sample_rate(&mut self, rate: u32) {
            self.effective_sample_rate = rate;
        }
    }

    impl Default for MockAudioOutput {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AudioOutput for MockAudioOutput {
        fn initialize(&mut self, sample_rate: u32, channels: u16) -> Result<(), AppError> {
            if let Some(ref msg) = self.initialize_error {
                return Err(AppError::AudioOutput(msg.clone()));
            }
            self.initialized.push((sample_rate, channels));
            Ok(())
        }

        fn start(&mut self) -> Result<(), AppError> {
            self.start_count += 1;
            Ok(())
        }

        fn stop(&mut self) -> Result<(), AppError> {
            self.stop_count += 1;
            Ok(())
        }

        fn write_samples(&mut self, samples: &[f32]) -> Result<usize, AppError> {
            if let Some(ref msg) = self.write_error {
                return Err(AppError::AudioOutput(msg.clone()));
            }
            self.buffer.extend_from_slice(samples);
            self.written.push(samples.to_vec());
            Ok(samples.len())
        }

        fn set_volume(&mut self, volume: f32) {
            self.volumes.push(volume);
        }

        fn effective_sample_rate(&self) -> u32 {
            self.effective_sample_rate
        }

        fn buffer_len(&self) -> usize {
            self.buffer.len()
        }

        fn clear_buffer(&mut self) {
            self.clear_count += 1;
            self.buffer.clear();
        }
    }

    /// Canned [`MetadataReader`]: returns configured values, or an injected
    /// `AppError::MetadataRead` from every method when `fail` is set.
    pub struct MockMetadataReader {
        pub fail: bool,
        pub metadata: TrackMetadata,
        pub duration: Option<Duration>,
        pub cover_source: CoverSource,
        pub audio_format: AudioFormatInfo,
    }

    impl Default for MockMetadataReader {
        fn default() -> Self {
            Self {
                fail: false,
                metadata: TrackMetadata::default(),
                duration: Some(Duration::from_secs(90)),
                cover_source: CoverSource::None,
                audio_format: AudioFormatInfo {
                    sample_rate: 44_100,
                    channels: 2,
                    duration: Some(Duration::from_secs(90)),
                },
            }
        }
    }

    impl MetadataReader for MockMetadataReader {
        fn read_metadata(&self, _path: &std::path::Path) -> Result<TrackMetadata, AppError> {
            if self.fail {
                return Err(AppError::MetadataRead("mock failure".to_string()));
            }
            Ok(self.metadata.clone())
        }

        fn read_duration(&self, _path: &std::path::Path) -> Result<Option<Duration>, AppError> {
            if self.fail {
                return Err(AppError::MetadataRead("mock failure".to_string()));
            }
            Ok(self.duration)
        }

        fn read_cover_source(&self, _path: &std::path::Path) -> Result<CoverSource, AppError> {
            if self.fail {
                return Err(AppError::MetadataRead("mock failure".to_string()));
            }
            Ok(self.cover_source.clone())
        }

        fn read_audio_format(&self, _path: &std::path::Path) -> Result<AudioFormatInfo, AppError> {
            if self.fail {
                return Err(AppError::MetadataRead("mock failure".to_string()));
            }
            Ok(self.audio_format.clone())
        }

        fn read_all(
            &self,
            _path: &std::path::Path,
        ) -> Result<
            (
                TrackMetadata,
                Option<Duration>,
                CoverSource,
                AudioFormatInfo,
            ),
            AppError,
        > {
            if self.fail {
                return Err(AppError::MetadataRead("mock failure".to_string()));
            }
            Ok((
                self.metadata.clone(),
                self.duration,
                self.cover_source.clone(),
                self.audio_format.clone(),
            ))
        }
    }

    /// Canned [`CoverLoader`]: returns a configured image, `None`, or an
    /// injected `AppError::CoverLoad`.
    pub struct MockCoverLoader {
        pub result: Result<Option<CoverImage>, String>,
    }

    impl CoverLoader for MockCoverLoader {
        fn load_cover(&self, _source: &CoverSource) -> Result<Option<CoverImage>, AppError> {
            self.result.clone().map_err(AppError::CoverLoad)
        }
    }

    /// Recording [`MetadataWriter`]: successful writes are kept (path + edit)
    /// for assertions; when `fail` is set every write returns an
    /// `AppError::MetadataWrite`, simulating an unwritable file (permission
    /// denied, disk full, etc.).
    pub struct MockMetadataWriter {
        pub fail: bool,
        /// When set, the first `n` writes succeed and every later write
        /// fails — lets one test script per-call fates deterministically
        /// (the worker processes requests sequentially).
        pub fail_after_writes: Option<usize>,
        pub writes: Mutex<Vec<(PathBuf, TagEdit)>>,
    }

    impl Default for MockMetadataWriter {
        fn default() -> Self {
            Self::recording()
        }
    }

    impl MockMetadataWriter {
        /// A writer that records every write and never fails.
        #[must_use]
        pub fn recording() -> Self {
            Self {
                fail: false,
                fail_after_writes: None,
                writes: Mutex::new(Vec::new()),
            }
        }

        /// A writer that fails every write with a `MetadataWrite` error.
        #[must_use]
        pub fn failing() -> Self {
            Self {
                fail: true,
                fail_after_writes: None,
                writes: Mutex::new(Vec::new()),
            }
        }

        /// A writer whose first `n` writes succeed (and are recorded) and
        /// every subsequent write fails.
        #[must_use]
        pub fn failing_after(n: usize) -> Self {
            Self {
                fail: false,
                fail_after_writes: Some(n),
                writes: Mutex::new(Vec::new()),
            }
        }

        /// Snapshot of every successfully written (path, edit) pair.
        #[must_use]
        pub fn recorded(&self) -> Vec<(PathBuf, TagEdit)> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl MetadataWriter for MockMetadataWriter {
        fn write_metadata(&self, path: &Path, edit: &TagEdit) -> Result<(), AppError> {
            let spent = self.writes.lock().unwrap().len();
            if self.fail || self.fail_after_writes.is_some_and(|n| spent >= n) {
                return Err(AppError::MetadataWrite(format!(
                    "permission denied: {}",
                    path.display()
                )));
            }
            self.writes
                .lock()
                .unwrap()
                .push((path.to_path_buf(), edit.clone()));
            Ok(())
        }
    }

    /// Which [`SettingsStore`] mutation a mock recorded.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum SettingsCall {
        Scalars,
        LibraryPaths,
        WatchStates,
    }

    /// Recording [`SettingsStore`]: starts from defaults, applies every save
    /// to in-memory state (so hydration round-trips), records the call
    /// sequence, and can be switched to fail every mutation.
    pub struct MockSettingsStore {
        pub state: Settings,
        pub calls: Vec<SettingsCall>,
        pub fail: bool,
    }

    impl Default for MockSettingsStore {
        fn default() -> Self {
            Self {
                state: Settings {
                    scalars: riff::app::state::ScalarSettings::default(),
                    library_paths: Vec::new(),
                    watch_states: std::collections::HashMap::new(),
                },
                calls: Vec::new(),
                fail: false,
            }
        }
    }

    impl SettingsStore for MockSettingsStore {
        fn load_settings(&self) -> Result<Settings, AppError> {
            Ok(self.state.clone())
        }

        fn save_scalars(
            &mut self,
            scalars: &riff::app::state::ScalarSettings,
        ) -> Result<(), AppError> {
            if self.fail {
                return Err(AppError::InvalidOperation("mock settings failure".into()));
            }
            self.state.scalars = *scalars;
            self.calls.push(SettingsCall::Scalars);
            Ok(())
        }

        fn save_library_paths(&mut self, paths: &[std::path::PathBuf]) -> Result<(), AppError> {
            if self.fail {
                return Err(AppError::InvalidOperation("mock settings failure".into()));
            }
            self.state.library_paths = paths.to_vec();
            self.calls.push(SettingsCall::LibraryPaths);
            Ok(())
        }

        fn save_watch_states(
            &mut self,
            states: &std::collections::HashMap<std::path::PathBuf, riff::app::state::WatchState>,
        ) -> Result<(), AppError> {
            if self.fail {
                return Err(AppError::InvalidOperation("mock settings failure".into()));
            }
            self.state.watch_states.clone_from(states);
            self.calls.push(SettingsCall::WatchStates);
            Ok(())
        }
    }

    /// Recording [`LibraryMutationStore`] fake: `apply_tag_refresh` snapshots
    /// every Track it was handed (so tests can pin the refreshed metadata AND
    /// the untouched play history) and can be switched to fail, simulating a
    /// failed store commit. `record_track_played` snapshots every
    /// `(id, played_at)` pair (so playback-continuation tests can pin the
    /// committed plays); the remaining mutations are no-ops returning benign
    /// defaults.
    pub struct MockLibraryMutationStore {
        /// When set, `apply_tag_refresh` fails with an `InvalidOperation`
        /// error and records nothing.
        pub fail_tag_refresh: bool,
        refreshed: Mutex<Vec<Track>>,
        played: Mutex<Vec<(TrackId, std::time::SystemTime)>>,
    }

    impl Default for MockLibraryMutationStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockLibraryMutationStore {
        /// A mutation store that records every tag refresh and never fails.
        #[must_use]
        pub fn new() -> Self {
            Self {
                fail_tag_refresh: false,
                refreshed: Mutex::new(Vec::new()),
                played: Mutex::new(Vec::new()),
            }
        }

        /// A mutation store whose every tag refresh fails with an
        /// `InvalidOperation` error and records nothing, simulating a failed
        /// store commit.
        #[must_use]
        pub fn failing_refresh() -> Self {
            Self {
                fail_tag_refresh: true,
                refreshed: Mutex::new(Vec::new()),
                played: Mutex::new(Vec::new()),
            }
        }

        /// Snapshot of every Track passed to `apply_tag_refresh`, in call
        /// order. Failed commits record nothing.
        #[must_use]
        pub fn refreshed(&self) -> Vec<Track> {
            self.refreshed.lock().unwrap().clone()
        }

        /// Snapshot of every `(id, played_at)` passed to
        /// `record_track_played`, in call order.
        #[must_use]
        pub fn played(&self) -> Vec<(TrackId, std::time::SystemTime)> {
            self.played.lock().unwrap().clone()
        }
    }

    impl LibraryMutationStore for MockLibraryMutationStore {
        fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, AppError> {
            Ok(0)
        }

        fn record_track_played(
            &mut self,
            id: &TrackId,
            played_at: std::time::SystemTime,
        ) -> Result<bool, AppError> {
            self.played.lock().unwrap().push((id.clone(), played_at));
            Ok(true)
        }

        fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), AppError> {
            if self.fail_tag_refresh {
                return Err(AppError::InvalidOperation(
                    "mock tag refresh failure".to_string(),
                ));
            }
            self.refreshed.lock().unwrap().push(track.clone());
            Ok(())
        }

        fn remove_library_path(&mut self, _root: &Path) -> Result<usize, AppError> {
            Ok(0)
        }

        fn clear_library(&mut self) -> Result<usize, AppError> {
            Ok(0)
        }
    }

    /// Empty [`PlaylistStore`] fake standing in for the Playlists section
    /// of the Application Store in `SessionViews` facade tests that exercise
    /// Library-side views: every read serves an empty result and every
    /// mutation reports "nothing changed". The playlist projection's own
    /// behavior is pinned against real `SQLite` scratch stores in the app
    /// tests, not against this stub.
    #[derive(Default)]
    pub struct MockPlaylistStore {
        /// When set, every read fails with an `InvalidOperation` error.
        pub fail_loads: bool,
    }

    impl PlaylistStore for MockPlaylistStore {
        fn load_playlists(&self) -> Result<Vec<Playlist>, AppError> {
            if self.fail_loads {
                return Err(AppError::InvalidOperation("playlists boom".to_string()));
            }
            Ok(Vec::new())
        }

        fn load_playlist_entries(
            &self,
            _id: &PlaylistId,
        ) -> Result<Vec<riff::app::store::PlaylistEntry>, AppError> {
            if self.fail_loads {
                return Err(AppError::InvalidOperation("entries boom".to_string()));
            }
            Ok(Vec::new())
        }

        fn create_playlist(
            &mut self,
            _name: &str,
            _initial_tracks: &[TrackId],
        ) -> Result<PlaylistId, AppError> {
            Ok(PlaylistId::new("mock"))
        }

        fn rename_playlist(&mut self, _id: &PlaylistId, _new_name: &str) -> Result<bool, AppError> {
            Ok(false)
        }

        fn delete_playlist(&mut self, _id: &PlaylistId) -> Result<bool, AppError> {
            Ok(false)
        }

        fn add_playlist_entry(
            &mut self,
            _id: &PlaylistId,
            _track: &TrackId,
        ) -> Result<bool, AppError> {
            Ok(false)
        }

        fn remove_playlist_entries(
            &mut self,
            _id: &PlaylistId,
            _track: &TrackId,
        ) -> Result<bool, AppError> {
            Ok(false)
        }

        fn reorder_playlist_entries(
            &mut self,
            _id: &PlaylistId,
            _ordered: &[TrackId],
        ) -> Result<bool, AppError> {
            Ok(false)
        }
    }

    /// Which [`LibraryQueryStore`] query a [`MockLibraryQueryStore`]
    /// recorded. Arguments are kept so assertions can pin both call counts
    /// and the exact query shapes the Session Views facade issues.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LibraryQueryCall {
        GetTrack(TrackId),
        TracksWindow(usize, usize),
        TrackCount,
        AllTrackIds,
        SearchWindow(usize, usize),
        SearchCount,
        AllArtists,
        ArtistAlbums(String),
        AlbumTracks(String, String),
        FolderHasAudio(PathBuf),
        FolderHasSearchMatch(PathBuf, String),
        TrackIdsInFolderTree(PathBuf),
        TracksInFolder(PathBuf),
        SubdirsWithAudio(PathBuf),
        SmartPlaylist(SmartPlaylistKind, usize),
    }

    /// Which [`LibraryQueryStore`] query fails while listed in
    /// [`MockLibraryQueryStore::failing`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailingQuery {
        GetTrack,
        TracksWindow,
        AllArtists,
        SmartPlaylist,
    }

    /// Canned [`LibraryQueryStore`] fake standing in for the Application
    /// Store's Library collection in `SessionViews` facade tests: every
    /// query serves its configured field, records one
    /// [`LibraryQueryCall`], and fails on demand while listed in
    /// `failing`.
    ///
    /// Configuration happens before wiring; recordings accumulate behind an
    /// internal mutex because the port takes `&self`. Tests that must hand
    /// ownership to the facade keep a shared handle (see
    /// `SharedMock` in the app tests) or lock through the `Arc`.
    pub struct MockLibraryQueryStore {
        // --- canned answers -------------------------------------------------
        /// `get_track` answers keyed by id.
        pub library: std::collections::HashMap<TrackId, Track>,
        /// Rows served by `tracks_window`, in canonical order.
        pub flat: Vec<Track>,
        /// Rows served by `search_window`, in canonical order.
        pub search: Vec<Track>,
        /// Queries that `search_count`/`search_window` treat as matching.
        /// An empty list matches every query (permissive default); once
        /// populated, only listed queries return the canned rows/count.
        pub matching_searches: Vec<String>,
        /// Artists served by `all_artists`.
        pub artists: Vec<Artist>,
        /// Albums served by `artist_albums`.
        pub albums: Vec<Album>,
        /// Tracks served by `album_tracks`.
        pub album_tracks: Vec<Track>,
        /// Tracks served by `smart_playlist`.
        pub smart: Vec<Track>,
        /// Answer served by `folder_has_audio`.
        pub folder_has_audio: bool,
        /// Answer served by `folder_has_search_match`.
        pub folder_search_match: bool,
        /// Ids served by `track_ids_in_folder_tree`.
        pub folder_tree_ids: Vec<TrackId>,
        /// Tracks served by `tracks_in_folder`.
        pub folder_direct_tracks: Vec<Track>,
        /// Children served by `subdirs_with_audio`.
        pub folder_children: Vec<PathBuf>,

        // --- failure injection -------------------------------------------------
        /// Queries that fail while listed here.
        pub failing: Vec<FailingQuery>,

        // --- recordings ---------------------------------------------------------
        /// Recorded queries, in call order. Internal: read through the
        /// accessors (`calls`, `window_calls`, `get_track_calls`,
        /// `count_of`) instead of touching this directly; it is `pub` only
        /// so test constructors can use `..Default::default()`.
        pub calls: Mutex<Vec<LibraryQueryCall>>,
    }

    impl Default for MockLibraryQueryStore {
        fn default() -> Self {
            Self {
                library: std::collections::HashMap::new(),
                flat: Vec::new(),
                search: Vec::new(),
                matching_searches: Vec::new(),
                artists: Vec::new(),
                albums: Vec::new(),
                album_tracks: Vec::new(),
                smart: Vec::new(),
                folder_has_audio: true,
                folder_search_match: true,
                folder_tree_ids: Vec::new(),
                folder_direct_tracks: Vec::new(),
                folder_children: Vec::new(),
                failing: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl MockLibraryQueryStore {
        /// Snapshot of every recorded query, in call order.
        #[must_use]
        pub fn calls(&self) -> Vec<LibraryQueryCall> {
            self.calls.lock().unwrap().clone()
        }

        /// Every bounded-window fetch as `(offset, limit)` pairs — flat and
        /// search windows alike — in call order.
        #[must_use]
        pub fn window_calls(&self) -> Vec<(usize, usize)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::TracksWindow(offset, limit)
                    | LibraryQueryCall::SearchWindow(offset, limit) => Some((*offset, *limit)),
                    _ => None,
                })
                .collect()
        }

        /// Every `get_track` id, in call order.
        #[must_use]
        pub fn get_track_calls(&self) -> Vec<TrackId> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::GetTrack(id) => Some(id.clone()),
                    _ => None,
                })
                .collect()
        }

        /// How often each single-call query kind fired (counts only).
        #[must_use]
        pub fn count_of(&self, call: &LibraryQueryCall) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|recorded| recorded == &call)
                .count()
        }

        fn record(&self, call: LibraryQueryCall) {
            self.calls.lock().unwrap().push(call);
        }

        /// Whether `query` counts as a match against the canned search rows.
        fn search_matches(&self, query: &str) -> bool {
            self.matching_searches.is_empty() || self.matching_searches.iter().any(|q| q == query)
        }
    }

    impl LibraryQueryStore for MockLibraryQueryStore {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, AppError> {
            self.record(LibraryQueryCall::GetTrack(id.clone()));
            if self.failing.contains(&FailingQuery::GetTrack) {
                return Err(AppError::InvalidOperation("store boom".to_string()));
            }
            Ok(self.library.get(id).cloned())
        }

        fn tracks_window(&self, offset: usize, limit: usize) -> Result<Vec<Track>, AppError> {
            self.record(LibraryQueryCall::TracksWindow(offset, limit));
            if self.failing.contains(&FailingQuery::TracksWindow) {
                return Err(AppError::InvalidOperation("loader boom".to_string()));
            }
            Ok(self.flat.iter().skip(offset).take(limit).cloned().collect())
        }

        fn track_count(&self) -> Result<usize, AppError> {
            self.record(LibraryQueryCall::TrackCount);
            Ok(self.flat.len())
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, AppError> {
            self.record(LibraryQueryCall::AllTrackIds);
            Ok(self.flat.iter().map(|t| t.id.clone()).collect())
        }

        fn search_window(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Track>, AppError> {
            self.record(LibraryQueryCall::SearchWindow(offset, limit));
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self
                .search
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn search_count(&self, query: &str) -> Result<usize, AppError> {
            self.record(LibraryQueryCall::SearchCount);
            if !self.search_matches(query) {
                return Ok(0);
            }
            Ok(self.search.len())
        }

        fn all_artists(&self) -> Result<Vec<Artist>, AppError> {
            self.record(LibraryQueryCall::AllArtists);
            if self.failing.contains(&FailingQuery::AllArtists) {
                return Err(AppError::InvalidOperation("artists boom".to_string()));
            }
            Ok(self.artists.clone())
        }

        fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, AppError> {
            self.record(LibraryQueryCall::ArtistAlbums(artist.to_string()));
            Ok(self.albums.clone())
        }

        fn album_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
        ) -> Result<Vec<Track>, AppError> {
            self.record(LibraryQueryCall::AlbumTracks(
                album_artist.to_string(),
                album_title.to_string(),
            ));
            Ok(self.album_tracks.clone())
        }

        fn folder_has_audio(&self, folder: &Path) -> Result<bool, AppError> {
            self.record(LibraryQueryCall::FolderHasAudio(folder.to_path_buf()));
            Ok(self.folder_has_audio)
        }

        fn folder_has_search_match(&self, folder: &Path, query: &str) -> Result<bool, AppError> {
            self.record(LibraryQueryCall::FolderHasSearchMatch(
                folder.to_path_buf(),
                query.to_string(),
            ));
            Ok(self.folder_search_match)
        }

        fn track_ids_in_folder_tree(&self, folder: &Path) -> Result<Vec<TrackId>, AppError> {
            self.record(LibraryQueryCall::TrackIdsInFolderTree(folder.to_path_buf()));
            Ok(self.folder_tree_ids.clone())
        }

        fn tracks_in_folder(&self, folder: &Path) -> Result<Vec<Track>, AppError> {
            self.record(LibraryQueryCall::TracksInFolder(folder.to_path_buf()));
            Ok(self.folder_direct_tracks.clone())
        }

        fn subdirs_with_audio(&self, folder: &Path) -> Result<Vec<PathBuf>, AppError> {
            self.record(LibraryQueryCall::SubdirsWithAudio(folder.to_path_buf()));
            Ok(self.folder_children.clone())
        }

        fn smart_playlist(
            &self,
            kind: SmartPlaylistKind,
            limit: usize,
        ) -> Result<Vec<Track>, AppError> {
            self.record(LibraryQueryCall::SmartPlaylist(kind, limit));
            if self.failing.contains(&FailingQuery::SmartPlaylist) {
                return Err(AppError::InvalidOperation(
                    "smart playlist boom".to_string(),
                ));
            }
            Ok(self.smart.clone())
        }
    }
}

// Integration test helper functions
pub mod integration_helpers {
    use crate::app::state::AppState;
    use std::sync::{Arc, Mutex};

    /// Create a test `AppState` with some pre-populated data
    pub fn create_test_app_state() -> Arc<Mutex<AppState>> {
        let state = AppState::new();
        Arc::new(Mutex::new(state))
    }
}
