//! System tray icon for riff music player (macOS/Windows only)

#[cfg(not(target_os = "linux"))]
use crossbeam_channel::Sender;
#[cfg(not(target_os = "linux"))]
use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem};
#[cfg(not(target_os = "linux"))]
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};
#[cfg(not(target_os = "linux"))]
use tray_icon::Icon;
#[cfg(not(target_os = "linux"))]
use crate::domain::PlaybackCommand;

/// Create a system tray icon with playback controls.
/// On Linux this is a no-op (tray-icon requires GTK which isn't always available).
#[cfg(not(target_os = "linux"))]
pub fn create_tray(command_sender: Sender<PlaybackCommand>) -> Result<TrayIcon, Box<dyn std::error::Error>> {
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
            if let Ok(event) = menu_channel.try_recv() {
                let id = event.id;
                if id == play_pause_id {
                    let _ = cmd_tx.send(PlaybackCommand::ToggleVisibility);
                } else if id == next_track_id {
                    let _ = cmd_tx.send(PlaybackCommand::Next);
                } else if id == prev_track_id {
                    let _ = cmd_tx.send(PlaybackCommand::Previous);
                } else if id == show_window_id {
                    let _ = cmd_tx.send(PlaybackCommand::ToggleVisibility);
                } else if id == quit_id {
                    let _ = cmd_tx.send(PlaybackCommand::Stop);
                    std::process::exit(0);
                }
            }

            if let Ok(event) = tray_channel.try_recv() {
                match event {
                    TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. } => {
                        let _ = cmd_tx.send(PlaybackCommand::ToggleVisibility);
                    }
                    _ => {}
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
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
    let size = 32usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let idx = (y * size + x) * 4;
            rgba[idx] = 64;
            rgba[idx + 1] = 128;
            rgba[idx + 2] = 192;
            rgba[idx + 3] = 255;
        }
    }
    let icon = Icon::from_rgba(rgba, size as u32, size as u32)?;
    Ok(icon)
}
