use thiserror::Error;

/// Errors that can occur in the application layer.
#[derive(Error, Debug, Clone)]
pub enum AppError {
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Audio output error: {0}")]
    AudioOutput(String),
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
