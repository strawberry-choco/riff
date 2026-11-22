//! The platform integration the plan's N14 asks for: the folder picker and
//! drop acceptance, the window's close resolution, the tray, and OS
//! notifications. Every piece is installed by the shell's mount, on the
//! engine thread, and none writes a bridge signal the pump owns.

pub mod library_paths;
pub mod notifications;
pub mod tray;
pub mod window_state;

use nsh_core::prelude::*;
#[cfg(any(target_os = "windows", target_os = "macos"))]
use std::path::PathBuf;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;

/// The window icon while the app runs from its source tree: the packaged
/// app carries its own (`nsh package` writes the executable's `.ico` on
/// Windows and the `.app` bundle's icon on macOS), but a dev `cargo run`
/// would otherwise show the OS default. `None` once the source-tree file
/// is absent, and on Linux, whose options have no runtime icon.
#[cfg(any(target_os = "windows", target_os = "macos"))]
#[must_use]
pub fn window_icon() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let file = "riff.ico";
    #[cfg(target_os = "macos")]
    let file = "riff-1024.png";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/brand")
        .join(file);
    path.is_file().then_some(path)
}

/// Install every platform behavior. Idempotent by construction: the window
/// attributes are the same set each mount, the tray's last set wins, and the
/// effects own their own state.
pub fn install(model: Stored<UiModel>, state: UiState) {
    window_state::install(model, state);
    tray::install(model, state);
    notifications::install(model, state);
}
