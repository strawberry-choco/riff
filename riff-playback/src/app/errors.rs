use thiserror::Error;

/// Failures raised by playback: decoding and audio output.
#[derive(Error, Debug, Clone)]
pub enum PlaybackError {
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Audio output error: {0}")]
    AudioOutput(String),
}

/// Re-export the persistence boundary error for use in playback projections.
pub use riff_persistence::errors::StoreError;