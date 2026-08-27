pub mod audio_engine;
pub mod cover_resolver;
pub mod cover_service;
pub mod errors;
pub mod facade;
pub mod gapless;
pub mod playback_coordinator;
pub mod playlist_manager;
mod projection;
pub mod scan;
pub mod scan_service;
pub mod state;
pub mod store;
pub mod tag_edit_service;
pub mod traits;
pub mod transport;
pub mod views;
pub mod watcher_manager;

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
