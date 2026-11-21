//! The platform integration the plan's N14 asks for: the folder picker and
//! drop acceptance, the window's close resolution, the tray, and OS
//! notifications. Every piece is installed by the shell's mount, on the
//! engine thread, and none writes a bridge signal the pump owns.

pub mod library_paths;
pub mod notifications;
pub mod tray;
pub mod window_state;

use nsh_core::prelude::*;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;

/// Install every platform behavior. Idempotent by construction: the window
/// attributes are the same set each mount, the tray's last set wins, and the
/// effects own their own state.
pub fn install(model: Stored<UiModel>, state: UiState) {
    window_state::install(model, state);
    tray::install(model, state);
    notifications::install(model, state);
}
