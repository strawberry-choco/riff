//! The backend bridge: the `riff-backend` edge made live. It owns the
//! `AppRuntime` (moved in from the Composition Root's
//! [`riff_backend::composition::AppRuntime::spawn`]), the signal store, and
//! the 16 ms pump that writes the session-derived signals — the nav signal
//! alone answers to the shell's selection handlers instead, whose writes a
//! store-blind pump could never see.
//!
//! - [`covers`] — the Cover Cache decision half, ported from the frozen egui
//!   frontend (no egui type; see the module doc for the supersession).
//! - [`pump`] — the poller and its per-tick comparison contract.
//! - [`runtime`] — the `UiModel` the engine thread owns.
//! - [`state`] — the `UiState` signals.
//! - [`crate::model`] — the view vocabulary and the projection reads.
//!
//! Everything here runs on the engine thread: nsh signals are `!Send` (P0
//! correction 9), so the model is engine-thread-only by construction.

pub mod covers;
pub mod pump;
pub mod runtime;
pub mod state;

pub use pump::install;
pub use runtime::UiModel;
