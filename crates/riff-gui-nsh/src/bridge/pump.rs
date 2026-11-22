//! The pump: one 16 ms poller on the engine thread. At runtime the
//! sidebar's selection handlers join it as writers of the
//! [`UiState`](crate::bridge::state::UiState) signals (N5's nav signal, N6's
//! selection), and the list row-press handler joins for the selection; in
//! debug builds the fixture's variant switcher and the readout's toggles
//! also write, from the same thread. The invariant the plan states is
//! "worker threads never touch a signal", and no write anywhere leaves the
//! engine thread.
//!
//! Why a poller and not an event sink: `BackendEvent` has only typed notices
//! and coalesced Library generations (P0 correction 1) — there is no
//! playback event — so the pump locks `PlaybackSession` and reads it every
//! tick, and the inbox serves notices and generations only.
//!
//! The idle-CPU contract (the spike's pump measured 15.65% because it
//! cloned the whole 50k-track queue every tick): this tick never clones the
//! queue. `sync_playback` takes the guard's queue by reference; the cheap
//! comparison below compares scalars, queue shape and the current track's
//! id *string* without cloning it; the full [`PlaybackView`] is built only
//! when something moved, and its `current`/`up_next` `Arc`s are reused from
//! the previous snapshot whenever the playing identity is unchanged.
//!
//! Signal writes happen strictly after the `stored` borrow ends (P0
//! correction 20): the tick computes an outcome inside the borrow and the
//! interval closure writes it outside. The headless bridge test is the
//! regression gate.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nsh_core::element::{Stored, set_interval};
use riff_backend::app::MutexExt;
use riff_backend::app::events::{BackendEvent, NoticePayload};
use riff_backend::app::state::{PlaybackSession, PlaybackState, RepeatMode, TrackSort};
use riff_backend::composition::AppRuntime;

use crate::bridge::covers::{CoverArrival, CoverCache};
use crate::bridge::runtime::UiModel;
use crate::model::{
    CoverViews, EntityView, ListWindow, PlaybackView, PlaylistRow, SettingsSnapshot, SidebarView,
    StageKind, StageRows, album_rows, entity_rows, reads, stage_plan_parts,
};

/// How far ahead of the current track the projection's Up Next window is
/// kept. The spike used the same window; N9's Now Playing screen may widen
/// it.
pub(crate) const UP_NEXT_LIMIT: usize = 8;

/// The pump tick's period: the plan's 16 ms.
const TICK: Duration = Duration::from_millis(16);

/// What one tick decided to write. Built inside the `stored` borrow, applied
/// outside it.
struct TickOutcome {
    playback: Option<PlaybackView>,
    library_revision: Option<u64>,
    playlist_revision: Option<u64>,
    sidebar: Option<SidebarView>,
    settings: Option<SettingsSnapshot>,
    notices: Option<Vec<NoticePayload>>,
    covers_arrived: bool,
    list_total: Option<usize>,
    list_revision: Option<u64>,
    columns: Option<Arc<[StageRows]>>,
    folder_roots: Option<Arc<[PathBuf]>>,
    custom_lists: Option<Result<Arc<[crate::model::CustomListRow]>, String>>,
    #[allow(clippy::option_option)]
    // Some(None) clears the signal when the open list goes; None keeps it.
    playlist_detail: Option<Option<crate::model::PlaylistDetail>>,
    #[allow(clippy::option_option)]
    smart_list_detail: Option<Option<crate::model::SmartListDetail>>,
    custom_list_total: Option<usize>,
    library_paths: Option<Arc<[crate::model::LibraryPathView]>>,
    cache_status: Option<String>,
    pass_progress: Option<(usize, usize)>,
    pass_status: Option<String>,
}

