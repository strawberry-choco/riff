use std::time::{Duration, Instant};

use riff_backend::app::MutexExt;
use riff_backend::app::state::TrackSort;
use riff_backend::composition::AppRuntime;
use riff_library::app::scan_service::ScanOutcome;

fn main() {
    let root = std::env::args().nth(1).expect("usage: scan <fixture-root> [store-path]");
    let store = std::env::args()
        .nth(2)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../../.scratch/nsh-p0/store/riff.sqlite3")
        });
    if let Some(parent) = store.parent() {
        std::fs::create_dir_all(parent).expect("create store dir");
    }

    let (rt, mut life) = AppRuntime::spawn(&store).expect("spawn runtime");
    let t0 = Instant::now();
    rt.scans.request(std::path::PathBuf::from(&root));

    loop {
        for outcome in rt.scans.poll() {
            match outcome {
                ScanOutcome::Complete { total_files, .. } => {
                    let mut rt = rt;
                    let count = rt.session_views.track_count("", TrackSort::NumberAsc);
                    println!(
                        "SCAN COMPLETE total_files={} indexed_tracks={} elapsed_ms={}",
                        total_files,
                        count,
                        t0.elapsed().as_millis()
                    );
                    life.shutdown();
                    return;
                }
                ScanOutcome::Failed { path, .. } => {
                    println!("SCAN FAILED path={:?}", path);
                    life.shutdown();
                    std::process::exit(1);
                }
                ScanOutcome::Progress { files_found, .. } => {
                    if files_found % 10000 == 0 {
                        println!("progress files_found={} elapsed_ms={}", files_found, t0.elapsed().as_millis());
                    }
                }
            }
        }
        if t0.elapsed() > Duration::from_secs(900) {
            println!("SCAN TIMEOUT");
            life.shutdown();
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}