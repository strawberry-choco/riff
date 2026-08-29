use crate::domain::{PlaybackPosition, PlaybackQueue, PlaybackState, PlaybackUpdate, RepeatMode};
use riff_persistence::track::TrackId;
use riff_persistence::store::{ScalarSettings, WatchState};
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

/// Re-exported from `riff_persistence::store`.

/// The Playback Session: exactly the fields the audio engine, playback
/// coordinator, and transport touch. Lives behind its own `Arc<Mutex<>>`,
/// separate from the Library Session, so no code path ever holds both
/// session locks at once.
///
/// The UI reads this through a per-frame [`Clone`] snapshot and writes back
/// only the UI-owned fields (volume, mute, shuffle, repeat, replay-gain) at
/// frame end — the engine and coordinator write `playback_state`,
/// `current_position`, and the queue's traversal index, none of which the UI
/// ever mutates, so the targeted write-back cannot clobber them.
#[derive(Debug, Clone)]
pub struct PlaybackSession {
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    /// Mute flag: independent of `current_volume` — the slider keeps its
    /// value while muted. The engine always receives
    /// [`Self::effective_volume`], so a muted app stays silent until unmuted.
    pub muted: bool,
    /// `ReplayGain` flag: opt-in loudness normalization. When `true`, the
    /// engine applies each track's `REPLAYGAIN_TRACK_GAIN` (peak-capped) in
    /// the audio output's volume-scaling step.
    pub replaygain_enabled: bool,
}

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
    Tracks,
    Artists,
    Albums,
    Playlists,
}

impl Default for PlaybackSession {
    fn default() -> Self {
        Self {
            queue: PlaybackQueue::default(),
            playback_state: PlaybackState::Stopped,
            current_position: PlaybackPosition::default(),
            current_volume: 1.0,
            muted: false,
            replaygain_enabled: false,
        }
    }
}

impl Default for LibrarySession {
    fn default() -> Self {
        Self {
            selected_track: None,
            view_mode: ViewMode::Tracks,
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

impl PlaybackSession {
    /// Effective volume the engine should use (respects mute).
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.current_volume
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