/// The pump's per-tick comparison state: what the last tick saw, so a tick
/// where nothing moved writes nothing.
pub(crate) struct PumpCache {
    cheap: Option<CheapSnapshot>,
    last_view: Option<PlaybackView>,
    last_settings: Option<SettingsSnapshot>,
    last_playlists: Option<Arc<[riff_backend::domain::Playlist]>>,
    playlist_revision: u64,
    library_generation: u64,
    notice_log: VecDeque<NoticePayload>,
    primed: bool,
    list_total: usize,
    list_revision: u64,
    /// The column plan the last build served, the search needle, the listing
    /// sort and the entity roots' direction — the content identity a move
    /// rebuilds against.
    stage: Option<(Vec<StageKind>, String, TrackSort, bool)>,
    /// The entity stages' rows the last build read, kept across a plan move
    /// so a drill-down reads only the column it added.
    entity_rows: Vec<(StageKind, Arc<[EntityView]>)>,
    /// The custom smart lists last read, as the id/name pairs the sidebar
    /// and editor paint.
    custom_lists: Option<Vec<crate::model::CustomListRow>>,
    /// The playlist whose detail the last build served.
    open_playlist: Option<riff_backend::domain::PlaylistId>,
    /// The built-in smart list whose detail the last build served.
    open_smart_list: Option<riff_backend::domain::SmartPlaylistKind>,
    /// The custom list whose total the last build served, and the total.
    open_custom_list: Option<riff_backend::domain::PlaylistId>,
    custom_total: usize,
    /// The pass progress and outcome the last tick reported, and the
    /// thumbnail-cache clear's last reported outcome.
    pass_progress: (usize, usize),
    pass_status: Option<String>,
    cache_status: Option<String>,
}

impl PumpCache {
    pub(crate) fn new() -> Self {
        Self {
            cheap: None,
            last_view: None,
            last_settings: None,
            last_playlists: None,
            playlist_revision: 0,
            library_generation: 0,
            notice_log: VecDeque::new(),
            // True so the first tick builds the sidebar once; the tick's
            // own `primed = false` makes every later tick event-gated. A
            // settled library never sends the LibraryChanged event the
            // event-gated path waits for.
            primed: true,
            list_total: 0,
            list_revision: 0,
            stage: None,
            entity_rows: Vec::new(),
            custom_lists: None,
            open_playlist: None,
            open_smart_list: None,
            open_custom_list: None,
            custom_total: 0,
            pass_progress: (0, 0),
            pass_status: None,
            cache_status: None,
        }
    }
}

/// The cheap comparison snapshot: every field that decides whether the
/// [`PlaybackView`] moved, at the cost the tick can pay 60 times a second —
/// scalars, queue shape, and the current track's id compared by string
/// content without cloning it.
struct CheapSnapshot {
    state: PlaybackState,
    /// The session position rounded to a tenth of a second: while playing
    /// the raw position moves every tick, and the readout works in tenths.
    /// A whole number of tenths is exact in `f32`, so equality here is
    /// exact.
    position_tenth: f32,
    duration_bits: Option<u32>,
    volume_bits: u32,
    muted: bool,
    shuffle: bool,
    repeat: RepeatMode,
    replaygain_enabled: bool,
    replaygain_mode: riff_backend::app::state::ReplayGainMode,
    queue_len: usize,
    queue_index: usize,
    /// The current track's path, compared against the previous tick's to
    /// decide whether the view's identity moved. A one-`String` clone per
    /// tick is bounded and negligible; the contract is that no track, queue
    /// or up-next window is ever cloned.
    current_id: Option<String>,
}

impl CheapSnapshot {
    /// Read `session`'s scalars, carrying `prev`'s id `String` over when the
    /// id is unchanged (the no-clone contract above).
    fn of_session(session: &PlaybackSession, prev: Option<&Self>) -> Self {
        let id_now: Option<&str> = session
            .queue
            .current_index
            .and_then(|i| session.queue.tracks.get(i))
            .map(|id| id.0.as_str());
        let id_unchanged = match (prev.map(|p| p.current_id.as_deref()), id_now) {
            (Some(Some(a)), Some(b)) => a == b,
            (Some(None), None) => true,
            _ => false,
        };
        let current_id = if id_unchanged {
            prev.and_then(|p| p.current_id.clone())
        } else {
            id_now.map(str::to_owned)
        };
        Self {
            state: session.playback_state,
            position_tenth: (session.current_position.current.as_secs_f32() * 10.0).round(),
            duration_bits: session
                .current_position
                .total
                .map(|d| d.as_secs_f32().to_bits()),
            volume_bits: session.current_volume.to_bits(),
            muted: session.muted,
            shuffle: session.queue.shuffle,
            repeat: session.queue.repeat,
            replaygain_enabled: session.replaygain_enabled,
            replaygain_mode: session.replaygain_mode,
            queue_len: session.queue.tracks.len(),
            queue_index: session.queue.current_index.unwrap_or(0),
            current_id,
        }
    }

