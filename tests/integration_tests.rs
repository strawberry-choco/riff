#[cfg(test)]
mod tests {
    use crate::app::MutexExt;
    use crate::app::state::PlaybackSession;
    use crate::domain::{PlaybackCommand, PlaybackState, TrackId};
    use crossbeam_channel::unbounded;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_app_state_mutex_safety() {
        let state = Arc::new(Mutex::new(PlaybackSession::default()));

        // Verify the initial state on the main thread before any mutation.
        {
            let s = state.lock_or_recover();
            assert_eq!(s.playback_state, PlaybackState::Stopped);
            assert!(crate::test_utils::float_close(s.current_volume, 1.0));
        }

        // Spawn a thread that mutates the shared state through the mutex.
        let state_clone = state.clone();
        let handle = std::thread::spawn(move || {
            let mut s = state_clone.lock_or_recover();
            s.playback_state = PlaybackState::Playing;
            s.current_volume = 0.8;
        });

        // Wait for the other thread to complete (the guard above is dropped, so
        // the spawned thread can acquire the lock and finish).
        handle.join().unwrap();

        // Verify the changes from the other thread are visible.
        let s = state.lock_or_recover();
        assert_eq!(s.playback_state, PlaybackState::Playing);
        assert!(crate::test_utils::float_close(s.current_volume, 0.8));
    }

    #[test]
    fn test_playback_command_channel() {
        let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
        let (_update_tx, _update_rx) = unbounded::<crate::domain::PlaybackUpdate>();

        // Test sending and receiving commands
        let track_id = TrackId("test.mp3".to_string());
        let cmd = PlaybackCommand::Play(track_id.clone());

        assert!(cmd_tx.send(cmd).is_ok());

        if let Ok(received_cmd) = cmd_rx.recv() {
            match received_cmd {
                PlaybackCommand::Play(received_id) => {
                    assert_eq!(received_id, track_id);
                }
                _ => panic!("Unexpected command type"),
            }
        } else {
            panic!("Failed to receive command");
        }
    }

