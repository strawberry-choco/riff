//! System tray icon for riff music player (macOS/Windows only).
//!
//! Issue 03 — window visibility is frontend-local. The tray icon does NOT
//! construct backend playback commands for the "Show Window" / left-click
//! path: those go over a frontend-only [`window_visibility::VisibilityTx`]
//! channel that the UI thread drains on every logic tick. Playback intents
//! (Play/Pause, Next, Previous, Stop) go through the facade transport's
//! [`Transport`] port exactly like the mouse and keyboard paths, so every
//! tray dispatch is recorded onto the facade's event inbox too.

#[cfg(not(target_os = "linux"))]
use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem};
#[cfg(not(target_os = "linux"))]
use std::sync::{Arc, Mutex, atomic::AtomicBool};
#[cfg(not(target_os = "linux"))]
use tray_icon::Icon;
#[cfg(not(target_os = "linux"))]
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::ui::window_visibility::{VisibilityMessage, VisibilityTx};
use riff_backend::app::FacadeTransport;
use riff_backend::app::MutexExt;
use riff_backend::app::state::PlaybackSession;
use riff_backend::app::transport::Transport;

/// Create a system tray icon with playback controls.
/// On Linux this is a no-op (tray-icon requires GTK which isn't always available).
#[cfg(not(target_os = "linux"))]
pub fn create_tray(
    transport: FacadeTransport,
    playback: Arc<Mutex<PlaybackSession>>,
    quit_flag: Arc<AtomicBool>,
    visibility_tx: VisibilityTx,
) -> Result<TrayIcon, Box<dyn std::error::Error>> {
    let menu = Menu::new();

    let play_pause = MenuItem::new("Play/Pause", true, None);
    let next_track = MenuItem::new("Next Track", true, None);
    let prev_track = MenuItem::new("Previous Track", true, None);
    let sep1 = PredefinedMenuItem::separator();
    let show_window = MenuItem::new("Show Window", true, None);
    let sep2 = PredefinedMenuItem::separator();
    let quit = MenuItem::new("Quit", true, None);

    menu.append(&play_pause)?;
    menu.append(&next_track)?;
    menu.append(&prev_track)?;
    menu.append(&sep1)?;
    menu.append(&show_window)?;
    menu.append(&sep2)?;
    menu.append(&quit)?;

    let icon = build_default_icon()?;

    let tray = TrayIconBuilder::new()
        .with_tooltip("riff")
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .build()?;

    let play_pause_id: MenuId = play_pause.id().clone();
    let next_track_id: MenuId = next_track.id().clone();
    let prev_track_id: MenuId = prev_track.id().clone();
    let show_window_id: MenuId = show_window.id().clone();
    let quit_id: MenuId = quit.id().clone();

    std::thread::spawn(move || {
        let menu_channel = muda::MenuEvent::receiver();
        let tray_channel = TrayIconEvent::receiver();

        loop {
            if quit_flag.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            if let Ok(event) = menu_channel.recv_timeout(std::time::Duration::from_millis(200)) {
                let id = event.id;
                if id == play_pause_id {
                    // Play/pause reads the live playback state (play vs
                    // resume vs pause), so it holds the playback session
                    // lock alone — never together with any other lock.
                    let session = playback.lock_or_recover();
                    transport.play_pause(&session);
                } else if id == next_track_id {
                    transport.next();
                } else if id == prev_track_id {
                    transport.previous();
                } else if id == show_window_id {
                    // Frontend-local: never touches backend state or the audio
                    // engine. Even mid-decode, the UI thread picks this up on
                    // its next logic tick (~200ms while hidden, <16ms visible).
                    let _ = visibility_tx.send(VisibilityMessage(true));
                } else if id == quit_id {
                    transport.stop();
                    quit_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }

            if let Ok(TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                ..
            }) = tray_channel.try_recv()
            {
                let _ = visibility_tx.send(VisibilityMessage(true));
            }
        }
    });

    Ok(tray)
}

/// Updates the tray icon tooltip with the given text.
#[cfg(not(target_os = "linux"))]
pub fn update_tooltip(tray: &TrayIcon, text: &str) {
    let _ = tray.set_tooltip(Some(text));
}

#[cfg(not(target_os = "linux"))]
fn build_default_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    const SIZE: u32 = 32;
    let side = SIZE as usize;
    let mut rgba = vec![0u8; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            let idx = (y * side + x) * 4;
            rgba[idx] = 64;
            rgba[idx + 1] = 128;
            rgba[idx + 2] = 192;
            rgba[idx + 3] = 255;
        }
    }
    let icon = Icon::from_rgba(rgba, SIZE, SIZE)?;
    Ok(icon)
}
