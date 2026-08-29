//! Re-export of the persistence playlist entities.
//!
//! `riff_backend::domain::Playlist`, `PlaylistId` are re-exported from
//! `riff_persistence::playlist` so existing qualified paths compile.

pub use riff_persistence::playlist::*;
