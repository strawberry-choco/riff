//! Re-export of the persistence track entities.
//!
//! `riff_backend::domain::Track`, `TrackId`, `TrackMetadata`, `Album`, `Artist`,
//! `CoverSource`, `SmartPlaylistKind` are re-exported from `riff_persistence::track`
/// so existing qualified paths (`riff_backend::domain::Track`) continue to compile.
pub use riff_persistence::track::*;
