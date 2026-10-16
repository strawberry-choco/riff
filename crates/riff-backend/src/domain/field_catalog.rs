//! Re-export of the Matchable Field catalog.
//!
//! `riff_backend::domain::field_catalog` is the one source of what each
//! Matchable Field permits, how it is displayed and which value widget the
//! editor draws; it is re-exported from `riff_persistence::field_catalog` at
//! the same qualified path `SmartPlaylistKind` reaches the frontend by, so a
//! consumer never imports the persistence crate directly.

pub use riff_persistence::field_catalog::*;
