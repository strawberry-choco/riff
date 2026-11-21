//! Window visibility and the close resolution: the egui
//! `ui/window_visibility.rs` behavior. Close asks the app first
//! (`close_on_press: false`); "Quit on close" closes for real, otherwise the
//! window hides to the tray and playback keeps running.

use nsh_core::desktop::WindowAttributes;
use nsh_core::element::desktop::current_window;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;

/// Install the close handler and stop the platform's own close-on-press.
pub fn install(model: Stored<UiModel>, _state: UiState) {
    let mut attributes = WindowAttributes::default();
    attributes.close_on_press = Some(false);
    current_window().set(attributes);
    current_window().on_close_press(move |_reason| {
        let quit = model.with(|m| m.rt.library.lock_or_recover().ui_flags.close_quits_app);
        if quit {
            model.with(|m| {
                m.rt.quit_flag
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            });
            current_window().close();
        } else {
            current_window().hide();
        }
    });
}

/// Show the window again (the tray's click, or a second launch's request).
pub fn show() {
    current_window().show();
}

/// Quit for real: the tray's Quit and the close path both end here.
pub fn quit(model: Stored<UiModel>) {
    model.with(|m| {
        m.rt.quit_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    });
    nsh_core::element::desktop::quit();
}
