#[cfg(test)]
mod tests {
    use crate::app::MutexExt;
    use crate::app::state::AppState;
    use crate::domain::{PlaybackCommand, PlaybackState, TrackId};
    use crossbeam_channel::unbounded;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_app_state_mutex_safety() {
        let state = Arc::new(Mutex::new(AppState::new()));

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
        use riff::app::scan_service::{ScanOutcome, ScanService, Scans};
        use riff::app::store::{LibraryQueryStore, StoreGeneration};
        use riff::infra::AudioFileScanner;
        use riff::infra::store::{MutexLibraryMutationStore, MutexLibraryQueryStore, SqliteStore};
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let shared = Arc::new(Mutex::new(
            SqliteStore::open_and_migrate(&db_path).expect("fresh store must open and migrate"),
        ));
        let mutations = MutexLibraryMutationStore::new(shared.clone(), StoreGeneration::new());
        let queries = MutexLibraryQueryStore::new(shared.clone());

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
