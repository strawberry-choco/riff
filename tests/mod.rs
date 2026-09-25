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
//! - `app_tests.rs`: Tests for application logic like `PlaybackSession` /
//!   `LibrarySession`, the Session Projections, and scan-side Track construction.
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
pub mod integration_tests;
pub mod ui_tests;

// --- Library re-exports ---------------------------------------------------
//
// Bring the library modules into the test crate root so qualified paths such as
// `crate::domain::TrackMetadata`, `crate::app::state::PlaybackSession` and
// `crate::app::scan_service::ScanService` resolve from inside the test modules.
pub use riff_backend::app;
pub use riff_backend::domain;
pub use riff_gui::ui;
pub use riff_infra;

// Prelude of bare names used inside the test bodies through `use super::*`.
// Kept explicit (rather than glob re-exports) to avoid name collisions.
pub use riff_backend::app::MutexExt;
pub use riff_backend::app::gapless::{
    GaplessConditions, QueueConditions, duration_from_frames, elapsed_from_samples,
    formats_gapless_compatible, frames_from_duration, is_gapless_eligible, pre_buffer_cap,
    repeat_one_handoff_eligible, samples_from_duration,
};
pub use riff_backend::app::state::{
    LibrarySession, LibraryStatus, PlaybackQueue, PlaybackSession, WatchState, replaygain_factor,
};
pub use riff_backend::app::store::SortDirection;
pub use riff_backend::app::transport::clamp_seek;
pub use riff_backend::domain::{
    Album, Artist, PlaybackCommand, PlaybackPosition, PlaybackState, PlaybackUpdate, Playlist,
    PlaylistId, RepeatMode, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};
pub use riff_gui::ui::app::{format_duration, lru_insert};
pub use riff_gui::ui::selection::TagDraft;
pub use riff_gui::ui::settings::{expand_tilde, suggest_directories};
pub use riff_infra::audio::{CpalAudioOutput, SymphoniaDecoder};
pub use riff_infra::filesystem::{AudioFileScanner, FilesystemWatcher};
pub use riff_infra::media::metadata_reader::parse_replaygain_gain;
pub use riff_infra::media::{ImageCoverLoader, LoftyMetadataReader, LoftyMetadataWriter};
pub use riff_persistence::store::ScanOptions;

// Standard-library names referenced unqualified in some suites.
pub use std::path::PathBuf;
pub use std::sync::atomic::AtomicBool;
pub use std::sync::{Arc, Mutex};

/// An `Arc<AtomicBool>` stop flag no test ever sets.
///
/// The service workers take their stop flag as a constructor argument, and
/// every test that builds a worker by hand ends it the way the code did
/// before the runtime gained a lifecycle: by dropping the front-end handle.
/// Those tests therefore need a flag that stays false for the whole test.
/// Tests that do exercise shutdown take their flags from the
/// `RuntimeLifecycle` the Composition Root returns instead of from here.
#[must_use]
pub fn inert_stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

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
            favorite: false,
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
            favorite: false,
            search_text: String::new(),
        }
    }
}

/// Shared fakes for exercising the port boundaries without real audio
/// hardware or files on disk. Each fake implements the ONE port trait the
/// production code consumes — the playback capability's audio ports and the
/// library capability's media ports — so a test's substitute and the real
/// adapter answer to the same interface.
///
/// The audio fakes record through an `Arc<Mutex<..>>` they hand out clones
/// of: the engine takes ownership of one clone and the test keeps another, so
/// the counters stay readable while and after it runs.
pub mod mocks {
    use riff_backend::app::state::PlaybackSession;
    use riff_backend::app::store::{
        LibraryMutationStore, LibraryQueryStore, PlaylistStore, Settings, SettingsStore,
        SortDirection,
    };
    use riff_backend::app::traits::{
        CoverLoader, DecodedCover, MetadataReader, MetadataWriter, RequestedSize, TagEdit,
    };
    use riff_backend::app::transport::clamp_seek;
    use riff_backend::domain::{
        Album, Artist, CoverSource, GenreCount, Playlist, PlaylistId, RepeatMode,
        SmartPlaylistKind, Track, TrackId, TrackMetadata,
    };
    use riff_library::app::errors::LibraryError;
    use riff_persistence::errors::StoreError;
    use riff_playback::app::errors::PlaybackError;
    use riff_playback::infra::ports::{AudioDecoder, AudioFormatInfo, AudioOutput};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Scripted [`AudioDecoder`]: `init` returns the configured format,
    /// `next_frames` drains a queue of sample batches and then reports EOF,
    /// and every `seek` is recorded and resets the stream to the start of the
    /// script. Cloning hands out another handle to the same recording state.
    #[derive(Clone)]
    pub struct MockAudioDecoder {
        /// The path this decoder instance last opened, as the engine sees it.
        source: PathBuf,
        state: Arc<Mutex<DecoderState>>,
    }

    struct DecoderState {
        format: AudioFormatInfo,
        duration: Option<Duration>,
        /// Full sample script; `queue` is refilled from this on `init` and
        /// reset on `seek`.
        scripted: Vec<Vec<f32>>,
        queue: Vec<Vec<f32>>,
        seeks: Vec<Duration>,
        opened: Vec<PathBuf>,
    }

    impl MockAudioDecoder {
        pub fn new(format: AudioFormatInfo) -> Self {
            Self {
                source: PathBuf::new(),
                state: Arc::new(Mutex::new(DecoderState {
                    format,
                    duration: None,
                    scripted: Vec::new(),
                    queue: Vec::new(),
                    seeks: Vec::new(),
                    opened: Vec::new(),
                })),
            }
        }

        /// Script the sample batches that `next_frames` will yield, in order.
        #[must_use]
        pub fn with_batches(self, batches: Vec<Vec<f32>>) -> Self {
            self.state.lock().unwrap().scripted = batches;
            self
        }

        /// Set what the port's `duration` answers with.
        #[must_use]
        pub fn with_duration(self, duration: Option<Duration>) -> Self {
            self.state.lock().unwrap().duration = duration;
            self
        }

        /// Every path this decoder has opened, in order.
        #[must_use]
        pub fn opened(&self) -> Vec<PathBuf> {
            self.state.lock().unwrap().opened.clone()
        }

        /// Every position this decoder has been seeked to, in order.
        #[must_use]
        pub fn seeks(&self) -> Vec<Duration> {
            self.state.lock().unwrap().seeks.clone()
        }
    }

