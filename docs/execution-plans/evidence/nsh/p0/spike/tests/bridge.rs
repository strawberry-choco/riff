use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use riff_gui_nsh_spike::bridge::BridgeApp;

#[test]
fn play_pause_seek_drive_the_real_runtime() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<BridgeApp>())).unwrap();
    app.wait_for("text:/^stopped/").unwrap();

    app.press("id:play").unwrap();
    app.wait_for("text:/^playing/").unwrap();
    app.wait_for("text:/queue 50000/").unwrap();

    app.press("id:playpause").unwrap();
    app.wait_for("text:/^paused/").unwrap();

    app.press("id:playpause").unwrap();
    app.wait_for("text:/^playing/").unwrap();

    app.press("id:seek").unwrap();
    app.wait_for("text:/^playing/").unwrap();

    let texts = app.screen().texts().join("\n");
    assert!(texts.contains("presses 1"), "play handler fired once:\n{texts}");
}