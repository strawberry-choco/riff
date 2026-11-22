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
    BrowseMode, BrowserSelection, LibrarySection, LibrarySession, PlaybackState, RepeatMode,
    ReplayGainMode, ScanPrefs, TrackSort, UiFlags, ViewMode,
};
use riff_backend::app::store::SortDirection;
use riff_backend::app::views::{PlaylistView, SessionViews};
use riff_backend::domain::{Album, PlaylistId, SmartPlaylistKind, Track, TrackId};

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
    /// The ordered drill-down path, deepest entry last — the session's
    /// `browser_path` as the selection handlers last wrote it (a drill-down
    /// write mutates no store, so the pump cannot see it; the mirror is the
    /// same contract the other view fields ride). The stage table reads it.
    pub browser_path: Vec<BrowserSelection>,
    /// The built-in smart list opened over the Library view.
    pub smart_list: Option<SmartPlaylistKind>,
    /// The playlist opened over the Library view.
    pub playlist: Option<PlaylistId>,
    /// The custom smart playlist opened over the Library view.
    pub custom_list: Option<PlaylistId>,
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
            browser_path: Vec::new(),
            smart_list: None,
            playlist: None,
            custom_list: None,
        }
    }
}

/// The Library view's stages: one variant per listing the browser can render.
/// The pure [`stage_plan_parts`] lays the active section's stages out left to
/// right, the pump reads a row batch per entity stage, and the stage renders
/// one column per entry.
///
/// One variant per listing the pump sizes and refreshes: the stage key the
/// pump's step-8 comparison and the scroll memory's slots both name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StageKind {
    /// The flat All Tracks listing (the search query rides the row window).
    Flat,
    /// A selected album's tracks — the stage's last column everywhere. In
    /// the Genres section the listing is genre-scoped.
    Tracks {
        artist: String,
        title: String,
        genre: Option<String>,
    },
    /// The Artists root: every artist, name-ordered.
    ArtistsRoot,
    /// An artist's albums.
    ArtistAlbums { artist: String },
    /// The Albums root.
    AlbumsRoot,
    /// The Genres root.
    GenresRoot,
    /// The artists carrying a genre.
    GenreArtists { genre: String },
    /// One artist's albums within a genre.
    GenreArtistAlbums { artist: String, genre: String },
    /// The Folders browse mode's tree (folders.rs's own renderer).
    Folders,
}

impl StageKind {
    /// Whether the stage's rows are the entity list the pump reads in one
    /// batch. `Flat`, `Tracks` and `Folders` render from other sources.
    #[must_use]
    pub fn is_entity_listing(&self) -> bool {
        matches!(
            self,
            Self::ArtistsRoot
                | Self::AlbumsRoot
                | Self::GenresRoot
                | Self::ArtistAlbums { .. }
                | Self::GenreArtists { .. }
                | Self::GenreArtistAlbums { .. }
        )
    }

    /// Whether the stage keeps a remembered scroll position. Section roots
    /// remember; every deeper column, the Tracks column included, resets on
    /// a selection or content change (the egui scroll-memory contract).
    #[must_use]
    pub fn remembers_scroll(&self) -> bool {
        matches!(
            self,
            Self::Flat | Self::ArtistsRoot | Self::AlbumsRoot | Self::GenresRoot
        )
    }
}

/// The Library stage's column plan, left to right, the egui `column_plan`
/// shape ported: the Section's root, one stage per valid drill-down level,
/// and the Tracks column when an album is selected. Path entries of the
/// wrong kind for their level are ignored (section switches reset the path,
/// but the plan stays defensive), so a stale or foreign entry falls back to
/// the deepest valid prefix.
#[must_use]
pub fn stage_plan_parts(
    browse_mode: BrowseMode,
    section: LibrarySection,
    path: &[BrowserSelection],
) -> Vec<StageKind> {
    match browse_mode {
        BrowseMode::Folders => vec![StageKind::Folders],
        BrowseMode::Library => match section {
            LibrarySection::AllTracks => vec![StageKind::Flat],
            LibrarySection::Artists => {
                let mut plan = vec![StageKind::ArtistsRoot];
                if let Some(BrowserSelection::Artist(artist)) = path.first() {
                    plan.push(StageKind::ArtistAlbums {
                        artist: artist.clone(),
                    });
                    if let Some(BrowserSelection::Album { artist, title }) = path.get(1) {
                        plan.push(StageKind::Tracks {
                            artist: artist.clone(),
                            title: title.clone(),
                            genre: None,
                        });
                    }
                }
                plan
            }
            LibrarySection::Albums => {
                let mut plan = vec![StageKind::AlbumsRoot];
                if let Some(BrowserSelection::Album { artist, title }) = path.first() {
                    plan.push(StageKind::Tracks {
                        artist: artist.clone(),
                        title: title.clone(),
                        genre: None,
                    });
                }
                plan
            }
            LibrarySection::Genres => {
                let mut plan = vec![StageKind::GenresRoot];
                if let Some(BrowserSelection::Genre(genre)) = path.first() {
                    plan.push(StageKind::GenreArtists {
                        genre: genre.clone(),
                    });
                    if let Some(BrowserSelection::Artist(artist)) = path.get(1) {
                        plan.push(StageKind::GenreArtistAlbums {
                            artist: artist.clone(),
                            genre: genre.clone(),
                        });
                        if let Some(BrowserSelection::Album { artist, title }) = path.get(2) {
                            plan.push(StageKind::Tracks {
                                artist: artist.clone(),
                                title: title.clone(),
                                genre: Some(genre.clone()),
                            });
                        }
                    }
                }
                plan
            }
        },
    }
}

