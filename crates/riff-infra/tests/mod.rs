//! riff-infra — adapter test suite.
//!
//! Single integration-test crate rooted at `tests/mod.rs` (`autotests =
//! false`, one `[[test]]` target), mirroring the workspace-root suite's
//! layout. It hosts the adapter tests that moved with the crate
//! (backend-crate-split issue 07): real `SQLite` against tempfile scratch
//! databases, real lofty tag round-trips, the real decoder/watcher/scanner
//! construction surface. Assertions are unchanged from their pre-extraction
//! form; only import paths were rewired.
//!
//! Run:
//!   cargo test -p riff-infra
//!   cargo test -p riff-infra `store_tests`

pub mod adapter_tests;
pub mod store_tests;

// --- Library re-exports -----------------------------------------------------
//
// Bring the persisted entities and store ports into the test crate root so
// qualified paths such as `crate::Track` resolve from inside the test
// modules (via `use super::*`), mirroring the workspace-root suite's prelude.
pub use riff_infra::audio::SymphoniaDecoder;
pub use riff_infra::filesystem::{AudioFileScanner, FilesystemWatcher};
pub use riff_infra::media::{
    ImageCoverLoader, LoftyMetadataReader, LoftyMetadataWriter, parse_replaygain_gain,
};
pub use riff_infra::store::SqliteStore;
pub use riff_library::app::errors::LibraryError;
pub use riff_library::app::traits::TagEdit;
pub use riff_persistence::errors::StoreError;
pub use riff_persistence::playlist::{Playlist, PlaylistId};
pub use riff_persistence::store::{
    LOST_GEMS_THRESHOLD, LibraryMutationStore, LibraryQueryStore, PlaylistEntry, PlaylistStore,
    ScalarSettings, ScanOptions, Settings, SettingsStore, StoreChanged, StoreGeneration,
    StoreMigrations, WatchState,
};
pub use riff_persistence::track::{
    Album, Artist, CoverSource, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};

// Standard-library names referenced unqualified in some suites.
pub use std::collections::HashMap;
pub use std::path::PathBuf;
pub use std::sync::Arc;
pub use std::sync::atomic::AtomicBool;
pub use std::time::Duration;
