use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use riff_gui_nsh_spike::bridge::BridgeApp;

fn readout(app: &Headless) -> String {
    app.screen()
        .texts()
        .into_iter()
        .find(|t| t.starts_with("playing") || t.starts_with("paused") || t.starts_with("stopped"))
        .unwrap_or_default()
        .to_string()
}

fn field(line: &str, key: &str) -> String {
    line.split_whitespace()
        .skip_while(|w| *w != key)
        .nth(1)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn play_pause_seek_next_drive_the_real_runtime() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<BridgeApp>())).unwrap();
    app.wait_for("text:/^stopped/").unwrap();

    app.press("id:play").unwrap();
    app.wait_for("text:/^playing/").unwrap();
    app.wait_for("text:/queue 50000/").unwrap();

    app.press("id:playpause").unwrap();
    app.wait_for("text:/^paused/").unwrap();
    let paused_pos = field(&readout(&app), "pos");

    app.press("id:playpause").unwrap();
    app.wait_for("text:/^playing/").unwrap();

    // Seek must move the position readout back to the seek target, not leave it.
    app.press("id:seek").unwrap();
    app.wait_for("text:/pos 1\\./").unwrap();

    // Next must advance the queue index.
    let before = field(&readout(&app), "at");
    app.press("id:next").unwrap();
    let mut advanced = false;
    for _ in 0..40 {
        let after = field(&readout(&app), "at");
        if after != before && !after.is_empty() {
            advanced = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(advanced, "next advanced the queue index from {before}");

    let texts = app.screen().texts().join("\n");
    assert!(texts.contains("presses 1"), "play handler fired once:\n{texts}");
    assert!(!paused_pos.is_empty(), "paused readout carried a position");
}