/// The deepest stage for one Library session's view state, the plan's last
/// entry. Path entries of the wrong kind for their level are ignored, so a
/// stale or foreign entry falls back to the deepest valid prefix.
#[must_use]
pub fn stage_of_parts(
    browse_mode: BrowseMode,
    section: LibrarySection,
    path: &[BrowserSelection],
) -> StageKind {
    stage_plan_parts(browse_mode, section, path)
        .pop()
        .expect("every stage plan names at least the section root")
}

/// The stage the shell mounts for the nav state: `None` when the Library
/// stage table does not own the view — a playlist or smart list opened over
/// it (N10/N12's stages), or a non-Library view mode (Now Playing, Settings;
/// their own units).
#[must_use]
pub fn stage_of(nav: &NavView) -> Option<StageKind> {
    if nav.playlist.is_some()
        || nav.smart_list.is_some()
        || nav.custom_list.is_some()
        || nav.view_mode != ViewMode::Library
    {
        return None;
    }
    Some(stage_of_parts(
        nav.browse_mode,
        nav.library_section,
        &nav.browser_path,
    ))
}

/// One entity row of a browse stage — the egui `BrowserItem`'s facts minus
/// the two the render resolves itself (the selection, from the path; the
/// pixels, from the covers cache): the identity a press selects by, the
/// label and muted detail line it paints, and the track whose cover the row
/// asks for (`None` paints the placeholder).
///
/// An album's key carries the `artist\u{1f}title` separator — the store's
/// album identity joined, the egui `album_item`'s key bytes — so a press
/// selects and the dispatch reads the same value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityView {
    pub key: String,
    pub label: String,
    pub detail: String,
    pub cover_track: Option<TrackId>,
}

/// One entity column's batched rows, keyed by the stage that reads them.
/// The pump publishes the active plan's entity stages as one vector; a
/// column looks its own stage up and never reads the store per row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRows {
    pub stage: StageKind,
    pub rows: Arc<[EntityView]>,
}

/// The track listings' display sort, the egui `sort_tracks` ported: the
/// canonical order the store served is the `NumberAsc` anchor, `NumberDesc`
/// its exact reversal, the title modes compare case-insensitively and —
/// being stable sorts — keep the canonical order on ties. The display
/// vocabulary's one crossing onto a copy the caller owns.
pub(crate) fn sort_tracks(rows: &mut [Arc<Track>], sort: TrackSort) {
    match sort {
        TrackSort::NumberAsc => {}
        TrackSort::NumberDesc => rows.reverse(),
        TrackSort::TitleAsc => rows.sort_by_key(|track| {
            track
                .metadata
                .display_title(&track.file_path)
                .to_lowercase()
        }),
        TrackSort::TitleDesc => rows.sort_by_key(|track| {
            std::cmp::Reverse(
                track
                    .metadata
                    .display_title(&track.file_path)
                    .to_lowercase(),
            )
        }),
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

/// One SMART LISTS custom row: the identity a selection write opens the
/// list with and the name the sidebar and editor paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomListRow {
    pub id: PlaylistId,
    pub name: String,
}

/// One user playlist's detail: its identity, saved name and resolved
/// entries, in playlist order. Dangling entries stay listed with
/// `track: None` (ADR 0001) and are skipped by playback.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistDetail {
    pub id: PlaylistId,
    pub name: String,
    pub rows: Arc<[PlaylistRowView]>,
}

/// One playlist entry as the row renders it: the identity written through
/// any reorder, the resolved track when the Library still holds it, and
/// whether the store's valid verdict says so.
#[derive(Debug, Clone)]
pub struct PlaylistRowView {
    pub id: TrackId,
    pub track: Option<Arc<Track>>,
    pub valid: bool,
}

impl PartialEq for PlaylistRowView {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.valid == other.valid
            && match (&self.track, &other.track) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// Build one playlist's detail from the read seam, or `None` when the store
/// no longer knows the list.
pub(crate) fn playlist_detail(views: &mut SessionViews, id: &PlaylistId) -> Option<PlaylistDetail> {
    let name = reads::playlists(views)
        .iter()
        .find(|playlist| &playlist.id == id)
        .map(|playlist| playlist.name.clone())?;
    let view = reads::playlist_view(views, id)?;
    let rows = view
        .rows
        .iter()
        .map(|(id, track, valid)| PlaylistRowView {
            id: id.clone(),
            track: track.clone().map(Arc::new),
            valid: *valid,
        })
        .collect::<Vec<_>>();
    Some(PlaylistDetail {
        id: id.clone(),
        name,
        rows: rows.into(),
    })
}

/// One built-in smart list's detail: the kind, its display name and its
/// tracks under the list's own bound.
#[derive(Debug, Clone)]
pub struct SmartListDetail {
    pub kind: SmartPlaylistKind,
    pub name: String,
    pub tracks: Arc<[Track]>,
}

impl PartialEq for SmartListDetail {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.name == other.name
            && Arc::ptr_eq(&self.tracks, &other.tracks)
    }
}

