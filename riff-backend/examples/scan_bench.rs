//! Library Scan benchmark harness (dependency-improvements-plan P4).
//!
//! A standalone example binary (NOT part of the test suite) so it can never
//! perturb the integration-test process. Run it in release mode:
//!
//! ```bash
//! cargo run --release --example scan_bench
//! ```
//!
//! It generates a synthetic library fixture (valid PCM WAV files, same
//! byte layout as the suite's `write_minimal_wav` fixture helper) in a
//! tempdir, points the REAL scan pipeline at it — real walker, real Lofty
//! metadata reads, real `SQLite` Application Store with durable ~10-track
//! batch commits, worker on its own thread, wired exactly like the
//! composition root — and prints wall-clock timings for:
//!
//! 1. a full COLD rescan end-to-end (walk → freshness filter → metadata →
//!    batched commits) — the P4 decision-gate number;
//! 2. a WARM rescan (freshness filter short-circuits metadata reads);
//! 3. the filesystem walk alone, to show how much of the end-to-end time
//!    a parallel walker (jwalk) could even hope to touch.
//!
//! Coverage note: fixture files are tiny valid WAVs without tags, so the
//! numbers cover the pipeline's structural costs (walk, store lookups,
//! tag parsing overhead of untagged files, batched commits) but not the
//! heavier tag payloads of real-world MP3/FLAC libraries.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn minimal_wav_bytes() -> Vec<u8> {
    const SAMPLES: u32 = 800; // 0.1 s at 8 kHz
    let data_size = SAMPLES * 2; // 16-bit mono
    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&8000u32.to_le_bytes()); // sample rate
    bytes.extend_from_slice(&16000u32.to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..SAMPLES {
        let sample = ((i % 100) as i16).wrapping_mul(64);
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Generate the synthetic library: 100 artists x 5 albums x 40 tracks =
/// 20,000 audio files in a nested tree.
fn generate_fixture(root: &std::path::Path) -> std::io::Result<()> {
    const ARTISTS: u32 = 100;
    const ALBUMS_PER_ARTIST: u32 = 5;
    const TRACKS_PER_ALBUM: u32 = 40;

    let wav = minimal_wav_bytes();
    for artist in 0..ARTISTS {
        for album in 0..ALBUMS_PER_ARTIST {
            let album_dir = root
                .join(format!("artist_{artist:03}"))
                .join(format!("album_{album:02}"));
            std::fs::create_dir_all(&album_dir)?;
            for track in 0..TRACKS_PER_ALBUM {
                std::fs::write(album_dir.join(format!("track_{track:02}.wav")), &wav)?;
            }
        }
    }
    Ok(())
}

/// Wire the real scan pipeline exactly like the composition root: real
/// walker closure over the shared cancel flag, real Lofty reader, real
/// `SQLite` store ports, serial worker thread. Returns the front end.
fn wire_pipeline(
    queries: riff_backend::infra::store::SqliteStore,
    mutations: riff_backend::infra::store::SqliteStore,
    cancel_flag: Arc<AtomicBool>,
) -> riff_library::app::scan_service::ScanService {
    use riff_backend::infra::{AudioFileScanner, LoftyMetadataReader};

    let scanner = AudioFileScanner::new(cancel_flag.clone());
    let (scans, worker) = riff_library::app::scan_service::ScanService::new(
        Box::new(LoftyMetadataReader::new()),
        Box::new(queries),
        Box::new(mutations),
        cancel_flag,
        move |path| scanner.scan(path),
    );
    std::thread::spawn(move || worker.run());
    scans
}

/// Request a scan of `root` and block until its `Complete` outcome lands,
/// returning the wall-clock duration and the reported file total.
fn timed_scan(
    scans: &riff_library::app::scan_service::ScanService,
    root: &std::path::Path,
    label: &str,
) -> (Duration, usize) {
    use riff_library::app::scan_service::{ScanOutcome, Scans};

    let start = Instant::now();
    scans.request(root.to_path_buf());
    loop {
        for outcome in scans.poll() {
            if let ScanOutcome::Complete { path, total_files } = outcome
                && path == root
            {
                let elapsed = start.elapsed();
                let rate = f64::from(u32::try_from(total_files).unwrap_or_default())
                    / elapsed.as_secs_f64();
                println!("{label}: {elapsed:>10.3?}  ({total_files} files, {rate:.0} files/sec)");
                return (elapsed, total_files);
            }
        }
        assert!(
            start.elapsed().as_secs() < 600,
            "{label}: scan never completed within 10 minutes"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn main() {
    use riff_backend::app::store::LibraryQueryStore;
    use riff_backend::infra::store::SqliteStore;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bench.sqlite3");
    // The new store signature takes a generation-bump channel that the
    // mutation adapter writes to (issue 04: emit-beside-the-bump). The
    // benchmark doesn't drain it; we only care about the side effects.
    let (changes_tx, _changes_rx) = crossbeam_channel::unbounded();
    let store = SqliteStore::open_and_migrate(&db_path, changes_tx).expect("store must open");
    println!("generating fixture (20,000 WAVs) ...");
    let gen_start = Instant::now();
    // The library lives one level below the scratch dir: tempfile's dir
    // name starts with '.', and the scanner's hidden-entry filter would
    // prune the walk at such a root.
    let root = dir.path().join("library");
    std::fs::create_dir_all(&root).expect("library root must be creatable");
    generate_fixture(&root).expect("fixture generation must succeed");
    println!(
        "fixture ready in {:.2?} (excluded from measurements)",
        gen_start.elapsed()
    );

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let scans = wire_pipeline(store.clone(), store.clone(), cancel_flag.clone());

    // 1. THE DECISION-GATE NUMBER: full cold rescan end-to-end.
    let (cold_elapsed, total_files) = timed_scan(&scans, &root, "cold rescan (end-to-end)");

    // The pipeline really processed everything.
    assert_eq!(total_files, 20_000, "walker must discover every file");
    assert_eq!(
        store.track_count().expect("track_count"),
        20_000,
        "every discovered file must be committed"
    );

    // 2. Warm rescan: freshness filter short-circuits metadata reads.
    let (warm_elapsed, _) = timed_scan(&scans, &root, "warm rescan (end-to-end)");

    // 3. Walk alone (cache-warm): the ceiling on what a parallel walker
    //    could save from the end-to-end time.
    let scanner = riff_backend::infra::AudioFileScanner::new(cancel_flag);
    let walk_start = Instant::now();
    let walked = scanner.scan(&root);
    println!(
        "walk only (sequential walkdir, cache-warm): {:>10.3?}  ({} files)",
        walk_start.elapsed(),
        walked.len()
    );
    assert_eq!(walked.len(), 20_000);

    println!("\nsummary:");
    println!("  cold end-to-end : {cold_elapsed:.3?}");
    println!("  warm end-to-end : {warm_elapsed:.3?}");
}
