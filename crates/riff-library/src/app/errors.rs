use thiserror::Error;

/// Failures raised by the music collection: metadata read and write, cover
/// loading, and filesystem IO.
#[derive(Error, Debug, Clone)]
pub enum LibraryError {
    #[error("Metadata read error: {0}")]
    MetadataRead(String),
    #[error("Failed to write tags: {0}")]
    MetadataWrite(String),
    #[error("Cover load error: {0}")]
    CoverLoad(String),
    #[error("IO error: {0}")]
    Io(String),
}
