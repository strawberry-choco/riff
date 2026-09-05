pub mod errors;
pub mod facade;
pub mod state;
pub mod store;
pub mod tag_edit_service;
pub mod traits;
pub mod views;
pub mod watcher_manager;

/// Re-export library capability from riff-library.
pub use riff_library::app::{
    cover_resolver::CoverResolver,
    cover_service::{COVER_CACHE_CAP, CoverService, CoverWorker, Covers, lru_insert},
    errors::LibraryError,
    playlist_manager::{PlaylistManager, PlaylistManagerWorker, Playlists},
    projection::{
        BrowsingProjection, FolderProjection, GenreProjection, PlaylistProjection,
        SmartPlaylistsProjection, TrackListProjection,
    },
    scan::build_tracks,
    scan_service::{SCAN_BATCH_SIZE, ScanOutcome, ScanService, ScanWorker, Scans},
};

/// Module-qualified re-exports of the library and playback surfaces, so the
/// frontend and integration tests keep their historical
/// `riff_backend::app::<module>::` import paths across the crate split.
pub use riff_library::app::{cover_service, playlist_manager, projection, scan, scan_service};
pub use riff_playback::app::{gapless, playback_coordinator, transport};
pub use riff_playback::infra::audio_engine;
pub use riff_playback::infra::ports::{AudioDecoder, AudioOutput, DecoderFactory};

/// Re-export playback capability from riff-playback.
pub use riff_playback::app::{
    errors::PlaybackError,
    gapless::{
        GaplessConditions, QueueConditions, duration_from_frames, elapsed_from_samples,
        formats_gapless_compatible, frames_from_duration, is_gapless_eligible, pre_buffer_cap,
        repeat_one_handoff_eligible, samples_from_duration,
    },
    playback_coordinator::PlaybackCoordinator,
    projection::PlaybackProjection,
    state::PlaybackSession,
    transport::{ChannelTransport, FacadeTransport, Transport, clamp_seek},
};

use std::sync::{Mutex, MutexGuard};

/// Extension trait for graceful `Mutex` access.
///
/// `std::sync::Mutex` *poisons* itself when a thread panics while holding the
/// lock, causing every subsequent `lock().unwrap()` to panic and bring down the
/// whole application. Real-time audio apps degrade more gracefully by recovering
/// the (possibly inconsistent) inner data instead of crashing.
pub trait MutexExt<T> {
    /// Acquire the guard, recovering from a poisoned lock rather than panicking.
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