    /// Whether the fields the view renders from moved since `prev`.
    // `position_tenth` is `round()`'s output — whole tenths, exact in `f32`
    // at any position a track length reaches — so exact equality is the
    // correct comparison, not an epsilon.
    #[allow(clippy::float_cmp)]
    fn moved_from(&self, prev: &Self) -> bool {
        self.state != prev.state
            || self.position_tenth != prev.position_tenth
            || self.duration_bits != prev.duration_bits
            || self.volume_bits != prev.volume_bits
            || self.muted != prev.muted
            || self.shuffle != prev.shuffle
            || self.repeat != prev.repeat
            || self.queue_len != prev.queue_len
            || self.queue_index != prev.queue_index
            || self.current_id != prev.current_id
    }
}

/// Install the pump on the engine thread: copy the signal handles out once
/// and poll every tick. Called once, by the app's root component (and by the
/// headless bridge tests over a mock runtime).
pub fn install(model: Stored<UiModel>) {
    // The handles, copied once (they are `Copy`): the interval closure never
    // re-borrows the store to reach them, so a write can never happen inside
    // the borrow (P0 correction 20).
    let sigs = model.with(|m| m.state);
    set_interval(TICK, move || {
        let outcome = model.with(|m| {
            let open_playlist = m.open_playlist.clone();
            let open_smart_list = m.open_smart_list;
            let open_custom_list = m.open_custom_list.clone();
            tick(
                &mut m.rt,
                &mut m.pump,
                &mut m.covers,
                &mut m.cover_views,
                &mut m.list_window,
                &mut m.album_rows,
                open_playlist.as_ref(),
                open_smart_list,
                open_custom_list.as_ref(),
            )
        });
        // Every write below is outside the borrow: `set` notifies bindings
        // that may read the same store, and re-borrowing the `RefCell` here
        // would panic (the spike's first pump did exactly that).
        if let Some(view) = outcome.playback {
            sigs.playback.set(view);
        }
        if let Some(revision) = outcome.library_revision {
            sigs.library_revision.set(revision);
        }
        if let Some(revision) = outcome.playlist_revision {
            sigs.playlist_revision.set(revision);
        }
        if let Some(sidebar) = outcome.sidebar {
            sigs.sidebar.set(sidebar);
        }
        if let Some(settings) = outcome.settings {
            sigs.settings.set(settings);
        }
        if let Some(notices) = outcome.notices {
            sigs.notices.set(notices);
        }
        if let Some(total) = outcome.list_total {
            sigs.list_total.set(total);
        }
        if let Some(revision) = outcome.list_revision {
            sigs.list_revision.set(revision);
        }
        if let Some(rows) = outcome.columns {
            sigs.columns.set(rows);
        }
        if let Some(roots) = outcome.folder_roots {
            sigs.folder_roots.set(roots);
        }
        if let Some(read) = outcome.custom_lists {
            match read {
                Ok(rows) => {
                    sigs.custom_lists.set(rows);
                    sigs.custom_lists_error.set(None);
                }
                Err(reason) => sigs.custom_lists_error.set(Some(reason)),
            }
        }
        if let Some(detail) = outcome.playlist_detail {
            sigs.playlist_detail.set(detail);
        }
        if let Some(detail) = outcome.smart_list_detail {
            sigs.smart_list_detail.set(detail);
        }
        if let Some(total) = outcome.custom_list_total {
            sigs.custom_list_total.set(total);
        }
        if let Some(paths) = outcome.library_paths {
            sigs.library_paths.set(paths);
        }
        if let Some(status) = outcome.cache_status {
            sigs.cache_status.set(Some(status));
        }
        if let Some(progress) = outcome.pass_progress {
            sigs.pass_progress.set(progress);
        }
        if let Some(status) = outcome.pass_status {
            sigs.pass_status.set(Some(status));
        }
        if outcome.covers_arrived {
            // `update` always notifies, which is right here: the count only
            // moves when covers landed (P0 correction 9).
            sigs.covers.update(|revision| *revision += 1);
        }
    });
}

