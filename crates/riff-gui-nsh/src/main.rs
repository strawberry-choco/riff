//! The `riff-nsh` binary: mounts the Rust UI set on the machine's native nsh
//! backend. A thin composition — at N1 it opens no store and spawns no
//! runtime workers.

// Stops a console window flashing on Windows release builds; debug builds
// keep the console so logs stay visible. This must be an INNER (crate-level)
// attribute with a leading `!`: `windows_subsystem` is a crate-level setting,
// so an outer attribute on `fn main` applies to the function instead, which
// newer rustc rejects with `unused_attributes`. The `cfg_attr` fires only in
// release, so the debug gates never see it — a green `cargo clippy` is not
// evidence the attribute is well-formed; only a release build is.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    color_eyre::install().expect("failed to install color_eyre");
    // One logging install: `fmt::init()` already installs the `log` bridge
    // (tracing-subscriber's default `tracing-log` feature), so nsh's `log`
    // records reach the same subscriber. A second `LogTracer::init()` panics
    // at startup.
    tracing_subscriber::fmt::init();
    platform::run_app(
        platform::AppSource::Rust(
            platform::Root::of::<riff_gui_nsh::App>().tokens(riff_gui_nsh::theme_tokens()),
        ),
        platform::AppOptions {
            title: riff_gui_nsh::APP_NAME.to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: riff_gui_nsh::APP_ID.to_owned(),
            // The egui frontend's opening size
            // (crates/riff-gui/src/ui/chrome.rs:102); N4 replaces it with
            // persisted geometry.
            size: (1200, 800),
            fps: std::env::var_os("NSH_FPS").is_some(),
            // Left to their defaults on purpose: `cache` is N6's (the cover
            // pipeline decides where the image cache lives), `icon` is N16's
            // (the packager writes the executable's icon), `login_item` is
            // already false, and `data_dir` stays the per-id default — N1
            // opens no store, so no unit has claimed an override.
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!(
    "Run this binary on macOS, Windows or Linux; use the TSX entry with nsh run for iOS and Android."
);
