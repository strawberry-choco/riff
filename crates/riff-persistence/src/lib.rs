//! riff-persistence — the stored entities and the Application Store contract.
//!
//! Everything that crosses the persistence boundary lives here: the stored
//! entities (tracks, albums, artists, playlists) and the Application Store
//! ports plus their DTOs. This crate implements nothing — the `SQLite` adapter
//! lives in `riff-infra` — and it depends on `std` alone, so the persistence
//! contract never drags in an audio, image, or database crate.
//!
//! `riff-backend` re-exports this surface at its historical qualified paths
//! (`riff_backend::domain`, `riff_backend::app::store`, ...), so the frontend
//! and the integration tests keep their existing imports.

pub mod errors;
pub mod playlist;
pub mod store;
pub mod track;
