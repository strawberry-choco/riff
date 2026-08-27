//! riff GUI — egui frontend.
//!
//! Owns the egui UI, tray icon, native dialogs, and fonts. Depends on
//! riff-backend only, through the port/domain types. Contains no audio,
//! metadata, scanner, watcher, or `SQLite` dependency.

pub mod ui;