    #[test]
    fn test_library_scan_service_drives_a_real_scan_end_to_end() {
        // The former channel-simulation is gone: the Library Scan lives
        // behind the Scan Service seam (ADR 0006), so this drives the real
        // service pair end to end — real walker over dummy files, real
        // SQLite Application Store in a scratch dir, worker on its own
        // thread, exactly like the composition root wires it.
        use crate::mocks::MockMetadataReader;
        use riff_backend::app::scan_service::{ScanOutcome, ScanService, Scans};
        use riff_backend::app::store::LibraryQueryStore;
        use riff_infra::filesystem::AudioFileScanner;
        use riff_infra::store::SqliteStore;
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let store = SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("fresh store must open and migrate");
        let mutations = store.clone();
        let queries = store;

        let root = dir.path().join("music");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..3 {
            std::fs::write(root.join(format!("song_{i}.mp3")), b"dummy audio").unwrap();
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let scanner = AudioFileScanner::new(cancel_flag.clone());
        let (scans, worker) = ScanService::new(
            Box::new(MockMetadataReader::default()),
            Box::new(queries.clone()),
            Box::new(mutations),
            cancel_flag,
            move |path| scanner.scan(path),
        );
        std::thread::spawn(move || worker.run());

        assert!(!scans.is_scanning(&root), "nothing requested yet");
        scans.request(root.clone());

        // Drain outcomes until the Complete for this root lands (bounded so
        // a wedged worker fails the test instead of hanging it).
        let start = Instant::now();
        let mut outcomes: Vec<ScanOutcome> = Vec::new();
        let complete = loop {
            outcomes.extend(scans.poll());
            if let Some(pos) = outcomes.iter().position(
                |o| matches!(o, ScanOutcome::Complete { path, .. } if path.as_path() == root),
            ) {
                break outcomes.remove(pos);
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "worker never completed the scan; got {outcomes:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        };

        // The walk, the durable commits, and the outcome stream all line up.
        assert_eq!(
            complete,
            ScanOutcome::Complete {
                path: root.clone(),
                total_files: 3
            }
        );
        assert!(!scans.is_scanning(&root), "the scan ended");
        assert_eq!(
            queries.track_count().unwrap(),
            3,
            "every discovered file committed durably"
        );
        assert!(
            outcomes
                .iter()
                .any(|o| matches!(o, ScanOutcome::Progress { files_found: 3, .. })),
            "progress was reported before completion: {outcomes:?}"
        );
    }

    /// Wire a real Library Scan Service pair over mocks whose walk returns
    /// instantly (every request publishes one `Complete`), run the worker on
    /// its own thread exactly like the composition root, and return the
    /// front-end handle.
    fn instant_scan_service() -> (
        riff_backend::app::scan_service::ScanService,
        crossbeam_channel::Receiver<Vec<std::path::PathBuf>>,
    ) {
        use crate::mocks::{MockLibraryMutationStore, MockLibraryQueryStore, MockMetadataReader};
        use riff_backend::app::scan_service::ScanService;
        use std::path::PathBuf;
        use std::sync::atomic::AtomicBool;

        let (_watch_tx, watch_rx) = unbounded::<Vec<PathBuf>>();
        let (scans, worker) = ScanService::new(
            Box::new(MockMetadataReader::default()),
            Box::new(MockLibraryQueryStore::default()),
            Box::new(MockLibraryMutationStore::new()),
            Arc::new(AtomicBool::new(false)),
            move |_path| Vec::new(),
        );
        std::thread::spawn(move || worker.run());
        (scans, watch_rx)
    }

    /// Drain scan outcomes into `outcomes` until `expected` `Complete`
    /// outcomes for `root` have landed in total. Bounded so a wedged worker
    /// fails the test instead of hanging it.
    fn drain_until_completes(
        scans: &riff_backend::app::scan_service::ScanService,
        root: &std::path::Path,
        expected: usize,
        budget: std::time::Duration,
        outcomes: &mut Vec<riff_backend::app::scan_service::ScanOutcome>,
    ) {
        use riff_backend::app::scan_service::{ScanOutcome, Scans};
        use std::time::{Duration, Instant};

        let start = Instant::now();
        loop {
            outcomes.extend(scans.poll());
            let completes = outcomes
                .iter()
                .filter(
                    |o| matches!(o, ScanOutcome::Complete { path, .. } if path.as_path() == root),
                )
                .count();
            if completes >= expected {
                return;
            }
            assert!(
                start.elapsed() < budget,
                "only {completes}/{expected} completes for {root:?}; got {outcomes:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Wire a real Library Scan Service pair whose walk blocks until the
    /// returned release channel fires (to hold a scan open mid-flight),
    /// running the worker on its own thread like the composition root.
    fn gated_scan_service() -> (
        riff_backend::app::scan_service::ScanService,
        crossbeam_channel::Sender<()>,
    ) {
        use crate::mocks::{MockLibraryMutationStore, MockLibraryQueryStore, MockMetadataReader};
        use riff_backend::app::scan_service::ScanService;
        use std::sync::atomic::AtomicBool;

        let (release_tx, release_rx) = unbounded::<()>();
        let (scans, worker) = ScanService::new(
            Box::new(MockMetadataReader::default()),
            Box::new(MockLibraryQueryStore::default()),
            Box::new(MockLibraryMutationStore::new()),
            Arc::new(AtomicBool::new(false)),
            move |_path| {
                let _ = release_rx.recv();
                Vec::new()
            },
        );
        std::thread::spawn(move || worker.run());
        (scans, release_tx)
    }

    /// A [`WatcherManager`] over a real watcher with `root` registered as a
    /// watched library root. Returns the manager, the scratch dir backing it
    /// (must outlive the manager), and the canonicalized root.
    fn watched_manager(
        scans: riff_backend::app::scan_service::ScanService,
    ) -> (
        riff_backend::app::watcher_manager::WatcherManager,
        tempfile::TempDir,
        std::path::PathBuf,
    ) {
        use riff_backend::app::watcher_manager::WatcherManager;
        use riff_infra::filesystem::FilesystemWatcher;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // The event channel stays idle: tests feed synthetic batches that
        // stand in for debouncer flushes arriving over the boundary.
        let watcher = FilesystemWatcher::new(unbounded().0).expect("watcher must build");
        let mut mgr = WatcherManager::new(Some(Box::new(watcher)), scans);
        mgr.start_watching(dir.path()).expect("watchable root");
        (mgr, dir, root)
    }

    /// How many `Complete` outcomes in `outcomes` finished for `root`.
    fn completes_for(
        outcomes: &[riff_backend::app::scan_service::ScanOutcome],
        root: &std::path::Path,
    ) -> usize {
        use riff_backend::app::scan_service::ScanOutcome;
        outcomes
            .iter()
            .filter(|o| matches!(o, ScanOutcome::Complete { path, .. } if path.as_path() == root))
            .count()
    }

    #[test]
    fn test_watcher_manager_requests_rescan_immediately_per_debounced_batch() {
        let (scans, _watch_rx) = instant_scan_service();
        let (mut mgr, _dir, root) = watched_manager(scans.clone());

        // ONE debounced burst: several paths under the same root.
        let batch = vec![root.join("album").join("a.mp3"), root.join("b.mp3")];
        mgr.on_fs_events(&batch);

        // The batch was already debounced upstream, so the rescan decision
        // must happen NOW — no `poll()`, no second quiet-window wait.
        let mut outcomes = Vec::new();
        drain_until_completes(
            &scans,
            &root,
            1,
            std::time::Duration::from_secs(10),
            &mut outcomes,
        );
        assert_eq!(completes_for(&outcomes, &root), 1);
    }

    #[test]
    fn test_watcher_manager_burst_in_one_batch_triggers_exactly_one_rescan() {
        let (scans, _watch_rx) = instant_scan_service();
        let (mut mgr, _dir, root) = watched_manager(scans.clone());

        // A whole-album drop lands as one debounced flush of many paths.
        let batch: Vec<std::path::PathBuf> = (0..5)
            .map(|i| root.join(format!("track_{i}.mp3")))
            .collect();
        mgr.on_fs_events(&batch);

        let mut outcomes = Vec::new();
        drain_until_completes(
            &scans,
            &root,
            1,
            std::time::Duration::from_secs(10),
            &mut outcomes,
        );
        assert_eq!(
            completes_for(&outcomes, &root),
            1,
            "one coalesced burst must trigger exactly one rescan"
        );
    }

    #[test]
    fn test_watcher_manager_separate_batches_trigger_separate_rescans() {
        let (scans, _watch_rx) = instant_scan_service();
        let (mut mgr, _dir, root) = watched_manager(scans.clone());

        // Two bursts separated outside the debounce window arrive as two
        // distinct batches; each drives its own rescan decision.
        let mut outcomes = Vec::new();
        mgr.on_fs_events(&[root.join("first.mp3")]);
        drain_until_completes(
            &scans,
            &root,
            1,
            std::time::Duration::from_secs(10),
            &mut outcomes,
        );

        mgr.on_fs_events(&[root.join("second.mp3")]);
        drain_until_completes(
            &scans,
            &root,
            2,
            std::time::Duration::from_secs(10),
            &mut outcomes,
        );
        assert_eq!(
            completes_for(&outcomes, &root),
            2,
            "batches outside the window must trigger separate rescans"
        );
    }

    #[test]
    fn test_watcher_manager_changes_during_scan_defer_one_follow_up() {
        use riff_backend::app::scan_service::Scans;
        use std::time::{Duration, Instant};

        let (scans, release) = gated_scan_service();
        let (mut mgr, _dir, root) = watched_manager(scans.clone());

        // Start a scan and hold it open mid-walk.
        mgr.on_fs_events(&[root.join("initial.mp3")]);
        let start = Instant::now();
        while !scans.is_scanning(&root) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "scan never started"
            );
            std::thread::sleep(Duration::from_millis(2));
        }

        // A multi-path burst lands while the scan runs: exactly one
        // follow-up rescan is remembered for when it ends.
        mgr.on_fs_events(&[
            root.join("mid_1.mp3"),
            root.join("mid_2.mp3"),
            root.join("mid_3.mp3"),
        ]);

        release.send(()).expect("walk released");
        let mut outcomes = Vec::new();
        drain_until_completes(&scans, &root, 1, Duration::from_secs(10), &mut outcomes);

        // The UI's per-frame poll fires the single deferred follow-up; the
        // gated walk needs one more release to finish it.
        mgr.poll();
        release.send(()).expect("follow-up walk released");
        drain_until_completes(&scans, &root, 2, Duration::from_secs(10), &mut outcomes);
        assert_eq!(
            completes_for(&outcomes, &root),
            2,
            "mid-scan changes must collapse into exactly one follow-up"
        );
    }

    #[test]
    fn test_watcher_manager_stop_watching_drops_pending_follow_up() {
        use riff_backend::app::scan_service::Scans;
        use std::time::{Duration, Instant};

        let (scans, release) = gated_scan_service();
        let (mut mgr, dir, root) = watched_manager(scans.clone());

        mgr.on_fs_events(&[root.join("initial.mp3")]);
        let start = Instant::now();
        while !scans.is_scanning(&root) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "scan never started"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        mgr.on_fs_events(&[root.join("deferred.mp3")]);

        release.send(()).expect("walk released");
        let mut outcomes = Vec::new();
        drain_until_completes(&scans, &root, 1, Duration::from_secs(10), &mut outcomes);

        // Unwatching the root removes its pending state: no follow-up ever
        // fires, even though changes landed mid-scan.
        mgr.stop_watching(dir.path());
        mgr.poll();
        std::thread::sleep(Duration::from_millis(300));
        outcomes.extend(scans.poll());
        assert_eq!(
            completes_for(&outcomes, &root),
            1,
            "unwatch must remove the pending follow-up"
        );
    }

    #[test]
    fn test_watcher_manager_unwatchable_root_reports_warning_diagnostic() {
        use std::path::Path;

        let (scans, _watch_rx) = instant_scan_service();
        let (mut mgr, dir, _root) = watched_manager(scans);

        // A nonexistent path cannot be watched; the error text becomes the
        // UI's `WatchState::Warning` diagnostic.
        let missing = dir.path().join("does-not-exist");
        let err = mgr
            .start_watching(Path::new(&missing))
            .expect_err("unwatchable root must fail the start");

        assert!(
            err.contains("Watch failed") && err.len() > "Watch failed".len(),
            "warning diagnostic must carry the reason: {err}"
        );
    }

    #[test]
    fn test_audio_buffer_simulation() {
        // Simulate audio buffer operations (the real output buffer is
        // `Arc<Mutex<VecDeque<f32>>>`, so we mirror the element type here).
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};

        let buffer = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let samples = [0.1, 0.2, 0.3, 0.4];

        // Simulate writing samples
        {
            let mut buf = buffer.lock_or_recover();
            buf.extend(samples.iter());
        }

        // Simulate reading samples
        {
            let buf = buffer.lock_or_recover();
            assert_eq!(buf.len(), 4);
            assert!(crate::test_utils::float_close(buf[0], 0.1));
            assert!(crate::test_utils::float_close(buf[3], 0.4));
        }
    }
}

/// End-to-end proof for the Composition Root seam (backend-crate-split
/// issue 08): one constructor call wires real adapters into real ports,
/// spawns the worker threads, and the frontend-facing handles observe the
/// results through their existing public seams — never through internals.
#[cfg(test)]
mod composition_root_tests {
    use crate::app::MutexExt;
    use crate::domain::TrackId;
    use riff_backend::app::facade::{BackendEvent, NoticeSeverity, NoticeSource};
    use riff_backend::app::scan_service::{ScanOutcome, Scans};
    use riff_backend::composition::AppRuntime;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// Write a tiny but fully valid PCM WAV file (0.1 s of mono 8 kHz audio)
    /// so the real scanner and metadata-reader adapters have a real audio
    /// file to work on.
    fn write_minimal_wav(path: &Path) {
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
        std::fs::write(path, bytes).expect("temp WAV fixture must be writable");
    }

    /// Poll `probe` until it yields a value, bounded so a wedged worker
    /// thread fails the test instead of hanging it.
    fn poll_until<T>(budget: Duration, what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(value) = probe() {
                return value;
            }
            assert!(
                start.elapsed() < budget,
                "never observed {what} within budget"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn test_composition_root_spawns_real_adapters_and_worker_threads_run() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let music = dir.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_minimal_wav(&music.join("song_a.wav"));
        write_minimal_wav(&music.join("song_b.wav"));

        // The seam under test: one spawn over a scratch Application Store.
        let mut rt = AppRuntime::spawn(&db_path).expect("runtime must spawn over a fresh store");

        // The audio pipeline runs: a Play command for a missing track travels
        // the UI transport into the audio-engine thread, comes back as a
        // playback error, and lands on the facade as a typed notice relayed
        // by the coordinator thread.
        rt.ui_transport
            .play(TrackId("missing-track.mp3".to_string()));
        let notice = poll_until(
            Duration::from_secs(10),
            "typed playback error notice",
            || {
                rt.facade
                    .lock_or_recover()
                    .events()
                    .into_iter()
                    .find_map(|ev| match ev {
                        BackendEvent::TypedNotice(payload)
                            if payload.source == NoticeSource::Playback =>
                        {
                            Some(payload)
                        }
                        _ => None,
                    })
            },
        );
        assert_eq!(notice.severity, NoticeSeverity::Error);

        // The scan pipeline runs: real walker, real metadata reader, and
        // real durable SQLite commits on the scan-worker thread, driven
        // through the same `Scans` seam the UI holds.
        rt.scans.request(music.clone());
        let total = poll_until(Duration::from_secs(10), "scan complete", || {
            rt.scans
                .poll()
                .into_iter()
                .find_map(|outcome| match outcome {
                    ScanOutcome::Complete { path, total_files } if path == music => {
                        Some(total_files)
                    }
                    _ => None,
                })
        });
        assert_eq!(total, 2, "every fixture file discovered");

        // The read seam observes the committed scan through the store's
        // session generations, and the facade relays the library change the
        // store announced on its change channel.
        poll_until(
            Duration::from_secs(10),
            "committed tracks in SessionViews",
            || (rt.session_views.track_list("", 0).total == 2).then_some(()),
        );
        poll_until(
            Duration::from_secs(10),
            "LibraryChanged on the facade",
            || {
                rt.facade
                    .lock_or_recover()
                    .events()
                    .into_iter()
                    .any(|ev| matches!(ev, BackendEvent::LibraryChanged { .. }))
                    .then_some(())
            },
        );
    }
}
