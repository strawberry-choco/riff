//! Port traits for library infrastructure.
//!
//! The single definitions live in [`crate::infra::ports`]; this module
//! re-exports them so application-layer code (scan, scan service) keeps its
//! historical `crate::app::traits::` import paths. Infrastructure
//! implementations (lofty metadata, image cover, walkdir scanner, notify
//! watcher) live in `riff-infra` and implement these traits.

pub use crate::infra::ports::{
    AudioFormatInfo, CoverImage, CoverLoader, FilesystemWatch, MetadataReader, MetadataWriter,
    TagEdit,
};
pub use riff_persistence::track::CoverSource;
