pub mod errors;
pub mod events;
pub mod library_paths;
pub mod preferences;
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
    projection::{
        BrowsingProjection, FolderProjection, GenreProjection, HitListProjection, HitProjection,
        PlaylistProjection, SmartPlaylistsProjection, TrackListProjection, WindowedListProjection,
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
    transport::{ChannelTransport, Transport, clamp_seek},
};

/// The single mutex-poison recovery policy, defined in `riff-persistence`
/// beside the shared `std`-only utilities that crate owns. Re-exported at
/// this historical path (`riff_backend::app::MutexExt`) for the frontend and
/// the integration suite, which import the trait from here.
pub use riff_persistence::sync::MutexExt;
