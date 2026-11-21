//! The pump: one 16 ms poller on the engine thread. At runtime the
//! sidebar's selection handlers join it as writers of the
//! [`UiState`](crate::bridge::state::UiState) signals (N5's nav signal);
//! in debug builds the fixture's variant switcher and the readout's toggles
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
use std::sync::Arc;
use std::time::Duration;

use nsh_core::element::{Stored, set_interval};
use riff_backend::app::MutexExt;
use riff_backend::app::events::{BackendEvent, NoticePayload};
use riff_backend::app::state::{PlaybackSession, PlaybackState, RepeatMode};
use riff_backend::composition::AppRuntime;

use crate::bridge::covers::{CoverArrival, CoverCache};
use crate::bridge::runtime::UiModel;
use crate::model::{CoverMap, PlaybackView, PlaylistRow, SettingsSnapshot, SidebarView, reads};

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
        let outcome =
            model.with(|m| tick(&mut m.rt, &mut m.pump, &mut m.covers, &mut m.covers_map));
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
        if outcome.covers_arrived {
            // `update` always notifies, which is right here: the count only
            // moves when covers landed (P0 correction 9).
            sigs.covers.update(|revision| *revision += 1);
        }
    });
}

/// One tick: read the backend, decide what moved, return the writes.
#[allow(clippy::too_many_lines)]
fn tick(
    rt: &mut AppRuntime,
    cache: &mut PumpCache,
    covers: &mut CoverCache,
    covers_map: &mut CoverMap,
) -> TickOutcome {
    let mut outcome = TickOutcome {
        playback: None,
        library_revision: None,
        playlist_revision: None,
        sidebar: None,
        settings: None,
        notices: None,
        covers_arrived: false,
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
    //    the sidebar is due, and the settings comparison (compare first, so
    //    a nothing-moved tick rebuilds nothing).
    let folder_roots;
    {
        let library = rt.library.lock_or_recover();
        folder_roots = library.library_paths.paths().len();
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
    if sidebar_dirty {
        let counts = rt.session_views.sidebar_counts(folder_roots);
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

    // 7. Covers: drain the service through the ported decision half; every
    //    arrival (artless verdicts included, as terminal non-answers) leaves
    //    the in-flight set, and the ones with pixels enter the View map.
    for arrival in covers.settle(rt.covers.as_ref()) {
        let CoverArrival { key, cover } = arrival;
        covers_map.insert(key, crate::bridge::covers::arrival_image(cover));
        outcome.covers_arrived = true;
    }

    // 8. The settings round-trip: the pump is this frontend's frame end.
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
