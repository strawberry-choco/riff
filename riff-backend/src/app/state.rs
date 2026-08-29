use crate::domain::TrackId;
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
    if let Some(p) = peak
        && p > 0.0
    {
        linear = linear.min(1.0 / p);
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

/// Re-exported from `riff_playback::domain`.
pub use riff_playback::domain::{
    PlaybackCommand, PlaybackPosition, PlaybackQueue, PlaybackState, PlaybackUpdate, RepeatMode,
};

/// Re-exported from `riff_persistence::store`.
pub use riff_persistence::store::{ScalarSettings, WatchState};

/// Re-exported from `riff_playback::app::state` — the canonical playback
/// session the Transport, coordinator, and engine all take. The backend keeps
/// the library-side session types (`LibrarySession`, `ViewMode`, `UiFlags`).
pub use riff_playback::app::state::PlaybackSession;

/// The Library Session: everything that is not playback — selection, views,
/// search, library roots and their statuses, scan status, browse mode, UI
/// flags, and per-root watch states. Lives behind its own `Arc<Mutex<>>`.
pub struct LibrarySession {
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub search_query: String,
    pub library_paths: Vec<PathBuf>,
    pub library_statuses: HashMap<PathBuf, LibraryStatus>,
    pub scan_status: Option<String>,
    pub browse_mode: BrowseMode,
    pub selected_folder: Option<PathBuf>,
    /// Library-browser and accessibility flags, grouped to keep the session
    /// cohesive (see [`UiFlags`]).
    pub ui_flags: UiFlags,
    pub watch_states: HashMap<PathBuf, WatchState>,
}

/// UI display flags grouped out of [`LibrarySession`] so the top-level state
/// struct stays cohesive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each persisted display preference is an independent toggle"
)]
pub struct UiFlags {
    /// Library explorer sub-view: `true` shows the Artists hierarchy instead
    /// of the flat All Tracks list.
    pub show_artists_view: bool,
    /// Progressive disclosure flag (REQ-UI-006): when `false` the UI stays
    /// minimal and hides power features (tag editing, smart playlists,
    /// stop/repeat transport controls) behind an explicit, persisted toggle.
    pub advanced_mode: bool,
    /// Accessibility flag (REQ-UI-007): when `true` the UI uses a persisted
    /// high-contrast theme (extreme text, strong borders, bright focus
    /// outlines) as a variant over the regular light/dark palette.
    pub high_contrast: bool,
    /// `true` = compact list density; `false` = comfortable density.
    pub compact_density: bool,
    /// `true` = show track numbers in the list.
    pub show_track_numbers: bool,
    /// `true` = show album art thumbnails in the list.
    pub show_artwork: bool,
    /// `true` = show duration column.
    pub show_duration: bool,
    /// `true` = show play count column.
    pub show_play_count: bool,
    /// `true` = show date added column.
    pub show_date_added: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Library,
    NowPlaying,
    Settings,
}

impl Default for LibrarySession {
    fn default() -> Self {
        Self {
            selected_track: None,
            view_mode: ViewMode::Library,
            search_query: String::new(),
            library_paths: Vec::new(),
            library_statuses: HashMap::new(),
            scan_status: None,
            browse_mode: BrowseMode::default(),
            selected_folder: None,
            ui_flags: UiFlags::default(),
            watch_states: HashMap::new(),
        }
    }
}

impl LibrarySession {
    /// Watch state for a given root, defaulting to `Disabled`.
    pub fn watch_state(&self, root: &PathBuf) -> WatchState {
        self.watch_states
            .get(root)
            .cloned()
            .unwrap_or(WatchState::Disabled)
    }
}
