//! Store adapters: the `SQLite` Application Store implementing
//! riff-persistence's store ports.

pub mod sqlite;

pub use sqlite::{SqliteStore, default_store_path};
