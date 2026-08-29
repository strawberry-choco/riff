use thiserror::Error;

/// Failures raised by the music collection: metadata read and write, cover
/// loading, filesystem IO, library scanning, and track lookup.
#[derive(Error, Debug, Clone)]
pub enum LibraryError {
    #[error("Metadata read error: {0}")]
    MetadataRead(String),
    #[error("Failed to write tags: {0}")]
    MetadataWrite(String),
    #[error("Cover load error: {0}")]
    CoverLoad(String),
    #[error("Library scan error: {0}")]
    LibraryScan(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Track not found: {0}")]
    TrackNotFound(String),
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}
