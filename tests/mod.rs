//! riff Music Player - Test Suite
//! 
//! This module contains comprehensive unit and integration tests for the riff music player.
//! 
//! # Test Organization
//! 
//! - `domain_tests.rs`: Tests for domain objects like Track, TrackId, PlaybackState, etc.
//! - `app_tests.rs`: Tests for application logic like AppState, LibraryManager, etc.
//! - `infra_tests.rs`: Tests for infrastructure components like audio decoders, metadata readers, etc.
//! - `ui_tests.rs`: Tests for UI-related functionality like settings storage, etc.
//! - `integration_tests.rs`: End-to-end integration tests that test multiple components together.
//! 
//! # Running Tests
//! 
//! ```bash
//! cargo test                # Run all tests
//! cargo test domain_tests   # Run specific test module
//! cargo test --lib          # Run only unit tests
//! cargo test -- --nocapture  # Run tests with output
//! ```

pub mod domain_tests;
pub mod app_tests;
pub mod infra_tests;
pub mod ui_tests;
pub mod integration_tests;

// Test utilities that can be used across test modules
pub mod test_utils {
    use std::path::PathBuf;
    use crate::domain::{TrackId, TrackMetadata};
    
    /// Create a test track with the given ID and file path
    pub fn create_test_track(id: &str, file_path: &str) -> crate::domain::Track {
        crate::domain::Track {
            id: TrackId::from(id),
            file_path: PathBuf::from(file_path),
            metadata: TrackMetadata::default(),
        }
    }
    
    /// Create a test track with custom metadata
    pub fn create_test_track_with_metadata(
        id: &str, 
        file_path: &str, 
        artist: &str, 
        title: &str, 
        album: &str
    ) -> crate::domain::Track {
        crate::domain::Track {
            id: TrackId::from(id),
            file_path: PathBuf::from(file_path),
            metadata: TrackMetadata {
                artist: Some(artist.to_string()),
                title: Some(title.to_string()),
                album: Some(album.to_string()),
                album_artist: None,
                genre: None,
                year: None,
                track_number: None,
                disc_number: None,
                duration: None,
            },
        }
    }
}

// Integration test helper functions
pub mod integration_helpers {
    use std::sync::{Arc, Mutex};
    use crate::app::state::AppState;
    
    /// Create a test AppState with some pre-populated data
    pub fn create_test_app_state() -> Arc<Mutex<AppState>> {
        let state = AppState::new();
        Arc::new(Mutex::new(state))
    }
    
    /// Create a mock library with some test tracks
    pub fn create_mock_library() -> crate::app::library_manager::LibraryManager {
        let mut library = crate::app::library_manager::LibraryManager::new();
        
        // Add some test tracks
        let track1 = super::test_utils::create_test_track_with_metadata(
            "track1.mp3",
            "music/artist1/album1/track1.mp3",
            "Artist 1",
            "Track 1",
            "Album 1"
        );
        
        let track2 = super::test_utils::create_test_track_with_metadata(
            "track2.mp3",
            "music/artist1/album1/track2.mp3",
            "Artist 1",
            "Track 2",
            "Album 1"
        );
        
        library.tracks.insert(track1.id.clone(), track1);
        library.tracks.insert(track2.id.clone(), track2);
        
        library
    }
}