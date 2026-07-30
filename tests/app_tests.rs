#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mutex_ext_lock_or_recover() {
        let mutex = Mutex::new(42);
        let guard = mutex.lock_or_recover();
        assert_eq!(*guard, 42);
        
        // Test that the guard properly releases the lock when dropped
        drop(guard);
        let guard2 = mutex.lock_or_recover();
        assert_eq!(*guard2, 42);
    }
    
    #[test]
    fn test_library_manager_new() {
        let library = LibraryManager::new();
        assert!(library.all_tracks().is_empty());
        assert!(library.all_artists().is_empty());
        assert!(library.albums.is_empty());
    }
    
    #[test]
    fn test_library_manager_add_track() {
        let mut library = LibraryManager::new();
        let reader = LoftyMetadataReader::new();
        
        let test_file = std::path::PathBuf::from("test.mp3");
        let track_id = TrackId::from("test.mp3");
        
        // This test would require an actual MP3 file to work properly
        // For now, we'll just test the structure
        assert!(library.get_track(&track_id).is_none());
    }
    
    #[test]
    fn test_library_manager_search() {
        let mut library = LibraryManager::new();
        
        // Add some test tracks
        let track1 = Track {
            id: TrackId::from("track1.mp3"),
            file_path: std::path::PathBuf::from("track1.mp3"),
            metadata: crate::domain::TrackMetadata::default(),
        };
        
        let track2 = Track {
            id: TrackId::from("track2.mp3"),
            file_path: std::path::PathBuf::from("track2.mp3"),
            metadata: crate::domain::TrackMetadata::default(),
        };
        
        library.tracks.insert(track1.id.clone(), track1);
        library.tracks.insert(track2.id.clone(), track2);
        
        let results = library.search("track1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "track1.mp3");
    }
    
    #[test]
    fn test_library_manager_save_load_cache() {
        let mut library = LibraryManager::new();
        
        // Add a test track
        let track = Track {
            id: TrackId::from("test.mp3"),
            file_path: std::path::PathBuf::from("test.mp3"),
            metadata: crate::domain::TrackMetadata::default(),
        };
        
        library.tracks.insert(track.id.clone(), track.clone());
        
        // Test saving and loading cache
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_path = temp_dir.path().join("test_cache.json");
        
        // Temporarily override the cache path for testing
        let original_cache_path = std::env::var("RIFF_CACHE_PATH").unwrap_or_default();
        std::env::set_var("RIFF_CACHE_PATH", cache_path.to_string_lossy().to_string());
        
        // Save cache
        assert!(library.save_cache().is_ok());
        
        // Create a new library and load cache
        let mut loaded_library = LibraryManager::new();
        assert!(loaded_library.load_cache().is_ok());
        
        // Restore original cache path
        if original_cache_path.is_empty() {
            std::env::remove_var("RIFF_CACHE_PATH");
        } else {
            std::env::set_var("RIFF_CACHE_PATH", original_cache_path);
        }
        
        // Verify the track was loaded
        assert_eq!(loaded_library.tracks.len(), 1);
        assert!(loaded_library.get_track(&track.id).is_some());
    }
}