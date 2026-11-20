use riff_backend::app::state::TrackSort;
use riff_backend::composition::AppRuntime;
use riff_gui_nsh_spike::list::store_path;

#[test]
fn projection_read_cost() {
    let (mut rt, mut life) = AppRuntime::spawn(&store_path()).unwrap();
    let sort = TrackSort::NumberAsc;

    let t = std::time::Instant::now();
    let n = rt.session_views.track_count("", sort);
    println!("track_count={n} in {} us", t.elapsed().as_micros());

    let t = std::time::Instant::now();
    for i in 0..200 {
        let _ = rt.session_views.track_row("", sort, i);
    }
    let seq = t.elapsed().as_micros() as f64 / 200.0;
    println!("sequential track_row: {seq:.1} us/call");

    let t = std::time::Instant::now();
    for k in 0..200 {
        let i = (k * 7919) % n;
        let _ = rt.session_views.track_row("", sort, i);
    }
    let rnd = t.elapsed().as_micros() as f64 / 200.0;
    println!("scattered track_row: {rnd:.1} us/call");

    let t = std::time::Instant::now();
    for _ in 0..57 {
        let i = 40000;
        let _ = rt.session_views.track_row("", sort, i);
    }
    let same = t.elapsed().as_micros() as f64 / 57.0;
    println!("repeated same-index track_row: {same:.1} us/call");

    life.shutdown();
}