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
//! - `app_tests.rs`: Tests for application logic like `AppState`, `LibraryManager`, etc.
//! - `infra_tests.rs`: Tests for infrastructure components like audio decoders, metadata readers, etc.
//! - `ui_tests.rs`: Tests for UI-related functionality like settings storage, etc.
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
pub use riff::app::gapless::{
    duration_from_frames, elapsed_from_samples, formats_gapless_compatible, frames_from_duration,
    is_gapless_eligible, pre_buffer_cap, samples_from_duration, GaplessConditions, QueueConditions,
};
pub use riff::app::library_manager::LibraryManager;
pub use riff::app::state::{replaygain_factor, AppState, LibraryStatus, WatchState};
pub use riff::app::MutexExt;
pub use riff::domain::{
    Album, Artist, PlaybackCommand, PlaybackPosition, PlaybackQueue, PlaybackState, PlaybackUpdate,
    Playlist, PlaylistId, RepeatMode, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};
pub use riff::infra::metadata_reader::parse_replaygain_gain;
pub use riff::infra::{
    AudioFileScanner, CpalAudioOutput, FilesystemWatcher, ImageCoverLoader, LoftyMetadataReader,
    LoftyMetadataWriter, SymphoniaDecoder,
};
pub use riff::ui::app::{clamp_seek, format_duration, high_contrast_visuals, TagEditState};
pub use riff::ui::settings::{
    expand_tilde, load_advanced_mode, load_high_contrast, load_library_paths, load_replaygain,
    load_volume, load_watch_states, restore_from_backup_if_corrupted, save_advanced_mode,
    save_high_contrast, save_library_paths, save_replaygain, save_volume, save_watch_states,
    suggest_directories,
};

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
        }
    }
}

/// Shared trait-based mocks for exercising the port boundaries
/// (`src/app/traits.rs`) without real audio hardware or files on disk.
/// These are intentionally reusable: later suites (e.g. gapless-playback
/// tests) build on the same scripted decoder/output behavior.
pub mod mocks {
    use riff::app::errors::AppError;
    use riff::app::traits::{
        AudioDecoder, AudioFormatInfo, AudioOutput, CoverImage, CoverLoader, MetadataReader,
        MetadataWriter, TagEdit,
    };
    use riff::domain::{CoverSource, TrackMetadata};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::Duration;

    /// Scripted [`AudioDecoder`]: `open` returns a configured format (or an
    /// injected error), `next_frames` drains a queue of sample batches and
    /// then reports EOF, and every `seek` is recorded and resets the stream
    /// to the start of the script.
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

        fn next_frames(&mut self, _samples: usize) -> Result<Option<Vec<f32>>, AppError> {
            if let Some(ref msg) = self.decode_error {
                return Err(AppError::Decode(msg.clone()));
            }
            if self.queue.is_empty() {
                Ok(None)
            } else {
                Ok(Some(self.queue.remove(0)))
            }
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
                buffer: Vec::new(),
            }
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
                writes: Mutex::new(Vec::new()),
            }
        }

        /// A writer that fails every write with a `MetadataWrite` error.
        #[must_use]
        pub fn failing() -> Self {
            Self {
                fail: true,
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
            if self.fail {
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

    /// Create a mock library with some test tracks
    pub fn create_mock_library() -> crate::app::library_manager::LibraryManager {
        let mut library = crate::app::library_manager::LibraryManager::new();

        // Add some test tracks
        let track1 = super::test_utils::create_test_track_with_metadata(
            "track1.mp3",
            "music/artist1/album1/track1.mp3",
            "Artist 1",
            "Track 1",
            "Album 1",
        );

        let track2 = super::test_utils::create_test_track_with_metadata(
            "track2.mp3",
            "music/artist1/album1/track2.mp3",
            "Artist 1",
            "Track 2",
            "Album 1",
        );

        library.add_track(track1);
        library.add_track(track2);

        library
    }
}
