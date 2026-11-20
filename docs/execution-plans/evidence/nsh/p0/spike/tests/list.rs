use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use riff_gui_nsh_spike::list::ListApp;

#[test]
fn fifty_thousand_rows_virtualize_scroll_and_select() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<ListApp>())).unwrap();
    app.wait_for("id:track-list").unwrap();

    let visible = app.screen().count("id:row-index").unwrap();
    assert!(visible > 0 && visible < 200, "virtualized window, got {visible}");

    app.press("id:jump").unwrap();
    app.wait_for("text:40000").unwrap();

    app.press("id:row-index >> nth=0").unwrap();
    let texts = app.screen().texts().join("\n");
    let sel = texts
        .lines()
        .find(|l| l.starts_with("selected "))
        .and_then(|l| l.trim_start_matches("selected ").parse::<usize>().ok())
        .expect("selected readout");
    assert_ne!(sel, usize::MAX, "a row press selected an index");

    let t0 = std::time::Instant::now();
    for _ in 0..20 {
        app.scroll("id:track-list", nsh_core::prelude::px(2000.0)).unwrap();
    }
    let ms = t0.elapsed().as_millis();
    println!("HEADLESS scroll 20 steps of 2000pt over 50k rows: total {ms} ms, per-step {} ms", ms as f64 / 20.0);
}