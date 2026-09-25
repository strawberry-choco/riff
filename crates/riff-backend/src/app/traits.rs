//! Re-export surface for the ports the backend's services, the frontend and
//! the test suite name through `riff_backend::app::traits::`.
//!
//! This module defines nothing. Every port has one home in the capability
//! that owns it: the library ports here come from `riff_library::app::traits`,
//! and the playback ports (`AudioDecoder`, `AudioOutput`, `DecoderFactory`)
//! are re-exported one level up at `riff_backend::app` from
//! `riff_playback::infra::ports`. The qualified paths here exist only so
//! historical imports keep resolving — ADR 0009's as-built rule for the
//! re-export surface.

pub use riff_library::app::traits::{
    AudioFormatInfo, CoverLoader, DecodedCover, FilesystemWatch, MetadataReader, MetadataWriter,
    RequestedSize, TagEdit,
};
