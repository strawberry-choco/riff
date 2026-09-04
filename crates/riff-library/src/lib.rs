//! riff-library — the collection capability.
//!
//! Pure-Rust crate owning the library domain: scanning, filesystem watching,
//! Session Projections and views, playlist management, tag editing, cover
//! resolution and service, the ports it consumes (metadata reader/writer with
//! the tag-edit DTO, cover loader with the decoded-image DTO, and the
//! filesystem-watcher port from ticket 01), the library error, and the
//! library session.
//!
//! Depends only on `riff-persistence` + pure-Rust utilities; no edge to
//! `riff-playback` and no native dependencies.
//!
//! `riff-backend` re-exports this surface at its historical qualified paths
//! (`riff_backend::app::scan_service`, `riff_backend::app::playlist_manager`, ...),
//! so the frontend and integration tests keep their existing imports.

pub mod app;
pub mod domain;
pub mod infra;
