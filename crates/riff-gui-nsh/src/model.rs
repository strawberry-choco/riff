//! The view vocabulary: the shapes the bridge populates from the backend and
//! the screens render from (plan "Bridge surface"). Every type here is what
//! one signal carries — [`PlaybackView`] the plan's literal, the rest only as
//! wide as today's reads can fill them. N13 widens [`SettingsSnapshot`] to
//! the settings pages; the deferred fields are listed on each type.

use std::sync::Arc;
use std::time::SystemTime;

use nsh_core::element::ImageSrc;
use riff_backend::app::cover_service::lru_insert;
use riff_backend::app::state::{
    BrowseMode, LibrarySection, LibrarySession, PlaybackState, RepeatMode, ReplayGainMode,
    ScanPrefs, TrackSort, UiFlags, ViewMode,
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
/// (P0 correction 21). The map itself is [`CoverViews`], the bounded View
/// half.
pub type CoverMap =
    std::collections::HashMap<crate::bridge::covers::CoverCacheKey, nsh_core::element::ImageSrc>;

/// The View half's bounds, ported from the egui crate (`ui/artwork.rs`) so
/// the nsh map keeps the entry cap and the byte budget the texture map
/// obeyed: nsh's own image cache carries no numeric bound (P0 correction
/// 10), so the policy lives here. A 56x56 thumbnail decodes to 12.5 KB of
/// RGBA and a 200x200 card to 160 KB, so in practice the entry cap binds
/// first — the budget is the heroes' guard (N9's 512x512 box is 1 MB a
/// piece, 80x a thumbnail, exactly the spread the egui budget existed for).
pub const COVER_VIEWS_CAP: usize = 200;
/// The View half's RGBA byte budget, the egui store's 64 MB carried over.
pub const COVER_VIEWS_BYTE_BUDGET: usize = 64 * 1024 * 1024;

/// The covers View half: the decoded pixels as [`ImageSrc`]s, bounded where
/// the egui texture map was bounded. One writer (the pump, on arrivals), one
/// reader (the screens' bindings); eviction runs on insert and reports the
/// dropped keys to the decision half, whose `arrived` set is the mirror — a
/// row that asks again after eviction gets a request, not a blank.
///
/// The eviction order is insertion order (`lru_insert`, no touch on read):
/// a read inside a render binding stays read-only, and with the cap at 200
/// entries the difference from a true LRU is one entry's residency at the
/// steady state — the window refill re-asks what the scroll walked away
/// from either way. The order's pairs carry each entry's byte cost, the
/// number the budget is enforced against, so the map needs no second index.
#[derive(Default)]
pub struct CoverViews {
    map: CoverMap,
    order: Vec<(crate::bridge::covers::CoverCacheKey, usize)>,
    bytes: usize,
}

impl CoverViews {
    /// File one arrival: `rgba_bytes` is the buffer's length, the entry's
    /// cost. A re-arrival of a key still held replaces in place — the key
    /// names the decode box, so the old entry cost the same bytes — and
    /// keeps its original insertion slot. Hands back the keys this insert
    /// pushed out.
    pub fn insert(
        &mut self,
        key: crate::bridge::covers::CoverCacheKey,
        image: ImageSrc,
        rgba_bytes: usize,
    ) -> Vec<crate::bridge::covers::CoverCacheKey> {
        let replaced = self.map.insert(key.clone(), image).is_some();
        if !replaced {
            self.bytes += rgba_bytes;
        }
        let mut evicted = lru_insert(&mut self.order, (key, rgba_bytes), COVER_VIEWS_CAP);
        for (old, cost) in &evicted {
            self.map.remove(old);
            self.bytes = self.bytes.saturating_sub(*cost);
        }
        evicted.extend(self.enforce_byte_budget());
        evicted.into_iter().map(|(key, _)| key).collect()
    }

    /// The pixels under `key`, if held. Read-only: a render binding's read
    /// never mutates the eviction order.
    #[must_use]
    pub fn get(&self, key: &crate::bridge::covers::CoverCacheKey) -> Option<&ImageSrc> {
        self.map.get(key)
    }

    /// How many entries the View half holds — the bound's live read (the
    /// eviction tests read it; no screen does).
    #[must_use]
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Drop entries oldest-first until the map fits the byte budget,
    /// reporting what went.
    fn enforce_byte_budget(&mut self) -> Vec<(crate::bridge::covers::CoverCacheKey, usize)> {
        let mut dropped = Vec::new();
        while self.bytes > COVER_VIEWS_BYTE_BUDGET {
            let Some((key, cost)) = self.order.first().cloned() else {
                break;
            };
            self.map.remove(&key);
            self.order.remove(0);
            self.bytes = self.bytes.saturating_sub(cost);
            dropped.push((key, cost));
        }
        dropped
    }
}

/// The list screen's warm row window: the rows the prefetch handler read,
/// held as shared handles the row bindings index into with no store read.
///
/// The window covers `[start, start + rows.len())` of one listing, stamped
/// with the listing's sort — the row bindings' fallback read (a row the
/// window does not hold) uses the stamp, never a session lock. It is
/// filled by the viewability handler (ascending reads, P0 correction 22's
/// sequential discipline) and reset by the pump when the listing's content
/// identity moves (a committed Library mutation, or a sort change) — a
/// reset the [`crate::bridge::state::UiState::list_revision`] bump
/// announces to the bindings, which re-read the window when it fires.
#[derive(Default)]
pub struct ListWindow {
    sort: TrackSort,
    start: usize,
    rows: Vec<Option<Arc<Track>>>,
}

impl ListWindow {
    /// The stamped sort — the bindings' fallback read's key.
    #[must_use]
    pub fn sort(&self) -> TrackSort {
        self.sort
    }

    /// Re-stamp the listing's sort; the rows behind a sort change are a
    /// different listing, so they drop. Returns whether rows were dropped —
    /// the pump answers a drop with the list-revision bump that repaints
    /// the bindings reading the window.
    pub fn set_sort(&mut self, sort: TrackSort) -> bool {
        if self.sort == sort {
            return false;
        }
        self.sort = sort;
        self.rows.clear();
        self.start = 0;
        true
    }

    /// Drop the rows: the listing behind them moved.
    pub fn reset(&mut self) {
        self.rows.clear();
        self.start = 0;
    }

    /// Whether any row is held (the reset tests read it; no screen does).
    #[must_use]
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row `index` when the window covers it.
    #[must_use]
    pub fn row(&self, index: usize) -> Option<Arc<Track>> {
        if index < self.start {
            return None;
        }
        self.rows.get(index - self.start).and_then(Clone::clone)
    }

    /// The covered range, `first..=last` — the prefetch handler's skip set.
    #[must_use]
    pub fn covers(&self) -> Option<(usize, usize)> {
        (!self.rows.is_empty()).then(|| (self.start, self.start + self.rows.len() - 1))
    }

    /// Hand the window's rows over (its covered range dies with them) —
    /// the incremental refill's starting point.
    #[must_use]
    pub fn take(&mut self) -> (usize, Vec<Option<Arc<Track>>>) {
        let rows = std::mem::take(&mut self.rows);
        let start = std::mem::replace(&mut self.start, 0);
        (start, rows)
    }

    /// Record the rows read for `[start..=start + rows.len())` under the
    /// listing's current sort.
    pub fn fill(&mut self, start: usize, rows: Vec<Option<Arc<Track>>>) {
        self.start = start;
        self.rows = rows;
    }
}

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
    use riff_backend::domain::{Album, GenreCount, Playlist, PlaylistId, Track, TrackId};
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

    /// The session's selected track — the inspector's resolve (the egui
    /// `resolve_inspector`'s first arm). A stale id reads as `None`, which
    /// the caller shows as the empty state.
    pub(crate) fn selected_track(views: &mut SessionViews, id: &TrackId) -> Option<Track> {
        views.selected_track(id)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The View half's eviction holds both bounds, and every eviction is
    /// reported so the decision half's `arrived` set stays the mirror.
    #[test]
    fn the_view_half_evicts_to_its_cap_and_reports_the_drops() {
        let mut views = CoverViews::default();
        let key = |n: usize| {
            crate::bridge::covers::cover_cache_key(
                &format!("cover-{n}"),
                crate::bridge::covers::COVER_THUMB,
            )
        };
        let image = |n: usize| {
            ImageSrc::rgba(
                56,
                56,
                vec![u8::try_from(n % 256).unwrap_or(0); 56 * 56 * 4],
            )
        };
        for n in 0..(COVER_VIEWS_CAP + 10) {
            let dropped = views.insert(key(n), image(n), 56 * 56 * 4);
            assert!(
                dropped.len() <= 1,
                "one insert pushes out at most one entry"
            );
            assert!(
                !dropped.iter().any(|k| *k == key(n)),
                "the fresh key never drops"
            );
        }
        assert_eq!(views.len(), COVER_VIEWS_CAP, "the cap holds");
        assert!(views.get(&key(0)).is_none(), "the oldest entry went");
        assert!(
            views.get(&key(COVER_VIEWS_CAP + 9)).is_some(),
            "the newest entry stands"
        );
    }

    /// A re-arrival replaces in place: no growth, no eviction of the
    /// entry's own slot-mates.
    #[test]
    fn a_re_arrival_replaces_without_growth() {
        let mut views = CoverViews::default();
        let key = crate::bridge::covers::cover_cache_key("one", crate::bridge::covers::COVER_THUMB);
        views.insert(
            key.clone(),
            ImageSrc::rgba(56, 56, vec![0; 56 * 56 * 4]),
            56 * 56 * 4,
        );
        views.insert(
            key.clone(),
            ImageSrc::rgba(56, 56, vec![1; 56 * 56 * 4]),
            56 * 56 * 4,
        );
        assert_eq!(views.len(), 1);
    }

    /// The byte budget evicts oldest-first beyond the cap's reach — the
    /// heroes' guard, exercised with entries a thumbnail budget can pass.
    #[test]
    fn the_byte_budget_evicts_oldest_first() {
        let mut views = CoverViews::default();
        // A tiny budget of one 56x56 entry, reached through the real
        // constant by scaling the entries: each arrival carries one entry's
        // bytes, so the budget passed here is the real one divided.
        let key = |n: usize| {
            crate::bridge::covers::cover_cache_key(
                &format!("big-{n}"),
                crate::bridge::covers::COVER_HERO,
            )
        };
        let bytes = COVER_VIEWS_BYTE_BUDGET / 2;
        let mut dropped = Vec::new();
        for n in 0..4 {
            dropped = views.insert(
                key(n),
                ImageSrc::rgba(512, 512, vec![0; 512 * 512 * 4]),
                bytes,
            );
        }
        assert!(
            views.len() <= 3,
            "the budget capped the hold, got {}",
            views.len()
        );
        assert!(
            views.get(&key(0)).is_none(),
            "the oldest big entry went first"
        );
        let _ = dropped;
    }

    /// The warm window answers only what it covers, and a re-stamp drops
    /// the rows behind a sort change.
    #[test]
    fn the_list_window_answers_its_covered_range_only() {
        let mut window = ListWindow::default();
        assert!(window.row(5).is_none());
        window.set_sort(TrackSort::NumberAsc);
        window.fill(10, vec![None, None, None]);
        assert!(window.row(9).is_none());
        assert!(window.row(10).is_none(), "a held None reads as no row");
        assert!(window.row(12).is_none());
        assert!(window.row(13).is_none());
        assert_eq!(window.covers(), Some((10, 12)));
        assert_eq!(window.sort(), TrackSort::NumberAsc);
        window.set_sort(TrackSort::TitleAsc);
        assert!(window.is_empty(), "the sort change dropped the rows");
        assert_eq!(window.sort(), TrackSort::TitleAsc);
    }
}
