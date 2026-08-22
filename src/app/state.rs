use crate::app::library_manager::LibraryManager;
use crate::domain::{PlaybackPosition, PlaybackQueue, PlaybackState, Playlist, TrackId};
use std::collections::HashMap;
use std::path::PathBuf;

/// Compute the linear playback-gain multiplier for `ReplayGain` (Task 4.3).
///
/// Disabled, or no gain tag → `1.0` (no adjustment). Otherwise the dB value is
/// converted to a linear factor (`10^(dB/20)`). When a peak is known and
/// positive, the factor is capped at `1.0 / peak` so `factor * peak <= 1.0`
/// and amplified samples cannot clip. Pure f32 math — no external crates.
pub fn replaygain_factor(enabled: bool, gain_db: Option<f32>, peak: Option<f32>) -> f32 {
    if !enabled {
        return 1.0;
    }
    let Some(g) = gain_db else {
        return 1.0;
    };
    let mut linear = 10f32.powf(g / 20.0);
    if let Some(p) = peak {
        if p > 0.0 {
            linear = linear.min(1.0 / p);
        }
    }
    linear
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum LibraryStatus {
    #[default]
    Idle,
    Scanning {
        files_found: usize,
    },
    Scanned(usize),
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowseMode {
    #[default]
    Library,
    Folders,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WatchState {
    #[default]
    Disabled,
    Enabled,
    Warning(String),
}

pub struct AppState {
    pub library: LibraryManager,
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    /// Mute flag (REQ-UI-003-08): independent of `current_volume` — the slider
    /// keeps its value while muted. The engine always receives
    /// [`Self::effective_volume`], so a muted app stays silent until unmuted.
    pub muted: bool,
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub window_visible: bool,
    pub search_query: String,
    pub library_paths: Vec<PathBuf>,
    pub library_statuses: HashMap<PathBuf, LibraryStatus>,
    pub scan_status: Option<String>,
    pub browse_mode: BrowseMode,
    pub selected_folder: Option<PathBuf>,
    /// Library-browser and accessibility flags, grouped to keep `AppState`
    /// cohesive (see [`UiFlags`]).
    pub ui_flags: UiFlags,
    pub watch_states: HashMap<PathBuf, WatchState>,
    /// `ReplayGain` flag (Task 4.3): opt-in loudness normalization. When
    /// `true`, the engine applies each track's `REPLAYGAIN_TRACK_GAIN`
    /// (peak-capped) in the audio output's volume-scaling step.
    pub replaygain_enabled: bool,
    /// User playlists (Task 4.2). Session Projection of the Application
    /// Store's Playlists section: refreshed from the store through the
    /// `PlaylistStore` port, never authoritative. Playlists survive a Clear
    /// Library (which wipes collection data only).
    pub playlists: Vec<Playlist>,
}

/// The single-row scalar preferences persisted in the Application Store's
/// typed settings table. Volume is `None` while the user has not yet moved
/// the slider, so the caller applies its own default.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScalarSettings {
    pub volume: Option<f32>,
    pub advanced_mode: bool,
    pub high_contrast: bool,
    pub replaygain_enabled: bool,
}

/// UI display flags grouped out of [`AppState`] so the top-level state struct
/// stays cohesive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiFlags {
    /// Library explorer sub-view: `true` shows the Artists hierarchy instead
    /// of the flat All Tracks list.
    pub show_artists_view: bool,
    /// Progressive disclosure flag (REQ-UI-006): when `false` the UI stays
    /// minimal and hides power features (tag editing, smart playlists,
    /// stop/repeat transport controls) behind an explicit, persisted toggle.
    pub advanced_mode: bool,
    /// Accessibility flag (REQ-UI-007): when `true` the UI uses a persisted
    /// high-contrast theme (near-black background, white text, bright focus
    /// outlines) instead of the elegance light/dark theme.
    pub high_contrast: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Library,
    NowPlaying,
    Settings,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            library: LibraryManager::new(),
            queue: PlaybackQueue::default(),
            playback_state: PlaybackState::Stopped,
            current_position: PlaybackPosition::default(),
            current_volume: 1.0,
            muted: false,
            selected_track: None,
            view_mode: ViewMode::Library,
            window_visible: true,
            search_query: String::new(),
            library_paths: Vec::new(),
            library_statuses: HashMap::new(),
            scan_status: None,
            browse_mode: BrowseMode::default(),
            selected_folder: None,
            ui_flags: UiFlags::default(),
            watch_states: HashMap::new(),
            replaygain_enabled: false,
            playlists: Vec::new(),
        }
    }

    /// The volume the audio engine should apply: `0.0` while muted, otherwise
    /// `current_volume`. Muting never moves the slider; unmuting restores it.
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.current_volume
        }
    }
}
