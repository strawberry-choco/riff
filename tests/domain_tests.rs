#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_app_state_new() {
        let state = AppState::new();
        assert_eq!(state.playback_state, PlaybackState::Stopped);
        assert_eq!(state.current_volume, 1.0);
        assert!(state.library_paths.is_empty());
        assert!(state.library_statuses.is_empty());
        assert!(state.watch_states.is_empty());
    }
    
    #[test]
    fn test_playback_queue_operations() {
        let mut queue = PlaybackQueue::default();
        let track1 = TrackId::from("track1.mp3");
        let track2 = TrackId::from("track2.mp3");
        
        // Test initial state
        assert!(queue.current_track().is_none());
        assert!(queue.next().is_none());
        assert!(queue.previous().is_none());
        
        // Test adding tracks
        queue.append(track1.clone());
        queue.append(track2.clone());
        
        assert_eq!(queue.tracks.len(), 2);
        assert_eq!(queue.current_index, None);
        
        // Test setting current track
        queue.set_current_index(0);
        assert_eq!(queue.current_index, Some(0));
        assert_eq!(queue.current_track(), Some(&track1));
        
        // Test next track
        assert_eq!(queue.next(), Some(&track2));
        
        // Test previous track
        queue.set_current_index(1);
        assert_eq!(queue.previous(), Some(&track1));
    }
    
    #[test]
    fn test_playback_state_display() {
        assert_eq!(format!("{}", PlaybackState::Stopped), "Stopped");
        assert_eq!(format!("{}", PlaybackState::Playing), "Playing");
        assert_eq!(format!("{}", PlaybackState::Paused), "Paused");
    }
    
    #[test]
    fn test_track_id_from_path() {
        let track_id = TrackId::from("path/to/track.mp3");
        assert_eq!(track_id.0, "path/to/track.mp3");
    }
    
    #[test]
    fn test_track_display_methods() {
        let track = Track {
            id: TrackId::from("test.mp3"),
            file_path: std::path::PathBuf::from("test.mp3"),
            metadata: crate::domain::TrackMetadata::default(),
        };
        
        assert_eq!(track.metadata.display_artist(), "Unknown Artist");
        assert_eq!(track.metadata.display_title(&track.file_path), "Unknown Title");
        assert_eq!(track.metadata.display_album(), "Unknown Album");
    }
}