//! Re-export of the Custom Smart Playlist rule vocabulary.
//!
//! `riff_backend::domain::Rule`, `RuleBox`, `RootBox`, `MatchableField`,
//! `Comparison`, `Semantic`, `Combine`, `RuleChild`, `RuleValue` and
//! `CivilDateTime` are re-exported from `riff_persistence::rule` so the
//! frontend and the integration tests reach them at the same qualified path
//! `SmartPlaylistKind` already uses.

pub use riff_persistence::rule::*;
