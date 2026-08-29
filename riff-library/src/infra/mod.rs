//! Infrastructure port traits for library.
//!
//! This module contains the port traits that infrastructure implements:
//! - `MetadataReader` — reading audio metadata
//! - `MetadataWriter` — writing audio metadata
//! - `CoverLoader` — loading cover art
//! - `FilesystemWatch` — filesystem watching

pub mod ports;

pub use ports::{
    AudioFormatInfo, CoverImage, CoverLoader, FilesystemWatch, MetadataReader, MetadataWriter,
    TagEdit,
};
