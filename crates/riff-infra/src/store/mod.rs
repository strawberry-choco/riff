//! Store adapters: the `SQLite` Application Store implementing
//! riff-persistence's store ports, plus the store's read side narrowed to the
//! Audio Engine's two queries (`StorePlaybackLibrary`, serving
//! riff-playback's
//! [`PlaybackLibrary`](riff_playback::infra::ports::PlaybackLibrary) port).

pub mod field_columns;
pub mod playback_library;
pub mod smart_playlist_tree;
pub mod sqlite;

pub use field_columns::{FIELD_COLUMNS, FieldColumn, field_column, is_genre_set, picker_column};
pub use playback_library::StorePlaybackLibrary;
pub use sqlite::{SqliteStore, default_store_path};