/// Build one built-in smart list's detail through the seam.
pub(crate) fn smart_list_detail(
    views: &mut SessionViews,
    kind: SmartPlaylistKind,
) -> SmartListDetail {
    SmartListDetail {
        kind,
        name: kind.display_name().to_owned(),
        tracks: reads::smart_list(views, kind),
    }
}

/// The tag editor's draft: the eleven editable values as the fields show
/// them (empty = unset), plus the album aggregate's per-field `(different)`
/// marks. The draft is engine-thread state in the bridge signals; the save
/// builds one request per written track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagDraft {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: String,
    pub track_number: String,
    pub track_gain: String,
    pub track_peak: String,
    pub album_gain: String,
    pub album_peak: String,
    /// Which fields the album's tracks disagree on, in the field order
    /// above. A marked field shows `(different)` until edited.
    pub different: [bool; 11],
}

/// The draft field order, the one place the eleven names live.
pub const TAG_FIELDS: [&str; 11] = [
    "Title",
    "Artist",
    "Album",
    "Album artist",
    "Genre",
    "Year",
    "Track number",
    "Track gain",
    "Track peak",
    "Album gain",
    "Album peak",
];

impl TagDraft {
    /// The draft from one track's metadata.
    #[must_use]
    pub fn of_track(track: &Track) -> Self {
        Self {
            title: track.metadata.title.clone().unwrap_or_default(),
            artist: track.metadata.artist.clone().unwrap_or_default(),
            album: track.metadata.album.clone().unwrap_or_default(),
            album_artist: track.metadata.album_artist.clone().unwrap_or_default(),
            genre: track.metadata.genre.clone().unwrap_or_default(),
            year: track
                .metadata
                .year
                .map(|y| y.to_string())
                .unwrap_or_default(),
            track_number: track
                .metadata
                .track_number
                .map(|n| n.to_string())
                .unwrap_or_default(),
            track_gain: number(track.metadata.replaygain_track_gain),
            track_peak: number(track.metadata.replaygain_track_peak),
            album_gain: number(track.metadata.replaygain_album_gain),
            album_peak: number(track.metadata.replaygain_album_peak),
            different: [false; 11],
        }
    }

    /// The album aggregate: a field all tracks share keeps its value; one
    /// they disagree on is blanked and marked `(different)`.
    #[must_use]
    pub fn of_album(tracks: &[Track]) -> Self {
        let Some(first) = tracks.first() else {
            return Self::default();
        };
        let mut draft = Self::of_track(first);
        for track in &tracks[1..] {
            let other = Self::of_track(track);
            let pairs = [
                (&mut draft.title, &other.title),
                (&mut draft.artist, &other.artist),
                (&mut draft.album, &other.album),
                (&mut draft.album_artist, &other.album_artist),
                (&mut draft.genre, &other.genre),
                (&mut draft.year, &other.year),
                (&mut draft.track_number, &other.track_number),
                (&mut draft.track_gain, &other.track_gain),
                (&mut draft.track_peak, &other.track_peak),
                (&mut draft.album_gain, &other.album_gain),
                (&mut draft.album_peak, &other.album_peak),
            ];
            for (index, (held, incoming)) in pairs.into_iter().enumerate() {
                if held != incoming {
                    draft.different[index] = true;
                    held.clear();
                }
            }
        }
        draft
    }

    /// The field at `index`, for the render and the save.
    #[must_use]
    pub fn field(&self, index: usize) -> &str {
        match index {
            0 => &self.title,
            1 => &self.artist,
            2 => &self.album,
            3 => &self.album_artist,
            4 => &self.genre,
            5 => &self.year,
            6 => &self.track_number,
            7 => &self.track_gain,
            8 => &self.track_peak,
            9 => &self.album_gain,
            _ => &self.album_peak,
        }
    }

    /// Write the field at `index` from the input's text and clear its
    /// `(different)` mark (the user typed a value the album now shares).
    pub fn set_field(&mut self, index: usize, value: String) {
        match index {
            0 => self.title = value,
            1 => self.artist = value,
            2 => self.album = value,
            3 => self.album_artist = value,
            4 => self.genre = value,
            5 => self.year = value,
            6 => self.track_number = value,
            7 => self.track_gain = value,
            8 => self.track_peak = value,
            9 => self.album_gain = value,
            _ => self.album_peak = value,
        }
        if let Some(mark) = self.different.get_mut(index) {
            *mark = false;
        }
    }

