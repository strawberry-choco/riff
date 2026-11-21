//! The system tray: riff's icon, its playback controls and the two window
//! commands. The plan's tray decision is proved at P0 (nsh ships `set_tray`),
//! so close-to-hide has its affordance back: the window is hidden, and the
//! tray's Show brings it back.
//!
//! The menu is installed once and its handlers hold the model; Play/Pause
//! dispatches through the UI transport, so the tray is a second door to the
//! same engine.

use nsh_core::element::desktop::{MenuItem, set_tray, tray};
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::platform::window_state;

/// Install the tray. Off desktops `set_tray` is a no-op.
pub fn install(model: Stored<UiModel>, _state: UiState) {
    let show = window_state::show;
    let play_pause = move || {
        model.with(|m| {
            let session = m.rt.playback.lock_or_recover();
            m.rt.ui_transport.play_pause(&session);
        });
    };
    let next = move || {
        model.with(|m| m.rt.ui_transport.next());
    };
    let previous = move || {
        model.with(|m| m.rt.ui_transport.previous());
    };
    let quit = move || window_state::quit(model);
    let items = vec![
        MenuItem::new("Show riff").on_select(show),
        MenuItem::separator(),
        MenuItem::new("Play/Pause").on_select(play_pause),
        MenuItem::new("Next").on_select(next),
        MenuItem::new("Previous").on_select(previous),
        MenuItem::separator(),
        MenuItem::new("Quit").on_select(quit),
    ];
    let mut tray = tray().tooltip("riff").menu(items);
    tray = tray.icon(asset!("brand/riff-1024.png"));
    set_tray(tray.on_click(|_| window_state::show()));
}
