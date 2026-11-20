#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

fn main() -> impl std::process::Termination {
    env_logger::init();
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<riff_gui_nsh_spike::bridge::BridgeApp>()),
        platform::AppOptions {
            title: "riff bridge proof".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "io.github.strawberry-choco.riff".to_owned(),
            size: (900, 400),
            ..Default::default()
        },
    )
}