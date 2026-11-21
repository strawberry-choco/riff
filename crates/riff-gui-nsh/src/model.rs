//! The view vocabulary: the shapes the bridge populates from the backend and
//! the screens render from (plan "Bridge surface"). Every type here is what
//! one signal carries — [`PlaybackView`] the plan's literal, the rest only as
//! wide as today's reads can fill them. N13 widens [`SettingsSnapshot`] to
//! the settings pages; the deferred fields are listed on each type.

use std::sync::Arc;
use std::time::SystemTime;

use riff_backend::app::state::{
    BrowseMode, LibrarySection, LibrarySession, PlaybackState, RepeatMode, ReplayGainMode,
    ScanPrefs, UiFlags, ViewMode,
};
use riff_backend::app::views::{PlaylistView, SessionViews};
use riff_backend::domain::{PlaylistId, SmartPlaylistKind, Track};

/// One playback snapshot, exactly the plan's lines 200-213.
///
/// [`PartialEq`] is manual on purpose: `Track` carries no `PartialEq`, and
/// the pump reuses the previous snapshot's `Arc`s whenever the playing
/// identity is unchanged, so pointer identity *is* the view's equality — a
/// refetched-but-unequal `Arc` means fresh metadata that a binding must see.
#[derive(Debug, Clone)]
pub struct PlaybackView {
    pub state: PlaybackState,
    /// Quantized to a tenth of a second by the pump: while playing, the
    /// session's position moves every tick, and the signal's equality would
    /// otherwise notify sixty times a second for the same tenth on screen.
    pub position_secs: f32,
    pub duration_secs: Option<f32>,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub current: Option<Arc<Track>>,
    pub up_next: Arc<[Track]>,
    /// The playing position in the queue; the session's `None` (nothing
    /// playing) reads as `0`.
    pub queue_index: usize,
    pub queue_len: usize,
}

impl PartialEq for PlaybackView {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
            && self.position_secs == other.position_secs
            && self.duration_secs == other.duration_secs
            && self.volume == other.volume
            && self.muted == other.muted
            && self.shuffle == other.shuffle
            && self.repeat == other.repeat
            && match (&self.current, &other.current) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && Arc::ptr_eq(&self.up_next, &other.up_next)
            && self.queue_index == other.queue_index
            && self.queue_len == other.queue_len
    }
}

impl Default for PlaybackView {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            position_secs: 0.0,
            duration_secs: None,
            volume: 1.0,
            muted: false,
            shuffle: false,
            repeat: RepeatMode::None,
            current: None,
            up_next: Arc::from([]),
            queue_index: 0,
            queue_len: 0,
        }
    }
}

/// The sidebar's counts, row lists and freshness — the facts `SessionViews`
/// serves. [`NavView`] carries where the app stands (the selection writes);
/// this view carries only what the store answers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SidebarView {
    pub tracks: usize,
    pub artists: usize,
    pub albums: usize,
    pub genres: usize,
    pub folder_roots: usize,
    /// Each built-in smart list's total, in `SmartPlaylistKind::ALL` order —
    /// `SessionViews::sidebar_counts`'s own shape.
    pub smart_lists: Arc<[(SmartPlaylistKind, usize)]>,
    /// The user playlists' rows, in creation order.
    pub playlists: Arc<[PlaylistRow]>,
    /// When the last full scan finished; `None` = never.
    pub last_scan: Option<SystemTime>,
}

/// One PLAYLISTS row: the identity a selection write opens the list with,
/// the name it paints, and its entry count — the egui sidebar's
/// `"Name (count)"` label facts. The full entry list stays in the
/// projection; a nav row needs none of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistRow {
    pub id: PlaylistId,
    pub name: String,
    /// Entry count, dangling references included — the egui row's
    /// `playlist.tracks.len()`.
    pub tracks: usize,
}

/// Where the app's navigation stands: the Library session's view fields as
/// the shell's selection handlers last wrote them, plus the two open-list
/// slots the egui frame kept as frontend state (`parts::smart_playlist_view`
/// and `parts::playlist_view`). The handlers write both sides together —
/// the session through its guard, this view through the signal — so the
/// sidebar's active-row highlight survives a pump tick without any session
/// lock in a render binding (the pump never writes this signal; a selection
/// write mutates no store, so its sidebar rebuild would not fire).
///
/// The custom smart lists' identity joins the slot with N10's rows; today
/// the six built-ins are the whole SMART LISTS surface.
#[derive(Debug, Clone, PartialEq)]
pub struct NavView {
    pub view_mode: ViewMode,
    pub browse_mode: BrowseMode,
    pub library_section: LibrarySection,
    /// The built-in smart list opened over the Library view.
    pub smart_list: Option<SmartPlaylistKind>,
    /// The playlist opened over the Library view.
    pub playlist: Option<PlaylistId>,
}

impl Default for NavView {
    /// The values `LibrarySession::default()` carries — the view fields
    /// themselves have no `Default` (`ViewMode` is the backend's), so the
    /// fresh session's own defaults are spelled here once.
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Library,
            browse_mode: BrowseMode::default(),
            library_section: LibrarySection::default(),
            smart_list: None,
            playlist: None,
        }
    }
}