/// One tick: read the backend, decide what moved, return the writes.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn tick(
    rt: &mut AppRuntime,
    cache: &mut PumpCache,
    covers: &mut CoverCache,
    cover_views: &mut CoverViews,
    list_window: &mut ListWindow,
    album_rows_out: &mut Vec<Arc<riff_backend::domain::Track>>,
    open_playlist: Option<&riff_backend::domain::PlaylistId>,
    open_smart_list: Option<riff_backend::domain::SmartPlaylistKind>,
    open_custom_list: Option<&riff_backend::domain::PlaylistId>,
) -> TickOutcome {
    let mut outcome = TickOutcome {
        playback: None,
        library_revision: None,
        playlist_revision: None,
        sidebar: None,
        settings: None,
        notices: None,
        covers_arrived: false,
        list_total: None,
        list_revision: None,
        columns: None,
        folder_roots: None,
        custom_lists: None,
        playlist_detail: None,
        smart_list_detail: None,
        custom_list_total: None,
        library_paths: None,
        cache_status: None,
        pass_progress: None,
        pass_status: None,
    };
    let mut sidebar_dirty = cache.primed;

    // 1. The inbox: notices and coalesced Library generations only (P0
    //    correction 1).
    for event in rt.backend_events.lock_or_recover().events() {
        match event {
            BackendEvent::TypedNotice(notice) => {
                cache.notice_log.push_back(notice);
                while cache.notice_log.len() > crate::bridge::state::NOTICE_HISTORY {
                    cache.notice_log.pop_front();
                }
                outcome.notices = Some(cache.notice_log.iter().cloned().collect());
            }
            BackendEvent::LibraryChanged { generation } => {
                if generation > cache.library_generation {
                    cache.library_generation = generation;
                    outcome.library_revision = Some(generation);
                    // The Library moved: the sidebar's counts and last-scan
                    // freshness are read off it.
                    sidebar_dirty = true;
                }
            }
        }
    }

    // 2. Playback: one lock, a cheap comparison, and — every tick, because
    //    it is a stamp comparison when nothing moved — the projection's
    //    `sync_playback` over the queue *by reference* (P0 correction 2; the
    //    no-clone contract is the whole idle-CPU fix).
    let cheap;
    {
        let session = rt.playback.lock_or_recover();
        cheap = CheapSnapshot::of_session(&session, cache.cheap.as_ref());
        rt.session_views
            .sync_playback(&session.queue, UP_NEXT_LIMIT);
    }

    // 3. The Library session's scalars: the sidebar's folder-root count when
    //    the sidebar is due, the listing's sort for the track list's reads,
    //    the stage the view state names (the mount table's and the row
    //    window's shared computation, from the same pure fn the layout
    //    reads), and the settings comparison (compare first, so a
    //    nothing-moved tick rebuilds nothing). The search needle is cloned
    //    only when it moved — a per-tick keystroke clone is the clone
    //    discipline's one exception, not a habit.
    let folder_roots;
    let track_sort;
    let plan;
    let stage_desc;
    let mut stage_query: Option<String> = None;
    let mut path_views: Option<Arc<[crate::model::LibraryPathView]>> = None;
    {
        let library = rt.library.lock_or_recover();
        folder_roots = library.library_paths.paths().len();
        if sidebar_dirty {
            use riff_backend::app::state::WatchState;
            path_views = Some(
                library
                    .library_paths
                    .paths()
                    .iter()
                    .map(|path| crate::model::LibraryPathView {
                        path: path.clone(),
                        readiness: crate::model::LibraryPathView::readiness_of(
                            &library.library_paths.readiness(path),
                        ),
                        watching: library.library_paths.watch_state(path) == WatchState::Enabled,
                        watch_warning: match library.library_paths.watch_state(path) {
                            WatchState::Warning(reason) => Some(reason),
                            _ => None,
                        },
                    })
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        track_sort = library.track_sort;
        plan = stage_plan_parts(
            library.browse_mode,
            library.library_section,
            &library.browser_path,
        );
        stage_desc = library.browser_sort_desc;
        if cache
            .stage
            .as_ref()
            .is_none_or(|(_, held, _, _)| held != library.search_query.as_str())
        {
            stage_query = Some(library.search_query.clone());
        }
        let moved = cache.last_settings.as_ref().is_none_or(|last| {
            !last.matches_sessions(cheap.replaygain_enabled, cheap.replaygain_mode, &library)
        });
        if moved {
            outcome.settings = Some(SettingsSnapshot::of_sessions(
                cheap.replaygain_enabled,
                cheap.replaygain_mode,
                &library,
            ));
        }
    }
    outcome.library_paths = path_views;

    // 4. PlaybackView: built only when the cheap comparison moved (or on the
    //    priming tick), with the identity `Arc`s reused when the playing
    //    identity is unchanged.
    let playback_moved = cache
        .cheap
        .as_ref()
        .is_none_or(|prev| cheap.moved_from(prev));
    if playback_moved {
        let library_moved = outcome.library_revision.is_some();
        let identity_moved = library_moved
            || cache.cheap.as_ref().is_none_or(|prev| {
                prev.queue_len != cheap.queue_len
                    || prev.queue_index != cheap.queue_index
                    || prev.current_id != cheap.current_id
            });
        let (current, up_next) = if identity_moved {
            (
                rt.session_views.playback_current().cloned().map(Arc::new),
                Arc::from(rt.session_views.playback_up_next().to_vec()),
            )
        } else {
            // Refcount bumps only: the Arcs are the view's own equality
            // (see `PlaybackView`'s `PartialEq`).
            cache.last_view.as_ref().map_or_else(
                || (None, Arc::from(Vec::new())),
                |view| (view.current.clone(), view.up_next.clone()),
            )
        };
        let view = PlaybackView {
            state: cheap.state,
            position_secs: cheap.position_tenth / 10.0,
            duration_secs: cheap.duration_bits.map(f32::from_bits),
            volume: f32::from_bits(cheap.volume_bits),
            muted: cheap.muted,
            shuffle: cheap.shuffle,
            repeat: cheap.repeat,
            current,
            up_next,
            queue_index: cheap.queue_index,
            queue_len: cheap.queue_len,
        };
        cache.last_view = Some(view.clone());
        outcome.playback = Some(view);
    }
    cache.cheap = Some(cheap);

    // 5. Playlists: the playlist generation has no inbox event, so the pump
    //    observes the projection's cache — a refetch (generation moved)
    //    hands out a fresh `Arc`, and pointer identity is the change. A
    //    moved list is also a sidebar change: the PLAYLISTS rows are the
    //    list's rows (N5), so it feeds the sidebar's dirty flag.
    let playlists = reads::playlists(&mut rt.session_views);
    let playlists_moved = cache
        .last_playlists
        .as_ref()
        .is_none_or(|last| !Arc::ptr_eq(last, &playlists));
    if playlists_moved {
        cache.last_playlists = Some(Arc::clone(&playlists));
        cache.playlist_revision += 1;
        outcome.playlist_revision = Some(cache.playlist_revision);
        sidebar_dirty = true;
    }

    // 6. Sidebar: built once on the first tick (a settled library never
    //    sends the LibraryChanged event the later ticks wait for), then
    //    refreshed when the Library generation moves or the playlists
    //    refetched. The signal's own equality absorbs a same-value rewrite.
    //    The folder roots ride the same build: the Folders tree's top level
    //    is the session's registered paths, and a render binding cannot
    //    lock the session to read them.
    if sidebar_dirty {
        let counts = rt.session_views.sidebar_counts(folder_roots);
        let roots: Arc<[PathBuf]> = rt
            .library
            .lock_or_recover()
            .library_paths
            .paths()
            .iter()
            .cloned()
            .collect();
        outcome.folder_roots = Some(roots);
        let rows = playlists
            .iter()
            .map(|playlist| PlaylistRow {
                id: playlist.id.clone(),
                name: playlist.name.clone(),
                tracks: playlist.tracks.len(),
            })
            .collect();
        let view = SidebarView {
            tracks: counts.tracks,
            artists: counts.artists,
            albums: counts.albums,
            genres: counts.genres,
            folder_roots,
            smart_lists: Arc::clone(&counts.smart_lists),
            playlists: rows,
            last_scan: reads::last_scan(&mut rt.session_views),
        };
        outcome.sidebar = Some(view);
    }

    // 6b. The custom smart playlists: the sidebar's SMART LISTS rows and the
    //     editor's load both read them. They move with the playlist
    //     generation (names and identity) and with the Library generation
    //     (membership is a query over tracks), so the same dirty flag
    //     serves. The failure reason travels alone, never a degraded empty.
    if sidebar_dirty || cache.custom_lists.is_none() {
        match reads::try_custom_smart_playlists(&mut rt.session_views) {
            Ok(lists) => {
                let rows: Vec<crate::model::CustomListRow> = lists
                    .into_iter()
                    .map(|list| crate::model::CustomListRow {
                        id: list.id,
                        name: list.name,
                    })
                    .collect();
                if cache.custom_lists.as_ref() != Some(&rows) {
                    cache.custom_lists = Some(rows.clone());
                    outcome.custom_lists = Some(Ok(rows.into()));
                }
            }
            Err(reason) => {
                outcome.custom_lists = Some(Err(reason));
            }
        }
    }

    // 6c. The open playlist's detail: rebuilt when the open list changes or
    //     either generation behind its rows moved. The nav signal is not
    //     read here — the selection handler records the open list on the
    //     model instead.
    let detail_moved = cache.open_playlist.as_ref() != open_playlist
        || outcome.playlist_revision.is_some()
        || outcome.library_revision.is_some();
    if detail_moved {
        cache.open_playlist = open_playlist.cloned();
        outcome.playlist_detail = Some(
            open_playlist.and_then(|id| crate::model::playlist_detail(&mut rt.session_views, id)),
        );
    }

    // 6d. The open built-in smart list: its tracks re-read when the list
    //     opens or the Library generation moves.
    let smart_moved =
        cache.open_smart_list != open_smart_list || outcome.library_revision.is_some();
    if smart_moved {
        cache.open_smart_list = open_smart_list;
        outcome.smart_list_detail = Some(
            open_smart_list
                .map(|kind| crate::model::smart_list_detail(&mut rt.session_views, kind)),
        );
    }

    // 6e. The open custom smart list's total: re-read when the list opens or
    //     either generation moves (membership is a Library query, identity a
    //     playlist one). Rows are read by the stage's virtual list through
    //     the seam's own bounded window.
    let custom_moved = cache.open_custom_list.as_ref() != open_custom_list
        || outcome.library_revision.is_some()
        || outcome.playlist_revision.is_some();
    if custom_moved {
        cache.open_custom_list = open_custom_list.cloned();
        let total = open_custom_list.map_or(0, |id| {
            rt.session_views
                .custom_list_count(id, TrackSort::NumberAsc, None)
                .unwrap_or_default()
        });
        if total != cache.custom_total {
            cache.custom_total = total;
            outcome.custom_list_total = Some(total);
        }
    }

    // 7. Covers: drain the service through the ported decision half; every
    //    arrival (artless verdicts included, as terminal non-answers) leaves
    //    the in-flight set, and the ones with pixels enter the View map,
    //    bounded — what the insert pushed out is forgotten by the decision
    //    half, so an evicted row's next ask re-requests.
    for arrival in covers.settle(rt.covers.as_ref()) {
        let CoverArrival { key, cover } = arrival;
        let bytes = cover.rgba.len();
        let evicted = cover_views.insert(key, crate::bridge::covers::arrival_image(cover), bytes);
        covers.forget(evicted);
        outcome.covers_arrived = true;
    }

    // 8. The stage's signals: the listing's total (the track stages' virtual
    //    list sizes with it; other stages size nothing), the warm row
    //    window's freshness, and every entity column's rows. The plan,
    //    search needle, listing sort and entity direction form the content
    //    identity a move rebuilds against; a plan move that only appends the
    //    Tracks column reuses the parent columns' rows and reads just the
    //    stage it added.
    let listing_moved = match cache.stage.as_ref() {
        None => true,
        Some((_, held_query, held_sort, held_desc)) => {
            stage_query
                .as_deref()
                .is_some_and(|needle| needle != held_query.as_str())
                || *held_sort != track_sort
                || *held_desc != stage_desc
        }
    };
    let plan_moved = cache
        .stage
        .as_ref()
        .is_none_or(|(held, _, _, _)| held != &plan);
    let stage_moved = listing_moved || plan_moved;
    let entity_columns_moved = {
        let next: Vec<&StageKind> = plan.iter().filter(|s| s.is_entity_listing()).collect();
        let held: Vec<&StageKind> = cache.entity_rows.iter().map(|(stage, _)| stage).collect();
        listing_moved || next != held
    };
    let query: &str = stage_query.as_deref().unwrap_or_else(|| {
        cache
            .stage
            .as_ref()
            .map_or("", |(_, held, _, _)| held.as_str())
    });
    let deepest = plan
        .last()
        .expect("every stage plan names at least the section root");
    let total = match deepest {
        StageKind::Flat => reads::track_count(&mut rt.session_views, query, track_sort),
        StageKind::Tracks { .. } => {
            if stage_moved {
                *album_rows_out = album_rows(&mut rt.session_views, deepest, query, track_sort);
            }
            album_rows_out.len()
        }
        _ => 0,
    };
    if entity_columns_moved {
        let retained = std::mem::take(&mut cache.entity_rows);
        let mut rows = Vec::new();
        for stage in plan.iter().filter(|stage| stage.is_entity_listing()) {
            let held = retained
                .iter()
                .find(|(held, _)| held == stage)
                .map(|(_, rows)| rows.clone());
            let stage_rows = if listing_moved {
                entity_rows(&mut rt.session_views, stage, query, stage_desc)
            } else {
                held.unwrap_or_else(|| entity_rows(&mut rt.session_views, stage, query, stage_desc))
            };
            rows.push(StageRows {
                stage: stage.clone(),
                rows: stage_rows,
            });
        }
        cache.entity_rows = rows
            .iter()
            .map(|column| (column.stage.clone(), column.rows.clone()))
            .collect();
        outcome.columns = Some(Arc::from(rows));
    }
    if stage_moved {
        let needle = stage_query
            .clone()
            .or_else(|| cache.stage.as_ref().map(|(_, held, _, _)| held.clone()))
            .unwrap_or_default();
        cache.stage = Some((plan.clone(), needle.clone(), track_sort, stage_desc));
        list_window.set_query(needle.as_str());
        list_window.set_sort(track_sort);
        list_window.reset();
        cache.list_revision += 1;
        outcome.list_revision = Some(cache.list_revision);
    }
    if total != cache.list_total {
        cache.list_total = total;
        outcome.list_total = Some(total);
    }

    // 8b. The background services' settled reports: the ReplayGain pass's
    //     progress and outcome, and the thumbnail cache clear's verdict. The
    //     reads are non-blocking cell reads, cheap enough for every tick.
    let progress = rt.passes.poll_progress();
    if progress != cache.pass_progress {
        cache.pass_progress = progress;
        outcome.pass_progress = Some(progress);
    }
    if let Some(report) = rt.passes.poll() {
        let mut line = format!(
            "ReplayGain pass: measured {}, skipped {}, failed {}",
            report.measured, report.skipped, report.failed
        );
        if report.cancelled {
            line.push_str(" (cancelled)");
        }
        if let Some(reason) = report.first_failure {
            line.push_str(": ");
            line.push_str(&reason);
        }
        if cache.pass_status.as_deref() != Some(line.as_str()) {
            cache.pass_status = Some(line.clone());
            outcome.pass_status = Some(line);
        }
    }
    if let Some(clear) = rt.covers.poll_cache_clear() {
        let line = match clear {
            riff_backend::app::cover_service::ClearCacheOutcome::Cleared => {
                "Thumbnail cache cleared".to_owned()
            }
            riff_backend::app::cover_service::ClearCacheOutcome::Failed { reason } => {
                format!("Could not clear the thumbnail cache: {reason}")
            }
        };
        if cache.cache_status.as_deref() != Some(line.as_str()) {
            cache.cache_status = Some(line.clone());
            outcome.cache_status = Some(line);
        }
    }

    // 9. The settings round-trip: the pump is this frontend's frame end.
    //    Both session locks, acquired in `hydrate`'s order (playback, then
    //    library — never the reverse), held only for the diff, which reads
    //    scalar fields and clones no queue.
    {
        let playback_guard = rt.playback.lock_or_recover();
        let library_guard = rt.library.lock_or_recover();
        rt.preferences
            .commit_if_changed(&playback_guard, &library_guard, rt.settings.as_mut());
    }

    cache.primed = false;
    outcome
}
