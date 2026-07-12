use std::collections::HashMap;
use std::path::PathBuf;
use crate::app::library_manager::LibraryManager;
use crate::domain::{TrackId, PlaybackQueue, PlaybackState, PlaybackPosition};

#[derive(Debug, Clone, PartialEq)]
pub enum LibraryStatus {
    Idle,
    Scanning { files_found: usize },
    Scanned(usize),
    Unavailable,
}

impl Default for LibraryStatus {
    fn default() -> Self { Self::Idle }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseMode {
    Library,
    Folders,
}

impl Default for BrowseMode {
    fn default() -> Self { Self::Library }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WatchState {
    Disabled,
    Enabled,
    Warning(String),
}

impl Default for WatchState {
    fn default() -> Self { Self::Disabled }
}

pub struct AppState {
    pub library: LibraryManager,
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub window_visible: bool,
    pub search_query: String,
    pub library_paths: Vec<PathBuf>,
    pub library_statuses: HashMap<PathBuf, LibraryStatus>,
    pub scan_status: Option<String>,
    pub theme: Theme,
    pub browse_mode: BrowseMode,
    pub selected_folder: Option<PathBuf>,
    pub show_artists_view: bool,
    pub watch_states: HashMap<PathBuf, WatchState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Library,
    NowPlaying,
    Settings,
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
            selected_track: None,
            view_mode: ViewMode::Library,
            window_visible: true,
            search_query: String::new(),
            library_paths: Vec::new(),
            library_statuses: HashMap::new(),
            scan_status: None,
            theme: Theme::Dark,
            browse_mode: BrowseMode::default(),
            selected_folder: None,
            show_artists_view: false,
            watch_states: HashMap::new(),
        }
    }
}
