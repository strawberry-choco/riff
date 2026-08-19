//! System tray icon for riff music player (macOS/Windows only)

#[cfg(not(target_os = "linux"))]
use crate::domain::PlaybackCommand;
#[cfg(not(target_os = "linux"))]
use crossbeam_channel::Sender;
#[cfg(not(target_os = "linux"))]
use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem};
#[cfg(not(target_os = "linux"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_os = "linux"))]
use std::sync::Arc;
#[cfg(not(target_os = "linux"))]
use tray_icon::Icon;
#[cfg(not(target_os = "linux"))]
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Create a system tray icon with playback controls.
/// On Linux this is a no-op (tray-icon requires GTK which isn't always available).
#[cfg(not(target_os = "linux"))]
pub fn create_tray(
    command_sender: Sender<PlaybackCommand>,
    quit_flag: Arc<AtomicBool>,
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

    let cmd_tx = command_sender.clone();
    let play_pause_id: MenuId = play_pause.id().clone();
    let next_track_id: MenuId = next_track.id().clone();
    let prev_track_id: MenuId = prev_track.id().clone();
    let show_window_id: MenuId = show_window.id().clone();
    let quit_id: MenuId = quit.id().clone();
    std::thread::spawn(move || {
        let menu_channel = muda::MenuEvent::receiver();
        let tray_channel = TrayIconEvent::receiver();

        loop {
            if quit_flag.load(Ordering::Relaxed) {
                break;
            }

            // Block up to 200ms waiting for a menu event instead of busy-polling.
            // recv_timeout returns immediately when an event arrives, and times
            // out otherwise so we can still observe the quit flag and tray clicks.
            if let Ok(event) = menu_channel.recv_timeout(std::time::Duration::from_millis(200)) {
                let id = event.id;
                if id == play_pause_id {
                    let _ = cmd_tx.send(PlaybackCommand::PlayPause);
                } else if id == next_track_id {
                    let _ = cmd_tx.send(PlaybackCommand::Next);
                } else if id == prev_track_id {
                    let _ = cmd_tx.send(PlaybackCommand::Previous);
                } else if id == show_window_id {
                    let _ = cmd_tx.send(PlaybackCommand::ToggleVisibility);
                } else if id == quit_id {
                    // Graceful shutdown: stop playback and signal the UI to close
                    // so eframe can persist storage and the library cache is saved.
                    let _ = cmd_tx.send(PlaybackCommand::Stop);
                    quit_flag.store(true, Ordering::Relaxed);
                }
            }

            if let Ok(TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                ..
            }) = tray_channel.try_recv()
            {
                let _ = cmd_tx.send(PlaybackCommand::ToggleVisibility);
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
