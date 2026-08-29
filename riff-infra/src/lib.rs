//! riff-infra — the adapter crate.
//!
//! Every port implementation and every native/external dependency lives here:
//! the `SQLite` Application Store, the symphonia decoder, the cpal output, the
//! lofty metadata reader/writer, the image cover loader, the walkdir scanner,
//! and the notify watcher. It implements the ports defined in `riff-persistence`,
//! `riff-library`, and `riff-playback`; the dependency arrow points at the
//! slices, never the reverse.
//!
//! Internal module seams (store / audio / media / filesystem) are preserved so
//! the crate can be split further later without redesign if compile times ever
//! demand it.

pub mod audio;
pub mod filesystem;
pub mod media;
pub mod store;

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
