use crate::app::library_paths::LibraryPaths;
use crate::domain::TrackId;
use std::path::PathBuf;

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

/// Re-exported from `riff_playback::domain`.
pub use riff_playback::domain::{
    PlaybackCommand, PlaybackPosition, PlaybackQueue, PlaybackState, PlaybackUpdate, RepeatMode,
};

/// Re-exported from `riff_persistence::store`.
pub use riff_persistence::store::{ScalarSettings, WatchState};

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
        }
    }
}

/// Re-exported from `riff_playback::app::state` — the canonical playback
/// session the Transport, coordinator, and engine all take. The backend keeps
/// the library-side session types (`LibrarySession`, `ViewMode`, `UiFlags`).
pub use riff_playback::app::state::{PlaybackSession, replaygain_factor};

/// The Library Session: everything that is not playback — selection, views,
/// search, the registered [`LibraryPaths`] and their readiness and watch
/// states, scan status, browse mode, and UI flags. Lives behind its own
/// `Arc<Mutex<>>`.
pub struct LibrarySession {
    pub selected_track: Option<TrackId>,
    pub view_mode: ViewMode,
    pub search_query: String,
    /// The registered roots with their Readiness and Watch States — one value,
    /// because one fact-set moves together (see [`crate::app::library_paths`]).
    pub library_paths: LibraryPaths,
    pub scan_status: Option<String>,
    pub browse_mode: BrowseMode,
    /// Which LIBRARY section the sidebar has selected (see
    /// [`LibrarySection`]) — the browser variant the sidebar rows open.
    pub library_section: LibrarySection,
    pub selected_folder: Option<PathBuf>,
    /// `true` when the browser column's A–Z sort is flipped to Z–A (issue
    /// 08). Session state, not persisted — the design pins no default past
    /// A–Z.
    pub browser_sort_desc: bool,
    /// The ordered drill-down path of entity selections (issue 08), deepest
    /// entry last: what the detail column (issue 09) resolves from the
    /// current (deepest) selection. Selecting at a level truncates any
    /// deeper entries; section or browse-mode navigation resets the path.
    /// Volatile session state, never persisted.
    pub browser_path: Vec<BrowserSelection>,
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
    /// `true` = the Smart Lists sidebar section is folded to its header
    /// (persisted display preference, restored on launch).
    pub smart_lists_collapsed: bool,
    /// `true` = the custom title-bar close button quits the app instead of
    /// minimizing it to the system tray (persisted; inert on Linux, which has
    /// no tray). `false` (the default) keeps the minimize-to-tray behavior.
    pub close_quits_app: bool,
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
            library_paths: LibraryPaths::default(),
            scan_status: None,
            browse_mode: BrowseMode::default(),
            library_section: LibrarySection::default(),
            selected_folder: None,
            browser_sort_desc: false,
            browser_path: Vec::new(),
            queue_open: false,
            ui_flags: UiFlags::default(),
            scan_prefs: ScanPrefs::default(),
        }
    }
}

impl LibrarySession {
    /// Select an entity at drill-down level `level`: truncates the path to
    /// its first `level` entries (dropping any deeper ones) and appends
    /// `selection`, so the path ends at `level + 1` entries with the new
    /// selection deepest. An entity selection also clears the selected track:
    /// the two selections are mutually exclusive, so the detail panel follows
    /// whichever the user clicked last — a track row or an entity row.
    pub fn select_at(&mut self, level: usize, selection: BrowserSelection) {
        self.browser_path.truncate(level);
        self.browser_path.push(selection);
        self.selected_track = None;
    }

    /// The deepest entity in the drill-down path — the selection the detail
    /// column resolves — or `None` while the path is empty.
    pub fn current_selection(&self) -> Option<&BrowserSelection> {
        self.browser_path.last()
    }

    /// Keep only the first `level` entries of the drill-down path (a
    /// breadcrumb climb); `level` past the current length changes nothing.
    pub fn truncate_path(&mut self, level: usize) {
        self.browser_path.truncate(level);
    }

    /// Clear the drill-down path entirely (a section or browse-mode switch
    /// starts navigation over at the root listing).
    pub fn reset_browser_path(&mut self) {
        self.browser_path.clear();
    }
}
