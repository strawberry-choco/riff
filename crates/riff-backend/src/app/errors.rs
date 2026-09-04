use thiserror::Error;

/// Re-export of the persistence boundary error.
pub use riff_persistence::errors::StoreError;

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
}

/// Failures raised by playback: decoding and audio output.
#[derive(Error, Debug, Clone)]
pub enum PlaybackError {
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Audio output error: {0}")]
    AudioOutput(String),
}