/// The scalar settings as the two sessions carry them after hydration —
/// `AppRuntime::spawn` has already run `Preferences::hydrate`, so this is a
/// read of hydrated state, never a second hydrate. Deferred to N13: the
/// Library Paths detail, the scan formats as an editable list, and the
/// per-pass value fields.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsSnapshot {
    pub flags: UiFlags,
    pub scan_prefs: ScanPrefs,
    pub replaygain_enabled: bool,
    pub replaygain_mode: ReplayGainMode,
}

impl SettingsSnapshot {
    /// The snapshot the sessions currently imply, from the playback scalars
    /// the pump already read and the library session it is holding. `volume`
    /// and `muted` live on [`PlaybackView`] and are not duplicated here.
    pub(crate) fn of_sessions(
        replaygain_enabled: bool,
        replaygain_mode: ReplayGainMode,
        library: &LibrarySession,
    ) -> Self {
        Self {
            flags: library.ui_flags,
            scan_prefs: library.scan_prefs.clone(),
            replaygain_enabled,
            replaygain_mode,
        }
    }

    /// Whether the sessions still imply this snapshot — the pump's cheap
    /// comparison, which avoids rebuilding (and re-allocating the
    /// scan-format list) on ticks where nothing moved.
    pub(crate) fn matches_sessions(
        &self,
        replaygain_enabled: bool,
        replaygain_mode: ReplayGainMode,
        library: &LibrarySession,
    ) -> bool {
        self.flags == library.ui_flags
            && self.scan_prefs == library.scan_prefs
            && self.replaygain_enabled == replaygain_enabled
            && self.replaygain_mode == replaygain_mode
    }
}

/// The covers the View half holds, keyed by the decision half's composite
/// [`crate::bridge::covers::CoverCacheKey`]: one artwork identity at one
/// requested box.
///
/// Not a `Signal<CoverMap>` as the plan sketched: nsh signals dedup by
/// `PartialEq`, which would compare every image's bytes per write, and the
/// `ImageSrc` values are not `PartialEq` anyway. The map lives in the
/// engine-thread-only model (`stored`; P0 correction 9) and the `UiState`
/// `covers` signal carries a revision count that screens read inside
/// `dynamic` blocks — reading a signal is what re-runs a block on arrival
/// (P0 correction 21). Eviction is N6's.
pub type CoverMap =
    std::collections::HashMap<crate::bridge::covers::CoverCacheKey, nsh_core::element::ImageSrc>;

/// The projection readers the bridge serves the screens through, over one
/// `&mut SessionViews` (the runtime's owned read seam; P0 correction 6 —
/// no second projection instance, no sharing across threads).
///
/// The pump reads two of these every tick; the rest are the plan's named
/// read seam, wired in by the screens as their units land (N5 the library
/// lists, N10 playlists, N12 smart playlists). They are declared here so
/// the seam is one place from day one, hence the dead-code allowance until
/// their consumers arrive.
#[allow(dead_code)]
pub(crate) mod reads {
    use super::{PlaylistView, SessionViews};
    use riff_backend::app::state::TrackSort;
    use riff_backend::app::store::CustomSmartPlaylist;
    use riff_backend::domain::{Album, GenreCount, Playlist, PlaylistId, Track};
    use std::sync::Arc;
    use std::time::SystemTime;

    // Each is one delegation. They exist as the bridge's named read seam —
    // the plan's "Exact reads" list — so a screen (N5+) never touches a port
    // and the test crate can drive every read over mock ports.

    pub(crate) fn track_count(views: &mut SessionViews, query: &str, sort: TrackSort) -> usize {
        views.track_count(query, sort)
    }

    pub(crate) fn track_row(
        views: &mut SessionViews,
        query: &str,
        sort: TrackSort,
        index: usize,
    ) -> Option<Arc<Track>> {
        views.track_row(query, sort, index)
    }

    pub(crate) fn artist_albums(views: &mut SessionViews, artist: &str) -> Arc<[Album]> {
        views.artist_albums(artist)
    }

    pub(crate) fn album_tracks(
        views: &mut SessionViews,
        album_artist: &str,
        album_title: &str,
    ) -> Arc<[Track]> {
        views.album_tracks(album_artist, album_title)
    }

    pub(crate) fn playlists(views: &mut SessionViews) -> Arc<[Playlist]> {
        views.playlists()
    }

    pub(crate) fn playlist_view(views: &mut SessionViews, id: &PlaylistId) -> Option<PlaylistView> {
        views.playlist_view(id)
    }

    pub(crate) fn genres(views: &mut SessionViews) -> Arc<[GenreCount]> {
        views.genres()
    }

    pub(crate) fn last_scan(views: &mut SessionViews) -> Option<SystemTime> {
        views.last_scan()
    }

    /// The fallible custom-smart-playlist read, unchanged in shape: the
    /// failure reason travels alone, never as a degraded empty list.
    pub(crate) fn try_custom_smart_playlists(
        views: &mut SessionViews,
    ) -> Result<Vec<CustomSmartPlaylist>, String> {
        views.try_custom_smart_playlists()
    }
}
