use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use riff_gui_nsh_spike::interactive::InteractiveApp;

fn desktop() -> EngineConfig {
    let mut config = EngineConfig::new(Root::of::<InteractiveApp>());
    config.environment.os = nsh_core::platform::Os::Windows;
    config
}

#[test]
fn keyboard_map_drives_focus_and_playback_keys() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("id:mode").unwrap();

    app.press_key("mod+k").unwrap();
    app.wait_for("focus [search]").unwrap();

    app.press_key("escape").unwrap();
    app.wait_for("focus []").unwrap();

    app.press_key("space").unwrap();
    app.wait_for("keys [space ]").unwrap();

    app.press_key("down").unwrap();
    app.wait_for("index 1").unwrap();
}

#[test]
fn palette_switches_at_runtime_through_a_signal() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("mode dark").unwrap();
    app.press("id:toggle").unwrap();
    app.wait_for("mode light").unwrap();
    app.press("id:toggle").unwrap();
    app.wait_for("mode dark").unwrap();
}

#[test]
fn folder_drop_reaches_the_handler() {
    let mut app = Headless::start(desktop()).unwrap();
    app.wait_for("dropped 0").unwrap();
    app.drop_files(
        "id:root",
        vec!["file:///C:/projects/riff/.scratch/nsh-p0/library/artist_000/album_00".to_string()],
    )
    .unwrap();
    app.wait_for("dropped 1").unwrap();
}