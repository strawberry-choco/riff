//! Store adapters: the `SQLite` Application Store implementing
//! riff-persistence's store ports, plus the store's read side narrowed to the
//! Audio Engine's two queries (`StorePlaybackLibrary`, serving
//! riff-playback's
//! [`PlaybackLibrary`](riff_playback::infra::ports::PlaybackLibrary) port).
//!
//! `rule_sql` is the Custom Smart Playlist query compiler: the one place a
//! Matchable Field id names a `tracks` column, and the one place a Rule tree
//! becomes one SQL predicate over `tracks`.

mod rule_sql;

pub mod playback_library;
pub mod sqlite;

pub use playback_library::StorePlaybackLibrary;
pub use sqlite::{SqliteStore, default_store_path};
