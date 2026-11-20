use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn tap_counts() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<riff_gui_nsh_spike::App>())).unwrap();
    app.wait_for("Count: 0").unwrap();
    app.press("id:tap").unwrap();
    app.wait_for("Count: 1").unwrap();
    app.press("id:tap").unwrap();
    app.wait_for("Count: 2").unwrap();
}

#[test]
fn test_id_selector_resolves() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<riff_gui_nsh_spike::App>())).unwrap();
    app.wait_for("id:count").unwrap();
    assert_eq!(app.screen().count("id:tap").unwrap(), 1);
    assert!(!app.screen().layout_nodes().is_empty());
}