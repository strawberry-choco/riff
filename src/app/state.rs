use std::path::PathBuf;
use crate::app::library_manager::LibraryManager;
use crate::domain::{TrackId, PlaybackQueue, PlaybackState, PlaybackPosition};

/// Central state for the entire application.
pub struct AppState {
    pub library: LibraryManager,
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    pub current_cover: Option<Vec<u8>>,
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub window_visible: bool,
    pub search_query: String,
    pub library_path: Option<PathBuf>,
    pub is_scanning: bool,
    pub scan_status: Option<String>,
    pub theme: Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Library,
    NowPlaying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            library: LibraryManager::new(),
            queue: PlaybackQueue::default(),
            playback_state: PlaybackState::Stopped,
            current_position: PlaybackPosition::default(),
            current_volume: 1.0,
            current_cover: None,
            selected_track: None,
            view_mode: ViewMode::Library,
            window_visible: true,
            search_query: String::new(),
            library_path: None,
            is_scanning: false,
            scan_status: None,
            theme: Theme::Dark,
        }
    }
}
