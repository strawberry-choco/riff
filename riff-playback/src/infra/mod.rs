//! Infrastructure ports and implementations for playback.
//!
//! This module contains the port traits that infrastructure implements:
//! - `AudioDecoder` / `DecoderFactory` — decoding audio files
//! - `AudioOutput` — platform audio output
//! - `AudioFormatInfo` — decoder output format metadata

pub mod audio_engine;
pub mod ports;

pub use ports::{AudioDecoder, AudioFormatInfo, AudioOutput, DecoderFactory};