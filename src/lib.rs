//! riff Music Player — library crate.
//!
//! This library owns the module tree so that both the binary (`src/main.rs`)
//! and the integration test suite (`tests/`) can share the same code. The
//! binary is a thin wrapper over this library.
//!
//! # Architecture
//!
//! - [`domain`]: Pure business logic. No external crate imports.
//! - [`app`]: Use cases, state, and trait interfaces (ports).
//! - [`infra`]: Trait implementations backed by external crates.
//! - [`ui`]: egui widgets, tray icon, settings persistence, fonts.

pub mod app;
pub mod domain;
pub mod infra;
pub mod ui;
