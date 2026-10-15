//! riff GUI — egui frontend.
//!
//! Owns the egui UI (the feature composites and the host adapters), the tray
//! icon, and native dialogs. The design authority and the presentation
//! primitives it renders with live in `riff-gui-theme` and
//! `riff-gui-components` (ADR 0018). Depends on riff-backend only, through
//! the port/domain types. Contains no audio, metadata, scanner, watcher, or
//! `SQLite` dependency.

pub mod ui;
