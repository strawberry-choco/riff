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

/// Which LIBRARY section the sidebar has selected (design-handoff issue 07):
/// the browser variant the sidebar rows open. All Tracks, Artists, Albums,
/// and Genres browse the Library mode; Folders is its own
/// [`BrowseMode::Folders`] mode. The browser column (issue 08) renders one
/// listing per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibrarySection {
    /// The flat all-tracks list.
    #[default]
    AllTracks,
    /// The artist/album hierarchy.
    Artists,
    /// Album browsing (until the browser column lands, served by the
    /// artist/album hierarchy).
    Albums,
    /// Genre browsing backed by the genre read model.
    Genres,
}

/// What the listener selected in the browser column (design-handoff issue
/// 08): the identity the detail column (issue 09) and selection panel
/// (issue 10) resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserSelection {
    /// An artist row, by album-artist name.
    Artist(String),
    /// An album row, by the store's `(album artist, title)` identity.
    Album { artist: String, title: String },
    /// A genre row, by genre name.
    Genre(String),
}

/// The album the selection panel (design-handoff issue 10) reads out: the
/// last album identity selected anywhere in the browser — a browser column
/// row or a detail-column drill. A non-album selection (artist, genre) and
/// plain section navigation never clear it, so the panel shows a coherent
/// readout instead of blanking or showing stale state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumSelection {
    /// The album's album-artist name (the store's `(album artist, title)`
    /// identity).
    pub artist: String,
    /// The album's title.
    pub title: String,
}

/// How the library browser column renders its entries (design-handoff issue
/// 06): a flat list or a grid of cards. The top bar's list/grid toggle writes
/// it; the browser column (issue 08) reads it. Persisted through
/// [`crate::app::store::SettingsStore`] so the choice survives restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserLayout {
    #[default]
    List,
    Grid,
}

impl BrowserLayout {
    /// The store encoding persisted in the scalar settings row
    /// (`0` = list, `1` = grid — the same boolean-column convention as the
    /// other `app_settings` toggles).
    #[must_use]
    pub fn as_store_code(self) -> i64 {
        match self {
            Self::List => 0,
            Self::Grid => 1,
        }
    }

    /// Decode a stored scalar code; unknown values fall back to the list
    /// default so a hand-edited store can never break the session.
    #[must_use]
    pub fn from_store_code(code: i64) -> Self {
        if code == 1 { Self::Grid } else { Self::List }
    }
}

/// Re-exported from `riff_playback::domain`.
pub use riff_playback::domain::{
    PlaybackCommand, PlaybackPosition, PlaybackQueue, PlaybackState, PlaybackUpdate, RepeatMode,
};

/// Re-exported from `riff_persistence::store`.
pub use riff_persistence::store::{MissingArtworkStrategy, ScalarSettings, WatchState};

/// The Library Scan preferences the Settings Library pane drives
/// (design-handoff issue 12), hydrated from the Application Store's scalar
/// row at startup and written back on every change. The default mirrors the
/// scanner's historical behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanPrefs {
    /// Skip hidden (dot-prefixed) files and directories during scans.
    pub skip_hidden_files: bool,
    /// The enabled audio extensions, lowercase without dots — which file
    /// types are indexed on the next scan.
    pub scan_formats: Vec<String>,
    /// Read artwork embedded in track tags before filesystem fallbacks.
    pub read_embedded_artwork: bool,
    /// What renders for Tracks and Albums with no artwork.
    pub missing_artwork_strategy: MissingArtworkStrategy,
}

impl Default for ScanPrefs {
    fn default() -> Self {
        Self {
            skip_hidden_files: true,
            scan_formats: riff_persistence::store::AUDIO_EXTENSIONS
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
            read_embedded_artwork: true,
            missing_artwork_strategy: MissingArtworkStrategy::default(),
        }
    }
}

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
    /// Which LIBRARY section the sidebar has selected (see
    /// [`LibrarySection`]) — the browser variant the sidebar rows open.
    pub library_section: LibrarySection,
    pub selected_folder: Option<PathBuf>,
    /// How the browser column renders (list vs. grid, issue 06): the top
    /// bar's toggle writes it, the browser column reads it, and it persists
    /// through the settings store.
    pub browser_layout: BrowserLayout,
    /// `true` when the browser column's A–Z sort is flipped to Z–A (issue
    /// 08). Session state, not persisted — the design pins no default past
    /// A–Z.
    pub browser_sort_desc: bool,
    /// The genre chip narrowing the browser column's artist/album listings
    /// (issue 08). Session state; `None` is no filter.
    pub genre_filter: Option<String>,
    /// What the listener selected in the browser column (issue 08) — the
    /// identity the detail column (issue 09) resolves.
    pub browser_selection: Option<BrowserSelection>,
    /// The last album selected anywhere in the browser (issue 10) — the
    /// identity the selection panel resolves. See [`AlbumSelection`].
    pub selected_album: Option<AlbumSelection>,
    /// Whether the player bar's queue panel (design-handoff issue 13) is
    /// open over the shell. Session state, not persisted — the design pins
    /// no default past closed.
    pub queue_open: bool,
    /// Library-browser and accessibility flags, grouped to keep the session
    /// cohesive (see [`UiFlags`]).
    pub ui_flags: UiFlags,
    /// The Library Scan preferences (see [`ScanPrefs`]) the Settings Library
    /// pane drives and the scan/cover workers honor.
    pub scan_prefs: ScanPrefs,
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
    /// Progressive disclosure flag (REQ-UI-006): when `false` the UI stays
    /// minimal and hides power features (tag editing, the Advanced-only
    /// smart lists, stop/repeat transport controls) behind an explicit,
    /// persisted toggle.
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
            library_section: LibrarySection::default(),
            selected_folder: None,
            browser_layout: BrowserLayout::default(),
            browser_sort_desc: false,
            genre_filter: None,
            browser_selection: None,
            selected_album: None,
            queue_open: false,
            ui_flags: UiFlags::default(),
            scan_prefs: ScanPrefs::default(),
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
