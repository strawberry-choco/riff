//! Media adapters: the lofty metadata reader/writer and the image cover
//! loader, serving riff-library's metadata and cover ports.

pub mod cover_loader;
pub mod metadata_reader;
pub mod metadata_writer;

pub use cover_loader::ImageCoverLoader;
pub use metadata_reader::{LoftyMetadataReader, parse_replaygain_gain};
pub use metadata_writer::LoftyMetadataWriter;