    /// The placeholder a field shows: `(different)` when the album
    /// disagrees, `(none)` when the value is unset.
    #[must_use]
    pub fn placeholder(&self, index: usize) -> &'static str {
        if self.different.get(index).copied().unwrap_or(false) {
            "(different)"
        } else {
            "(none)"
        }
    }

    /// The text/number half of the draft as a `TagEdit`: only non-empty
    /// fields travel, so a blank field never clears a file's tag.
    #[must_use]
    pub fn edit(&self) -> riff_backend::app::traits::TagEdit {
        riff_backend::app::traits::TagEdit {
            title: non_empty(&self.title),
            artist: non_empty(&self.artist),
            album: non_empty(&self.album),
            album_artist: non_empty(&self.album_artist),
            genre: non_empty(&self.genre),
            track_number: self.track_number.trim().parse().ok(),
            disc_number: None,
            year: self.year.trim().parse().ok(),
            composer: None,
            comment: None,
        }
    }

    /// The `ReplayGain` half of the draft, only non-empty values.
    #[must_use]
    pub fn replaygain(&self) -> riff_backend::app::traits::ReplayGainTags {
        riff_backend::app::traits::ReplayGainTags {
            track_gain: self.track_gain.trim().parse().ok(),
            track_peak: self.track_peak.trim().parse().ok(),
            album_gain: self.album_gain.trim().parse().ok(),
            album_peak: self.album_peak.trim().parse().ok(),
        }
    }

    /// Whether the draft differs from `track` in any field — the per-track
    /// save's dirtiness and the album save's skip rule.
    #[must_use]
    pub fn differs_from(&self, track: &Track) -> bool {
        let theirs = Self::of_track(track);
        (0..11).any(|index| self.field(index) != theirs.field(index))
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn number(value: Option<f32>) -> String {
    value.map_or_else(String::new, |v| v.to_string())
}

/// One registered library path as the Settings page shows it: the path, its
/// readiness word, and its watch state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPathView {
    pub path: std::path::PathBuf,
    pub readiness: String,
    pub watching: bool,
    /// The watcher's refusal, when its last attempt failed.
    pub watch_warning: Option<String>,
}

impl LibraryPathView {
    /// The readiness column for a session status.
    #[must_use]
    pub fn readiness_of(status: &riff_backend::app::state::LibraryStatus) -> String {
        use riff_backend::app::state::LibraryStatus;
        match status {
            LibraryStatus::Idle => "Ready".to_owned(),
            LibraryStatus::Scanning { files_found } => format!("Scanning ({files_found})"),
            LibraryStatus::Scanned(count) => format!("Scanned ({count})"),
            LibraryStatus::Unavailable => "Folder unavailable".to_owned(),
        }
    }
}

/// The Settings pages, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    #[default]
    Library,
    Playback,
    Appearance,
    Advanced,
    About,
}

impl SettingsPage {
    pub const ALL: [Self; 5] = [
        Self::Library,
        Self::Playback,
        Self::Appearance,
        Self::Advanced,
        Self::About,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Playback => "Playback",
            Self::Appearance => "Appearance",
            Self::Advanced => "Advanced",
            Self::About => "About",
        }
    }
}

/// The appearance choice. Dark and Light are explicit; System follows the
/// OS. High contrast is a separate persisted flag over whichever base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppearanceMode {
    #[default]
    Dark,
    Light,
    System,
    DarkHighContrast,
    LightHighContrast,
}

impl AppearanceMode {
    pub const ALL: [Self; 5] = [
        Self::Dark,
        Self::Light,
        Self::System,
        Self::DarkHighContrast,
        Self::LightHighContrast,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "System",
            Self::DarkHighContrast => "Dark high contrast",
            Self::LightHighContrast => "Light high contrast",
        }
    }

    /// The palette this mode resolves to.
    #[must_use]
    pub fn palette(self) -> crate::theme::Palette {
        use crate::theme::Palette;
        match self {
            Self::Dark => Palette::dark(),
            Self::Light => Palette::light(),
            Self::System => {
                if nsh_core::element::dark() {
                    Palette::dark()
                } else {
                    Palette::light()
                }
            }
            Self::DarkHighContrast => Palette::dark().high_contrast(),
            Self::LightHighContrast => Palette::light().high_contrast(),
        }
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
    query: String,
    start: usize,
    rows: Vec<Option<Arc<Track>>>,
}

impl ListWindow {
    /// The stamped sort — the bindings' fallback read's key.
    #[must_use]
    pub fn sort(&self) -> TrackSort {
        self.sort
    }

    /// The stamped query — the flat listing's needle (empty = the whole
    /// library), the bindings' fallback read's other key.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
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

