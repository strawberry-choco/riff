#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_load_save_library_paths() {
        // Create a mock storage
        let mut storage = MockStorage::new();
        
        // Test saving and loading library paths
        let paths = vec![
            std::path::PathBuf::from("path1"),
            std::path::PathBuf::from("path2"),
        ];
        
        save_library_paths(&mut storage, &paths);
        let loaded_paths = load_library_paths(Some(&storage));
        
        assert_eq!(loaded_paths, paths);
    }
    
    #[test]
    fn test_load_save_volume() {
        let mut storage = MockStorage::new();
        
        // Test saving and loading volume
        let volume = 0.75;
        save_volume(&mut storage, volume);
        let loaded_volume = load_volume(Some(&storage));
        
        assert_eq!(loaded_volume, Some(volume));
    }
    
    #[test]
    fn test_load_save_watch_states() {
        let mut storage = MockStorage::new();
        
        // Test saving and loading watch states
        let mut states = std::collections::HashMap::new();
        states.insert(
            std::path::PathBuf::from("path1"),
            WatchState::Enabled,
        );
        
        save_watch_states(&mut storage, &states);
        let loaded_states = load_watch_states(Some(&storage));
        
        assert_eq!(loaded_states, states);
    }
    
    #[test]
    fn test_restore_from_backup_if_corrupted() {
        let mut storage = MockStorage::new();
        
        // Set up corrupted primary storage
        storage.data.insert("library_paths".to_string(), "invalid json".to_string());
        
        // Set up valid backup storage
        let valid_paths = serde_json::to_string(&vec!["path1".to_string(), "path2".to_string()])
            .unwrap();
        storage.data.insert("library_paths_backup".to_string(), valid_paths);
        
        // Restore from backup
        restore_from_backup_if_corrupted(&mut storage);
        
        // Verify the primary storage was restored
        let restored_paths = load_library_paths(Some(&storage));
        assert_eq!(restored_paths, vec![
            std::path::PathBuf::from("path1"),
            std::path::PathBuf::from("path2"),
        ]);
    }
    
    // Mock storage for testing
    struct MockStorage {
        data: std::collections::HashMap<String, String>,
    }
    
    impl MockStorage {
        fn new() -> Self {
            Self {
                data: std::collections::HashMap::new(),
            }
        }
    }
    
    impl eframe::Storage for MockStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.data.get(key).cloned()
        }
        
        fn set_string(&mut self, key: &str, value: &str) {
            self.data.insert(key.to_string(), value.to_string());
        }
        
        fn get_bool(&self, key: &str) -> Option<bool> {
            self.data.get(key)
                .and_then(|v| v.parse::<bool>().ok())
        }
        
        fn set_bool(&mut self, key: &str, value: bool) {
            self.data.insert(key.to_string(), value.to_string());
        }
        
        fn remove(&mut self, key: &str) {
            self.data.remove(key);
        }
    }
}