    impl AudioDecoder for MockAudioDecoder {
        fn source_path(&self) -> &Path {
            &self.source
        }

        fn init(&mut self, path: &Path) -> Result<AudioFormatInfo, PlaybackError> {
            self.source = path.to_path_buf();
            let mut state = self.state.lock().unwrap();
            state.opened.push(path.to_path_buf());
            state.queue = state.scripted.clone();
            Ok(state.format.clone())
        }

        fn next_frames(&mut self, buf: &mut [f32]) -> Option<usize> {
            let mut state = self.state.lock().unwrap();
            // `None` here is the scripted EOF.
            let batch = state.queue.first_mut()?;
            // Fill as much of `buf` as the current scripted batch holds; a
            // batch larger than `buf` keeps its remainder queued for the next
            // call, mirroring how the real decoder spills oversized packets
            // into `pending_samples` (nothing is ever dropped).
            let n = buf.len().min(batch.len());
            buf[..n].copy_from_slice(&batch[..n]);
            batch.drain(..n);
            if batch.is_empty() {
                state.queue.remove(0);
            }
            Some(n)
        }

        fn seek(&mut self, position: Duration) -> Duration {
            let mut state = self.state.lock().unwrap();
            state.seeks.push(position);
            state.queue = state.scripted.clone();
            position
        }

        fn duration(&self) -> Option<Duration> {
            self.state.lock().unwrap().duration
        }
    }

    /// Recording [`AudioOutput`]: tracks every invocation the engine makes on
    /// the port. Cloning hands out another handle to the same recording state.
    #[derive(Clone)]
    pub struct MockAudioOutput {
        state: Arc<Mutex<OutputState>>,
    }

    struct OutputState {
        /// The `(sample rate, channels)` of each stream the engine started.
        started: Vec<(u32, u16)>,
        stop_count: usize,
        written: Vec<Vec<f32>>,
        volumes: Vec<f32>,
    }

