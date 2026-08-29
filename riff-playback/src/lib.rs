//! riff-playback — the playback capability.
//!
//! Pure-Rust crate owning the playback domain: the Playback Queue and
//! repeat mode, the playback command/update/state/position types, the
//! playback ports (decoder, decoder factory, output, audio format info),
//! the playback error, the audio engine, gapless logic, the playback
//! coordinator, the Transport trait, the playback session — and the
//! Up Next / playback read model.
//!
//! Depends only on `riff-persistence` + pure-Rust utilities; no native or
//! external audio dependencies.
//!
//! `riff-backend` re-exports this surface at its historical qualified paths
//! (`riff_backend::domain::playback::*`, `riff_backend::app::transport::*`,
//! ...), so the frontend and integration tests keep their existing imports.

pub mod app;
pub mod domain;
pub mod infra;