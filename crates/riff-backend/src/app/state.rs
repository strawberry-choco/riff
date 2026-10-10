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
pub use riff_playback::app::state::{
    PlaybackSession, REPLAYGAIN_GAIN_LIMIT_DB, ReplayGainMode, replaygain_factor,
};

/// The sort mode of a track listing — the Tracks column, All Tracks, a user
/// playlist, or a smart list. The default is each listing's canonical order:
/// track-number ascending within an album, playlist order for a playlist,
/// path order for the flat list. `TrackSort::as_sort_key` feeds the Scroll
/// Memory's content fingerprint, so the discriminants are part of its
/// contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TrackSort {
    /// The listing's canonical order, ascending.
    #[default]
    NumberAsc = 0,
    /// The canonical order reversed.
    NumberDesc = 1,
    /// Title A–Z (case-insensitive; ties keep the canonical order).
    TitleAsc = 2,
    /// Title Z–A (case-insensitive; ties keep the canonical order).
    TitleDesc = 3,
}

impl TrackSort {
    /// The Scroll Memory fingerprint key for this mode: two listings render
    /// the same content identity only when this value matches, so a sort
    /// change resets the remembered scroll instead of restoring a stale
    /// offset.
    #[must_use]
    pub fn as_sort_key(self) -> u8 {
        self as u8
    }
}

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
    /// A–Z. Shared by the section root columns (Artists, Albums, Genres),
    /// exactly like the one sort control they render.
    pub browser_sort_desc: bool,
    /// `true` when the browser's drill columns' A–Z sort is flipped to Z–A.
    /// Session state, not persisted, and shared across the drill columns
    /// (an artist's Albums, a Genre's Artists, a Genre artist's Albums) the
    /// same way [`Self::browser_sort_desc`] is shared across the roots —
    /// every drill column's rows are name-keyed, so one direction answers
    /// for all of them.
    pub drill_sort_desc: bool,
    /// The track listings' sort mode (the Tracks column, All Tracks, user
    /// playlists, smart lists). Session state, not persisted; the default is
    /// each listing's canonical order. One mode answers for every track
    /// listing, like the one shared direction the entity columns share.
    pub track_sort: TrackSort,
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
    /// The `ReplayGain` Pass's Settings gating (see [`PassPrefs`]): which value
    /// kinds a library-wide pass writes, on demand and after every Library
    /// Scan. Force is not persisted and lives with the Settings surface.
    pub pass_prefs: PassPrefs,
}

/// The `ReplayGain` Pass's Settings gating, hydrated from the Application
/// Store's scalar row at startup and written back on every change. Both
/// default off: a scan's behavior is unchanged until the listener opts in.
/// Menu commands never read this — a targeted pass always does exactly what
/// its label says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassPrefs {
    /// Library-wide passes write each measured Track's own pair.
    pub track_values: bool,
    /// Library-wide passes write each affected Album's aggregate to its
    /// Tracks.
    pub album_values: bool,
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
    /// Accessibility flag: when `true` the UI suppresses non-essential motion
    /// (view transitions, animated reveals) in favor of instant changes,
    /// honouring the listener's system reduce-motion preference. `false`
    /// (the default) leaves motion as it is (persisted, restored on launch).
    pub reduce_motion: bool,
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
            drill_sort_desc: false,
            track_sort: TrackSort::default(),
            browser_path: Vec::new(),
            queue_open: false,
            ui_flags: UiFlags::default(),
            scan_prefs: ScanPrefs::default(),
            pass_prefs: PassPrefs::default(),
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

    /// Clear the drill-down path entirely (a section or browse-mode switch
    /// starts navigation over at the root listing).
    pub fn reset_browser_path(&mut self) {
        self.browser_path.clear();
    }
}
