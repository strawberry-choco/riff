pub mod cover_resolver;
pub mod cover_service;
pub mod errors;
pub mod facade;
pub mod playlist_manager;
mod projection;
pub mod scan;
pub mod scan_service;
pub mod state;
pub mod store;
pub mod tag_edit_service;
pub mod traits;
pub mod views;
pub mod watcher_manager;

/// Re-export playback capability from riff-playback.
pub use riff_playback::app::{
    errors::PlaybackError,
    gapless::{GaplessConditions, QueueConditions, duration_from_frames, elapsed_from_samples, formats_gapless_compatible, frames_from_duration, is_gapless_eligible, pre_buffer_cap, repeat_one_handoff_eligible, samples_from_duration},
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