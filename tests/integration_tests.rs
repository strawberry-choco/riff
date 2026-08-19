#[cfg(test)]
mod tests {
    use crate::app::state::AppState;
    use crate::app::MutexExt;
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
    fn test_library_scan_simulation() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let (cmd_tx, cmd_rx) = unbounded::<crate::app::commands::LibraryCommand>();

        // Simulate a library scan
        let scan_path = std::path::PathBuf::from("test_directory");
        cmd_tx
            .send(crate::app::commands::LibraryCommand::ScanDirectory(
                scan_path.clone(),
            ))
            .unwrap();

        // Process the scan command (this would normally be done in a separate
        // thread). Use `try_recv` to drain the pending commands without blocking
        // once the channel is empty.
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let crate::app::commands::LibraryCommand::ScanDirectory(path) = cmd {
                let mut state = state.lock_or_recover();
                state
                    .library_statuses
                    .insert(path, crate::app::state::LibraryStatus::Scanned(10));
            }
        }

        // Verify the scan status was updated
        let state = state.lock_or_recover();
        assert!(state.library_statuses.contains_key(&scan_path));
        assert!(matches!(
            state.library_statuses[&scan_path],
            crate::app::state::LibraryStatus::Scanned(_)
        ));
    }

    #[test]
    fn test_settings_persistence_simulation() {
        // Simulate settings persistence across app restarts
        let mut storage_data = std::collections::HashMap::new();

        // Simulate saving settings
        let paths = vec![std::path::PathBuf::from("music")];
        let json = serde_json::to_string(&paths).unwrap();
        storage_data.insert("library_paths".to_string(), json);

        // Simulate loading settings
        if let Some(json) = storage_data.get("library_paths") {
            if let Ok(loaded_strings) = serde_json::from_str::<Vec<String>>(json) {
                let loaded_paths: Vec<std::path::PathBuf> = loaded_strings
                    .into_iter()
                    .map(std::path::PathBuf::from)
                    .collect();
                assert_eq!(loaded_paths, paths);
            }
        }
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
