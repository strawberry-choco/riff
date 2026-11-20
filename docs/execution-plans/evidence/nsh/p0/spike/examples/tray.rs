use nsh_core::element::desktop::{MenuItem, main_window, quit, set_tray, tray};
use nsh_core::prelude::*;

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[component]
pub fn TrayApp() -> impl IntoElement {
    let presses = signal(0_u32);
    set_tray(
        tray()
            .tooltip("riff tray proof")
            .on_click(move |_| {
                presses.update(|p| *p += 1);
                main_window().focus();
            })
            .menu([
                MenuItem::new("Focus window").on_select(|| main_window().focus()),
                MenuItem::separator(),
                MenuItem::new("Quit").on_select(quit),
            ]),
    );

    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .child(text(move || format!("Tray clicks: {}", presses.get())).test_id("tray-clicks"))
        .child(text("Close this window; the tray should stay.").test_id("hint"))
}

fn main() -> impl std::process::Termination {
    env_logger::init();
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<TrayApp>()),
        platform::AppOptions {
            title: "riff tray proof".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "io.github.strawberry-choco.riff".to_owned(),
            size: (600, 400),
            ..Default::default()
        },
    )
}