    /// Re-stamp the listing's search needle; a keystroke retargets the
    /// projection like a sort change does, so the rows drop the same way.
    /// Returns whether rows were dropped.
    pub fn set_query(&mut self, query: &str) -> bool {
        if self.query == query {
            return false;
        }
        query.clone_into(&mut self.query);
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

/// Per-stage scroll memory, the egui `ui/scroll_memory.rs` contract adapted
/// to the stage table. Only the stages that keep a position ask for it
/// ([`StageKind::remembers_scroll`]): a Section root or the flat list, whose
/// list remounts after a section switch. Every deeper column resets on a
/// selection or content change, so it never reads or writes a slot.
///
/// The slot key is the stage's own identity ([`StageKind`], entity names
/// included), the content fingerprint is the search query, the sort key and
/// the Library generation the UI last saw. A fingerprint mismatch on restore
/// resets to the top instead of restoring a stale offset, exactly the egui
/// contract. Slots are bounded (oldest evicted); a restart starts every
/// stage at the top.
#[derive(Default)]
pub struct ScrollMemory {
    slots: Vec<(StageKind, ScrollSlot)>,
}

/// How many stage slots the memory keeps — deeper than any real walk goes,
/// bounded so a pathological session cannot grow it without end.
const SCROLL_SLOTS: usize = 32;

/// One stage's remembered position, under the fingerprint it was captured
/// with.
#[derive(Clone)]
struct ScrollSlot {
    query: String,
    sort_key: u8,
    generation: u64,
    row: usize,
}

impl ScrollMemory {
    /// Record the stage's first viewable row under the content identity it
    /// was rendered with. One slot per stage: the same stage overwrites.
    pub fn remember(
        &mut self,
        stage: &StageKind,
        query: &str,
        sort_key: u8,
        generation: u64,
        row: usize,
    ) {
        let slot = ScrollSlot {
            query: query.to_owned(),
            sort_key,
            generation,
            row,
        };
        if let Some((_, held)) = self.slots.iter_mut().find(|(held, _)| held == stage) {
            *held = slot;
        } else {
            if self.slots.len() >= SCROLL_SLOTS {
                self.slots.remove(0);
            }
            self.slots.push((stage.clone(), slot));
        }
    }

    /// The row to restore the stage to, when a slot is held whose
    /// fingerprint still matches the stage's content identity. A stale
    /// fingerprint (the query, the sort or the Library moved behind the
    /// position) answers `None` — the stage opens at the top.
    #[must_use]
    pub fn restore(
        &self,
        stage: &StageKind,
        query: &str,
        sort_key: u8,
        generation: u64,
    ) -> Option<usize> {
        let slot = self
            .slots
            .iter()
            .find(|(held, _)| held == stage)
            .map(|(_, slot)| slot)?;
        (slot.query == query && slot.sort_key == sort_key && slot.generation == generation)
            .then_some(slot.row)
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
    use riff_backend::app::store::{CustomSmartPlaylist, SortDirection};
    use riff_backend::domain::{
        Album, Artist, GenreCount, Playlist, PlaylistId, SmartPlaylistKind, Track, TrackId,
    };
    use std::path::{Path, PathBuf};
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

    pub(crate) fn artist_first_track(views: &mut SessionViews, artist: &str) -> Option<TrackId> {
        views.artist_first_track(artist)
    }

    // The browse roots and the search-hit roots, paged (the egui root
    // columns' reads): a count and one row by index, the projection's
    // bounded-window reads. `direction` is part of the query signature —
    // the store applies it in SQL, so a row index names the same row before
    // and after a flip.

    pub(crate) fn artist_row(
        views: &mut SessionViews,
        direction: SortDirection,
        index: usize,
    ) -> Option<Arc<Artist>> {
        views.artist_row(direction, index)
    }

    pub(crate) fn artist_count(views: &mut SessionViews, direction: SortDirection) -> usize {
        views.artist_count(direction)
    }

    pub(crate) fn album_row(
        views: &mut SessionViews,
        direction: SortDirection,
        index: usize,
    ) -> Option<Arc<Album>> {
        views.album_row(direction, index)
    }

    pub(crate) fn album_count(views: &mut SessionViews, direction: SortDirection) -> usize {
        views.album_count(direction)
    }

    pub(crate) fn hit_artist_row(
        views: &mut SessionViews,
        query: &str,
        index: usize,
    ) -> Option<Arc<Artist>> {
        views.hit_artist_row(query, index)
    }

    pub(crate) fn hit_artist_count(views: &mut SessionViews, query: &str) -> usize {
        views.hit_artist_count(query)
    }

    pub(crate) fn hit_album_row(
        views: &mut SessionViews,
        query: &str,
        index: usize,
    ) -> Option<Arc<Album>> {
        views.hit_album_row(query, index)
    }

    pub(crate) fn hit_album_count(views: &mut SessionViews, query: &str) -> usize {
        views.hit_album_count(query)
    }

    // The scoped reads a drill column under a query needs: whether an
    // entity is itself a name hit, and which of its rows match. The store
    // owns the matching rule (the search_text column's rules); the drill
    // holds none of its own.

    pub(crate) fn artist_is_name_hit(views: &mut SessionViews, artist: &str, query: &str) -> bool {
        views.artist_is_name_hit(artist, query)
    }

    pub(crate) fn album_is_name_hit(
        views: &mut SessionViews,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> bool {
        views.album_is_name_hit(album_artist, album_title, query)
    }

    pub(crate) fn album_hit_tracks(
        views: &mut SessionViews,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Arc<[Track]> {
        views.album_hit_tracks(album_artist, album_title, query)
    }

    pub(crate) fn album_hit_tracks_in_genre(
        views: &mut SessionViews,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        query: &str,
    ) -> Arc<[Track]> {
        views.album_hit_tracks_in_genre(album_artist, album_title, genre, query)
    }

    pub(crate) fn album_tracks_in_genre(
        views: &mut SessionViews,
        album_artist: &str,
        album_title: &str,
        genre: &str,
    ) -> Arc<[Track]> {
        views.album_tracks_in_genre(album_artist, album_title, genre)
    }

    pub(crate) fn artists_in_genre(views: &mut SessionViews, genre: &str) -> Arc<[Artist]> {
        views.artists_in_genre(genre)
    }

    pub(crate) fn artist_albums_in_genre(
        views: &mut SessionViews,
        artist: &str,
        genre: &str,
    ) -> Arc<[Album]> {
        views.artist_albums_in_genre(artist, genre)
    }

    // The folder tree's reads, one cached answer per generation: the
    // children of an expanded folder, the tracks directly inside it, and
    // the two probes the egui node renderer gates and prunes with.

    pub(crate) fn folder_has_audio(views: &mut SessionViews, folder: &Path) -> bool {
        views.folder_has_audio(folder)
    }

    pub(crate) fn folder_search_match(
        views: &mut SessionViews,
        folder: &Path,
        query: &str,
    ) -> bool {
        views.folder_search_match(folder, query)
    }

    pub(crate) fn folder_children(views: &mut SessionViews, folder: &Path) -> Arc<[PathBuf]> {
        views.folder_children(folder)
    }

    pub(crate) fn folder_direct_tracks(views: &mut SessionViews, folder: &Path) -> Arc<[Track]> {
        views.folder_direct_tracks(folder)
    }

    pub(crate) fn playlists(views: &mut SessionViews) -> Arc<[Playlist]> {
        views.playlists()
    }

    /// One built-in smart list's tracks under its own bound, the sidebar's
    /// and the stage's shared limit choice.
    pub(crate) fn smart_list(views: &mut SessionViews, kind: SmartPlaylistKind) -> Arc<[Track]> {
        views.smart_list(kind, crate::menus::smart_list_limit(kind))
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

/// The active entity stage's rows, read through the seam and shaped into
/// [`EntityView`]s. One batched read per stage build (the pump rebuilds on
/// a stage, query, direction or generation move) — never per row: the
/// drill reads are one generation-cached list each, the roots' rows are
/// read ascending so the paged projections' window fetches stay sequential
/// (the same discipline the row window keeps). The query filters per the
/// backend's matching rules, the egui columns' scoping ported: the hit
/// roots under a query, the Genres root never (hit-scoped genre counts are
/// the egui spec's isolated cut), a name-hit drill expanding to its full
/// list.
pub(crate) fn entity_rows(
    views: &mut SessionViews,
    stage: &StageKind,
    query: &str,
    desc: bool,
) -> Arc<[EntityView]> {
    let direction = if desc {
        SortDirection::Descending
    } else {
        SortDirection::Ascending
    };
    let rows: Vec<EntityView> = match stage {
        StageKind::ArtistsRoot => {
            let count = if query.is_empty() {
                reads::artist_count(views, direction)
            } else {
                reads::hit_artist_count(views, query)
            };
            (0..count)
                .filter_map(|at| {
                    let artist = if query.is_empty() {
                        reads::artist_row(views, direction, at)
                    } else {
                        reads::hit_artist_row(views, query, at)
                    }?;
                    Some(EntityView {
                        // The artist row's cover is resolved lazily — the
                        // first-track read per artist is the stage's
                        // dominant cost on a big library, and only the
                        // visible rows ever show one (the egui root asked
                        // per visible row too). The row view and the
                        // viewability handler resolve it for the rows they
                        // serve, through the seam's own per-artist cache.
                        cover_track: None,
                        key: artist.name.clone(),
                        label: artist.name.clone(),
                        detail: album_word(artist.albums.len()),
                    })
                })
                .collect()
        }
        StageKind::AlbumsRoot => {
            let count = if query.is_empty() {
                reads::album_count(views, direction)
            } else {
                reads::hit_album_count(views, query)
            };
            (0..count)
                .filter_map(|at| {
                    let album = if query.is_empty() {
                        reads::album_row(views, direction, at)
                    } else {
                        reads::hit_album_row(views, query, at)
                    }?;
                    Some(album_view(&album, None))
                })
                .collect()
        }
        StageKind::GenresRoot => views
            .genres()
            .iter()
            .map(|genre| EntityView {
                cover_track: None,
                key: genre.genre.clone(),
                label: genre.genre.clone(),
                detail: track_word(genre.tracks),
            })
            .collect(),
        StageKind::ArtistAlbums { artist } => {
            let all = reads::artist_albums(views, artist);
            // A name-hit artist drills to *all* its albums (the downward
            // name-hit expansion); a track-hit artist's hit albums show —
            // the albums that are themselves name-hits or carry a matching
            // track. The name-hit decision is the store's, asked through
            // the read seam; the drill holds no hit rule of its own.
            let albums: Vec<Album> = if query.is_empty()
                || reads::artist_is_name_hit(views, artist, query)
            {
                all.iter().cloned().collect()
            } else {
                all.iter()
                    .filter(|album| {
                        reads::album_is_name_hit(views, &album.artist, &album.title, query)
                            || !reads::album_hit_tracks(views, &album.artist, &album.title, query)
                                .is_empty()
                    })
                    .cloned()
                    .collect()
            };
            albums.iter().map(|album| album_view(album, None)).collect()
        }
        StageKind::GenreArtists { genre } => reads::artists_in_genre(views, genre)
            .iter()
            .map(|artist| {
                let albums = reads::artist_albums_in_genre(views, &artist.name, genre);
                EntityView {
                    cover_track: albums
                        .first()
                        .and_then(|album| album.tracks.first())
                        .cloned(),
                    key: artist.name.clone(),
                    label: artist.name.clone(),
                    detail: album_word(albums.len()),
                }
            })
            .collect(),
        StageKind::GenreArtistAlbums { artist, genre } => {
            reads::artist_albums_in_genre(views, artist, genre)
                .iter()
                .map(|album| album_view(album, None))
                .collect()
        }
        // Track stages ride the row window; the folder tree renders itself.
        StageKind::Flat | StageKind::Tracks { .. } | StageKind::Folders => Vec::new(),
    };
    rows.into()
}

/// One track stage's rows: the album's tracks in canonical order (or their
/// genre-scoped set in the Genres section), filtered per the backend's
/// matching rules under a query — a name-hit album opening its full list so
/// the drill never dead-ends empty — then sorted into the display order the
/// session's track sort names. One read per stage build; the row window
/// copies from it.
pub(crate) fn album_rows(
    views: &mut SessionViews,
    stage: &StageKind,
    query: &str,
    sort: TrackSort,
) -> Vec<Arc<Track>> {
    let StageKind::Tracks {
        artist,
        title,
        genre,
    } = stage
    else {
        return Vec::new();
    };
    let name_hit = !query.is_empty() && reads::album_is_name_hit(views, artist, title, query);
    let tracks = match genre {
        Some(genre) if !name_hit => {
            if query.is_empty() {
                reads::album_tracks_in_genre(views, artist, title, genre)
            } else {
                reads::album_hit_tracks_in_genre(views, artist, title, genre, query)
            }
        }
        Some(genre) => reads::album_tracks_in_genre(views, artist, title, genre),
        None if !name_hit && !query.is_empty() => {
            reads::album_hit_tracks(views, artist, title, query)
        }
        None => reads::album_tracks(views, artist, title),
    };
    let mut rows: Vec<Arc<Track>> = tracks.iter().map(|track| Arc::new(track.clone())).collect();
    sort_tracks(&mut rows, sort);
    rows
}

/// One album entity row: the egui `album_item`'s shape — the store's
/// `(album artist, title)` identity joined as the key, the title as the
/// label, the `artist · year` line as the detail, the caller's cover track
/// override.
fn album_view(album: &Album, cover: Option<TrackId>) -> EntityView {
    EntityView {
        cover_track: cover.or_else(|| album.tracks.first().cloned()),
        key: format!("{}\u{1f}{}", album.artist, album.title),
        label: album.title.clone(),
        detail: album.year.map_or_else(
            || album.artist.clone(),
            |year| format!("{} \u{b7} {year}", album.artist),
        ),
    }
}

/// The "N albums" detail line's plural shape, the egui count line's.
fn album_word(count: usize) -> String {
    if count == 1 {
        "1 album".to_owned()
    } else {
        format!("{count} albums")
    }
}

/// The "N tracks" detail line's plural shape.
fn track_word(count: usize) -> String {
    if count == 1 {
        "1 track".to_owned()
    } else {
        format!("{count} tracks")
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

    /// A query re-stamp drops the rows the same way a sort change does: the
    /// search retarget is a different listing.
    #[test]
    fn a_query_change_drops_the_window_rows() {
        let mut window = ListWindow::default();
        window.fill(0, vec![None, None]);
        assert!(window.set_query("amber"), "the needle moved");
        assert!(window.is_empty(), "the query change dropped the rows");
        assert_eq!(window.query(), "amber");
        assert!(!window.set_query("amber"), "the same needle keeps the rows");
    }

    /// The scroll memory: a stage's position survives navigation away and
    /// back under a matching fingerprint, and a stale one (query, sort or
    /// generation moved) resets to the top.
    #[test]
    fn the_scroll_memory_restores_only_under_a_matching_fingerprint() {
        let mut memory = ScrollMemory::default();
        let stage = StageKind::ArtistAlbums {
            artist: "Amber Signals".to_owned(),
        };
        memory.remember(&stage, "", 0, 7, 42);
        assert_eq!(
            memory.restore(&stage, "", 0, 7),
            Some(42),
            "the same content restores"
        );
        assert_eq!(
            memory.restore(&stage, "amber", 0, 7),
            None,
            "a moved query resets"
        );
        assert_eq!(
            memory.restore(&stage, "", 1, 7),
            None,
            "a moved sort resets"
        );
        assert_eq!(
            memory.restore(&stage, "", 0, 8),
            None,
            "a moved library resets"
        );
        let other = StageKind::ArtistAlbums {
            artist: "Paper Lanterns".to_owned(),
        };
        assert_eq!(
            memory.restore(&other, "", 0, 7),
            None,
            "another stage's slot is its own"
        );
        memory.remember(&stage, "", 0, 7, 10);
        assert_eq!(
            memory.restore(&stage, "", 0, 7),
            Some(10),
            "the same stage overwrites its slot"
        );
    }

    /// The stage plan: the side-by-side columns each section and path opens,
    /// and a path entry of the wrong kind for its level adds no column.
    #[test]
    fn the_stage_plan_walks_the_drill_down_paths() {
        let artist = BrowserSelection::Artist("Amber Signals".to_owned());
        let album = BrowserSelection::Album {
            artist: "Amber Signals".to_owned(),
            title: "Neon Tide".to_owned(),
        };
        let genre = BrowserSelection::Genre("Ambient".to_owned());
        assert_eq!(
            stage_plan_parts(BrowseMode::Library, LibrarySection::AllTracks, &[]),
            vec![StageKind::Flat]
        );
        assert_eq!(
            stage_plan_parts(BrowseMode::Folders, LibrarySection::AllTracks, &[]),
            vec![StageKind::Folders]
        );
        assert_eq!(
            stage_plan_parts(BrowseMode::Library, LibrarySection::Artists, &[]),
            vec![StageKind::ArtistsRoot]
        );
        assert_eq!(
            stage_plan_parts(
                BrowseMode::Library,
                LibrarySection::Artists,
                std::slice::from_ref(&artist)
            ),
            vec![
                StageKind::ArtistsRoot,
                StageKind::ArtistAlbums {
                    artist: "Amber Signals".to_owned()
                },
            ]
        );
        assert_eq!(
            stage_plan_parts(
                BrowseMode::Library,
                LibrarySection::Artists,
                &[artist.clone(), album.clone()]
            ),
            vec![
                StageKind::ArtistsRoot,
                StageKind::ArtistAlbums {
                    artist: "Amber Signals".to_owned()
                },
                StageKind::Tracks {
                    artist: "Amber Signals".to_owned(),
                    title: "Neon Tide".to_owned(),
                    genre: None,
                },
            ]
        );
        assert_eq!(
            stage_plan_parts(
                BrowseMode::Library,
                LibrarySection::Genres,
                &[genre.clone(), artist.clone(), album]
            ),
            vec![
                StageKind::GenresRoot,
                StageKind::GenreArtists {
                    genre: "Ambient".to_owned()
                },
                StageKind::GenreArtistAlbums {
                    artist: "Amber Signals".to_owned(),
                    genre: "Ambient".to_owned(),
                },
                StageKind::Tracks {
                    artist: "Amber Signals".to_owned(),
                    title: "Neon Tide".to_owned(),
                    genre: Some("Ambient".to_owned()),
                },
            ]
        );
        // A foreign entry at the album level (a genre there, say) adds no
        // track column: the plan keeps the root.
        assert_eq!(
            stage_plan_parts(BrowseMode::Library, LibrarySection::Albums, &[genre]),
            vec![StageKind::AlbumsRoot]
        );
        let deepest = stage_plan_parts(
            BrowseMode::Library,
            LibrarySection::Artists,
            &[
                BrowserSelection::Artist("Amber Signals".to_owned()),
                BrowserSelection::Album {
                    artist: "Amber Signals".to_owned(),
                    title: "Neon Tide".to_owned(),
                },
            ],
        );
        assert_eq!(
            deepest.last(),
            Some(&StageKind::Tracks {
                artist: "Amber Signals".to_owned(),
                title: "Neon Tide".to_owned(),
                genre: None,
            })
        );
    }

    fn tag_track(title: &str, year: u32) -> Track {
        Track::fresh_scan(
            crate::model::TrackId(format!("/music/{title}.wav")),
            std::path::PathBuf::from(format!("/music/{title}.wav")),
            riff_backend::domain::TrackMetadata {
                title: Some(title.to_owned()),
                artist: Some("Shared Artist".to_owned()),
                album: Some("Shared Album".to_owned()),
                year: Some(year),
                ..riff_backend::domain::TrackMetadata::default()
            },
            None,
            None,
            None,
            None,
        )
    }

    /// The album aggregate keeps a shared value and blanks a field the
    /// tracks disagree on, marking it `(different)`.
    #[test]
    fn the_album_draft_marks_what_the_tracks_disagree_on() {
        let tracks = [tag_track("One", 2001), tag_track("Two", 2002)];
        let draft = TagDraft::of_album(&tracks);
        assert_eq!(draft.artist, "Shared Artist");
        assert_eq!(draft.album, "Shared Album");
        assert!(draft.title.is_empty(), "the title differs");
        assert!(draft.different[0], "the title is marked");
        assert!(!draft.different[1], "the artist is shared");
        assert_eq!(draft.placeholder(0), "(different)");
        assert_eq!(
            draft.placeholder(1),
            "(none)",
            "the field is shared but unset"
        );
    }

    /// Only non-empty fields travel: a blank field never clears a tag, and a
    /// bad number is no number.
    #[test]
    fn the_draft_writes_only_what_it_carries() {
        let mut draft = TagDraft::of_track(&tag_track("One", 2001));
        draft.genre = "  ".to_owned();
        draft.track_number = "not a number".to_owned();
        let edit = draft.edit();
        assert_eq!(edit.title.as_deref(), Some("One"));
        assert_eq!(edit.genre, None, "blank text is not written");
        assert_eq!(
            edit.track_number, None,
            "an unreadable number is not written"
        );
        assert_eq!(edit.year, Some(2001));
    }

    /// Dirtiness compares against the track's own values.
    #[test]
    fn a_draft_is_dirty_only_when_a_field_moves() {
        let track = tag_track("One", 2001);
        let same = TagDraft::of_track(&track);
        assert!(!same.differs_from(&track));
        let mut changed = same.clone();
        changed.title = "Renamed".to_owned();
        assert!(changed.differs_from(&track));
    }
}