    impl MockAudioOutput {
        pub fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(OutputState {
                    started: Vec::new(),
                    stop_count: 0,
                    written: Vec::new(),
                    volumes: Vec::new(),
                })),
            }
        }

        /// The format each started stream was built with, in order.
        #[must_use]
        pub fn started(&self) -> Vec<(u32, u16)> {
            self.state.lock().unwrap().started.clone()
        }

        /// How many streams were started in total.
        #[must_use]
        pub fn start_count(&self) -> usize {
            self.state.lock().unwrap().started.len()
        }

        #[must_use]
        pub fn stop_count(&self) -> usize {
            self.state.lock().unwrap().stop_count
        }

        /// Every sample batch written, in order.
        #[must_use]
        pub fn written(&self) -> Vec<Vec<f32>> {
            self.state.lock().unwrap().written.clone()
        }

        /// Every volume set, in order.
        #[must_use]
        pub fn volumes(&self) -> Vec<f32> {
            self.state.lock().unwrap().volumes.clone()
        }
    }

    impl Default for MockAudioOutput {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AudioOutput for MockAudioOutput {
        fn start(&mut self, format: AudioFormatInfo) -> Result<(), PlaybackError> {
            self.state
                .lock()
                .unwrap()
                .started
                .push((format.sample_rate, format.channels));
            Ok(())
        }

        fn write(&mut self, samples: &[f32]) -> usize {
            let mut state = self.state.lock().unwrap();
            state.written.push(samples.to_vec());
            samples.len()
        }

        fn stop(&mut self) {
            self.state.lock().unwrap().stop_count += 1;
        }

        fn set_volume(&mut self, volume: f32) {
            self.state.lock().unwrap().volumes.push(volume);
        }

        fn latency(&self) -> u32 {
            0
        }
    }

    /// Canned [`MetadataReader`] over the library capability's reader port:
    /// returns configured values, or an injected `LibraryError::MetadataRead`
    /// when `fail` is set.
    pub struct MockMetadataReader {
        pub fail: bool,
        pub metadata: TrackMetadata,
        pub duration: Option<Duration>,
        pub cover_source: CoverSource,
        pub audio_format: riff_library::app::traits::AudioFormatInfo,
    }

    impl Default for MockMetadataReader {
        fn default() -> Self {
            Self {
                fail: false,
                metadata: TrackMetadata::default(),
                duration: Some(Duration::from_secs(90)),
                cover_source: CoverSource::None,
                audio_format: riff_library::app::traits::AudioFormatInfo {
                    sample_rate: 44_100,
                    channels: 2,
                },
            }
        }
    }

    impl MetadataReader for MockMetadataReader {
        fn read_cover_source(&self, _path: &Path) -> Result<CoverSource, LibraryError> {
            if self.fail {
                return Err(LibraryError::MetadataRead("mock failure".to_string()));
            }
            Ok(self.cover_source.clone())
        }

        fn read_all(
            &self,
            _path: &Path,
        ) -> Result<
            (
                TrackMetadata,
                Duration,
                CoverSource,
                riff_library::app::traits::AudioFormatInfo,
            ),
            LibraryError,
        > {
            if self.fail {
                return Err(LibraryError::MetadataRead("mock failure".to_string()));
            }
            Ok((
                self.metadata.clone(),
                self.duration.unwrap_or_default(),
                self.cover_source.clone(),
                self.audio_format.clone(),
            ))
        }
    }

    /// Recording [`FilesystemWatch`](riff_backend::app::traits::FilesystemWatch)
    /// — the second adapter at the watch seam that ADR 0006 anticipates. It
    /// answers which roots the watcher was asked to follow and which it was
    /// asked to drop, so a test can prove a retired Library Path stops being
    /// watched without touching a platform event stream.
    #[derive(Clone, Default)]
    pub struct MockFilesystemWatch {
        calls: Arc<Mutex<Vec<(WatchAction, PathBuf)>>>,
    }

    /// What the watch port was asked to do with a path.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum WatchAction {
        Watch,
        Unwatch,
    }

    impl MockFilesystemWatch {
        /// The roots asked to be followed, in order.
        #[must_use]
        pub fn watched(&self) -> Vec<PathBuf> {
            self.paths_for(WatchAction::Watch)
        }

        /// The roots asked to be dropped, in order.
        #[must_use]
        pub fn unwatched(&self) -> Vec<PathBuf> {
            self.paths_for(WatchAction::Unwatch)
        }

        fn paths_for(&self, action: WatchAction) -> Vec<PathBuf> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(recorded, _)| *recorded == action)
                .map(|(_, path)| path.clone())
                .collect()
        }

        fn record(&self, action: WatchAction, path: &Path) {
            self.calls
                .lock()
                .unwrap()
                .push((action, path.to_path_buf()));
        }
    }

    impl riff_backend::app::traits::FilesystemWatch for MockFilesystemWatch {
        fn watch(&mut self, path: &Path) -> Result<(), LibraryError> {
            self.record(WatchAction::Watch, path);
            Ok(())
        }

        fn unwatch(&mut self, path: &Path) -> Result<(), LibraryError> {
            self.record(WatchAction::Unwatch, path);
            Ok(())
        }
    }

    /// Canned [`CoverLoader`]: returns a configured decoded cover, `None`, or
    /// an injected `LibraryError::CoverLoad`. The port belongs to the library
    /// slice, so it answers in that slice's error type.
    pub struct MockCoverLoader {
        pub result: Result<Option<DecodedCover>, String>,
    }

    impl CoverLoader for MockCoverLoader {
        fn load_cover(
            &self,
            _source: &CoverSource,
            _size: RequestedSize,
        ) -> Result<Option<DecodedCover>, LibraryError> {
            self.result.clone().map_err(LibraryError::CoverLoad)
        }
    }

    /// Recording [`MetadataWriter`]: successful writes are kept (path + edit)
    /// for assertions; when `fail` is set every write returns an
    /// `LibraryError::MetadataWrite`, simulating an unwritable file (permission
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
        fn write_tags(&self, path: &Path, edit: &TagEdit) -> Result<(), LibraryError> {
            let spent = self.writes.lock().unwrap().len();
            if self.fail || self.fail_after_writes.is_some_and(|n| spent >= n) {
                return Err(LibraryError::MetadataWrite(format!(
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
    ///
    /// The `calls` log is owned, which is what the suites that call
    /// `Preferences` themselves want. A test that hands the mock to something
    /// which owns it — the app shell — cannot read `calls` back afterwards, so
    /// it builds the mock with [`Self::with_shared_calls`] and reads the log
    /// through the handle it keeps.
    pub struct MockSettingsStore {
        pub state: Settings,
        pub calls: Vec<SettingsCall>,
        pub fail: bool,
        /// A second copy of every recorded call, for the owned-elsewhere case.
        /// `None` in the owned-and-inspected style.
        pub shared_calls: Option<Arc<Mutex<Vec<SettingsCall>>>>,
    }

    impl Default for MockSettingsStore {
        fn default() -> Self {
            Self {
                state: Settings {
                    scalars: riff_backend::app::state::ScalarSettings::default(),
                    library_paths: Vec::new(),
                    watch_states: std::collections::HashMap::new(),
                },
                calls: Vec::new(),
                fail: false,
                shared_calls: None,
            }
        }
    }

    impl MockSettingsStore {
        /// A mock that also records into `shared`, which the caller keeps and
        /// reads after the owner holding the mock has dropped.
        pub fn with_shared_calls(shared: Arc<Mutex<Vec<SettingsCall>>>) -> Self {
            Self {
                shared_calls: Some(shared),
                ..Self::default()
            }
        }

        /// Record one mutation, into the owned log and the shared one alike.
        fn record(&mut self, call: SettingsCall) {
            if let Some(shared) = &self.shared_calls {
                shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(call.clone());
            }
            self.calls.push(call);
        }
    }

    impl SettingsStore for MockSettingsStore {
        fn load_settings(&self) -> Result<Settings, StoreError> {
            Ok(self.state.clone())
        }

        fn save_scalars(
            &mut self,
            scalars: &riff_backend::app::state::ScalarSettings,
        ) -> Result<(), StoreError> {
            if self.fail {
                return Err(StoreError::InvalidOperation("mock settings failure".into()));
            }
            self.state.scalars = scalars.clone();
            self.record(SettingsCall::Scalars);
            Ok(())
        }

        fn save_library_paths(&mut self, paths: &[std::path::PathBuf]) -> Result<(), StoreError> {
            if self.fail {
                return Err(StoreError::InvalidOperation("mock settings failure".into()));
            }
            self.state.library_paths = paths.to_vec();
            self.record(SettingsCall::LibraryPaths);
            Ok(())
        }

        fn save_watch_states(
            &mut self,
            states: &std::collections::HashMap<
                std::path::PathBuf,
                riff_backend::app::state::WatchState,
            >,
        ) -> Result<(), StoreError> {
            if self.fail {
                return Err(StoreError::InvalidOperation("mock settings failure".into()));
            }
            self.state.watch_states.clone_from(states);
            self.record(SettingsCall::WatchStates);
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
        favorites: Mutex<Vec<(TrackId, bool)>>,
        /// Every root passed to `remove_library_path`, in call order.
        removed: Mutex<Vec<PathBuf>>,
        /// How many `clear_library` calls landed.
        clears: Mutex<usize>,
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
                favorites: Mutex::new(Vec::new()),
                removed: Mutex::new(Vec::new()),
                clears: Mutex::new(0),
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
                favorites: Mutex::new(Vec::new()),
                removed: Mutex::new(Vec::new()),
                clears: Mutex::new(0),
            }
        }

        /// Snapshot of every Track passed to `apply_tag_refresh`, in call
        /// order. Failed commits record nothing.
        #[must_use]
        pub fn refreshed(&self) -> Vec<Track> {
            self.refreshed.lock().unwrap().clone()
        }

        /// Snapshot of every root passed to `remove_library_path`, in call
        /// order.
        #[must_use]
        pub fn removals(&self) -> Vec<PathBuf> {
            self.removed.lock().unwrap().clone()
        }

        /// How many `clear_library` calls landed.
        #[must_use]
        pub fn clear_count(&self) -> usize {
            *self.clears.lock().unwrap()
        }

        /// Snapshot of every `(id, played_at)` passed to
        /// `record_track_played`, in call order.
        #[must_use]
        pub fn played(&self) -> Vec<(TrackId, std::time::SystemTime)> {
            self.played.lock().unwrap().clone()
        }

        /// Snapshot of every `(id, favorite)` passed to
        /// `set_track_favorite`, in call order.
        #[must_use]
        pub fn favorites(&self) -> Vec<(TrackId, bool)> {
            self.favorites.lock().unwrap().clone()
        }
    }

    impl LibraryMutationStore for MockLibraryMutationStore {
        fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn stamp_metadata_version(&mut self, _version: u32) -> Result<(), StoreError> {
            Ok(())
        }

        fn record_track_played(
            &mut self,
            id: &TrackId,
            played_at: std::time::SystemTime,
        ) -> Result<bool, StoreError> {
            self.played.lock().unwrap().push((id.clone(), played_at));
            Ok(true)
        }

        fn set_track_favorite(&mut self, id: &TrackId, favorite: bool) -> Result<bool, StoreError> {
            self.favorites.lock().unwrap().push((id.clone(), favorite));
            Ok(true)
        }

        fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError> {
            if self.fail_tag_refresh {
                return Err(StoreError::InvalidOperation(
                    "mock tag refresh failure".to_string(),
                ));
            }
            self.refreshed.lock().unwrap().push(track.clone());
            Ok(())
        }

        fn remove_library_path(&mut self, root: &Path) -> Result<usize, StoreError> {
            self.removed.lock().unwrap().push(root.to_path_buf());
            Ok(0)
        }

        fn clear_library(&mut self) -> Result<usize, StoreError> {
            *self.clears.lock().unwrap() += 1;
            Ok(0)
        }

        fn record_full_scan_completed(
            &mut self,
            _summary: riff_backend::app::store::FullScanSummary,
        ) -> Result<(), StoreError> {
            Ok(())
        }
    }

    /// Empty [`PlaylistStore`] fake standing in for the Playlists section
    /// of the Application Store in `SessionViews` seam tests that exercise
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
        fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError> {
            if self.fail_loads {
                return Err(StoreError::InvalidOperation("playlists boom".to_string()));
            }
            Ok(Vec::new())
        }

        fn load_playlist_entries(
            &self,
            _id: &PlaylistId,
        ) -> Result<Vec<riff_backend::app::store::PlaylistEntry>, StoreError> {
            if self.fail_loads {
                return Err(StoreError::InvalidOperation("entries boom".to_string()));
            }
            Ok(Vec::new())
        }

        fn create_playlist(
            &mut self,
            _name: &str,
            _initial_tracks: &[TrackId],
        ) -> Result<PlaylistId, StoreError> {
            Ok(PlaylistId::new("mock"))
        }

        fn rename_playlist(
            &mut self,
            _id: &PlaylistId,
            _new_name: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn delete_playlist(&mut self, _id: &PlaylistId) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn add_playlist_entry(
            &mut self,
            _id: &PlaylistId,
            _track: &TrackId,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn remove_playlist_entries(
            &mut self,
            _id: &PlaylistId,
            _track: &TrackId,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn reorder_playlist_entries(
            &mut self,
            _id: &PlaylistId,
            _ordered: &[TrackId],
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
    }

    /// One recorded [`Transport`](riff_backend::app::transport::Transport) intent,
    /// in issue order. Seek/volume intents carry the ADAPTED values (clamped
    /// target, effective volume), mirroring what
    /// [`ChannelTransport`](riff_backend::app::transport::ChannelTransport) would
    /// put on the wire.
    #[derive(Debug, Clone, PartialEq)]
    pub enum TransportIntent {
        Play(TrackId),
        PlayMany(TrackId, Vec<TrackId>),
        PlayNext(TrackId),
        AddToQueue(TrackId),
        Pause,
        Resume,
        Next,
        Previous,
        Stop,
        /// The clamped seek target.
        Seek(Duration),
        /// The volume command sent to the engine — the effective volume
        /// (zero while muted), per the port's `set_volume` contract.
        ApplyVolume(f32),
        PlayPause,
        ToggleShuffle(bool),
        ToggleRepeat(RepeatMode),
    }

    /// Recording [`Transport`](riff_backend::app::transport::Transport) fake: keeps
    /// every issued intent behind an internal mutex so UI-layer tests can
    /// assert on playback intents instead of raw channel bytes. The
    /// state-coupled methods reuse the production clamp/volume math, exactly
    /// like the real adapter.
    pub struct MockTransport {
        intents: Mutex<Vec<TransportIntent>>,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self {
                intents: Mutex::new(Vec::new()),
            }
        }

        /// Snapshot of every recorded intent, in issue order.
        #[must_use]
        pub fn recorded(&self) -> Vec<TransportIntent> {
            self.intents.lock().unwrap().clone()
        }

        fn record(&self, intent: TransportIntent) {
            self.intents.lock().unwrap().push(intent);
        }
    }

    impl Default for MockTransport {
        fn default() -> Self {
            Self::new()
        }
    }

    impl riff_backend::app::transport::Transport for MockTransport {
        fn play(&self, track: TrackId) {
            self.record(TransportIntent::Play(track));
        }

        fn play_many(&self, first: TrackId, rest: Vec<TrackId>) {
            self.record(TransportIntent::PlayMany(first, rest));
        }

        fn play_next(&self, track: TrackId) {
            self.record(TransportIntent::PlayNext(track));
        }

        fn add_to_queue(&self, track: TrackId) {
            self.record(TransportIntent::AddToQueue(track));
        }

        fn pause(&self) {
            self.record(TransportIntent::Pause);
        }

        fn resume(&self) {
            self.record(TransportIntent::Resume);
        }

        fn next(&self) {
            self.record(TransportIntent::Next);
        }

        fn previous(&self) {
            self.record(TransportIntent::Previous);
        }

        fn stop(&self) {
            self.record(TransportIntent::Stop);
        }

        fn seek(&self, session: &PlaybackSession, secs: f32) {
            self.record(TransportIntent::Seek(clamp_seek(
                secs,
                session.current_position.total,
            )));
        }

        fn set_volume(&self, session: &mut PlaybackSession, vol: f32) {
            session.current_volume = vol.clamp(0.0, 1.0);
            self.record(TransportIntent::ApplyVolume(session.effective_volume()));
        }

        fn toggle_mute(&self, session: &mut PlaybackSession) {
            session.muted = !session.muted;
            self.record(TransportIntent::ApplyVolume(session.effective_volume()));
        }

        fn toggle_shuffle(&self, session: &mut PlaybackSession) {
            session.queue.set_shuffle(!session.queue.shuffle);
            self.record(TransportIntent::ToggleShuffle(session.queue.shuffle));
        }

        fn toggle_repeat(&self, session: &mut PlaybackSession) {
            session.queue.toggle_repeat();
            self.record(TransportIntent::ToggleRepeat(session.queue.repeat));
        }

        fn play_pause(&self, _session: &PlaybackSession) {
            self.record(TransportIntent::PlayPause);
        }
    }

    /// Which [`LibraryQueryStore`] query a [`MockLibraryQueryStore`]
    /// recorded. Arguments are kept so assertions can pin both call counts
    /// and the exact query shapes the Session Views seam issues. One
    /// Listing Page read records exactly one entry: its total and its window
    /// come from a single store read, so the recording names that read with
    /// its own arguments (ADR 0002's 2026-09-22 amendment).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LibraryQueryCall {
        GetTrack(TrackId),
        TracksPage(usize, usize),
        AllTrackIds,
        SearchPage(String, usize, usize),
        AllArtists,
        ArtistAlbums(String),
        AlbumTracks(String, String),
        FolderHasAudio(PathBuf),
        FolderHasSearchMatch(PathBuf, String),
        TrackIdsInFolderTree(PathBuf),
        TracksInFolder(PathBuf),
        SubdirsWithAudio(PathBuf),
        SmartPlaylist(SmartPlaylistKind, usize),
        LibraryCounts,
        SmartListCounts,
        FolderTrackCount(PathBuf),
        LastFullScan,
        GenreCounts,
        ArtistsInGenre(String),
        ArtistAlbumsInGenre(String, String),
        AlbumTracksInGenre(String, String, String),
        HitAlbumsPage(String, usize, usize),
        HitArtistsPage(String, usize, usize),
        AlbumHitTracks(String, String),
        AlbumIsNameHit(String, String),
        HitAlbumsInGenre(String, usize, usize),
        HitArtistsInGenre(String, usize, usize),
        AlbumHitTracksInGenre(String, String, String),
        HitGenreCounts,
        ArtistsPage(SortDirection, usize, usize),
        AlbumsPage(SortDirection, usize, usize),
        GenresPage(SortDirection, usize, usize),
        ArtistsInGenrePage(String, SortDirection, usize, usize),
        ArtistAlbumsInGenrePage(String, String, SortDirection, usize, usize),
    }

    /// Which [`LibraryQueryStore`] query fails while listed in
    /// [`MockLibraryQueryStore::failing`].
    ///
    /// A Listing Page read is one store read, but it still composes the two
    /// halves it replaced (a total and a window), so a page read is gated by
    /// the `*Count` and `*Window` switches it grew from: either one makes
    /// that single page read return `Err`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailingQuery {
        GetTrack,
        TracksWindow,
        AllArtists,
        SmartPlaylist,
        TrackIdsInFolderTree,
        GenreCounts,
        LibraryCounts,
        HitAlbums,
        HitAlbumsCount,
        HitArtists,
        HitArtistsCount,
        AlbumHitTracks,
        AlbumIsNameHit,
        HitAlbumsInGenre,
        HitAlbumsInGenreCount,
        HitArtistsInGenre,
        HitArtistsInGenreCount,
        AlbumHitTracksInGenre,
        HitGenreCounts,
        ArtistsWindow,
        ArtistsCount,
        AlbumsWindow,
        AlbumsCount,
        GenresWindow,
        GenresCount,
        ArtistsInGenreWindow,
        ArtistsInGenreCount,
        ArtistAlbumsInGenreWindow,
        ArtistAlbumsInGenreCount,
    }

    /// Canned [`LibraryQueryStore`] fake standing in for the Application
    /// Store's Library collection in `SessionViews` seam tests: every
    /// query serves its configured field, records one
    /// [`LibraryQueryCall`], and fails on demand while listed in
    /// `failing`.
    ///
    /// Configuration happens before wiring; recordings accumulate behind an
    /// internal mutex because the port takes `&self`. Tests that must hand
    /// ownership to the seam keep a shared handle (see
    /// `SharedMock` in the app tests) or lock through the `Arc`.
    pub struct MockLibraryQueryStore {
        // --- canned answers -------------------------------------------------
        /// `get_track` answers keyed by id.
        pub library: std::collections::HashMap<TrackId, Track>,
        /// Rows served by `tracks_page`, in canonical order.
        pub flat: Vec<Track>,
        /// Rows served by `search_page`, in canonical order.
        pub search: Vec<Track>,
        /// Queries that `search_page` treats as matching.
        /// An empty list matches every query (permissive default); once
        /// populated, only listed queries return the canned rows/count.
        pub matching_searches: Vec<String>,
        /// Artists served by `all_artists` and by `artists_page`.
        pub artists: Vec<Artist>,
        /// Albums served by `artist_albums`.
        pub albums: Vec<Album>,
        /// Tracks served by `album_tracks`.
        pub album_tracks: Vec<Track>,
        /// Tracks served by `smart_playlist`.
        pub smart: Vec<Track>,
        /// Answer served by `library_counts`.
        pub library_counts: riff_backend::app::store::LibraryCounts,
        /// Answer served by `smart_list_counts`.
        pub smart_list_counts: Vec<(SmartPlaylistKind, usize)>,
        /// Answers served by `folder_track_count`, keyed by folder.
        pub folder_track_counts: std::collections::HashMap<PathBuf, usize>,
        /// Answer served by `last_full_scan`.
        pub last_full_scan: Option<std::time::SystemTime>,
        /// Rows served by `genre_counts`.
        pub genre_counts: Vec<GenreCount>,
        /// Artists served by `artists_in_genre` and `artists_in_genre_page`
        /// for any genre.
        pub genre_artists: Vec<Artist>,
        /// Albums served by `artist_albums_in_genre` and
        /// `artist_albums_in_genre_page` for any artist/genre.
        pub genre_albums: Vec<Album>,
        /// Tracks served by `album_tracks_in_genre` for any album/genre.
        pub genre_album_tracks: Vec<Track>,
        /// Albums served by `hit_albums_page`, in canonical order.
        pub hit_albums: Vec<Album>,
        /// Artists served by `hit_artists_page`, name-ascending.
        pub hit_artists: Vec<Artist>,
        /// Tracks served by `album_hit_tracks` for any album.
        pub album_hit_tracks: Vec<Track>,
        /// Albums whose `album_is_name_hit` answers `true` (keys "artist - title").
        pub album_name_hits: Vec<String>,
        /// Albums served by `hit_albums_in_genre`, in canonical order.
        pub hit_albums_in_genre: Vec<Album>,
        /// Artists served by `hit_artists_in_genre`, name-ascending.
        pub hit_artists_in_genre: Vec<Artist>,
        /// Tracks served by `album_hit_tracks_in_genre` for any album/genre.
        pub album_hit_tracks_in_genre: Vec<Track>,
        /// Rows served by `hit_genre_counts`.
        pub hit_genre_counts: Vec<GenreCount>,
        /// Albums served by `albums_page` (flat browsing order); the page
        /// slices this with the direction applied.
        pub paged_albums: Vec<Album>,
        /// Rows served by `genres_page`; the page slices this with the
        /// direction applied.
        pub paged_genres: Vec<GenreCount>,
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
                library_counts: riff_backend::app::store::LibraryCounts::default(),
                smart_list_counts: Vec::new(),
                folder_track_counts: std::collections::HashMap::new(),
                last_full_scan: None,
                genre_counts: Vec::new(),
                genre_artists: Vec::new(),
                genre_albums: Vec::new(),
                genre_album_tracks: Vec::new(),
                hit_albums: Vec::new(),
                hit_artists: Vec::new(),
                album_hit_tracks: Vec::new(),
                album_name_hits: Vec::new(),
                hit_albums_in_genre: Vec::new(),
                hit_artists_in_genre: Vec::new(),
                album_hit_tracks_in_genre: Vec::new(),
                hit_genre_counts: Vec::new(),
                paged_albums: Vec::new(),
                paged_genres: Vec::new(),
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

        /// Every bounded-window Listing Page read as its `(offset, limit)`
        /// pair — flat, search, and paged-browse listings alike — in call
        /// order. One entry per store read: the pair names the window that
        /// single page fetch served.
        #[must_use]
        pub fn window_calls(&self) -> Vec<(usize, usize)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::TracksPage(offset, limit)
                    | LibraryQueryCall::SearchPage(_, offset, limit)
                    | LibraryQueryCall::ArtistsPage(_, offset, limit)
                    | LibraryQueryCall::AlbumsPage(_, offset, limit)
                    | LibraryQueryCall::GenresPage(_, offset, limit)
                    | LibraryQueryCall::ArtistsInGenrePage(_, _, offset, limit)
                    | LibraryQueryCall::ArtistAlbumsInGenrePage(_, _, _, offset, limit) => {
                        Some((*offset, *limit))
                    }
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

        /// Slice `rows` into one bounded window with the direction applied —
        /// the shared backing of the paged browse Listing Page reads
        /// (artists, albums, genres, and the genre drill-downs all serve
        /// their canned list).
        fn window_rows<T: Clone>(
            &self,
            rows: &[T],
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Vec<T> {
            let mut rows = rows.to_vec();
            if direction == SortDirection::Descending {
                rows.reverse();
            }
            rows.into_iter().skip(offset).take(limit).collect()
        }
    }

    impl LibraryQueryStore for MockLibraryQueryStore {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            self.record(LibraryQueryCall::GetTrack(id.clone()));
            if self.failing.contains(&FailingQuery::GetTrack) {
                return Err(StoreError::InvalidOperation("store boom".to_string()));
            }
            Ok(self.library.get(id).cloned())
        }

        fn metadata_version(&self) -> Result<u32, StoreError> {
            Ok(riff_persistence::track::METADATA_VERSION)
        }

        fn tracks_page(
            &self,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.record(LibraryQueryCall::TracksPage(offset, limit));
            if self.failing.contains(&FailingQuery::TracksWindow) {
                return Err(StoreError::InvalidOperation("loader boom".to_string()));
            }
            let total = self.flat.len();
            let rows = self.flat.iter().skip(offset).take(limit).cloned().collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn library_counts(&self) -> Result<riff_backend::app::store::LibraryCounts, StoreError> {
            self.record(LibraryQueryCall::LibraryCounts);
            if self.failing.contains(&FailingQuery::LibraryCounts) {
                return Err(StoreError::InvalidOperation("counts boom".to_string()));
            }
            Ok(self.library_counts)
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            self.record(LibraryQueryCall::AllTrackIds);
            Ok(self.flat.iter().map(|t| t.id.clone()).collect())
        }

        fn search_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.record(LibraryQueryCall::SearchPage(
                query.to_string(),
                offset,
                limit,
            ));
            if !self.search_matches(query) {
                return Ok(riff_persistence::store::Page::new(0, Vec::new()));
            }
            let total = self.search.len();
            let rows = self
                .search
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            self.record(LibraryQueryCall::AllArtists);
            if self.failing.contains(&FailingQuery::AllArtists) {
                return Err(StoreError::InvalidOperation("artists boom".to_string()));
            }
            Ok(self.artists.clone())
        }

        fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError> {
            self.record(LibraryQueryCall::ArtistAlbums(artist.to_string()));
            Ok(self.albums.clone())
        }

        fn album_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumTracks(
                album_artist.to_string(),
                album_title.to_string(),
            ));
            Ok(self.album_tracks.clone())
        }

        fn folder_has_audio(&self, folder: &Path) -> Result<bool, StoreError> {
            self.record(LibraryQueryCall::FolderHasAudio(folder.to_path_buf()));
            Ok(self.folder_has_audio)
        }

        fn folder_has_search_match(&self, folder: &Path, query: &str) -> Result<bool, StoreError> {
            self.record(LibraryQueryCall::FolderHasSearchMatch(
                folder.to_path_buf(),
                query.to_string(),
            ));
            Ok(self.folder_search_match)
        }

        fn track_ids_in_folder_tree(&self, folder: &Path) -> Result<Vec<TrackId>, StoreError> {
            self.record(LibraryQueryCall::TrackIdsInFolderTree(folder.to_path_buf()));
            if self.failing.contains(&FailingQuery::TrackIdsInFolderTree) {
                return Err(StoreError::InvalidOperation("folder tree boom".to_string()));
            }
            Ok(self.folder_tree_ids.clone())
        }

        fn tracks_in_folder(&self, folder: &Path) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::TracksInFolder(folder.to_path_buf()));
            Ok(self.folder_direct_tracks.clone())
        }

        fn folder_track_count(&self, folder: &Path) -> Result<usize, StoreError> {
            self.record(LibraryQueryCall::FolderTrackCount(folder.to_path_buf()));
            Ok(self.folder_track_counts.get(folder).copied().unwrap_or(0))
        }

        fn last_full_scan(
            &self,
        ) -> Result<Option<riff_backend::app::store::FullScanSummary>, StoreError> {
            self.record(LibraryQueryCall::LastFullScan);
            Ok(self
                .last_full_scan
                .map(|at| riff_backend::app::store::FullScanSummary {
                    at,
                    files: 0,
                    errors: 0,
                }))
        }

        fn subdirs_with_audio(&self, folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
            self.record(LibraryQueryCall::SubdirsWithAudio(folder.to_path_buf()));
            Ok(self.folder_children.clone())
        }

        fn smart_playlist(
            &self,
            kind: SmartPlaylistKind,
            limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::SmartPlaylist(kind, limit));
            if self.failing.contains(&FailingQuery::SmartPlaylist) {
                return Err(StoreError::InvalidOperation(
                    "smart playlist boom".to_string(),
                ));
            }
            Ok(self.smart.clone())
        }

        fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError> {
            self.record(LibraryQueryCall::SmartListCounts);
            Ok(self.smart_list_counts.clone())
        }

        fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError> {
            self.record(LibraryQueryCall::GenreCounts);
            if self.failing.contains(&FailingQuery::GenreCounts) {
                return Err(StoreError::InvalidOperation(
                    "genre counts boom".to_string(),
                ));
            }
            Ok(self.genre_counts.clone())
        }

        fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError> {
            self.record(LibraryQueryCall::ArtistsInGenre(genre.to_string()));
            Ok(self.genre_artists.clone())
        }

        fn artist_albums_in_genre(
            &self,
            artist: &str,
            genre: &str,
        ) -> Result<Vec<Album>, StoreError> {
            self.record(LibraryQueryCall::ArtistAlbumsInGenre(
                artist.to_string(),
                genre.to_string(),
            ));
            Ok(self.genre_albums.clone())
        }

        fn album_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumTracksInGenre(
                album_artist.to_string(),
                album_title.to_string(),
                genre.to_string(),
            ));
            Ok(self.genre_album_tracks.clone())
        }

        fn hit_albums_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.record(LibraryQueryCall::HitAlbumsPage(
                query.to_string(),
                offset,
                limit,
            ));
            if self.failing.contains(&FailingQuery::HitAlbumsCount) {
                return Err(StoreError::InvalidOperation(
                    "hit albums count boom".to_string(),
                ));
            }
            if self.failing.contains(&FailingQuery::HitAlbums) {
                return Err(StoreError::InvalidOperation("hit albums boom".to_string()));
            }
            if !self.search_matches(query) {
                return Ok(riff_persistence::store::Page::new(0, Vec::new()));
            }
            let total = self.hit_albums.len();
            let rows = self
                .hit_albums
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn hit_artists_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.record(LibraryQueryCall::HitArtistsPage(
                query.to_string(),
                offset,
                limit,
            ));
            if self.failing.contains(&FailingQuery::HitArtistsCount) {
                return Err(StoreError::InvalidOperation(
                    "hit artists count boom".to_string(),
                ));
            }
            if self.failing.contains(&FailingQuery::HitArtists) {
                return Err(StoreError::InvalidOperation("hit artists boom".to_string()));
            }
            if !self.search_matches(query) {
                return Ok(riff_persistence::store::Page::new(0, Vec::new()));
            }
            let total = self.hit_artists.len();
            let rows = self
                .hit_artists
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn album_hit_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumHitTracks(
                album_artist.to_string(),
                album_title.to_string(),
            ));
            if self.failing.contains(&FailingQuery::AlbumHitTracks) {
                return Err(StoreError::InvalidOperation(
                    "album hit tracks boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self.album_hit_tracks.clone())
        }

        fn album_is_name_hit(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<bool, StoreError> {
            self.record(LibraryQueryCall::AlbumIsNameHit(
                album_artist.to_string(),
                album_title.to_string(),
            ));
            if self.failing.contains(&FailingQuery::AlbumIsNameHit) {
                return Err(StoreError::InvalidOperation(
                    "album name hit boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(false);
            }
            Ok(self
                .album_name_hits
                .contains(&format!("{album_artist} - {album_title}")))
        }

        fn hit_albums_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Album>, StoreError> {
            self.record(LibraryQueryCall::HitAlbumsInGenre(
                genre.to_string(),
                offset,
                limit,
            ));
            if self.failing.contains(&FailingQuery::HitAlbumsInGenre) {
                return Err(StoreError::InvalidOperation(
                    "hit albums in genre boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self
                .hit_albums_in_genre
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn hit_artists_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Artist>, StoreError> {
            self.record(LibraryQueryCall::HitArtistsInGenre(
                genre.to_string(),
                offset,
                limit,
            ));
            if self.failing.contains(&FailingQuery::HitArtistsInGenre) {
                return Err(StoreError::InvalidOperation(
                    "hit artists in genre boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self
                .hit_artists_in_genre
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn album_hit_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumHitTracksInGenre(
                album_artist.to_string(),
                album_title.to_string(),
                genre.to_string(),
            ));
            if self.failing.contains(&FailingQuery::AlbumHitTracksInGenre) {
                return Err(StoreError::InvalidOperation(
                    "album hit tracks in genre boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self.album_hit_tracks_in_genre.clone())
        }

        fn hit_genre_counts(&self, query: &str) -> Result<Vec<GenreCount>, StoreError> {
            self.record(LibraryQueryCall::HitGenreCounts);
            if self.failing.contains(&FailingQuery::HitGenreCounts) {
                return Err(StoreError::InvalidOperation(
                    "hit genre counts boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self.hit_genre_counts.clone())
        }

        fn artists_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.record(LibraryQueryCall::ArtistsPage(direction, offset, limit));
            if self.failing.contains(&FailingQuery::ArtistsCount) {
                return Err(StoreError::InvalidOperation(
                    "artists count boom".to_string(),
                ));
            }
            if self.failing.contains(&FailingQuery::ArtistsWindow) {
                return Err(StoreError::InvalidOperation(
                    "artists window boom".to_string(),
                ));
            }
            let total = self.artists.len();
            let rows = self.window_rows(&self.artists, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn albums_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.record(LibraryQueryCall::AlbumsPage(direction, offset, limit));
            if self.failing.contains(&FailingQuery::AlbumsCount) {
                return Err(StoreError::InvalidOperation(
                    "albums count boom".to_string(),
                ));
            }
            if self.failing.contains(&FailingQuery::AlbumsWindow) {
                return Err(StoreError::InvalidOperation(
                    "albums window boom".to_string(),
                ));
            }
            let total = self.paged_albums.len();
            let rows = self.window_rows(&self.paged_albums, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn genres_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<GenreCount>, StoreError> {
            self.record(LibraryQueryCall::GenresPage(direction, offset, limit));
            if self.failing.contains(&FailingQuery::GenresCount) {
                return Err(StoreError::InvalidOperation(
                    "genres count boom".to_string(),
                ));
            }
            if self.failing.contains(&FailingQuery::GenresWindow) {
                return Err(StoreError::InvalidOperation(
                    "genres window boom".to_string(),
                ));
            }
            let total = self.paged_genres.len();
            let rows = self.window_rows(&self.paged_genres, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn artists_in_genre_page(
            &self,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.record(LibraryQueryCall::ArtistsInGenrePage(
                genre.to_string(),
                direction,
                offset,
                limit,
            ));
            if self.failing.contains(&FailingQuery::ArtistsInGenreCount) {
                return Err(StoreError::InvalidOperation(
                    "genre artists count boom".to_string(),
                ));
            }
            if self.failing.contains(&FailingQuery::ArtistsInGenreWindow) {
                return Err(StoreError::InvalidOperation(
                    "genre artists window boom".to_string(),
                ));
            }
            let total = self.genre_artists.len();
            let rows = self.window_rows(&self.genre_artists, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn artist_albums_in_genre_page(
            &self,
            artist: &str,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.record(LibraryQueryCall::ArtistAlbumsInGenrePage(
                artist.to_string(),
                genre.to_string(),
                direction,
                offset,
                limit,
            ));
            if self
                .failing
                .contains(&FailingQuery::ArtistAlbumsInGenreCount)
            {
                return Err(StoreError::InvalidOperation(
                    "genre album count boom".to_string(),
                ));
            }
            if self
                .failing
                .contains(&FailingQuery::ArtistAlbumsInGenreWindow)
            {
                return Err(StoreError::InvalidOperation(
                    "genre album window boom".to_string(),
                ));
            }
            let total = self.genre_albums.len();
            let rows = self.window_rows(&self.genre_albums, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }
    }

    /// Scripted [`Scans`] front end for UI tests that drive `RiffApp`: the
    /// test queues the outcomes the app will poll, and the mock records the
    /// paths the app asked for. `request`/`cancel` stay fire-and-forget;
    /// `poll` hands back the queue and empties it.
    ///
    /// `Clone` shares the queue, so a test keeps a handle after boxing another
    /// clone into the app. `is_scanning` is always false: the shell never
    /// gates rendering on it, so modelling an in-flight scan would be
    /// modelling something no test here can observe.
    #[derive(Clone, Default)]
    pub struct MockScans {
        /// Outcomes the next `poll` returns, oldest first.
        pub queued: Arc<Mutex<Vec<riff_library::app::scan_service::ScanOutcome>>>,
        /// Every path `request` was called with, in call order.
        pub requested: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl MockScans {
        /// Queue one outcome for the app's next `poll`.
        pub fn queue(&self, outcome: riff_library::app::scan_service::ScanOutcome) {
            lock_cell(&self.queued).push(outcome);
        }

        /// The paths `request` was called with so far.
        pub fn requested_paths(&self) -> Vec<PathBuf> {
            lock_cell(&self.requested).clone()
        }
    }

    /// Lock a mock's shared cell, tolerating poisoning: one failing test must
    /// not cascade into every later test through a poisoned mock.
    pub(crate) fn lock_cell<T>(cell: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        cell.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// No-op [`TagEdits`] for UI tests that exercise `RiffApp` but never submit
    /// a tag edit. `submit` discards; `poll` always returns `None`.
    #[derive(Default)]
    pub struct MockTagEdits;

    /// No-op [`Covers`] for UI tests that exercise `RiffApp` but never request
    /// a cover. `request` discards; `poll` always returns an empty Vec.
    #[derive(Default)]
    pub struct MockCovers;
}

// --- Minimal trait impls for the no-op UI test mocks -----------------------

impl riff_library::app::scan_service::Scans for crate::mocks::MockScans {
    fn request(&self, path: std::path::PathBuf) {
        crate::mocks::lock_cell(&self.requested).push(path);
    }

    fn cancel(&self) {}

    fn poll(&self) -> Vec<riff_library::app::scan_service::ScanOutcome> {
        std::mem::take(&mut *crate::mocks::lock_cell(&self.queued))
    }

    fn is_scanning(&self, _path: &std::path::Path) -> bool {
        false
    }
}

impl riff_backend::app::tag_edit_service::TagEdits for crate::mocks::MockTagEdits {
    fn submit(&self, _request: riff_backend::app::tag_edit_service::TagEditRequest) {}
    fn poll(&self) -> Option<riff_backend::app::tag_edit_service::TagEditOutcome> {
        None
    }
}

impl riff_library::app::cover_service::Covers for crate::mocks::MockCovers {
    fn request(
        &self,
        _track_id: riff_backend::domain::TrackId,
        _path: std::path::PathBuf,
        _size: riff_library::app::traits::RequestedSize,
    ) {
    }
    fn request_folder(
        &self,
        _folder: &std::path::Path,
        _size: riff_library::app::traits::RequestedSize,
    ) {
    }
    fn poll(
        &self,
    ) -> Vec<(
        riff_backend::domain::TrackId,
        riff_library::app::traits::RequestedSize,
        Option<riff_library::app::traits::DecodedCover>,
    )> {
        Vec::new()
    }
}

// Integration test helper functions
pub mod integration_helpers {
    use crate::app::state::{LibrarySession, PlaybackSession};
    use std::sync::{Arc, Mutex};

    /// Create paired test sessions: a `PlaybackSession` and a `LibrarySession`,
    /// each in their own `Arc<Mutex<>>`. Callers unpack with
    /// `let (playback, library) = create_test_sessions();`.
    #[allow(clippy::type_complexity)]
    pub fn create_test_sessions() -> (Arc<Mutex<PlaybackSession>>, Arc<Mutex<LibrarySession>>) {
        (
            Arc::new(Mutex::new(PlaybackSession::default())),
            Arc::new(Mutex::new(LibrarySession::default())),
        )
    }
}
