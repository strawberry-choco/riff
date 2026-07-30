#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use crossbeam_channel::unbounded;
    use crate::app::state::AppState;
    use crate::app::MutexExt;
    use crate::domain::{PlaybackCommand, PlaybackState, TrackId};
    
    #[test]
    fn test_app_state_mutex_safety() {
        let state = Arc::new(Mutex::new(AppState::new()));
        
        // Test that multiple threads can safely access the state
        let state_clone = state.clone();
        let handle = std::thread::spawn(move || {
            let mut state = state_clone.lock_or_recover();
            state.playback_state = PlaybackState::Playing;
            state.current_volume = 0.8;
        });
        
        // Main thread also accesses the state
        let mut state = state.lock_or_recover();
        assert_eq!(state.playback_state, PlaybackState::Stopped);
        assert_eq!(state.current_volume, 1.0);
        
        // Wait for the other thread to complete
        handle.join().unwrap();
        
        // Verify the changes from the other thread
        let mut state = state.lock_or_recover();
        assert_eq!(state.playback_state, PlaybackState::Playing);
        assert_eq!(state.current_volume, 0.8);
    }
    
    #[test]
    fn test_playback_command_channel() {
        let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
        let (update_tx, update_rx) = unbounded::<crate::domain::PlaybackUpdate>();
        
        // Test sending and receiving commands
        let track_id = TrackId::from("test.mp3");
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
        cmd_tx.send(crate::app::commands::LibraryCommand::ScanDirectory(scan_path.clone()))
            .unwrap();
        
        // Process the scan command (this would normally be done in a separate thread)
        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                crate::app::commands::LibraryCommand::ScanDirectory(path) => {
                    let mut state = state.lock_or_recover();
                    state.library_statuses.insert(path, crate::app::state::LibraryStatus::Scanned(10));
                }
                _ => {}
            }
        }
        
        // Verify the scan status was updated
        let state = state.lock_or_recover();
        assert!(state.library_statuses.contains_key(&scan_path));
        assert!(matches!(state.library_statuses[&scan_path], crate::app::state::LibraryStatus::Scanned(_)));
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
            if let Ok(paths) = serde_json::from_str::<Vec<String>>(json) {
                let loaded_paths: Vec<std::path::PathBuf> = paths.into_iter().map(std::path::PathBuf::from).collect();
                assert_eq!(loaded_paths, paths);
            }
        }
    }
    
    #[test]
    fn test_audio_buffer_simulation() {
        // Simulate audio buffer operations
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};
        
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        
        // Simulate writing samples
        {
            let mut buf = buffer.lock_or_recover();
            buf.extend(samples.iter());
        }
        
        // Simulate reading samples
        {
            let buf = buffer.lock_or_recover();
            assert_eq!(buf.len(), 4);
            assert_eq!(buf[0], 0.1);
            assert_eq!(buf[3], 0.4);
        }
    }
}