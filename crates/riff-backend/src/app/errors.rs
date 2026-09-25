//! Re-export of the persistence boundary error at the backend's historical
//! `riff_backend::app::errors::StoreError` path.
//!
//! The capability errors are not re-exported here: `LibraryError` and
//! `PlaybackError` each have exactly one definition, in the capability that
//! raises it (`riff_library::app::errors` and `riff_playback::app::errors`).

/// Re-export of the persistence boundary error.
pub use riff_persistence::errors::StoreError;
