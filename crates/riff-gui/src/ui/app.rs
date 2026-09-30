mod browser_pane;
mod library_picker;
mod selection_pane;
mod tag_editor;
mod track_menu;

pub use library_picker::register_library_path;
pub use tag_editor::InlineTagEditor;
pub use track_menu::{TrackMenuHost, TrackMenuOpen, TrackMenuSubject};

use crate::ui::chrome::TitleBarAction;
use crate::ui::column::ColumnIdentity;
use crate::ui::now_playing::NowPlayingAction;
use crate::ui::playerbar::PlayerBarAction;
use crate::ui::settings::SettingsSection;
use crate::ui::sidebar::UpNextEntry;
use crate::ui::theme::{self, Palette};
// Ungated, unlike the visibility *channel* fields: `CUSTOM_TITLEBAR_CLOSE` is
// compiled on every platform (it is the pure data `close_resolution` hands
// back, which is deliberately ungated so the close decision stays assertable
// from the Linux/Windows CI machines), so the type it is declared with must be
// in scope on Linux too. Gating this import with the channel would leave the
// const unnameable there.
use crate::ui::window_visibility::VisibilityMessage;
use eframe::egui;
// The Frame (ui-frame-deepening issue 04) owns the frame's order: its
// decision half, the three panel reports, and the six-field write-back. This
// file draws what those decisions hand it and enacts what the frame decided.
use crate::ui::frame::{
    ControlBarReport, Frame, FrameInput, FrameOutput, FrameParts, FrameViewport, SidebarAction,
    SidebarReport, StageReport, TitlebarReport,
};
// The Frame's own public appliers keep their historical `ui::app::` paths
// resolving, so the test suite that has asserted them since before the Frame
// existed needed no editing.
pub use crate::ui::frame::TitleKey;
pub use crate::ui::frame::{
    apply_backend_events, apply_now_playing_action, apply_player_bar_action,
};
use riff_backend::app::MutexExt;
use riff_backend::app::Transport;
pub use riff_backend::app::cover_service::{
    COVER_CACHE_CAP, ClearCacheOutcome, Covers, lru_insert,
};
// The artwork cache-key space moved to `ui::artwork` with the artwork primitive
// (issue 13); these keep the historical `ui::app::` paths resolving.
pub use crate::ui::artwork::{
    COVER_CARD, COVER_HERO, COVER_IN_FLIGHT_CAP, COVER_TEXTURE_BYTE_BUDGET, COVER_THUMB,
    CoverCacheKey, cover_cache_key,
};
// The elastic stage's sizing policy moved in with the stage geometry itself;
// this keeps the historical `ui::app::column_widths` path resolving.
pub use crate::ui::stage::column_widths;
use riff_backend::app::events::BackendEvents;
use riff_backend::app::preferences::Preferences;
use riff_backend::app::scan_service::Scans;
use riff_backend::app::state::{
    BrowseMode, BrowserSelection, LibrarySection, LibrarySession, PlaybackSession, ViewMode,
};
use riff_backend::app::store::{LibraryMutationStore, PlaylistStore, SettingsStore};
use riff_backend::app::tag_edit_service::TagEdits;
use riff_backend::app::traits::RequestedSize;
use riff_backend::app::views::SessionViews;
use riff_backend::app::watcher_manager::WatcherManager;
use riff_backend::domain::{PlaybackState, PlaylistId, SmartPlaylistKind, Track, TrackId};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
// The app-wide quit intent, read by the macOS close block below. Gated to
// macOS with the field and the parameter that carry it: nothing on Windows or
// Linux names it, and an ungated import would be an `unused_imports` there.
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicBool;

/// Theme selection state: the light/dark choice plus the (dark, high-contrast)
/// combination last installed on the egui context, so the token style is
/// applied once at init and re-applied only when the user switches (Issue 01).
pub struct ThemeState {
    /// `true` = dark (mockup palette), `false` = light (derived per ADR 0004).
    /// Written by the Frame's titlebar action applier and read by the theme
    /// step and the titlebar's content, so it is visible to `ui::frame`.
    pub dark: bool,
    /// The resolved palette currently installed on the context (Issue 03):
    /// view code reads its semantic slots instead of hardcoding colors, so
    /// every themed surface follows the active palette (ADR 0004).
    pub active: theme::Palette,
    /// The `(dark, high_contrast)` pair currently installed on the context,
    /// or `None` before the first install.
    pub last_applied: Option<(bool, bool)>,
}

/// `"Artist - Title"` for one track — flat list, search, playlists, window
/// title. Formatted fresh each frame; staleness is the projections' job.
fn label_artist_title(track: &Track) -> String {
    format!(
        "{} - {}",
        track.metadata.display_artist(),
        track.metadata.display_title(&track.file_path)
    )
}

/// `"N. Title"` for one track — album and folder tree rows.
fn label_numbered(track: &Track) -> String {
    format!(
        "{}. {}",
        track.metadata.track_number.unwrap_or(0),
        track.metadata.display_title(&track.file_path)
    )
}

/// The now-playing title/meta lines, resolved once per frame by
/// [`RiffApp::render_control_bar`] (the bar renders before every stage) and
/// carried through [`RiffApp::now_playing_labels`] to the Now Playing stage,
/// so neither surface builds the strings twice.
type NowPlayingLabels = (Option<Arc<str>>, Option<Arc<str>>);

/// The state one interactive Track row needs that cannot stay behind
/// `&mut self` across [`sidebar::tree_row`], which borrows the icon cache
/// mutably — the very borrow that forced the state to be threaded parameter by
/// parameter through three render sites with seven to nine of them.
///
/// This bundle is the ownership shape that removes the threading: it is built
/// once per listing and handed down, so a row site reads four arguments instead
/// of nine and the "which session does a row write to" question has one answer.
/// Precedent: `PlaylistPromptSlots`, `CollectionMenuEffects`, `TrackMenuHost`.
///
/// `library` is the one mutable field, and deliberately so: the row's clicks
/// write selection straight into the session, and threading that back through a
/// return value would be the same defect wearing a hat.
pub struct TrackRowContext<'a> {
    /// The library session: a click selects, a right-click's opening selects,
    /// and the context menu's own effects land on the same slot.
    pub library: &'a mut LibrarySession,
    /// The playback snapshot, for the playing-row indicator.
    pub playback: &'a PlaybackSession,
    /// The playing Track's id, if any.
    pub current_track: Option<&'a TrackId>,
    /// The playlist this listing is, with the row's CANONICAL index — the
    /// drag-reorder gesture's coordinates. `None` for every listing that is not
    /// a user playlist's.
    pub reorder: Option<PlaylistSlot<'a>>,
}

/// One playlist entry's position: which playlist, which canonical index, and
/// whether the listing's current sort allows dragging at all (it does not: a
/// drag would persist positions read off the wrong order).
pub struct PlaylistSlot<'a> {
    /// The playlist whose entries these are.
    pub playlist_id: &'a PlaylistId,
    /// The entry's index in the playlist's CANONICAL order.
    pub index: usize,
    /// Whether the drag-and-drop wrapper is on.
    pub reorderable: bool,
}

/// One track row's own presentation: the Track it denotes and how it reads.
///
/// Split from [`TrackRowContext`] because this part is *data about the row* and
/// that part is *state about the listing* — the same distinction that keeps
/// `BrowserColumn` and `DetailColumn` from becoming one wide struct.
pub struct TrackRowSpec<'a> {
    /// The Track the row denotes.
    pub track: &'a Track,
    /// The painted label; callers keep their display formats ("Artist - Title",
    /// "01. Title").
    pub label: &'a str,
    /// The indent scale level.
    pub indent_level: usize,
}

/// What the sidebar's LIBRARY section looks like this frame: the one counts
/// read every nav row's live count comes from, and which rows highlight because
/// their browser variant is the one actually on screen.
///
/// Read once, resolved once, and handed to the row pass whole — the same
/// "resolve, then paint" shape [`TrackRowSpec`] gives the track rows, and the
/// same reason: the row pass should not be able to re-derive "is this section
/// live?" four times and get it wrong once.
pub struct LibraryNav<'a> {
    /// The counts read model, cached per store generation (handoff issue 05).
    pub counts: &'a riff_backend::app::views::SidebarCounts,
    /// A section row highlights only while its browser variant is on screen:
    /// the Library view, the Library browse mode, no list opened over it.
    pub library_section_live: bool,
    /// The Folders row's own liveness: the Library view in Folders mode.
    pub folder_section_live: bool,
}

/// Transient UI prompt/focus flags are genuinely two-state; the fourth bool
/// only exists on Linux (`settings_show_input`), which is where the lint fires.
#[allow(clippy::struct_excessive_bools)]
pub struct RiffApp {
    pub playback: Arc<Mutex<PlaybackSession>>,
    /// The library session: selection, view/browse mode, search, library
    /// roots + per-path statuses, scan status, UI flags, and per-path watch
    /// state. UI-owned; only the UI thread mutates it.
    pub library: Arc<Mutex<LibrarySession>>,
    /// The Transport port: the UI's intent-level playback front end. Command
    /// mapping, seek clamping, and volume math live behind it (in the
    /// `ChannelTransport` adapter); the UI never names engine commands.
    transport: Box<dyn Transport>,
    /// The Library Scan Service front end (ADR 0006): requests scans and
    /// yields polled outcomes; the whole walk/commit/cancel flow and the
    /// per-path scan state live behind it. The watcher thread holds its own
    /// clone of the same shareable service.
    pub(crate) scans: Box<dyn Scans>,
    cover_textures: std::collections::HashMap<CoverCacheKey, egui::TextureHandle>,
    /// The View half's LRU order for `cover_textures` — the map is one bound and
    /// this is how the other finds its victims.
    cover_lru_keys: Vec<CoverCacheKey>,
    /// The Cover Cache: what the application knows about a Cover — which are
    /// wanted, at which size, which are in flight, and which have arrived. It
    /// holds no picture, so the texture map above and this are separate facts
    /// that neither answers for the other; the two meet only through the
    /// `crate::ui::artwork` write and read paths, which report what the map's
    /// own bounds dropped.
    cover_cache: crate::ui::cover_cache::CoverCache,
    /// The Cover Service front end (ADR 0006): sends resolve intent and
    /// yields drained results; dedup and the negative cache live behind it.
    covers: Box<dyn Covers>,
    /// The Tag Edit Service front end (ADR 0006) lives inside the inline
    /// editor's controller: submits save intent and yields polled outcomes;
    /// the whole save flow lives behind it.
    tag_editor: InlineTagEditor,
    /// Which read-only smart playlist is open in the library explorer, if any.
    /// Transient UI state (precedent: `tag_edit`); the playlist contents are
    /// re-computed from library data on every frame, so nothing is cached.
    smart_playlist_view: Option<SmartPlaylistKind>,
    /// Which user playlist is open in the library explorer, if any.
    playlist_view: Option<PlaylistId>,
    /// The playing track's id whose cover the Now Playing stage last
    /// requested/rendered; only re-cloned when the track moves.
    now_playing_cover_key: Option<String>,
    /// This frame's resolved current-track `(title, meta_line)` lines:
    /// stashed once by `render_control_bar` (the bar renders before every
    /// stage) and read by the Now Playing stage, so both surfaces share one
    /// `Arc` handout and the strings are never built twice.
    now_playing_labels: NowPlayingLabels,
    /// The current `TrackId` the window title and tray tooltip were last
    /// pushed for (REQ-SI-001): both OS side effects only fire when this
    /// identity moves — pure command suppression, never staleness.
    last_title_key: TitleKey,
    /// Retained seek-row readout buffers for the playerbar and the
    /// Now Playing stage (allocation plan 2.4).
    playerbar_readouts: crate::ui::playerbar::SeekReadouts,
    stage_readouts: crate::ui::playerbar::SeekReadouts,
    /// Caller-retained action buffers for the shell widgets (allocation
    /// plan 2.5): cleared and refilled per frame so idle frames never build
    /// a fresh `Vec`.
    titlebar_actions: Vec<TitleBarAction>,
    playerbar_actions: Vec<PlayerBarAction>,
    now_playing_actions: Vec<NowPlayingAction>,
    /// The in-memory scroll record per Library Section (scroll-memory spec):
    /// one slot per Section plus the drill reset bookkeeping. Pure GUI
    /// presentation state — never crosses the session boundary, never
    /// touches the Application Store.
    pub(crate) scroll_memory: crate::ui::scroll_memory::ScrollMemory,
    /// Transient "New Playlist" name prompt (`Some` = open, holds the draft).
    playlist_create_name: Option<String>,
    /// Transient rename prompt: (playlist id, draft name).
    playlist_rename: Option<(PlaylistId, String)>,
    /// Transient Clear Library confirmation (`true` = awaiting confirm).
    /// Grouped with the other transient prompts on `RiffApp`.
    pub(crate) clear_library_confirm: bool,
    /// Transient Clear Thumbnail cache confirmation, same shape as the one above.
    pub(crate) clear_thumbnail_cache_confirm: bool,
    /// A Thumbnail-cache clear the worker has not answered yet. The frame drains
    /// its outcome while this is set, and a second press is ignored — the way the
    /// Tag Edit controller holds one outstanding record.
    pub(crate) clear_cache_in_flight: bool,
    /// Ctrl+K request flag (issue 06): one-shot focus request for the global
    /// search field, consumed on the frame it lands.
    global_search_focus: bool,
    pub(crate) watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
    /// The Application Store's settings section. `Preferences` diff-commits
    /// the sessions back through it at frame end, so preferences survive
    /// restarts through the store.
    pub(crate) settings_store: Box<dyn SettingsStore>,
    /// The Settings round-trip owner, hydrated by the App Runtime's
    /// composition before the frontend existed: the sessions already carry
    /// the stored Settings and the restored Library Paths are already being
    /// watched. The frontend only diff-commits session changes back to the
    /// Application Store at frame end, so a preference change is durable by
    /// construction.
    prefs: Preferences,
    /// The Application Store's playlists section. Every playlist mutation
    /// commits through it as one immediate durable transaction; its adapter
    /// owns the session playlist generation whose bumps invalidate the
    /// seam's playlist projection automatically — reads go through
    /// [`Self::views`] and no commit site refreshes or patches anything.
    pub(crate) playlist_store: Box<dyn PlaylistStore>,
    /// The Application Store's Library collection mutation port: committed
    /// metadata changes (e.g. tag edits) persist through it as one durable
    /// transaction per batch.
    pub(crate) library_mutations: Box<dyn LibraryMutationStore>,
    /// The Session Views seam (ADR 0002): every store-backed read the UI
    /// renders — flat list, search, browsing, folders, smart playlists, and
    /// the playback-side slots — goes through it. It owns the five bounded
    /// Session Projections, the Library query port, and the session-local
    /// generation counter, so view code never touches staleness handling or
    /// store-error fallbacks.
    pub(crate) views: SessionViews,
    pub(crate) theme: ThemeState,
    /// Vendored-glyph texture cache for the shell's icon controls (Issue 06).
    pub(crate) icons: crate::ui::icons::IconCache,
    /// The section the Settings modal's left nav currently shows (Issue 11).
    /// Opens on Library, the pane the Library settings rewire builds on.
    pub(crate) settings_section: SettingsSection,
    /// The structured feedback board the titlebar paints from (issue 11):
    /// independent persistent slots per source so a scan line cannot erase a
    /// playback error or a Tag Edit outcome, carrying severity, source, and any
    /// recovery intent to the paint boundary. Its `display_message` feeds the
    /// existing `scan_status` line unchanged.
    pub(crate) feedback: crate::ui::feedback::FeedbackBoard,
    /// Linux-only folder-picker input state (no native file dialog there).
    /// Grouped so the rest of the struct keeps its cross-platform shape.
    #[cfg(target_os = "linux")]
    pub(crate) settings_text_input: String,
    #[cfg(target_os = "linux")]
    pub(crate) settings_show_input: bool,
    #[cfg(target_os = "linux")]
    pub(crate) settings_path_error: Option<String>,
    #[cfg(not(target_os = "linux"))]
    tray_icon: Option<tray_icon::TrayIcon>,
    /// Last tooltip text pushed to the tray icon (REQ-SI-001). Used to
    /// deduplicate `set_tooltip` calls so it only runs when the text changes.
    #[cfg(not(target_os = "linux"))]
    last_tray_tooltip: String,
    /// Frontend-local visibility channel (Issue 03). The tray thread pushes
    /// [`VisibilityMessage`] requests over this and the UI thread drains it
    /// on every logic tick — no backend state, no audio engine involvement.
    #[cfg(not(target_os = "linux"))]
    visibility_listener: crate::ui::window_visibility::VisibilityListener,
    /// The custom titlebar X's own visibility sender (split-close-paths): the
    /// X is the only hide gesture on macOS/Windows, and it enqueues
    /// `VisibilityMessage(false)` over it so `logic()` applies the hide
    /// through the same drain every other visibility request uses.
    #[cfg(not(target_os = "linux"))]
    visibility_tx: crate::ui::window_visibility::VisibilityTx,
    /// Whether the app has hidden the window to the tray. The app's own record,
    /// because egui never reports real visibility back to it (see `logic`).
    #[cfg(not(target_os = "linux"))]
    window_hidden: bool,
    /// The app-wide quit intent, shared with the tray (a clone of
    /// `AppRuntime::spawn`'s `quit_flag`). The tray stores `true` BEFORE it
    /// enqueues its `Close`, and the macOS close block below loads it to tell a
    /// riff-initiated quit apart from an OS window close — a distinction
    /// `ViewportEvent::Close` does not carry, since that variant has no
    /// payload and is identical to the one the red traffic light produces.
    ///
    /// macOS-only: Windows has no native close request to resolve (its frameless
    /// OS close always quits) and Linux has no tray, so nothing there reads the
    /// flag and a `not(linux)` gate would make CI's `-D warnings` fail on
    /// `dead_code` for a field no Windows code path touches.
    #[cfg(target_os = "macos")]
    quit_flag: Arc<AtomicBool>,
    /// The Backend Events inbox: the observable surface both the Transport
    /// wrapper and the tray thread record dispatched commands onto, and the
    /// inbox the UI drains at the start of every frame.
    backend_events: Arc<Mutex<BackendEvents>>,
}

impl RiffApp {
    /// Composition-root constructor: the main thread wires every dependency
    /// by hand, so the parameter count is the wiring surface itself.
    ///
    /// `preferences` arrives already hydrated — the App Runtime's composition
    /// loaded the stored Settings into these two sessions and started the
    /// restored Library Paths' watchers before the frontend was built, so
    /// there is nothing for the first frame to restore.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        playback: Arc<Mutex<PlaybackSession>>,
        library: Arc<Mutex<LibrarySession>>,
        transport: Box<dyn Transport>,
        scans: Box<dyn Scans>,
        watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
        #[cfg(not(target_os = "linux"))] tray_icon: Option<tray_icon::TrayIcon>,
        settings_store: Box<dyn SettingsStore>,
        preferences: Preferences,
        playlist_store: Box<dyn PlaylistStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        views: SessionViews,
        tag_edits: Box<dyn TagEdits>,
        covers: Box<dyn Covers>,
        backend_events: Arc<Mutex<BackendEvents>>,
        #[cfg(not(target_os = "linux"))]
        visibility_listener: crate::ui::window_visibility::VisibilityListener,
        #[cfg(not(target_os = "linux"))] visibility_tx: crate::ui::window_visibility::VisibilityTx,
        // Trailing so the macOS-only wiring is an add-on to the cross-platform
        // list rather than interleaved with it.
        #[cfg(target_os = "macos")] quit_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            playback,
            library,
            transport,
            scans,
            cover_textures: std::collections::HashMap::new(),
            cover_lru_keys: Vec::new(),
            cover_cache: crate::ui::cover_cache::CoverCache::new(),
            covers,
            tag_editor: InlineTagEditor::new(tag_edits),
            smart_playlist_view: None,
            playlist_view: None,
            now_playing_cover_key: None,
            now_playing_labels: (None, None),
            last_title_key: TitleKey::Unset,
            playerbar_readouts: crate::ui::playerbar::SeekReadouts::new(),
            stage_readouts: crate::ui::playerbar::SeekReadouts::new(),
            titlebar_actions: Vec::new(),
            playerbar_actions: Vec::new(),
            now_playing_actions: Vec::new(),
            scroll_memory: crate::ui::scroll_memory::ScrollMemory::default(),
            playlist_create_name: None,
            playlist_rename: None,
            clear_library_confirm: false,
            clear_thumbnail_cache_confirm: false,
            clear_cache_in_flight: false,
            global_search_focus: false,
            watcher_manager,
            settings_store,
            prefs: preferences,
            playlist_store,
            library_mutations,
            views,
            theme: ThemeState {
                dark: true, // dark (mockup palette) by default
                active: theme::Palette::dark(),
                last_applied: None,
            },
            icons: crate::ui::icons::IconCache::new(),
            settings_section: SettingsSection::Library,
            feedback: crate::ui::feedback::FeedbackBoard::default(),
            #[cfg(target_os = "linux")]
            settings_text_input: String::new(),
            #[cfg(target_os = "linux")]
            settings_show_input: false,
            #[cfg(target_os = "linux")]
            settings_path_error: None,
            #[cfg(not(target_os = "linux"))]
            tray_icon,
            #[cfg(not(target_os = "linux"))]
            last_tray_tooltip: String::new(),
            #[cfg(not(target_os = "linux"))]
            visibility_listener,
            #[cfg(not(target_os = "linux"))]
            visibility_tx,
            #[cfg(not(target_os = "linux"))]
            window_hidden: false,
            #[cfg(target_os = "macos")]
            quit_flag,
            backend_events,
        }
    }

    /// Test-only constructor: the production wiring surface minus everything
    /// the platform supplies.
    ///
    /// It fills in what [`Self::new`] takes from the host — no tray icon, an
    /// empty watcher-manager handle, and a real visibility channel pair — so a
    /// test can build the whole shell over mock ports with no audio device, no
    /// tray, and no Application Store. The returned sender lets a test push a
    /// Show/Hide request through the same path the tray uses; on Linux there
    /// is nothing to drain it, so it is inert there.
    ///
    /// It also stands in for the App Runtime's hydration step, which is
    /// composition-root work in production: the store and transport it is
    /// handed are used to hydrate the sessions before the first frame, over the
    /// same empty watcher cell, so a test shell's session is as restored as
    /// the real one's and a preset mock store lands in it. (A root the mock
    /// store records as watched therefore restores to a fresh `Warning` here —
    /// no watcher is running in a test process — which is the same honest
    /// verdict production reports when its watcher could not be created.)
    ///
    /// It delegates to [`Self::new`] rather than repeating the struct literal,
    /// so the two constructors cannot drift field-for-field. That is also why
    /// it is `doc(hidden)`: `new` stays the one production path.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_test(
        playback: Arc<Mutex<PlaybackSession>>,
        library: Arc<Mutex<LibrarySession>>,
        transport: Box<dyn Transport>,
        scans: Box<dyn Scans>,
        settings_store: Box<dyn SettingsStore>,
        playlist_store: Box<dyn PlaylistStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        views: SessionViews,
        tag_edits: Box<dyn TagEdits>,
        covers: Box<dyn Covers>,
        backend_events: Arc<Mutex<BackendEvents>>,
    ) -> (Self, crate::ui::window_visibility::VisibilityTx) {
        let (visibility_tx, visibility_listener) =
            crate::ui::window_visibility::spawn_visibility_listener();
        #[cfg(target_os = "linux")]
        drop(visibility_listener);

        // The runtime's hydration, performed where the runtime would: the
        // empty watcher cell below is the one the app is built with.
        let watcher_manager = Arc::new(Mutex::new(None));
        let preferences = Preferences::hydrate(
            &playback,
            &library,
            settings_store.as_ref(),
            transport.as_ref(),
            &watcher_manager,
        );

        let app = Self::new(
            playback,
            library,
            transport,
            scans,
            watcher_manager,
            #[cfg(not(target_os = "linux"))]
            None,
            settings_store,
            preferences,
            playlist_store,
            library_mutations,
            views,
            tag_edits,
            covers,
            backend_events,
            #[cfg(not(target_os = "linux"))]
            visibility_listener,
            #[cfg(not(target_os = "linux"))]
            visibility_tx.clone(),
            // The app is the only consumer of the quit intent in a headless
            // test (no tray exists to set it), so it starts cleared and stays
            // that way — the same "a test pushes a request through the tray's
            // channel" arrangement the visibility sender below documents.
            #[cfg(target_os = "macos")]
            Arc::new(AtomicBool::new(false)),
        );
        (app, visibility_tx)
    }

    /// Apply the active theme to the context (REQ-UI-007, Issue 01).
    ///
    /// The *decision* half — resolve the palette, notice the flip, install the
    /// resolved palette on [`ThemeState`] — belongs to the Frame (step 2 of
    /// `Frame::advance`); this is the one act the draw half performs on its
    /// answer: pushing it onto the egui context. Installation happens once at
    /// init and again only when the selection changes, not every frame.
    ///
    /// The palette-family flip invalidates the shared placeholder tile: its
    /// well and glyph colours were derived for the old family's tokens, so it
    /// re-renders under the new one on its next lookup. The eviction is
    /// View-half work (it touches the texture map), so the Frame decides it and
    /// this performs it — in the frame's order, before any row looks a cover up.
    fn enact_theme_palette(&mut self, ctx: &egui::Context, out: &mut FrameOutput) {
        let Some(palette) = out.palette.take() else {
            return;
        };
        theme::install(ctx, &palette);
        if out.evict_generated {
            crate::ui::artwork::evict_generated(&mut self.cover_textures, &mut self.cover_lru_keys);
        }
    }

    /// Send cover intent for one track to the Cover Service. The Cover Cache
    /// owns the whole decision — the composite key, the marker set, and
    /// whether a request is due at all — so this reaches the service only
    /// through it; request deduplication and the negative cache live behind
    /// the service seam.
    fn request_cover(&mut self, track_id: &TrackId, file_path: &Path, size: RequestedSize) {
        self.cover_cache.want_track(
            self.covers.as_ref(),
            track_id.clone(),
            file_path.to_path_buf(),
            size,
        );
    }

    /// Drop the shared placeholder tile from the texture cache. For
    /// sibling modules (`ui::settings`): the artwork-policy toggle uses it
    /// so tracks resolved as artless under the old policy re-resolve.
    pub(crate) fn evict_generated_covers(&mut self) {
        crate::ui::artwork::evict_generated(&mut self.cover_textures, &mut self.cover_lru_keys);
        // The markers go with them. A row that asked under the old policy has an
        // outstanding request whose answer is about to be wrong for the new one, and
        // leaving it marked would suppress the re-ask that the eviction above exists
        // to cause. The arrivals stay: the tile was never one of them, and the
        // covers that really landed are still the covers.
        self.cover_cache.forget_in_flight();
    }

    /// Begin a Thumbnail-cache clear for `ui::settings`, the sibling of
    /// [`Self::evict_generated_covers`] in the same pane. The wipe runs on the cover
    /// worker and the Frame's step-4 drain reports it once it settles, so
    /// the frame only ever says "clearing" and never waits.
    pub(crate) fn request_thumbnail_cache_clear(&mut self) {
        if request_cache_clear(self.covers.as_ref(), &mut self.clear_cache_in_flight) {
            self.feedback.set_library(
                "Clearing the thumbnail cache\u{2026}".to_string(),
                riff_backend::app::events::NoticeSeverity::Info,
            );
        }
    }

    /// Open the per-selection draft for the resolved readout: a Track draft
    /// for a track readout, an album batch draft for an album readout —
    /// targets resolved through the Session Views seam (the same source every
    /// readout reads); the draft itself lives in the editor controller.
    pub fn open_inline_draft(&mut self, content: &InspectorContent) {
        let tags = content.tags.clone();
        match content.kind {
            InspectorKind::Track => {
                if let Some(track) = content
                    .track_ids
                    .first()
                    .and_then(|id| self.views.selected_track(id))
                {
                    self.tag_editor
                        .open_track(track.id.clone(), track.file_path, &tags);
                }
            }
            InspectorKind::Album => {
                let targets: Vec<(TrackId, PathBuf)> = content
                    .track_ids
                    .iter()
                    .filter_map(|id| {
                        self.views
                            .selected_track(id)
                            .map(|track| (track.id.clone(), track.file_path))
                    })
                    .collect();
                self.tag_editor.open_album(targets, &tags);
            }
            InspectorKind::Artist | InspectorKind::Genre => {}
        }
    }

    /// Resolve a cover texture through the shared cache, touching the LRU
    /// to mark it as recently used. A full miss (no real cover, no cached
    /// placeholder tile) resolves the shared music-icon placeholder tile
    /// into the cache — real art, when it arrives through the poll path,
    /// still wins.
    fn resolve_cover_texture(
        &mut self,
        ctx: &egui::Context,
        identity: &str,
        size: RequestedSize,
    ) -> egui::TextureHandle {
        let palette = self.theme.active;
        crate::ui::artwork::lookup_cover_texture(
            &mut self.cover_cache,
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ctx,
            &palette,
            identity,
            size,
        )
    }

    /// Attach the shared track context menu to `response`. See
    /// [`track_menu::paint_track_menu`] for the available actions.
    ///
    /// The **per-app Track-menu host**, borrowed for the length of one
    /// gesture.
    ///
    /// This is the app's single declaration of the handle set a right-click on
    /// a Track answers through, and the only place any of it is named. A
    /// Track-row surface calls [`TrackMenuHost::right_clicked`] and
    /// [`TrackMenuHost::item_chosen`] and supplies a
    /// [`TrackMenuSubject`] — never a handle — so adding a surface cannot
    /// forget to wire one, and cannot wire a different set than the app has.
    /// The test suite calls the same constructor, which is what replaced a
    /// field-for-field mirror of two assembly sites that could drift from both
    /// without anything failing.
    ///
    /// Borrowed rather than stored: storing it would mean moving four handles
    /// out of `RiffApp` and re-reaching them at every other site in this file
    /// to save a borrow that costs nothing. `selected_track` is passed rather
    /// than taken from the session, because the caller already holds the lock
    /// and this host is inside that same frame — it takes the one slot it
    /// writes, not the session.
    pub(crate) fn track_menu<'a>(
        &'a mut self,
        selected_track: &'a mut Option<TrackId>,
    ) -> TrackMenuHost<'a> {
        TrackMenuHost::new(
            self.transport.as_ref(),
            self.playlist_store.as_mut(),
            self.library_mutations.as_mut(),
            &mut self.tag_editor,
            selected_track,
        )
    }

    /// The ONE attach point for every Track row in the app: the flat All Tracks
    /// list and the search results under it, a smart playlist's Tracks, a folder
    /// node's Tracks, and a user playlist's entries — valid and missing alike.
    /// Selection is part of the attach rather than a fact each site works out
    /// for itself, which is what makes those six surfaces answer a right-click
    /// identically instead of merely similarly.
    ///
    /// What the gesture adds is the selection and nothing else: the row's
    /// click, double-click, and heart paths are untouched, so a right-click
    /// still starts no playback and still begins no drag, and the drag handle
    /// under a reorderable row still belongs to the drag.
    ///
    /// This site paints and reports; [`RiffApp::track_menu`] answers. The menu's
    /// props come off the subject's own Track, which is what makes one answer
    /// per row: the Favourite item's label flips on `track.favorite`, and a
    /// listing may hold a mix of Favourited and un-Favourited rows. The
    /// Favourite item's own write is the SAME durable setter the heart's is, so
    /// the two are one change with two doors.
    fn attach_track_menu(
        &mut self,
        response: &egui::Response,
        subject: TrackMenuSubject<'_>,
        selected_track: &mut Option<TrackId>,
    ) {
        // Arc clone out of the seam first: no `&self.views` borrow may live
        // across widget rendering.
        let playlists = self.views.playlists();
        let options: Vec<(PlaylistId, String)> = playlists
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        let palette = self.theme.active;
        // Paint first and answer afterwards: nothing can act while the popup
        // is open, and the host is not even borrowed until the paint returns.
        let (open, intents) = track_menu::paint_track_menu(response, &palette, &options, subject);
        let mut host = self.track_menu(selected_track);
        // The selection is the OPENING's effect, and it is asked for on its
        // own: a right-click that opens the menu and is then dismissed chose
        // nothing and has still selected. `intents` is empty on a frame that
        // never opened the popup, so the item path below is inert there
        // without needing to be gated.
        if open == TrackMenuOpen::Opened {
            host.right_clicked(subject);
        }
        for intent in intents {
            host.item_chosen(subject, intent);
        }
    }

    /// Commit one track's favorite flag: the heart every track row carries.
    /// One immediate durable transaction through the store port, and the
    /// committed mutation bumps the library generation, so the projections
    /// re-resolve the row on the next frame with zero caller action
    /// (ADR 0002). A failed commit changes nothing and is logged.
    fn commit_track_favorite(&mut self, id: &TrackId, favorite: bool) {
        if let Err(e) = self.library_mutations.set_track_favorite(id, favorite) {
            tracing::warn!("Failed to commit the favorite flag for {}: {e}", id.0);
        }
    }

    /// One library-list track row (Issue 07): a 40px tree row with the
    /// animated equalizer indicator on the now-playing row, click/double-click
    /// handling, and the shared context menu.
    fn render_track_row(&mut self, ui: &mut egui::Ui, track: &Track, ctx: TrackRowContext<'_>) {
        let label = label_artist_title(track);
        self.interactive_track_row(
            ui,
            TrackRowSpec {
                track,
                label: &label,
                indent_level: 0,
            },
            ctx,
        );
    }

    /// Shared clickable track row behind every track listing: restyled 40px
    /// tree row + selection/play/context-menu wiring. `label` lets callers
    /// keep their display formats ("Artist - Title", "01. Title").
    ///
    /// Three gestures, three separate things. A click selects; a double-click
    /// selects AND plays; a right-click opens the menu and selects, through
    /// [`Self::attach_track_menu`], which is what keeps the Detail Panel and the
    /// menu describing the same Track. The right-click is not a weaker click:
    /// it never reaches the transport, and the row's drag affordance is not its
    /// business.
    fn interactive_track_row(
        &mut self,
        ui: &mut egui::Ui,
        spec: TrackRowSpec<'_>,
        ctx: TrackRowContext<'_>,
    ) {
        use crate::ui::sidebar::{self, TreeRow};
        // The nine-argument form this replaced needed no `too_many_arguments`
        // allowance because the threading it took was never the point — see
        // `TrackRowContext`.
        let TrackRowSpec {
            track,
            label,
            indent_level,
        } = spec;
        let TrackRowContext {
            library,
            playback,
            current_track,
            reorder,
        } = ctx;
        let remove_from_playlist = reorder.map(|slot| slot.playlist_id);
        let is_selected = library.selected_track.as_ref() == Some(&track.id);
        let is_current = current_track == Some(&track.id);
        let playing = playback.playback_state == PlaybackState::Playing;

        self.request_cover(&track.id, &track.file_path, COVER_THUMB);

        // Every library track row carries a leading cover tile: the real
        // cover when one is cached, otherwise the shared music-icon
        // placeholder — artless tracks read as a uniform tile instead of an
        // empty gap. The request above keeps filling the cache with real art
        // as it lands (the placeholder lives under a separate key).
        let cover = Some(
            self.resolve_cover_texture(ui.ctx(), &track.id.0, COVER_THUMB)
                .id(),
        );

        let row = sidebar::tree_row(
            ui,
            &mut self.icons,
            &self.theme.active,
            TreeRow {
                indent_level,
                icon: None,
                cover,
                label,
                count: None,
                meta: Some(sidebar::RowMeta {
                    plays: Some(track.play_count),
                    time: track.duration,
                }),
                favorite: Some(track.favorite),
                selected: is_selected,
                now_playing: is_current,
                playing: is_current && playing,
                art_slot: false,
            },
        );
        if row.response.clicked() {
            library.selected_track = Some(track.id.clone());
        }
        if row.response.double_clicked() {
            library.selected_track = Some(track.id.clone());
            self.transport.play(track.id.clone());
        }
        if let Some(favorite) = row.favorite_toggled {
            self.commit_track_favorite(&track.id, favorite);
        }
        self.attach_track_menu(
            &row.response,
            TrackMenuSubject::resolved(track, remove_from_playlist),
            &mut library.selected_track,
        );
    }

    /// Read this frame's egui-owned facts into a [`FrameInput`].
    ///
    /// The one place the draw half reaches into egui on the Frame's behalf, and
    /// it only *reads*: a viewport flag, two key presses, and the window
    /// metric the native chrome branch lays out against. Everything else the
    /// frame decides is decided from these, which is what keeps
    /// `ui::frame` free of any egui type at all.
    fn read_frame_input(ui: &egui::Ui, frame: &eframe::Frame) -> FrameInput {
        let ctx = ui.ctx();
        // Order matters and mirrors the pre-frame code exactly: the "no widget
        // owns the keyboard" test short-circuits BEFORE the key is consumed,
        // so a Space a text field wanted is left for the field.
        let toggle_playback = !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
        FrameInput {
            native_close_requested: ctx.input(|i| i.viewport().close_requested()),
            viewport_maximized: ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
            search_focus_requested: ctx
                .input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)),
            toggle_playback,
            traffic_lights_width: crate::ui::chrome::measured_traffic_lights_width(frame),
            zoom_factor: ctx.zoom_factor(),
        }
    }

    /// Borrow this frame's state out into a [`Frame`].
    ///
    /// One bundle of disjoint field borrows rather than a `&mut RiffApp`, so
    /// the frame can be driven with no application at all — which is why
    /// `tests/frame_tests.rs` needs no kittest harness and no eleven-argument
    /// constructor. It is built once per slot (see `ui()`), which is what lets
    /// the draw half keep its own `&mut self` between the Frame's steps.
    #[allow(clippy::too_many_arguments)]
    fn frame<'a>(
        &'a mut self,
        playback: &'a mut PlaybackSession,
        library: &'a mut LibrarySession,
    ) -> Frame<'a> {
        Frame::new(
            FrameParts {
                theme: &mut self.theme,
                feedback: &mut self.feedback,
                scroll_memory: &mut self.scroll_memory,
                views: &mut self.views,
                backend_events: &self.backend_events,
                transport: self.transport.as_ref(),
                scans: self.scans.as_ref(),
                tag_edits: &mut self.tag_editor,
                covers: self.covers.as_ref(),
                cover_cache: &mut self.cover_cache,
                watchers: &self.watcher_manager,
                prefs: &mut self.prefs,
                settings_store: self.settings_store.as_mut(),
                playlist_store: self.playlist_store.as_mut(),
                clear_cache_in_flight: &mut self.clear_cache_in_flight,
                global_search_focus: &mut self.global_search_focus,
                title_key: &mut self.last_title_key,
                playlist_view: &mut self.playlist_view,
                smart_playlist_view: &mut self.smart_playlist_view,
                playlist_rename: &mut self.playlist_rename,
                playlist_create_name: &mut self.playlist_create_name,
                playback_live: &self.playback,
                #[cfg(target_os = "macos")]
                quit_flag: &self.quit_flag,
            },
            playback,
            library,
        )
    }

    /// Enact the head half's egui-bound leaves, in the Frame's order.
    ///
    /// Everything here was *decided* by `Frame::advance` and is performed here
    /// only because it is egui-shaped: install the palette, flush the texture
    /// map a settled Thumbnail-cache clear emptied, upload the frame's Cover
    /// arrivals, and answer the native close request.
    ///
    /// The flush runs before the uploads on purpose — the order the pre-split
    /// frame ran. At clear-confirm time every visible row would re-request
    /// while the wipe was still queued behind those very requests, rebuilding
    /// the entries the user had just asked to delete; settled is the moment the
    /// disk is genuinely empty, so it is the moment the screen can be emptied
    /// with it. A `Failed` clear leaves every texture in place, because nothing
    /// was removed and so nothing has to be re-derived.
    fn enact_frame_head(&mut self, ctx: &egui::Context, out: &mut FrameOutput) {
        self.enact_theme_palette(ctx, out);
        if let Some(outcome) = out.cache_clear.clone() {
            flush_cleared_cache(&outcome, &mut self.cover_textures, &mut self.cover_lru_keys);
        }
        for arrival in out.cover_arrivals.drain(..) {
            crate::ui::artwork::store_cover_texture(
                &mut self.cover_cache,
                ctx,
                &mut self.cover_textures,
                &mut self.cover_lru_keys,
                arrival,
            );
        }
        #[cfg(target_os = "macos")]
        if out.cancels_native_close() {
            // eframe quits unless the frame that reported the close carries
            // the cancel, so this cannot be deferred to `enact_frame_tail`.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let _ = self.visibility_tx.send(CUSTOM_TITLEBAR_CLOSE);
        }
    }

    /// Enact the tail half's egui-bound leaves: the window title, the tray
    /// tooltip, the hide gesture, and the end-of-frame repaint tick.
    ///
    /// The tick keeps visible frames responsive (seek readouts, the playing
    /// row). A window hidden to the tray schedules no repaints — the tray wakes
    /// the loop on demand — so it is gated on `window_hidden`: without the gate,
    /// egui keeps re-requesting repaints forever on a window it still believes
    /// is visible (eframe 0.35 keeps calling `ui` for a hidden window). Linux
    /// has no hidden state, so it keeps the unconditional tick.
    fn enact_frame_tail(&mut self, ctx: &egui::Context, out: &mut FrameOutput) {
        for command in out.viewport.drain(..) {
            let command = match command {
                FrameViewport::Minimize => {
                    crate::ui::chrome::WindowControl::Minimize.viewport_command()
                }
                FrameViewport::Maximized(value) => egui::ViewportCommand::Maximized(value),
                FrameViewport::Close => egui::ViewportCommand::Close,
                FrameViewport::CancelClose => egui::ViewportCommand::CancelClose,
                FrameViewport::Title(title) => egui::ViewportCommand::Title(title),
            };
            ctx.send_viewport_cmd(command);
        }
        if out.pick_folder {
            // The platform folder picker: an OS dialog, performed here because
            // it is not a decision. Nothing left in this frame reads the
            // registered root, so it rides the tail with the viewport commands.
            let library_arc = self.library.clone();
            let mut guard = library_arc.lock_or_recover();
            self.add_library_via_platform_picker(&mut guard);
        }
        if out.hide_window {
            // The custom titlebar X is the only hide gesture on macOS/Windows,
            // and it hides through this same channel rather than sending a
            // `Close` — with the close-to-tray veto gone, any close that
            // reaches eframe quits.
            #[cfg(not(target_os = "linux"))]
            let _ = self.visibility_tx.send(CUSTOM_TITLEBAR_CLOSE);
        }
        // The tooltip is compared against the last push first, so a steady-state
        // frame sends nothing and formats nothing (REQ-SI-001): the identity is
        // what changed, not the text.
        #[cfg(not(target_os = "linux"))]
        if let Some(tooltip) = out.tray_tooltip.take()
            && self.last_tray_tooltip != tooltip
        {
            if let Some(ref tray) = self.tray_icon {
                crate::ui::tray::update_tooltip(tray, &tooltip);
            }
            self.last_tray_tooltip = tooltip;
        }
        #[cfg(target_os = "linux")]
        {
            let _ = out.tray_tooltip.take();
        }
        #[cfg(not(target_os = "linux"))]
        if !self.window_hidden {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        #[cfg(target_os = "linux")]
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// Draw the top 56px strip and report what the user did on it.
    ///
    /// The frameless titlebar (issue 04, ADR 0005) merged with the former top
    /// bar — wordmark, scan status, the theme/Now Playing/Settings/Advanced
    /// controls, and the custom minimize/close buttons over a full-width drag
    /// region. Content in, actions out: the search field edits a scratch buffer
    /// and the query rides back in the report, and the Ctrl+K focus request is
    /// consumed by [`Frame::apply_titlebar`] rather than cleared here.
    ///
    /// `scan_status` is the string the Frame composed at step 5, one line above
    /// every panel — so a slot filled after the compose really would be a frame
    /// late, which is what the headless order test pins.
    fn draw_titlebar(
        &mut self,
        ui: &mut egui::Ui,
        input: &FrameInput,
        playback: &mut PlaybackSession,
        library: &mut LibrarySession,
        out: &mut FrameOutput,
    ) {
        let scan_status = library.scan_status.clone();
        let mut report = TitlebarReport {
            // The viewport's maximized flag is egui-owned and the Frame cannot
            // read it, so it rides the report: the draw half reads, the Frame
            // decides what `ToggleMaximize` means.
            maximized: input.viewport_maximized,
            search_query: library.search_query.clone(),
            ..TitlebarReport::default()
        };
        egui::Panel::top("titlebar")
            .exact_size(theme::TITLEBAR_H)
            .frame(egui::Frame::NONE.fill(self.theme.active.surface))
            .show(ui, |ui| {
                let content = crate::ui::chrome::TitleBarContent {
                    scan_status: scan_status.as_deref(),
                    theme_dark: self.theme.dark,
                    active_nav: crate::ui::chrome::NavDestination::active(
                        library.view_mode,
                        library.browse_mode,
                    ),
                    // The chrome-mode decision, consumed here so the launch
                    // viewport and this renderer cannot drift apart. The
                    // clearance is measured from the window where eframe can
                    // measure it (macOS); the ignored-elsewhere value is the
                    // documented fallback.
                    chrome: crate::ui::chrome::chrome_mode(),
                    traffic_clearance: crate::ui::chrome::traffic_light_clearance(
                        input.traffic_lights_width,
                        input.zoom_factor,
                    ),
                };
                self.titlebar_actions.clear();
                let search_response = crate::ui::chrome::show_titlebar(
                    ui,
                    &mut self.icons,
                    &self.theme.active,
                    &content,
                    &mut report.search_query,
                    &mut self.titlebar_actions,
                );
                // Ctrl+K landed: focus the titlebar search field this frame.
                if self.global_search_focus {
                    search_response.request_focus();
                    report.focus_search = true;
                }
            });
        report.actions = std::mem::take(&mut self.titlebar_actions);
        self.frame(playback, library).apply_titlebar(out, &report);
    }

    /// Draw the left 280px column and report what the user did on it.
    ///
    /// The library browser (search, Library/Folders nav, playlists). Shared
    /// chrome per the mockup — present on every view; only the main stage
    /// switches. The restyled content (issue 07) keeps a 12px inset from the
    /// panel edge, and the panel carries the surface token itself rather than
    /// inheriting a default.
    ///
    /// The panel mutates NOTHING: every nav click, every smart-list row, every
    /// playlist row and every prompt resolution is a [`SidebarAction`], and
    /// [`Frame::apply_sidebar`] is the only writer of the library session.
    fn draw_sidebar(
        &mut self,
        ui: &mut egui::Ui,
        playback: &mut PlaybackSession,
        library: &mut LibrarySession,
        out: &mut FrameOutput,
    ) {
        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(12))
                    .fill(self.theme.active.surface),
            )
            .show(ui, |ui| {
                self.render_library_sidebar(ui, playback, library, out);
            });
    }

    /// Draw the bottom 88px strip and report what the user did on it.
    ///
    /// Transport + progress + volume at the exact 88px playerbar token height,
    /// plus the queue sheet above its right edge while the bar's queue button
    /// has it open. Both report through the same action drain, and
    /// [`Frame::apply_control_bar`] is the only applier.
    fn draw_control_bar(
        &mut self,
        ui: &mut egui::Ui,
        playback: &mut PlaybackSession,
        library: &mut LibrarySession,
        out: &mut FrameOutput,
    ) {
        let report = self.render_control_bar(ui, library, playback);
        self.frame(playback, library)
            .apply_control_bar(out, &report);
    }

    /// Draw the main stage and report what the user did on it.
    ///
    /// Exactly one View visible at a time. The Library and Settings stages
    /// answer through their own appliers (they are Views, and the Settings
    /// modal owns watcher and store effects the Frame has no business
    /// knowing); the Now Playing stage's actions travel back in the report.
    fn draw_stage(
        &mut self,
        ui: &mut egui::Ui,
        playback: &mut PlaybackSession,
        library: &mut LibrarySession,
        out: &mut FrameOutput,
    ) {
        let mut report = StageReport::default();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.theme.active.background))
            .show(ui, |ui| match library.view_mode {
                ViewMode::Library => {
                    // The elastic column stage (elastic-column spec): the
                    // section-driven column sequence plus the collapsible
                    // inspector replace the fixed three-pane explorer.
                    self.render_elastic_stage(ui, library, playback);
                }
                ViewMode::NowPlaying => {
                    self.show_now_playing_view(ui, playback, &mut report);
                }
                ViewMode::Settings => {
                    self.show_settings_view(ui, library, playback);
                }
            });
        self.frame(playback, library).apply_stage(out, &report);
    }
}

impl eframe::App for RiffApp {
    /// Per-frame logic that also runs while the window is hidden (eframe 0.35
    /// calls `logic` before every `ui`, and on the throttled repaints it gives
    /// an invisible window). No UI may be shown here — only state checks and
    /// viewport commands.
    ///
    /// There is no close-to-tray veto here (split-close-paths, owner decision
    /// 2026-09-19): a close that reaches eframe is a quit, period. On
    /// Windows/Linux the frameless OS close (Alt+F4, taskbar Close) and the
    /// tray Quit both pass through untouched. On macOS that is no longer quite
    /// the whole story — the red traffic light's close IS resolved against the
    /// "Quit on close" preference, as the Frame's first step, so that a window
    /// close can still hide to the tray. What survives unchanged on every
    /// platform is the rule that stops the resolver from eating a quit: a
    /// riff-initiated quit is identified by the shared quit flag and is never
    /// cancelled (see [`CloseIntent`]). The custom titlebar X never sends a
    /// `Close` anywhere; it hides through the frontend-local visibility
    /// channel drained below. On Linux there is no tray, so the default no-op
    /// `logic` applies and closing quits normally.
    #[cfg(not(target_os = "linux"))]
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Reconcile frontend-local visibility requests, drained from the tray's
        // own channel (Issue 03). Every request is carried out whether or not
        // this tick already believes it is in that state; no backend state is
        // touched — visibility is ephemeral frontend state. The minimized flag
        // is read only here, where the show un-minimizes a window the user
        // iconified: egui derives `viewport().visible()` from minimized/occluded
        // state that egui-winit never fills in, so the app keeps its own record.
        if let Some(request) = self.visibility_listener.drain() {
            self.window_hidden = !request.0;
            let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
            for command in crate::ui::window_visibility::viewport_commands_for(request, minimized) {
                ctx.send_viewport_cmd(command);
            }
        }

        // Re-centre the macOS traffic lights in riff's 56pt strip (a no-op
        // elsewhere, so the call stays ungated). This runs in `logic()` and not
        // in `ui()` for two reasons: eframe skips `ui()` while the window is
        // hidden, so a hidden window would come back un-recentred, and
        // `logic()` runs before `ui()` measures the clearance the titlebar
        // renders against — the lights are in place first, every frame.
        //
        // The parameter is named `frame` (not `_frame`) even though only the
        // macOS build has work to do: the no-op `apply` takes it on every other
        // platform, which is what keeps Windows from seeing an unused
        // variable. Clippy's `used_underscore_binding` rules out the other
        // way round.
        crate::ui::traffic_lights::apply(frame);
    }

    /// One frame. The driver, not the sequence: the ORDER lives in
    /// [`crate::ui::frame`], which states it, owns it, and asserts it headlessly
    /// (`crates/riff-gui/tests/frame_tests.rs`). What is left here is the part
    /// that genuinely is egui — reading this frame's input facts, drawing four
    /// panels, and enacting the decisions the Frame handed back — plus the two
    /// guards, whose shapes are load-bearing:
    ///
    /// * Both `Arc`s are cloned BEFORE locking: the guards borrow the clones,
    ///   so the whole frame can still call `self.<method>(...)` — exactly how
    ///   the pre-split frame loop handled `self.state`.
    /// * Playback is snapshotted (lock → clone → drop) and the library guard is
    ///   taken live. The engine and coordinator write playback state on their
    ///   own threads, so the frame renders from a plain clone and
    ///   [`Frame::finish`] writes back only the UI-owned fields — a whole
    ///   session replace would clobber engine-written position and traversal
    ///   index. The library session is UI-owned, so its guard is held for the
    ///   whole frame, and the frame-end Preferences commit sees it.
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let playback_arc = self.playback.clone();
        let library_arc = self.library.clone();

        let mut playback = playback_arc.lock_or_recover().clone();
        let mut library = library_arc.lock_or_recover();

        let input = Self::read_frame_input(ui, frame);

        // Steps 1-9: the native close, the theme, the event inbox, the three
        // background services, the status-line compose, the covers, the
        // watcher, the keyboard, and the OS title. Then the egui-bound leaves
        // they decided on, in the same order — before any panel draws, because
        // the rows resolve cover textures as they paint and the widgets need
        // the palette installed.
        let mut output = self.frame(&mut playback, &mut library).advance(&input);
        self.enact_frame_head(ui.ctx(), &mut output);

        // --- SHELL (Issue 06): unified Panel API at exact token dimensions ---
        //
        // Four slots, and the Frame answers each report at its own slot: the
        // titlebar's before the sidebar highlights the navigation it changed,
        // the control bar's before the stage picks the view it switched. That
        // is why these are four statements and not one loop.
        self.draw_titlebar(ui, &input, &mut playback, &mut library, &mut output);
        self.draw_sidebar(ui, &mut playback, &mut library, &mut output);
        self.draw_control_bar(ui, &mut playback, &mut library, &mut output);
        // --- MAIN STAGE: exactly one View visible at a time ---
        self.draw_stage(ui, &mut playback, &mut library, &mut output);

        // Step 14: the write-back, with one owner. The library guard is still
        // live, so the Preferences commit sees the frame's playback snapshot
        // together with the library session's preference fields.
        self.frame(&mut playback, &mut library).finish();
        drop(library);

        self.enact_frame_tail(ui.ctx(), &mut output);
    }
}

// --- Per-frame helpers -------------------------------------------------------

/// The custom titlebar X's hide intent on macOS/Windows (split-close-paths;
/// owner decision 2026-09-19). The custom X is the only hide gesture, so it
/// enqueues a frontend-local `VisibilityMessage(false)` through the
/// visibility channel and `logic()` applies the hide one frame later. The X
/// must never send a `Close`: with the close-to-tray veto gone, any close
/// that reaches eframe quits. On Linux there is no tray, so the titlebar
/// drain sends a real `ViewportCommand::Close` instead — the constant stays
/// compiled there as the pure data [`close_resolution`] hands back, but no
/// Linux production code sends it.
pub const CUSTOM_TITLEBAR_CLOSE: VisibilityMessage = VisibilityMessage(false);

/// Why a close reached eframe this frame.
///
/// The signal itself carries no provenance: egui-winit turns a
/// `ViewportCommand::Close` into `ViewportEvent::Close`, and
/// `egui::ViewportEvent` has exactly one variant with no payload, so that
/// event is bit-for-bit identical to the one winit's own
/// `WindowEvent::CloseRequested` produces for the macOS red traffic light.
/// There is no in-band way to tell the two apart at the point riff decides,
/// so the app consults a fact it already knows instead: the app-wide
/// `quit_flag` from `AppRuntime::spawn`, which the tray stores before it
/// enqueues the close and [`crate::ui::frame::Frame`] loads as the first
/// decision of the frame it lands in.
///
/// Note that OS-level Cmd+Q is NOT one of the two ambiguous cases, and only
/// by luck: winit installs a default macOS app menu whose Quit calls
/// `NSApplication::terminate:`, so Cmd+Q never reaches egui as a close at
/// all. That guarantee is fragile rather than designed — a
/// `with_default_menu(false)`, or any custom macOS menu, would route Cmd+Q
/// into `WindowEvent::CloseRequested` instead, and Cmd+Q would then silently
/// become a hide-to-tray whenever the preference is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseIntent {
    /// The OS asked to close the window (the macOS traffic-light red button).
    /// The persisted "Quit on close" preference decides what this does.
    WindowClose,
    /// riff itself asked to quit — the tray's Quit item, which enqueues a
    /// real `Close` so the whole shutdown path (eframe's save + destroy, then
    /// `RuntimeLifecycle::shutdown`) runs exactly once. A quit that riff has
    /// already committed is NEVER cancelled, whatever the preference says:
    /// cancelling it would strand a running, playback-stopped process with a
    /// dead tray menu and an unrecoverable window.
    Quit,
}

/// Resolve a close request that reached eframe through the persisted
/// Quit-on-close preference (split-close-paths / macos-native-title-bar
/// issue 01) — the same applier the custom X's action resolves through, so
/// close semantics keep one home.
///
/// A [`CloseIntent::WindowClose`] follows the preference. Off (the default),
/// the close is cancelled for the frame — eframe quits unless that frame's
/// viewport output carries [`egui::ViewportCommand::CancelClose`] — and the
/// window hides through the frontend-local visibility channel, the exact
/// gesture [`CUSTOM_TITLEBAR_CLOSE`] performs. On, the close passes through,
/// proceeds, and the app quits.
///
/// A [`CloseIntent::Quit`] passes through in BOTH preference states, because
/// the preference says what a *window* close should do and was never meant to
/// veto a quit riff has already committed. That cell is the tray-Quit fix.
///
/// Returns the cancel command plus the hide message, or `None` when the close
/// proceeds. Pure data on every platform — the macOS caller stays assertable
/// from the Linux/Windows CI machines; only the caller is macOS-only.
#[must_use]
pub fn close_resolution(
    intent: CloseIntent,
    quit_on_close: bool,
) -> Option<(egui::ViewportCommand, VisibilityMessage)> {
    match (intent, quit_on_close) {
        (_, true) | (CloseIntent::Quit, _) => None,
        (CloseIntent::WindowClose, false) => {
            Some((egui::ViewportCommand::CancelClose, CUSTOM_TITLEBAR_CLOSE))
        }
    }
}

// `apply_titlebar_action` is gone with the frame's own applier: it needed a
// `Context` for two viewport commands and read the maximized flag off egui's
// input, so none of it could be asserted headlessly. `Frame::apply_titlebar`
// is the same match in the Frame's own vocabulary — `Minimize` and
// `ToggleMaximize` become [`FrameViewport`] intents, and the Close arm that
// used to be a deliberate no-op here is the one that now resolves the
// preference (hide through the visibility channel on macOS/Windows, really
// close on Linux), because the Frame owns the report rather than a `Context`.

// `apply_browser_action` is gone, and with it the `ContextMenu { .. } => {}`
// no-op arm it carried. That arm existed because the section roots routed the
// menu report to `apply_collection_menu` — which needs the playback session, the
// transport and the Session Views seam, none of which this function held — and
// the match had to stay exhaustive over what was left. The per-Column dispatch
// (`app/browser_pane.rs`'s two bindings) answers all three actions itself, from
// the Column's stated identity, so there is nothing here for a no-op to stand
// in for. The root's sort flip and its row selection live in that binding now,
// which is also why it needs the identity: a root's own Section, not whatever
// the session currently says.

/// What kind of entity a browser row denotes.
///
/// One classifier for the six entity Columns, because a row's key alone does not
/// say: an Album's key is the `(artist, title)` composite and an Artist's or a
/// Genre's is a bare name, and one section hosts two of those at different
/// depths — an artist's drill column lists Albums, not Artists. Both consumers,
/// where the row's selection lands and which Tracks the row denotes, read the
/// row's kind from here, so a row can never be SELECTED as one thing and PLAYED
/// as another.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EntityRow {
    /// An Album, named by the store's own `(album artist, title)` identity.
    Album { artist: String, title: String },
    /// An Artist, named by its own name.
    Artist(String),
    /// A Genre, named by its own name.
    Genre(String),
}

impl EntityRow {
    /// The selection this row's click and right-click both land on.
    fn selection(&self) -> BrowserSelection {
        match self {
            Self::Album { artist, title } => BrowserSelection::Album {
                artist: artist.clone(),
                title: title.clone(),
            },
            Self::Artist(name) => BrowserSelection::Artist(name.clone()),
            Self::Genre(name) => BrowserSelection::Genre(name.clone()),
        }
    }
}

/// The entity a row at `level` of `section` denotes. `None` where the section
/// lists no entity rows — the All Tracks listing, and any depth that does not
/// exist — so a key that could mean nothing selects nothing and plays nothing.
fn entity_row(key: &str, section: LibrarySection, level: usize) -> Option<EntityRow> {
    match (section, level) {
        (LibrarySection::Albums, 0)
        | (LibrarySection::Artists, 1)
        | (LibrarySection::Genres, 2) => {
            key.split_once('\u{1f}')
                .map(|(artist, title)| EntityRow::Album {
                    artist: artist.to_owned(),
                    title: title.to_owned(),
                })
        }
        // The Genres section's first drill column lists that genre's ARTISTS,
        // which is the one place a bare name at level 1 is not an Album row.
        (LibrarySection::Artists, 0) | (LibrarySection::Genres, 1) => {
            Some(EntityRow::Artist(key.to_owned()))
        }
        (LibrarySection::Genres, 0) => Some(EntityRow::Genre(key.to_owned())),
        // The All Tracks listing has no entity rows, and neither has any depth
        // the stage does not plan: nothing to select, nothing to play.
        _ => None,
    }
}

/// Apply the selection one entity row's key denotes, at that row's own depth.
///
/// `column` is the identity the row's Column stated once — its Section, its
/// depth, its Scroll Memory slot — so the depth a row selects at is read from
/// the Column rather than supplied beside every call. Shared by both dispatch
/// bindings in `app/browser_pane.rs`, the section roots' and the Drill Columns',
/// so a click and a right-click on the same row are literally the same line of
/// code, and neither binding can disagree with the other about a Column's depth.
pub fn apply_entity_selection(key: &str, column: ColumnIdentity, library: &mut LibrarySession) {
    let level = column.level();
    if let Some(row) = entity_row(key, column.section(), level) {
        library.select_at(level, row.selection());
    }
}

/// The Track batch the entity a row denotes covers.
///
/// Resolved HERE, when the item is dispatched, and never when the menu was
/// built — so a scan or a tag edit that committed in between is reflected in
/// what gets played, queued, or shuffled.
///
/// The three kinds answer differently, and the difference is the point:
///
/// - an **Album** is that album's tracks, keyed by the store's `(artist,
///   title)` identity;
/// - an **Artist** is every track across its albums, so the walk is over the
///   artist's album table;
/// - a **Genre** is the tracks CARRYING that genre, which is a genuinely
///   different set — not the library's tracks filtered, but a different walk
///   (the genre's artists, their genre-scoped albums, and those albums'
///   genre-scoped tracks), gated on the genre still being in the read model.
///   The gate is what a gone genre's row looks like from here: nothing to play.
pub fn entity_track_ids(
    key: &str,
    column: ColumnIdentity,
    views: &mut SessionViews,
) -> Vec<TrackId> {
    match entity_row(key, column.section(), column.level()) {
        Some(EntityRow::Album { artist, title }) => views
            .album_tracks(&artist, &title)
            .iter()
            .map(|track| track.id.clone())
            .collect(),
        Some(EntityRow::Artist(name)) => views
            .artist_albums(&name)
            .iter()
            .flat_map(|album| album.tracks.iter().cloned())
            .collect(),
        Some(EntityRow::Genre(genre)) => {
            let known = views.genres().iter().any(|row| row.genre == genre);
            if !known {
                return Vec::new();
            }
            let mut ids = Vec::new();
            for artist in views.artists_in_genre(&genre).iter() {
                for album in views.artist_albums_in_genre(&artist.name, &genre).iter() {
                    for track in views
                        .album_tracks_in_genre(&album.artist, &album.title, &genre)
                        .iter()
                    {
                        ids.push(track.id.clone());
                    }
                }
            }
            ids
        }
        None => Vec::new(),
    }
}

/// Apply one [`crate::ui::detail::DetailAction`] (handoff issue 09) to the
/// sessions and the store.
///
/// The header's **Play all** and **Shuffle** used to arrive here as a batch of
/// the shown rows resolved by the caller; both are gone with the buttons, and
/// so is the batch parameter that fed them. What remains moves selection and
/// commits a favorite: a batch play is reached from a collection's menu
/// instead, through [`play_album_batch`].
///
/// A Track row's **menu** report is not one of these, and that is structural
/// rather than a no-op arm's worth of documentation: a menu report is a
/// [`crate::ui::menu::TrackMenuReport`] travelling in
/// [`crate::ui::detail::DetailReport`]'s other variant, answered by the
/// per-app [`TrackMenuHost`]. This applier cannot be handed one, so it needs
/// no arm to discard it.
pub fn apply_detail_action(
    action: crate::ui::detail::DetailAction,
    library: &mut LibrarySession,
    transport: &dyn Transport,
    library_mutations: &mut dyn LibraryMutationStore,
) {
    use crate::ui::detail::DetailAction as Action;
    match action {
        // Entity rows no longer render inside the detail column — they are
        // their own columns in the elastic stage — so this action cannot
        // fire from the app. The widget seam keeps the variant for its
        // own contract (tests render rows directly).
        Action::SelectRow(_) => {}
        Action::SelectTrack(key) => {
            library.selected_track = Some(TrackId(key));
        }
        Action::PlayTrack(key) => {
            library.selected_track = Some(TrackId(key.clone()));
            transport.play(TrackId(key));
        }
        Action::SetFavorite { key, favorite } => {
            if let Err(e) = library_mutations.set_track_favorite(&TrackId(key.clone()), favorite) {
                tracing::warn!("Failed to commit the favorite flag for {key}: {e}");
            }
        }
        // The track listings' shared sort state. The scroll side of the change
        // belongs to the render site that owns the Scroll Memory slot, so the
        // site that can receive this report resets its own slot.
        Action::TrackSortSelected(sort) => {
            library.track_sort = sort;
        }
    }
}

/// Sort borrowed track refs in place per `sort` — the `&Track` variant of
/// `browser_pane`'s `sort_track_rows`, for the listings that render straight
/// from a resolved track list (smart lists). The canonical order is the
/// no-op anchor; the title modes compare the display title
/// case-insensitively, and being stable sorts, ties keep the canonical
/// order.
fn sort_track_refs(rows: &mut [&Track], sort: riff_backend::app::state::TrackSort) {
    use riff_backend::app::state::TrackSort;
    match sort {
        TrackSort::NumberAsc => {}
        TrackSort::NumberDesc => rows.reverse(),
        TrackSort::TitleAsc => {
            rows.sort_by_key(|t| t.metadata.display_title(&t.file_path).to_lowercase());
        }
        TrackSort::TitleDesc => rows.sort_by_key(|t| {
            std::cmp::Reverse(t.metadata.display_title(&t.file_path).to_lowercase())
        }),
    }
}

// `apply_drill_action` is gone too: it was a one-line wrapper over
// `apply_entity_selection` that existed only because a drill Column re-supplied
// its own Section and level on every action. Both shapes of Column now reach
// `apply_entity_selection` from their stated identity, so a root's rows land at
// level 0 and a drill's at 1 or 2 without either spelling a number twice.

/// The one shared batch-play helper: a list's first Track plays and the rest
/// queue behind it as a single `play_many` batch — exactly [`play_folder`]'s
/// gesture. An empty batch starts nothing, and so never re-enables shuffle.
///
/// It has no caller in [`apply_detail_action`] any more — the Tracks column's
/// header buttons that used to reach it are gone — and it is deliberately not
/// folded into what replaced them. A whole-list menu's Play and Shuffle, on
/// the album, artist, genre, playlist, smart-playlist and folder surfaces, are
/// what still reach it, so it must never be deleted as dead code.
fn play_album_batch(album_tracks: &[TrackId], transport: &dyn Transport) {
    let Some(first) = album_tracks.first() else {
        return;
    };
    transport.play_many(first.clone(), album_tracks[1..].to_vec());
}

/// One frame of the Tracks column's content, resolved from the library
/// session through the Session Views seam: owned data so the render call
/// site can borrow it all at once. Everything re-reads the store per
/// generation, so scans and tag edits can never leave stale rows.
#[derive(Default)]
pub struct DetailContent {
    pub tracks: Vec<crate::ui::detail::TrackRow>,
}

/// The kinds of list column the elastic stage renders for
/// [`BrowseMode::Library`] sections. Entity listings below the album level
/// are their own columns now; the Tracks column is the existing
/// `DetailColumn` shape, a bare track list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// The section's root entity listing (Artists / Albums / Genres).
    Root,
    /// An artist's albums (Artists section, one level deep).
    ArtistAlbums,
    /// The artists carrying a genre (Genres section, one level deep).
    GenreArtists,
    /// An artist's albums within a genre (Genres section, two levels deep).
    GenreArtistAlbums,
    /// The All Tracks flat listing (list or grid per the top-bar toggle).
    Flat,
    /// A single-list stage's full-width listing: search results, an opened
    /// playlist or smart list, or the folder tree.
    Single,
    /// The Tracks column: the selected album's track list, and nothing else.
    Tracks,
}

/// The elastic stage's column plan for one [`LibrarySection`]: which list
/// columns render for the current drill-down path, in left-to-right order.
/// A column with no content is never included — the shape follows the
/// section's facet depth and the path's depth:
///
/// | Section    | Columns (per drill-down depth)                    |
/// | ---------- | ------------------------------------------------- |
/// | All Tracks | `[Flat]` (fills the stage)                        |
/// | Artists    | `[Root]` → `[Root, ArtistAlbums]` → `[+ Tracks]` |
/// | Albums     | `[Root]` → `[Root, Tracks]`                       |
/// | Genres     | `[Root]` → `[+ GenreArtists]` → `[+ GenreArtistAlbums]` → `[+ Tracks]` |
///
/// Path entries of the wrong kind for their level are ignored (section
/// switches reset the path, but the plan stays defensive), so a stale or
/// foreign entry never spawns a column.
#[must_use]
pub fn column_plan(section: LibrarySection, path: &[BrowserSelection]) -> Vec<ColumnKind> {
    use riff_backend::app::state::BrowserSelection;
    match section {
        LibrarySection::AllTracks => vec![ColumnKind::Flat],
        LibrarySection::Artists => {
            let mut columns = vec![ColumnKind::Root];
            if matches!(path.first(), Some(BrowserSelection::Artist(_))) {
                columns.push(ColumnKind::ArtistAlbums);
                if matches!(path.get(1), Some(BrowserSelection::Album { .. })) {
                    columns.push(ColumnKind::Tracks);
                }
            }
            columns
        }
        LibrarySection::Albums => {
            let mut columns = vec![ColumnKind::Root];
            if matches!(path.first(), Some(BrowserSelection::Album { .. })) {
                columns.push(ColumnKind::Tracks);
            }
            columns
        }
        LibrarySection::Genres => {
            let mut columns = vec![ColumnKind::Root];
            if matches!(path.first(), Some(BrowserSelection::Genre(_))) {
                columns.push(ColumnKind::GenreArtists);
                if matches!(path.get(1), Some(BrowserSelection::Artist(_))) {
                    columns.push(ColumnKind::GenreArtistAlbums);
                    if matches!(path.get(2), Some(BrowserSelection::Album { .. })) {
                        columns.push(ColumnKind::Tracks);
                    }
                }
            }
            columns
        }
    }
}

/// Resolve what the Tracks column renders for the current drill-down path:
/// the album's track list, genre-scoped in the Genres section — and nothing
/// else. The breadcrumb trail and the album header this column used to open
/// with are gone, and the album's name is resolved here no more: the column
/// that listed it already says which album is selected, and so does the
/// inspector, so a third copy said it again. Entity listings below the album
/// level are their own columns in the stage, so this resolver carries no
/// rows either.
///
/// Under a query the album drill shows only the album's hit tracks — a
/// name-hit album (its own artist/title matched) opens its full track list
/// instead, so the drill never dead-ends into an empty detail column.
pub fn resolve_detail_content(
    views: &mut SessionViews,
    library: &LibrarySession,
    query: &str,
) -> DetailContent {
    use riff_backend::app::state::BrowserSelection;

    let mut content = DetailContent::default();
    let Some(BrowserSelection::Album { artist, title }) = library.current_selection() else {
        return content;
    };
    let genre = match library.library_section {
        LibrarySection::Genres => library.browser_path.first().and_then(|entry| match entry {
            BrowserSelection::Genre(genre) => Some(genre.clone()),
            _ => None,
        }),
        _ => None,
    };
    // The album drill under a query: only the tracks that match — unless the
    // album itself is a name-hit (its own artist/title matched the query), in
    // which case the full track list opens so it never dead-ends empty.
    let name_hit = !query.is_empty() && views.album_is_name_hit(artist, title, query);
    let tracks = match &genre {
        Some(genre) if !name_hit => {
            if query.is_empty() {
                views.album_tracks_in_genre(artist, title, genre)
            } else {
                views.album_hit_tracks_in_genre(artist, title, genre, query)
            }
        }
        Some(genre) => views.album_tracks_in_genre(artist, title, genre),
        None if !name_hit && !query.is_empty() => views.album_hit_tracks(artist, title, query),
        None => views.album_tracks(artist, title),
    };
    let current = views.playback_current().map(|t| t.id.clone());
    content.tracks = tracks
        .iter()
        .map(|track| crate::ui::detail::TrackRow {
            key: track.id.0.clone(),
            title: track.metadata.display_title(&track.file_path),
            plays: track.play_count,
            duration: track.duration,
            favorite: track.favorite,
            selected: library.selected_track.as_ref() == Some(&track.id),
            now_playing: current.as_ref() == Some(&track.id),
        })
        .collect();
    content
}

/// What the inspector's readout shows: the selected track — single-clicking
/// a track row in any track listing sets it — or, when no track is selected,
/// the deepest entity in the drill-down path (album > artist > genre). Drives
/// the widget's kind chip and the batch Play/Queue resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorKind {
    /// An album at the deepest path entry — the full details grid.
    #[default]
    Album,
    /// An artist at the deepest path entry — name, album count, cover.
    Artist,
    /// A genre at the deepest path entry — name and track count.
    Genre,
    /// A selected track — the compact readout.
    Track,
}

/// One frame of the inspector (the collapsible selection panel, handoff
/// issue 10), resolved from the library session through the Session Views
/// seam. Owned data so the render call site can borrow it all at once.
/// `visible: false` is the hidden state — nothing selected, or the selection
/// the session remembers is no longer in the store (so the inspector can
/// never show stale art).
#[derive(Default)]
pub struct InspectorContent {
    /// Whether the inspector renders at all (a selection exists and the
    /// store still carries it).
    pub visible: bool,
    /// What the readout shows — drives which details resolve.
    pub kind: InspectorKind,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    /// The selection's cover art source (a track id); `None` renders the
    /// neutral placeholder block.
    pub art_track: Option<TrackId>,
    /// The selection's track ids in store order — the tag editor's targets.
    /// The batch Play and Add to Queue actions these once started are retired
    /// (they moved to the collection context menu); what reads this field now
    /// is [`RiffApp::open_inline_draft`], which opens the Track draft from the
    /// first id and the Album batch draft from all of them, and
    /// `InlineTagEditor::draft_belongs`, which checks an open draft still
    /// belongs to the readout it was opened from.
    pub track_ids: Vec<TrackId>,
    pub details: Vec<crate::ui::selection::SelectionDetail>,
    /// The tag section's seven resolved rows (Title → Track Number), for
    /// Track and Album readouts. Each row carries its display state and text
    /// plus the per-track original values the inline editor diff bases
    /// (tickets 02/03). Empty for Artist/Genre readouts and hidden states.
    pub tags: Vec<crate::ui::selection::TagRow>,
}

/// Resolve what the inspector renders for the session: the selected track —
/// single-clicking a track row in any track listing sets it, so the detail
/// panel follows the click into the track's readout — or, when no track is
/// selected, the deepest entity in the drill-down path (album > artist >
/// genre). Entity selections clear the selected track (see
/// [`riff_backend::app::state::LibrarySession::select_at`]), so the two never
/// compete. The album variant preserves today's selection-panel content
/// (cover, title, artist · year line, the aggregated tag section, and the
/// details grid: artist, track count · total time, plays, last played, path)
/// — all read
/// through the Session Views seam, so scans and tag edits can never leave
/// stale rows. Any selection the store no longer carries resolves hidden,
/// never a stale readout.
pub fn resolve_inspector(views: &mut SessionViews, library: &LibrarySession) -> InspectorContent {
    use riff_backend::app::state::BrowserSelection;

    // The selected track wins: single-clicking a track in the flat list,
    // search, playlists, folders, or the album's Tracks column shows that
    // track's readout in the detail panel. A stale id (the store dropped the
    // track) falls through to the path entity below.
    if let Some(track_id) = library.selected_track.clone()
        && let Some(track) = views.selected_track(&track_id)
    {
        return track_inspector(track);
    }
    let Some(selection) = library.current_selection() else {
        return InspectorContent::default();
    };
    match selection {
        BrowserSelection::Album { artist, title } => album_inspector(views, artist, title),
        BrowserSelection::Artist(name) => artist_inspector(views, name),
        BrowserSelection::Genre(genre) => genre_inspector(views, genre),
    }
}

/// Resolve the tag section rows for the given tracks. For an album every
/// field aggregates across its tracks: all carry the same value → the value
/// as-is; any disagreement, or a mix of present and missing, → `(different)`;
/// no track carries the field → `(none)`. Missing is a distinct comparison
/// value — there is no majority rule and no ignore-missing rule. For a
/// single-track readout the row carries that track's value or `(none)`. Each
/// row keeps the per-track originals the inline editor's draft (tickets
/// 02/03) diffs against, so the model never fabricates a value.
fn tag_rows(tracks: &[Track]) -> Vec<crate::ui::selection::TagRow> {
    use crate::ui::selection::{TagField, TagRow, TagRowState};

    TagField::ALL
        .iter()
        .map(|&field| {
            let originals: Vec<Option<String>> = tracks
                .iter()
                .map(|track| match field {
                    TagField::Title => track.metadata.title.clone(),
                    TagField::Artist => track.metadata.artist.clone(),
                    TagField::Album => track.metadata.album.clone(),
                    TagField::AlbumArtist => track.metadata.album_artist.clone(),
                    TagField::Genre => track.metadata.genre.clone(),
                    TagField::Year => track.metadata.year.map(|year| year.to_string()),
                    TagField::TrackNumber => track.metadata.track_number.map(|n| n.to_string()),
                })
                .collect();
            let state = if originals.iter().all(Option::is_none) {
                TagRowState::None
            } else if let Some(only) = originals.first().and_then(Option::as_ref)
                && originals.iter().all(|v| v.as_ref() == Some(only))
            {
                TagRowState::Value
            } else {
                TagRowState::Different
            };
            let text = match state {
                TagRowState::Value => originals
                    .first()
                    .and_then(Option::clone)
                    .unwrap_or_default(),
                TagRowState::Different => "(different)".to_string(),
                TagRowState::None => "(none)".to_string(),
            };
            TagRow {
                field,
                state,
                text,
                originals,
            }
        })
        .collect()
}

/// The compact track readout: title, artist, album, metadata, and the
/// single-track batch the Play/Queue actions start.
fn track_inspector(track: riff_backend::domain::Track) -> InspectorContent {
    let mut details = vec![
        crate::ui::selection::SelectionDetail {
            label: "Plays".to_string(),
            value: track.play_count.to_string(),
        },
        crate::ui::selection::SelectionDetail {
            label: "Last played".to_string(),
            value: track.last_played.map_or_else(
                || "Never".to_string(),
                |at| {
                    let elapsed = at.elapsed().unwrap_or(std::time::Duration::ZERO);
                    crate::ui::sidebar::format_last_scan_ago(elapsed)
                },
            ),
        },
        crate::ui::selection::SelectionDetail {
            label: "Path".to_string(),
            value: track.file_path.to_string_lossy().to_string(),
        },
    ];
    // ReplayGain is a read-only fact the file carries: the tag editor cannot
    // express it, and the album value is never applied. A file without the
    // tag grows no row — the DETAILS block has no `(none)` state (that
    // convention belongs to `tag_rows`).
    if let Some(gain) = track.metadata.replaygain_track_gain {
        details.push(crate::ui::selection::SelectionDetail {
            label: "ReplayGain (track)".to_string(),
            // `.2` is load-bearing: the f32 widens into the store's REAL
            // column and narrows back, so an unformatted print is
            // `-6.540000057220459`.
            value: format!("{gain:+.2} dB"),
        });
    }
    if let Some(gain) = track.metadata.replaygain_album_gain {
        details.push(crate::ui::selection::SelectionDetail {
            label: "ReplayGain (album)".to_string(),
            value: format!("{gain:+.2} dB"),
        });
    }
    InspectorContent {
        visible: true,
        kind: InspectorKind::Track,
        title: Some(track.metadata.display_title(&track.file_path)),
        subtitle: Some(track.metadata.display_artist()),
        art_track: Some(track.id.clone()),
        track_ids: vec![track.id.clone()],
        details,
        // The Artist/Album/Genre detail rows moved into the tag section;
        // the non-tag facts (Plays, Last played, Path) stay below it.
        tags: tag_rows(std::slice::from_ref(&track)),
    }
}

/// The album readout: cover, title, artist · year line, the aggregated tag
/// section, and the details grid (artist, track count · total time, plays,
/// last played, path). Hidden when the store no longer carries the album.
fn album_inspector(views: &mut SessionViews, artist: &str, title: &str) -> InspectorContent {
    let tracks = views.album_tracks(artist, title);
    if tracks.is_empty() {
        // The album is gone from the store (a rescan dropped it) — the
        // hidden state, never a stale readout.
        return InspectorContent::default();
    }
    // The album's year and genre come from its entry in the artist's album
    // table (the same year the albums column prints on its `Artist · Year`
    // detail line).
    let albums = views.artist_albums(artist);
    let album = albums.iter().find(|album| album.title == title);
    let mut details = vec![
        crate::ui::selection::SelectionDetail {
            label: "Artist".to_string(),
            value: artist.to_string(),
        },
        crate::ui::selection::SelectionDetail {
            label: "Tracks".to_string(),
            value: format!(
                "{}{}",
                tracks.len(),
                total_duration(&tracks)
                    .map(|total| format!(
                        " \u{b7} {}",
                        crate::ui::playerbar::format_duration(total)
                    ))
                    .unwrap_or_default()
            ),
        },
        crate::ui::selection::SelectionDetail {
            label: "Plays".to_string(),
            value: tracks
                .iter()
                .map(|track| track.play_count)
                .sum::<u32>()
                .to_string(),
        },
        crate::ui::selection::SelectionDetail {
            label: "Last played".to_string(),
            value: last_played_label(&tracks),
        },
        crate::ui::selection::SelectionDetail {
            label: "Path".to_string(),
            value: tracks[0]
                .file_path
                .parent()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
        },
    ];
    // One album has one album gain, so the first Track that carries it is the
    // honest read — there is no `(different)` state to reach for here. Absent
    // on every Track, no row.
    if let Some(gain) = tracks
        .iter()
        .find_map(|track| track.metadata.replaygain_album_gain)
    {
        details.push(crate::ui::selection::SelectionDetail {
            label: "ReplayGain".to_string(),
            value: format!("{gain:+.2} dB"),
        });
    }
    InspectorContent {
        visible: true,
        kind: InspectorKind::Album,
        title: Some(title.to_string()),
        subtitle: Some(album.and_then(|a| a.year).map_or_else(
            || artist.to_string(),
            |year| format!("{artist} \u{b7} {year}"),
        )),
        art_track: tracks.first().map(|track| track.id.clone()),
        track_ids: tracks.iter().map(|track| track.id.clone()).collect(),
        details,
        // The Released/Genre detail rows moved into the aggregated tag
        // section; the non-tag facts (Artist, Tracks, Plays, Last played,
        // Path) stay below it.
        tags: tag_rows(&tracks),
    }
}

/// The artist readout: name, album count, cover (the first album's first
/// track), and the artist's track batch. Hidden when the store no longer
/// carries the artist.
fn artist_inspector(views: &mut SessionViews, name: &str) -> InspectorContent {
    let albums = views.artist_albums(name);
    if albums.is_empty() {
        return InspectorContent::default();
    }
    let total_tracks: usize = albums.iter().map(|album| album.tracks.len()).sum();
    let details = vec![
        crate::ui::selection::SelectionDetail {
            label: "Albums".to_string(),
            value: album_count_label(albums.len()),
        },
        crate::ui::selection::SelectionDetail {
            label: "Tracks".to_string(),
            value: total_tracks.to_string(),
        },
    ];
    InspectorContent {
        visible: true,
        kind: InspectorKind::Artist,
        title: Some(name.to_string()),
        subtitle: Some(album_count_label(albums.len())),
        // The artist's cover: the first album's first track.
        art_track: albums
            .first()
            .and_then(|album| album.tracks.first().cloned()),
        track_ids: albums
            .iter()
            .flat_map(|album| album.tracks.iter().cloned())
            .collect(),
        details,
        // Tags attach to Tracks and Albums, never to the Artist entity.
        tags: Vec::new(),
    }
}

/// The genre readout: name and track count, with the genre-scoped track
/// batch. Hidden when the genre is gone from the read model.
fn genre_inspector(views: &mut SessionViews, genre: &str) -> InspectorContent {
    let count = views
        .genres()
        .iter()
        .find(|row| row.genre == genre)
        .map(|row| row.tracks);
    let Some(tracks) = count else {
        // The genre is gone from the read model — hidden, never a stale
        // readout.
        return InspectorContent::default();
    };
    let mut track_ids = Vec::new();
    for artist in views.artists_in_genre(genre).iter() {
        for album in views.artist_albums_in_genre(&artist.name, genre).iter() {
            for track in views
                .album_tracks_in_genre(&album.artist, &album.title, genre)
                .iter()
            {
                track_ids.push(track.id.clone());
            }
        }
    }
    let details = vec![crate::ui::selection::SelectionDetail {
        label: "Tracks".to_string(),
        value: tracks.to_string(),
    }];
    InspectorContent {
        visible: true,
        kind: InspectorKind::Genre,
        title: Some(genre.to_string()),
        subtitle: Some(format!("{tracks} tracks")),
        art_track: None,
        track_ids,
        details,
        tags: Vec::new(),
    }
}

/// `"1 album"` / `"N albums"` — the artist readout's count line.
fn album_count_label(count: usize) -> String {
    match count {
        1 => "1 album".to_string(),
        n => format!("{n} albums"),
    }
}

/// The album's known durations, summed; `None` when no track carries a
/// duration (the count then reads alone, without a `·` total).
fn total_duration(tracks: &[riff_backend::domain::Track]) -> Option<std::time::Duration> {
    tracks
        .iter()
        .filter_map(|track| track.duration)
        .reduce(|a, b| a + b)
}

/// The most recent play time across the album's tracks, as a coarse
/// relative readout (`5m ago` — the sidebar footer's buckets); `Never`
/// before the first play.
fn last_played_label(tracks: &[riff_backend::domain::Track]) -> String {
    let latest = tracks.iter().filter_map(|track| track.last_played).max();
    latest.map_or_else(
        || "Never".to_string(),
        |at| {
            let elapsed = at.elapsed().unwrap_or(std::time::Duration::ZERO);
            crate::ui::sidebar::format_last_scan_ago(elapsed)
        },
    )
}

/// The transient playlist-prompt slots the sidebar's playlist rows act on,
/// grouped so [`apply_playlist_row_action`] stays readable. They live on
/// [`RiffApp`] between frames; this borrow bundle is built per frame.
pub struct PlaylistPromptSlots<'a> {
    /// Which user playlist is open in the explorer.
    pub view: &'a mut Option<PlaylistId>,
    /// Which read-only smart playlist is open (closed when a user playlist
    /// opens).
    pub smart_view: &'a mut Option<SmartPlaylistKind>,
    /// The inline rename prompt: (playlist id, draft name).
    pub rename: &'a mut Option<(PlaylistId, String)>,
    /// The inline "New Playlist" prompt draft.
    pub create_name: &'a mut Option<String>,
}

/// The SMART LISTS sidebar section (design-handoff issue 07): the four core
/// lists — in the design's order — are always visible, and Never Played /
/// Lost Gems relocate behind Advanced mode (relocated, not deleted).
#[must_use]
pub fn smart_list_kinds(advanced: bool) -> Vec<SmartPlaylistKind> {
    const CORE: [SmartPlaylistKind; 4] = [
        SmartPlaylistKind::RecentlyAdded,
        SmartPlaylistKind::RecentlyPlayed,
        SmartPlaylistKind::MostPlayed,
        SmartPlaylistKind::Favorites,
    ];
    if !advanced {
        return CORE.to_vec();
    }
    let mut kinds = CORE.to_vec();
    kinds.push(SmartPlaylistKind::NeverPlayed);
    kinds.push(SmartPlaylistKind::LostGems);
    kinds
}

/// Whether a smart list may currently be open (handoff issue 07): the
/// Advanced-only lists (Never Played, Lost Gems) close once Advanced mode
/// flips off; the core four always stay openable.
#[must_use]
pub fn smart_list_openable(kind: SmartPlaylistKind, advanced: bool) -> bool {
    advanced
        || !matches!(
            kind,
            SmartPlaylistKind::NeverPlayed | SmartPlaylistKind::LostGems
        )
}

/// Apply one restyled playlist-row action (Issue 07) through the SAME Store
/// flows the pre-restyle buttons used (ADR 0002): every mutation commits to
/// the [`PlaylistStore`] and nothing else — the seam's playlist projection
/// invalidates itself via the mutation adapter's generation bump, so the
/// next [`SessionViews::playlists`] read reflects the commit with zero
/// caller action. Open/Rename only move transient prompt state; Rename
/// looks the playlist's current name up through the seam so callers never
/// need a per-frame name copy.
pub fn apply_playlist_row_action(
    action: crate::ui::sidebar::PlaylistRowAction,
    id: &PlaylistId,
    store: &mut dyn PlaylistStore,
    views: &mut SessionViews,
    slots: PlaylistPromptSlots<'_>,
) {
    match action {
        crate::ui::sidebar::PlaylistRowAction::Open => {
            *slots.view = Some(id.clone());
            *slots.smart_view = None;
        }
        crate::ui::sidebar::PlaylistRowAction::Rename => {
            let name = views
                .playlists()
                .iter()
                .find(|p| &p.id == id)
                .map_or_else(String::new, |p| p.name.clone());
            *slots.rename = Some((id.clone(), name));
            *slots.create_name = None;
        }
        crate::ui::sidebar::PlaylistRowAction::Delete => {
            if let Err(e) = store.delete_playlist(id) {
                tracing::warn!("Failed to delete playlist: {e}");
            }
            if slots.view.as_ref() == Some(id) {
                *slots.view = None;
            }
        }
    }
}

/// Commit the inline rename prompt's Save: trim the draft and rename through
/// the [`PlaylistStore`] as one durable transaction. Empty drafts are
/// ignored (pre-restyle behavior). The seam's next read reflects the commit
/// on its own — no projection refresh here.
pub fn commit_playlist_rename(store: &mut dyn PlaylistStore, id: &PlaylistId, draft: &str) {
    let draft = draft.trim().to_string();
    if draft.is_empty() {
        return;
    }
    if let Err(e) = store.rename_playlist(id, &draft) {
        tracing::warn!("Failed to rename playlist: {e}");
    }
}

/// Commit a playlist drag-reorder (Issue 12): compute the new entry order
/// from the gesture (`from` → `to`) via [`crate::app::playlist_manager::
/// reorder_tracks`] against the seam's current snapshot, then persist it
/// through the [`PlaylistStore`] port as one immediate durable transaction.
/// No-ops — the store is never touched — for self-drops, out-of-bounds
/// gestures, and unknown playlists. The committed mutation bumps the
/// playlist generation, so the seam's next read reflects the new order with
/// zero caller action (ADR 0002).
pub fn commit_playlist_reorder(
    views: &mut SessionViews,
    store: &mut dyn PlaylistStore,
    id: &PlaylistId,
    from: usize,
    to: usize,
) {
    let new_order = views
        .playlists()
        .iter()
        .find(|p| &p.id == id)
        .and_then(|playlist| {
            riff_backend::app::playlist_manager::reorder_tracks(&playlist.tracks, from, to)
        });
    let Some(new_order) = new_order else {
        return;
    };
    if let Err(e) = store.reorder_playlist_entries(id, &new_order) {
        tracing::warn!("Failed to reorder playlist entries: {e}");
    }
}

/// Global keyboard shortcuts: Ctrl+K focuses the global search (issue 06),
/// and Space toggles playback. Public so the shortcut contract is testable
/// headlessly (precedent: [`Preferences::hydrate`]).
pub fn handle_keyboard_shortcuts(
    ctx: &egui::Context,
    playback: &PlaybackSession,
    global_search_focus: &mut bool,
    transport: &dyn Transport,
) {
    // The "no widget owns the keyboard" test short-circuits BEFORE the key is
    // consumed, so a Space a text field wanted is left for the field — the same
    // order `RiffApp::read_frame_input` reads them in.
    let toggle_playback = !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
    crate::ui::frame::apply_keyboard(
        ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)),
        toggle_playback,
        playback,
        global_search_focus,
        transport,
    );
}

impl RiffApp {
    /// Bottom shell strip (Issues 06 + 08): transport, seek row, and volume
    /// at the exact 88px playerbar token height, drawn by the restyled
    /// playerbar widgets, plus the queue sheet above its right edge. Content
    /// in, actions out: the panel reads the sessions and **reports**; the Frame's
    /// [`Frame::apply_control_bar`] is the only applier, so each control still
    /// emits its engine command and the panel itself mutates no session state.
    /// Also stashes this frame's `(title, meta_line)` handout in
    /// [`Self::now_playing_labels`] for the Now Playing stage.
    fn render_control_bar(
        &mut self,
        ui: &mut egui::Ui,
        library: &LibrarySession,
        playback: &PlaybackSession,
    ) -> ControlBarReport {
        // Cover for the current track, served from the LRU texture cache;
        // misses enqueue a background resolve exactly like the other views.
        // The current Track comes from the Session Views seam over the
        // store's `get_track` query — never the in-memory mirror.
        let mut cover = None;
        self.views
            .sync_playback(&playback.queue, crate::ui::now_playing::UP_NEXT_LIMIT);
        if let Some(track) = self.views.playback_current() {
            let id = track.id.clone();
            let file_path = track.file_path.clone();
            self.request_cover(&id, &file_path, COVER_THUMB);
            cover = Some(self.resolve_cover_texture(ui.ctx(), &id.0, COVER_THUMB));
        }

        // The `{index}/{len}` queue-position label, formatted fresh each
        // frame from the live queue shape.
        let queue_position = format!(
            "{}/{}",
            playback.queue.current_index.map_or(0, |i| i + 1),
            playback.queue.tracks.len()
        );
        // The current track's display lines, resolved once per frame here —
        // the bar renders before every stage — and returned for the Now
        // Playing stage so the two surfaces never build the strings twice.
        let (title, meta_line) = match self.views.playback_current() {
            Some(track) => (
                Some(Arc::from(track.metadata.display_title(&track.file_path))),
                Some(Arc::from(format!(
                    "{} - {}",
                    track.metadata.display_artist(),
                    track.metadata.display_album()
                ))),
            ),
            None => (None, None),
        };
        self.playerbar_readouts.sync(
            playback.current_position.current,
            playback.current_position.total,
        );
        let content = crate::ui::playerbar::PlayerBarContent {
            cover,
            title: title.clone(),
            meta_line: meta_line.clone(),
            playback: playback.playback_state,
            position: playback.current_position.current,
            total: playback.current_position.total,
            volume: playback.current_volume,
            muted: playback.muted,
            shuffle: playback.queue.shuffle,
            repeat: playback.queue.repeat,
            queue_position: &queue_position,
            queue_open: library.queue_open,
            expanded: library.view_mode == ViewMode::NowPlaying,
            advanced: library.ui_flags.advanced_mode,
        };

        egui::Panel::bottom("playerbar")
            .exact_size(theme::PLAYERBAR_H)
            .show(ui, |ui| {
                crate::ui::playerbar::show_player_bar(
                    ui,
                    &mut self.icons,
                    &self.theme.active,
                    &content,
                    &mut self.playerbar_readouts,
                    &mut self.playerbar_actions,
                );
            });

        // Queue panel (handoff issue 13): while the bar's queue button has
        // the panel open, the existing Up Next read model is revealed in a
        // floating sheet above the bar's right edge; its rows report
        // `PlayNext` through the same action drain as the bar itself.
        if library.queue_open {
            self.views
                .sync_playback(&playback.queue, crate::ui::playerbar::QUEUE_PANEL_LIMIT);
            let up_next = crate::ui::sidebar::up_next_entries(
                self.views.playback_up_next(),
                crate::ui::playerbar::QUEUE_PANEL_LIMIT,
            );
            crate::ui::playerbar::show_queue_panel(
                ui,
                &mut self.icons,
                &self.theme.active,
                &up_next,
                &mut self.playerbar_actions,
            );
        }
        self.now_playing_labels = (title, meta_line);
        ControlBarReport {
            actions: std::mem::take(&mut self.playerbar_actions),
        }
    }
}

// --- Helper methods factored out to avoid borrow conflicts ---
impl RiffApp {
    /// Sidebar content (design-handoff issue 07): the flat, always-visible
    /// sectioned nav — LIBRARY, SMART LISTS, PLAYLISTS — every row carrying a
    /// live count from the counts read model, with the Add-folder /
    /// "Last scan X ago" footer pinned to the panel's bottom. Draws inside
    /// the shell's fixed-width sidebar panel, which is shared chrome present
    /// on every view; only the main stage switches. Every nav click reports a
    /// [`SidebarAction`] that lands on the library view (clearing any active
    /// search) so exactly one browser variant is visible after it — the variant
    /// itself renders in the browser column pane (issue 08), not here.
    ///
    /// The panel mutates no session state at all: it reads the library session
    /// and the frontend's transient playlist slots, and every click becomes a
    /// report the Frame answers.
    ///
    /// **Why the panel applies its own reports** rather than returning one:
    /// each sub-section reads what the one above it decided. The SMART LISTS
    /// chevron folds the rows drawn *below* it away; the Playlists "+" opens
    /// the prompt drawn *below* it; a prompt's confirmation moves a row's
    /// highlight drawn *below* it. Answering once at the end of the panel would
    /// make every one of those land a frame late, which is a behaviour change
    /// and not a refactor. So the sidebar has four *sub-slots*, each answered
    /// where the pre-split frame answered it — and the Frame stays the only
    /// writer either way.
    fn render_library_sidebar(
        &mut self,
        ui: &mut egui::Ui,
        playback: &mut PlaybackSession,
        library: &mut LibrarySession,
        out: &mut FrameOutput,
    ) {
        // One counts read per frame: every nav row's live count comes from
        // the counts read model (handoff issue 05), cached per store
        // generation so scans and playlist edits update it by the next frame.
        let counts = self
            .views
            .sidebar_counts(library.library_paths.paths().len());

        // A row highlights only while its browser variant is actually on
        // screen (the Library view, no list opened over it).
        let no_playlist = self.smart_playlist_view.is_none() && self.playlist_view.is_none();
        let library_section_live = library.view_mode == ViewMode::Library
            && library.browse_mode == BrowseMode::Library
            && no_playlist;
        let folder_section_live =
            library.view_mode == ViewMode::Library && library.browse_mode == BrowseMode::Folders;

        // --- LIBRARY ---------------------------------------------------------
        let mut section = SidebarReport::default();
        self.render_library_rows(
            ui,
            library,
            LibraryNav {
                counts: &counts,
                library_section_live,
                folder_section_live,
            },
            &mut section,
        );
        self.apply_sidebar_slot(playback, library, out, &section);
        ui.add_space(8.0);

        // --- SMART LISTS -------------------------------------------------------
        // Four core lists are always visible (no Advanced gate); Never Played
        // and Lost Gems relocate behind Advanced mode (relocated, not
        // deleted — handoff issue 07 / open decision 2).
        let mut section = SidebarReport::default();
        self.render_smart_lists_header(ui, library.ui_flags.smart_lists_collapsed, &mut section);
        self.apply_sidebar_slot(playback, library, out, &section);
        let mut section = SidebarReport::default();
        self.render_smart_list_rows(
            ui,
            library,
            library.ui_flags.smart_lists_collapsed,
            &counts,
            &mut section,
        );
        self.apply_sidebar_slot(playback, library, out, &section);

        // --- PLAYLISTS ---------------------------------------------------------
        // User playlists (Task 4.2): named, editable lists persisted in the
        // Application Store, with their existing create/rename/delete/reorder
        // flows. Always visible.
        let mut header = SidebarReport::default();
        self.render_playlists_header(ui, &mut header);
        self.apply_sidebar_slot(playback, library, out, &header);
        let mut prompts = SidebarReport::default();
        self.render_playlist_rows(ui, &mut prompts);
        self.apply_sidebar_slot(playback, library, out, &prompts);

        // --- FOOTER (pinned to the panel's bottom) ------------------------------
        let mut section = SidebarReport::default();
        egui::Panel::bottom("sidebar_footer")
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.render_sidebar_footer(ui, &mut section);
            });
        self.apply_sidebar_slot(playback, library, out, &section);
    }

    /// Answer one sidebar sub-section's report, at that sub-section's slot.
    ///
    /// The whole of "the Frame is the only writer of the library session", in
    /// one line: the panel pushes facts and this is where they land.
    fn apply_sidebar_slot(
        &mut self,
        playback: &mut PlaybackSession,
        library: &mut LibrarySession,
        out: &mut FrameOutput,
        report: &SidebarReport,
    ) {
        self.frame(playback, library).apply_sidebar(out, report);
    }

    /// The LIBRARY section's five rows (design-handoff issue 07): All Tracks,
    /// Artists, Albums, Genres, and Folders, each with its live count.
    /// Clicking one lands on the library view in that section (Folders
    /// switches the browse mode instead) and closes any opened list.
    fn render_library_rows(
        &mut self,
        ui: &mut egui::Ui,
        library: &LibrarySession,
        nav: LibraryNav<'_>,
        report: &mut SidebarReport,
    ) {
        use crate::ui::sidebar::{self, TreeRow};

        let LibraryNav {
            counts,
            library_section_live,
            folder_section_live,
        } = nav;
        let palette = self.theme.active;
        sidebar::section_header(ui, &palette, "Library");
        let library_rows: [(LibrarySection, &str, crate::ui::icons::Icon, usize); 4] = [
            (
                LibrarySection::AllTracks,
                "All Tracks",
                crate::ui::icons::Icon::ListMusic,
                counts.tracks,
            ),
            (
                LibrarySection::Artists,
                "Artists",
                crate::ui::icons::Icon::Library,
                counts.artists,
            ),
            (
                LibrarySection::Albums,
                "Albums",
                crate::ui::icons::Icon::Disc,
                counts.albums,
            ),
            (
                LibrarySection::Genres,
                "Genres",
                crate::ui::icons::Icon::Music,
                counts.genres,
            ),
        ];
        for (section, label, icon, count) in library_rows {
            let row = sidebar::tree_row(
                ui,
                &mut self.icons,
                &palette,
                TreeRow {
                    indent_level: 0,
                    icon: Some(icon),
                    cover: None,
                    label,
                    count: Some(count),
                    meta: None,
                    favorite: None,
                    selected: library_section_live && library.library_section == section,
                    now_playing: false,
                    playing: false,
                    art_slot: false,
                },
            );
            if row.response.clicked() {
                report.push(SidebarAction::Navigate { section });
            }
        }
        // Folders is the one LIBRARY row that switches browse mode instead of
        // section; its count is the registered library roots.
        let folders_row = sidebar::tree_row(
            ui,
            &mut self.icons,
            &palette,
            TreeRow {
                indent_level: 0,
                icon: Some(crate::ui::icons::Icon::Folder),
                cover: None,
                label: "Folders",
                count: Some(counts.folder_roots),
                meta: None,
                favorite: None,
                selected: folder_section_live,
                now_playing: false,
                playing: false,
                art_slot: false,
            },
        );
        if folders_row.response.clicked() {
            report.push(SidebarAction::NavigateFolders);
        }
    }

    /// The SMART LISTS section's header (design-handoff issue 07): the
    /// clickable chevron pinned to its right edge (the same slot the Playlists
    /// "+" button uses) folds the section away. The collapsed state is a
    /// persisted UI flag (`UiFlags::smart_lists_collapsed`) restored on launch
    /// through the scalar settings round-trip.
    ///
    /// Its OWN slot, in [`Self::render_library_sidebar`] and not inside the row
    /// pass: the chevron's fold has to land BEFORE the rows below are gated on
    /// the folded flag, exactly where the pre-split frame applied it.
    fn render_smart_lists_header(
        &mut self,
        ui: &mut egui::Ui,
        collapsed: bool,
        report: &mut SidebarReport,
    ) {
        use crate::ui::icons::Icon;
        use crate::ui::sidebar;

        let palette = self.theme.active;
        ui.horizontal(|ui| {
            sidebar::section_header(ui, &palette, "Smart Lists")
                .on_hover_text("Auto-generated, read-only lists built from your play history.");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let chevron_rect = egui::Rect::from_center_size(
                    egui::pos2(ui.max_rect().right() - 12.0, ui.cursor().center().y),
                    egui::vec2(24.0, 24.0),
                );
                let chevron = if collapsed {
                    Icon::ChevronRight
                } else {
                    Icon::ChevronDown
                };
                let label = if collapsed {
                    "Expand Smart Lists"
                } else {
                    "Collapse Smart Lists"
                };
                // The chevron keeps the smart-list rows' ink tint
                // (`ink_2`, the same color the Sparkles glyphs paint) and
                // does not flip on hover — it reads as part of the section,
                // not as an ephemeral control.
                let tint = palette.ink_2;
                let response = ui.interact(
                    chevron_rect,
                    ui.id().with("smart_lists_collapse"),
                    egui::Sense::click(),
                );
                let tex_id = self.icons.texture(ui.ctx(), chevron, 16.0, tint);
                let uv_full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter_at(chevron_rect)
                    .image(tex_id, chevron_rect.shrink(4.0), uv_full, tint);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
                });
                if response.clicked() {
                    report.push(SidebarAction::ToggleSmartListsCollapsed);
                }
                response.on_hover_text(label);
            });
        });
    }

    /// The SMART LISTS section's rows (design-handoff issue 07): the four core
    /// lists always, Never Played / Lost Gems behind Advanced mode. Clicking one
    /// opens it over the library view.
    fn render_smart_list_rows(
        &mut self,
        ui: &mut egui::Ui,
        library: &LibrarySession,
        collapsed: bool,
        counts: &riff_backend::app::views::SidebarCounts,
        report: &mut SidebarReport,
    ) {
        use crate::ui::sidebar::{self, TreeRow};

        let palette = self.theme.active;
        if collapsed {
            return;
        }

        let smart_count = |kind: SmartPlaylistKind| {
            counts
                .smart_lists
                .iter()
                .find(|(k, _)| *k == kind)
                .map_or(0, |(_, n)| *n)
        };
        for kind in smart_list_kinds(library.ui_flags.advanced_mode) {
            let row = sidebar::tree_row(
                ui,
                &mut self.icons,
                &palette,
                TreeRow {
                    indent_level: 0,
                    icon: Some(crate::ui::icons::Icon::Sparkles),
                    cover: None,
                    label: kind.display_name(),
                    count: Some(smart_count(kind)),
                    meta: None,
                    favorite: None,
                    selected: self.smart_playlist_view == Some(kind),
                    now_playing: false,
                    playing: false,
                    art_slot: false,
                },
            );
            if row.response.clicked() {
                report.push(SidebarAction::OpenSmartList(kind));
            }
        }
    }

    /// The sidebar footer's action side: the stamp text comes from the
    /// last-scan read model (issue 05) formatted by
    /// [`sidebar::format_last_scan_ago`]; Add folder routes through the
    /// EXISTING add-library-path flow.
    fn render_sidebar_footer(&mut self, ui: &mut egui::Ui, report: &mut SidebarReport) {
        use crate::ui::sidebar;

        let stamp = self.views.last_scan().map(|scan| {
            let elapsed = scan.elapsed().unwrap_or_default();
            format!("Last scan {}", sidebar::format_last_scan_ago(elapsed))
        });
        if sidebar::sidebar_footer(ui, &mut self.icons, &self.theme.active, stamp.as_deref()) {
            // The native folder picker is an OS dialog, not a decision, so the
            // Frame decides that one was asked for and the draw half performs
            // it (on Linux it is a view change, which the Frame applies itself
            // so this frame's stage already draws the input row).
            report.push(SidebarAction::AddFolderRequested);
        }
    }

    fn render_flat_view(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
        query: &str,
    ) {
        // Row-virtualization audit (Issue 12): every UNBOUNDED track listing
        // culls through `ScrollArea::show_rows` — this flat list and search
        // (bounded store windows via `SessionViews::track_row`), smart
        // playlists and user playlists (`render_smart_playlist_view` /
        // `render_playlist_view`), and the Up Next queue
        // (`now_playing::show_now_playing`). The artist/album and folder
        // trees render per-node loops instead, but each loop is bounded by
        // one album's or one folder's contents inside a collapsed-by-default
        // node — not a whole-library listing. Culling itself is pinned by
        // `test_large_library_fixture_culls_rows_to_the_visible_window`.
        //
        // The flat list and search box are served through the bounded
        // Session Projection behind the Session Views seam (ADR 0003):
        // only visible row windows fetch, invalidated by generation bumps
        // after committed mutations. This view asks the seam two things —
        // how many rows the listing has, and row *i* — and never learns the
        // window size, the window a row lives in, or when a refetch is due.
        let current_track = playback.queue.current_track().cloned();

        // The count read: its own store read, the value to size the row
        // range with.
        let total = self.views.track_count(query, library.track_sort);

        // ---- Scroll Memory (scroll-memory spec, issue 01) ----
        // The All Tracks flat list is the first Section slot: it declares its
        // slot to the Scroll Memory, which hands back the salt and the start
        // offset (the saved one when the fingerprint matches, zero on a stale
        // slot or none saved yet), and the visit token that records the actual
        // offset back under the same content identity.
        let (control, visit) = self.scroll_memory.begin_section(
            riff_backend::app::state::LibrarySection::AllTracks,
            query,
            library.track_sort.as_sort_key(),
        );
        if total == 0 {
            // Query-aware empty copy: the flat list explains a filtered-to-
            // empty search, never the empty-library copy.
            let (emp_title, emp_hint): (&str, String) = if query.is_empty() {
                (
                    "No tracks yet",
                    "Add a folder from the sidebar to start scanning your library.".to_string(),
                )
            } else {
                (
                    "No matching tracks",
                    format!("Nothing in your library matches '{query}'."),
                )
            };
            crate::ui::browser::empty_state(ui, &self.theme.active, emp_title, &emp_hint);
            self.scroll_memory.end_section(visit, 0.0);
            return;
        }

        // The track sort control, in the same top-right header slot the
        // entity columns' A–Z toggle occupies. Only rendered while there is
        // something to reorder — an empty listing hides it, like every other
        // column's sort control.
        if let Some(sort) =
            crate::ui::browser::track_sort_row(ui, &self.theme.active, library.track_sort)
        {
            library.track_sort = sort;
            // The sort is part of the content fingerprint above, so the next
            // frame's begin_section resets the scroll without extra
            // bookkeeping here.
        }

        // `.animated(false)`: this list is driven by a `ScrollControl`, so it
        // jumps to a named offset (a restored position, or the top after a
        // selection change) and a keep-in-view jump must not lerp. The flag
        // governs programmatic scroll-to offsets only, not wheel feel — the
        // reason is written once on
        // [`ScrollControl`](crate::ui::scroll_memory::ScrollControl). The other
        // `ScrollArea`s in this file (and elsewhere) leave the flag on on
        // purpose; do not "fix" the inconsistency by flipping them.
        let scroll_area = egui::ScrollArea::vertical()
            .id_salt(control.salt)
            .animated(false)
            .vertical_scroll_offset(control.start.unwrap_or(0.0));
        let output = scroll_area.show_rows(
            ui,
            theme::geometry::sidebar::ROW_H,
            total,
            |ui, row_range| {
                for i in row_range {
                    // Row *i*, by index: the seam decides whether that row
                    // leaves the window it has cached, and returns the row
                    // itself. The row is a shared handle, so the view holds
                    // it while it draws.
                    let Some(track) = self.views.track_row(query, library.track_sort, i) else {
                        continue;
                    };
                    self.render_track_row(
                        ui,
                        &track,
                        TrackRowContext {
                            library,
                            playback,
                            current_track: current_track.as_ref(),
                            reorder: None,
                        },
                    );
                }
            },
        );
        self.scroll_memory.end_section(visit, output.state.offset.y);
    }

    /// Render the tracks of a read-only smart playlist. The list reads
    /// through the Session Views seam over store queries (ADR 0002): every
    /// committed mutation bumps the generation, so the next frame
    /// regenerates from committed state — no manual refresh needed.
    fn render_smart_playlist_view(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        kind: SmartPlaylistKind,
    ) {
        // Bounded playlists cap at 50 entries; open-ended ones list all.
        let limit = match kind {
            SmartPlaylistKind::RecentlyAdded
            | SmartPlaylistKind::MostPlayed
            | SmartPlaylistKind::RecentlyPlayed => 50,
            SmartPlaylistKind::Favorites
            | SmartPlaylistKind::NeverPlayed
            | SmartPlaylistKind::LostGems => usize::MAX,
        };
        let tracks = self.views.smart_list(kind, limit);
        let current_track = playback.queue.current_track().cloned();

        // Header: name + count, clearly read-only (no edit/delete affordances),
        // with whole-list actions mirroring the album/folder header menu. The
        // sort control rides the header's right edge, like the other single
        // listings' — hidden while the list is empty (nothing to reorder).
        let header = ui.horizontal(|ui| {
            ui.heading(kind.display_name());
            ui.weak(format!("({} tracks, read-only)", tracks.len()));
            if !tracks.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(sort) = crate::ui::menu::track_sort_control(
                        ui,
                        &self.theme.active,
                        library.track_sort,
                    ) {
                        library.track_sort = sort;
                    }
                });
            }
        });
        if !tracks.is_empty() {
            let tids: Vec<TrackId> = tracks.iter().map(|t| t.id.clone()).collect();
            show_list_context_menu(
                &header_response(ui, &header),
                &self.theme.active,
                self.transport.as_ref(),
                playback,
                &tids,
            );
        }
        ui.separator();

        if tracks.is_empty() {
            ui.vertical_centered(|ui| {
                ui.label("No tracks in this playlist");
            });
            return;
        }

        // The listing renders the session's track sort over the computed
        // order — borrowed refs, so no per-frame copy of the tracks; the
        // canonical order is the no-op anchor and the title modes compare
        // case-insensitively, ties keeping the computed order.
        let mut rows: Vec<&Track> = tracks.iter().collect();
        sort_track_refs(&mut rows, library.track_sort);

        egui::ScrollArea::vertical().show_rows(
            ui,
            theme::geometry::sidebar::ROW_H,
            rows.len(),
            |ui, row_range| {
                for i in row_range {
                    if let Some(track) = rows.get(i).copied() {
                        self.render_track_row(
                            ui,
                            track,
                            TrackRowContext {
                                library,
                                playback,
                                current_track: current_track.as_ref(),
                                reorder: None,
                            },
                        );
                    }
                }
            },
        );
    }

    /// Render the "Playlists" section of the library explorer: the user's
    /// playlists as restyled rows whose hover-revealed edit/delete drive the
    /// existing rename/delete Store flows (Issue 07, ADR 0002), plus the
    /// create and rename prompts. Every mutation commits through the
    /// [`PlaylistStore`] port as one immediate durable transaction; reads
    /// come from the seam's playlist projection.
    fn render_playlists_header(&mut self, ui: &mut egui::Ui, report: &mut SidebarReport) {
        use crate::ui::icons::Icon;
        use crate::ui::sidebar;

        let palette = self.theme.active;
        ui.horizontal(|ui| {
            sidebar::section_header(ui, &palette, "Playlists")
                .on_hover_text("Your named playlists, saved across launches.");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let plus_rect = egui::Rect::from_center_size(
                    egui::pos2(ui.max_rect().right() - 12.0, ui.cursor().center().y),
                    egui::vec2(24.0, 24.0),
                );
                if sidebar::ghost_icon_button(
                    ui,
                    &mut self.icons,
                    &palette,
                    plus_rect,
                    ui.id().with("new_playlist"),
                    Icon::Plus,
                    "New Playlist",
                    false,
                ) {
                    report.push(SidebarAction::NewPlaylist);
                }
            });
        });
    }

    /// The playlist rows themselves, with the create prompt above them.
    fn render_playlist_rows(&mut self, ui: &mut egui::Ui, report: &mut SidebarReport) {
        use crate::ui::sidebar;

        let palette = self.theme.active;

        self.render_playlist_create_prompt(ui, report);

        // --- Playlist rows (open / hover-reveal rename / delete) ---
        //
        // Iterated by index straight over the seam's `Arc`'d snapshot
        // (allocation plan 2.5): no per-frame summaries Vec, no per-row
        // id/name clones. Each row's painted label is formatted from the
        // snapshot row; ids are only cloned on a click frame.
        let playlists = self.views.playlists();
        if playlists.is_empty() {
            // A labelled empty composition, not a bare hole under the header —
            // the shared empty-state owner every other listing surface uses.
            crate::ui::browser::empty_state(
                ui,
                &palette,
                "No playlists yet",
                "Select + to create one.",
            );
        }
        for index in 0..playlists.len() {
            let action = {
                let playlist = &playlists[index];
                let selected = self.playlist_view.as_ref() == Some(&playlist.id);
                let label = format!("{} ({})", playlist.name, playlist.tracks.len());
                sidebar::playlist_row(
                    ui,
                    &mut self.icons,
                    &palette,
                    &playlist.name,
                    &label,
                    selected,
                )
            };
            if let Some(action) = action {
                report.push(SidebarAction::PlaylistRow {
                    id: playlists[index].id.clone(),
                    action,
                });
            }

            self.render_playlist_rename_prompt(ui, &playlists[index].id, report);
        }
    }

    /// The inline "New Playlist" name prompt while it is open.
    ///
    /// Draws the draft it is handed and reports how the prompt resolved; the
    /// trim-and-commit store flow belongs to the Frame, which is the only
    /// writer of the prompt slot and of the playlist store here.
    fn render_playlist_create_prompt(&mut self, ui: &mut egui::Ui, report: &mut SidebarReport) {
        let Some(draft) = self.playlist_create_name.as_mut() else {
            return;
        };
        // The pure widget seam (golden-image gap audit P1-7) reports the
        // outcome; the store flow below is unchanged.
        if let Some(outcome) = crate::ui::prompts::playlist_create_prompt(ui, draft) {
            report.push(SidebarAction::PlaylistCreate(outcome));
        }
    }

    /// The inline rename prompt for one playlist while it is open. Addressed
    /// by playlist id so fresh frames never clone it.
    ///
    /// Draws the draft and reports how it resolved; the trim-and-commit store
    /// flow is the Frame's, at the sidebar's slot.
    fn render_playlist_rename_prompt(
        &mut self,
        ui: &mut egui::Ui,
        pid: &PlaylistId,
        report: &mut SidebarReport,
    ) {
        let renaming = self
            .playlist_rename
            .as_ref()
            .is_some_and(|(rid, _)| rid == pid);
        if !renaming {
            return;
        }
        let Some((_, draft)) = self.playlist_rename.as_mut() else {
            return;
        };
        // The pure widget seam (golden-image gap audit P1-7), same shape as
        // the create prompt.
        if let Some(outcome) = crate::ui::prompts::playlist_rename_prompt(ui, draft) {
            report.push(SidebarAction::PlaylistRename {
                id: pid.clone(),
                outcome,
            });
        }
    }

    /// Render the tracks of a user playlist, in order. Entries whose files
    /// have been moved or deleted are flagged invalid (dimmed, strikethrough,
    /// "missing" hint) and excluded from playback; valid entries get the
    /// standard track context menu plus "Remove from Playlist".
    ///
    /// Nothing here clones per frame in steady state: the header facts are
    /// read from the seam's `Arc`'d playlist list by reference (the borrow
    /// ends before the render loops take `self` mutably), and the resolved
    /// rows come from [`SessionViews::playlist_view`] as `Arc` clones.
    fn render_playlist_view(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        playlist_id: &PlaylistId,
    ) {
        use riff_backend::app::state::TrackSort;

        let playlists = self.views.playlists();
        let Some(playlist) = playlists.iter().find(|p| &p.id == playlist_id) else {
            ui.label("Playlist not found");
            return;
        };
        let playlist_name = &playlist.name;
        let track_count = playlist.tracks.len();

        let current_track = playback.queue.current_track().cloned();

        // Ready-to-render rows straight from the seam (ADR 0002): the
        // projection resolves every entry against the Library in one query
        // (LEFT-JOIN validity plus the read-time filesystem check), keeps
        // last good rows across store errors, and refetches whenever a
        // committed mutation moved either generation. `Arc` clones out — no
        // borrow held across rendering.
        let view = self.views.playlist_view(playlist_id).unwrap_or_default();
        let entries = view.rows;
        let valid_ids = view.valid_ids;

        // Header: name + count, with whole-list actions (valid tracks only),
        // mirroring the smart-playlist header menu. The sort control rides the
        // header's right edge — hidden while the list is empty (nothing to
        // reorder).
        let header = ui.horizontal(|ui| {
            ui.heading(playlist_name);
            ui.weak(format!("({track_count} tracks)"));
            if track_count > 0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(sort) = crate::ui::menu::track_sort_control(
                        ui,
                        &self.theme.active,
                        library.track_sort,
                    ) {
                        library.track_sort = sort;
                    }
                });
            }
        });
        if !valid_ids.is_empty() {
            show_list_context_menu(
                &header_response(ui, &header),
                &self.theme.active,
                self.transport.as_ref(),
                playback,
                &valid_ids,
            );
        }
        ui.separator();

        if track_count == 0 {
            ui.vertical_centered(|ui| {
                ui.label("No tracks in this playlist");
                ui.weak("Use a track's context menu \u{2192} Add to Playlist to add tracks.");
            });
            return;
        }

        // The listing renders the session's track sort over the playlist's
        // order: `sorted` maps each display position to its CANONICAL entry
        // index, so identity, menus, and removal keep addressing the playlist
        // the store knows. Reordering is the canonical order's own gesture —
        // a drag-and-drop on a sorted view would persist positions read off
        // the wrong order — so while sorted the rows render plain and the
        // canonical index still travels with each row.
        let sorted = match library.track_sort {
            TrackSort::NumberAsc => (0..entries.len()).collect::<Vec<_>>(),
            TrackSort::NumberDesc => (0..entries.len()).rev().collect(),
            TrackSort::TitleAsc | TrackSort::TitleDesc => {
                let desc = library.track_sort == TrackSort::TitleDesc;
                let key = |i: usize| {
                    entries[i].1.as_ref().map_or_else(
                        || entries[i].0.0.to_lowercase(),
                        |t| t.metadata.display_title(&t.file_path).to_lowercase(),
                    )
                };
                let mut order: Vec<usize> = (0..entries.len()).collect();
                if desc {
                    order.sort_by_key(|&i| std::cmp::Reverse(key(i)));
                } else {
                    order.sort_by_key(|&i| key(i));
                }
                order
            }
        };
        let reorderable = library.track_sort == TrackSort::NumberAsc;

        // One row per entry: the store-resolved track (if any) plus the
        // final playability verdict (Library-known AND file exists on disk).
        egui::ScrollArea::vertical().show_rows(
            ui,
            theme::geometry::sidebar::ROW_H,
            sorted.len(),
            |ui, row_range| {
                for i in row_range {
                    if let Some(&canonical) = sorted.get(i)
                        && let Some(entry) = entries.get(canonical)
                    {
                        self.render_playlist_entry(
                            ui,
                            entry,
                            TrackRowContext {
                                library,
                                playback,
                                current_track: current_track.as_ref(),
                                reorder: Some(PlaylistSlot {
                                    playlist_id,
                                    index: canonical,
                                    reorderable,
                                }),
                            },
                        );
                    }
                }
            },
        );
    }

    /// One row of [`Self::render_playlist_view`]: a normal track row for
    /// valid entries, a flagged "missing" row otherwise.
    ///
    /// The two shapes differ in what the menu can OFFER and in nothing else.
    /// A missing entry's menu is reduced — no playback action, no tag editor,
    /// and no Favourite — because its file cannot play, and because the heart
    /// this very branch replaces with a flagged label is what the menu's own
    /// Favourite item is gated on: the two agree because both are the same
    /// condition, not because either watches the other. Its right-click still
    /// selects it, because selection is a property of the row the menu is on
    /// rather than of the menu's contents, and the Detail Panel has to describe
    /// the same Track either way. The entry's identity survives the file: only
    /// the file went, so the store still resolves the Track and the readout is
    /// real.
    fn render_playlist_entry(
        &mut self,
        ui: &mut egui::Ui,
        entry: &(TrackId, Option<Track>, bool),
        ctx: TrackRowContext<'_>,
    ) {
        use std::path::PathBuf;
        let (tid, track, valid) = entry;
        if *valid && let Some(t) = track {
            self.render_reorderable_playlist_row(ui, t, ctx);
            return;
        }
        let playlist_id = ctx
            .reorder
            .expect("a playlist entry's listing always states its playlist")
            .playlist_id;
        let library = ctx.library;

        // Invalid entry: file moved or deleted. Flag it and exclude it from
        // playback; removal stays possible.
        let display = PathBuf::from(&tid.0)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&tid.0)
            .to_string();
        ui.horizontal(|ui| {
            ui.set_min_height(20.0);
            let response = ui
                .selectable_label(
                    false,
                    egui::RichText::new(format!("{display} (missing)"))
                        .strikethrough()
                        .color(ui.visuals().warn_fg_color),
                )
                .on_hover_text("File moved or deleted \u{2014} this entry won't play");
            self.attach_track_menu(
                &response,
                // A missing entry can still be removed from its Playlist: that
                // is a fact about the playlist row, not about the file behind it.
                TrackMenuSubject::unresolved(tid, Some(playlist_id)),
                &mut library.selected_track,
            );
        });
    }

    /// One drag-reorderable row of [`Self::render_playlist_view`] (Issue
    /// 12): the standard interactive track row wrapped in egui's built-in
    /// drag-and-drop support ([`sidebar::reorderable_row`]). Releasing a row
    /// on another persists the new order through the [`PlaylistStore`] port
    /// via [`commit_playlist_reorder`] (ADR 0002); clicks and double-clicks
    /// behave exactly as before. `reorderable: false` (the sorted views —
    /// a drag would persist positions read off the wrong order) renders the
    /// identical row shape without the drag wrapper, via [`sidebar::tree_row`].
    ///
    /// The row's drag hit-area sits UNDERNEATH the row's own response, and
    /// [`sidebar::reorderable_row`] documents why the ordering cannot be the
    /// other way round: egui's hit test swallows clicks that land on a
    /// drag-only widget stacked above a click widget. So that ordering is what
    /// lets a secondary click reach the row at all, which is why a right-click
    /// here opens the menu and selects the Track exactly as on every other
    /// Track row — and why it begins no drag. Reordering is the row's own
    /// gesture and the menu's is the right button; neither is the other's
    /// business.
    fn render_reorderable_playlist_row(
        &mut self,
        ui: &mut egui::Ui,
        track: &Track,
        ctx: TrackRowContext<'_>,
    ) {
        use crate::ui::sidebar::{self, TreeRow};

        let TrackRowContext {
            library,
            playback,
            current_track,
            reorder,
        } = ctx;
        let slot = reorder.expect("a playlist listing's rows always state their canonical slot");
        let PlaylistSlot {
            playlist_id,
            index,
            reorderable,
        } = slot;
        let is_selected = library.selected_track.as_ref() == Some(&track.id);
        let is_current = current_track == Some(&track.id);
        let playing = playback.playback_state == PlaybackState::Playing;
        let label = label_artist_title(track);

        self.request_cover(&track.id, &track.file_path, COVER_THUMB);

        // Same leading cover tile as the library rows: real art when cached,
        // otherwise the shared music-icon placeholder for artless tracks.
        let cover = Some(
            self.resolve_cover_texture(ui.ctx(), &track.id.0, COVER_THUMB)
                .id(),
        );

        let row = TreeRow {
            indent_level: 0,
            icon: None,
            cover,
            label: &label,
            count: None,
            meta: None,
            favorite: Some(track.favorite),
            selected: is_selected,
            now_playing: is_current,
            playing: is_current && playing,
            art_slot: false,
        };
        let (response, favorite_toggled, drop_from) = if reorderable {
            let outcome = sidebar::reorderable_row(
                ui,
                &mut self.icons,
                &self.theme.active,
                egui::Id::new(("riff_playlist_entry", &playlist_id.0, index)),
                index,
                row,
            );
            (
                outcome.response,
                outcome.favorite_toggled,
                outcome.drop_from,
            )
        } else {
            let outcome = sidebar::tree_row(ui, &mut self.icons, &self.theme.active, row);
            (outcome.response, outcome.favorite_toggled, None)
        };
        if response.clicked() {
            library.selected_track = Some(track.id.clone());
        }
        if response.double_clicked() {
            library.selected_track = Some(track.id.clone());
            self.transport.play(track.id.clone());
        }
        if let Some(favorite) = favorite_toggled {
            self.commit_track_favorite(&track.id, favorite);
        }
        if let Some(from) = drop_from {
            // One immediate durable transaction; the committed mutation
            // bumps the playlist generation, so the seam's next read serves
            // the new order with zero caller action (ADR 0002).
            commit_playlist_reorder(
                &mut self.views,
                self.playlist_store.as_mut(),
                playlist_id,
                from,
                index,
            );
        }
        self.attach_track_menu(
            &response,
            TrackMenuSubject::resolved(track, Some(playlist_id)),
            &mut library.selected_track,
        );
    }

    /// The restyled Now Playing stage (Issue 10): the 240px cover with its
    /// extra-large radius and brand glow, the 3xl title, the meta line, the
    /// in-view seek row, and the Up Next queue rows in Playback Queue order.
    /// Draws through the pure widget seam in
    /// [`crate::ui::now_playing::show_now_playing`] and *reports* every action
    /// into `report`, so Close still always lands on the Library View and the
    /// transport still emits engine commands — but only because the Frame's
    /// stage slot asks for it.
    fn show_now_playing_view(
        &mut self,
        ui: &mut egui::Ui,
        playback: &PlaybackSession,
        report: &mut StageReport,
    ) {
        // The title and meta lines ride the bar's once-per-frame handout.
        let (title, meta_line) = self.now_playing_labels.clone();
        let palette = self.theme.active;

        // Current track + cover from the LRU texture cache; misses enqueue a
        // background resolve exactly like the other views. Both the current
        // Track and the Up Next window come from the Session Views seam
        // over the store's `get_track` query — never the mirror. The cover
        // key is only re-cloned when the playing track moves, so fresh
        // frames allocate nothing here.
        self.views
            .sync_playback(&playback.queue, crate::ui::now_playing::UP_NEXT_LIMIT);
        let cover_key_changed = self.now_playing_cover_key.as_deref()
            != self.views.playback_current().map(|t| t.id.0.as_str());
        if cover_key_changed {
            if let Some(track) = self.views.playback_current() {
                let (id, file_path) = (track.id.clone(), track.file_path.clone());
                self.request_cover(&id, &file_path, COVER_HERO);
                self.now_playing_cover_key = Some(id.0);
            } else {
                self.now_playing_cover_key = None;
            }
        }
        // Take/restore keeps the borrow checker happy without copying the
        // key on fresh frames.
        let cover_key = self.now_playing_cover_key.take();
        let cover = cover_key
            .as_ref()
            .map(|key| self.resolve_cover_texture(ui.ctx(), key, COVER_HERO));
        self.now_playing_cover_key = cover_key;
        // The title and meta lines ride the bar's once-per-frame handout;
        // only the details line is stage-only, resolved here from the
        // playback projection (staleness is the projection's job, not the
        // widget layer's).
        let details = self
            .views
            .playback_current()
            .and_then(|track| crate::ui::now_playing::metadata_details(&track.metadata))
            .map(Arc::from);
        let up_next: Arc<[UpNextEntry]> = crate::ui::sidebar::up_next_entries(
            self.views.playback_up_next(),
            crate::ui::now_playing::UP_NEXT_LIMIT,
        )
        .into();

        let content = crate::ui::now_playing::NowPlayingContent {
            cover,
            title,
            meta_line,
            details,
            position: playback.current_position.current,
            total: playback.current_position.total,
            up_next,
        };

        self.now_playing_actions.clear();
        crate::ui::now_playing::show_now_playing(
            ui,
            &mut self.icons,
            &palette,
            &content,
            &mut self.stage_readouts,
            &mut self.now_playing_actions,
        );
        // Reported, not applied: `Frame::apply_stage` is the only applier, at
        // the stage's slot in the frame's order.
        report.actions.append(&mut self.now_playing_actions);
    }

    fn render_folder_tree(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        query: &str,
    ) {
        if library.library_paths.paths().is_empty() {
            crate::ui::browser::empty_state(
                ui,
                &self.theme.active,
                "No folders yet",
                "Add a folder from the sidebar to start scanning your library.",
            );
            return;
        }

        // Folder views read through the Session Views seam over store
        // queries (ADR 0002/0003): escaped prefix matching over stored track
        // paths, cached until the next committed mutation bumps the
        // generation. No in-memory mirror involved.
        let lib_paths = library.library_paths.paths().to_vec();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for lib_path in &lib_paths {
                if !self.views.folder_has_audio(lib_path) {
                    continue;
                }
                self.render_folder_node(ui, library, playback, lib_path, 0, query);
            }
        });
    }

    /// One folder node of the Folders tree (Issue 07): a restyled 40px
    /// collapsible row on the indent scale whose click toggles AND selects —
    /// the exact gesture set of the former `CollapsingHeader` header.
    #[allow(clippy::too_many_arguments)]
    fn render_folder_node(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        path: &std::path::Path,
        level: usize,
        query: &str,
    ) {
        use crate::ui::icons::Icon;
        use crate::ui::sidebar::{self, TreeRow};
        use egui::collapsing_header::CollapsingState;

        if !self.views.folder_has_audio(path) {
            return;
        }

        if !query.is_empty() && !self.views.folder_search_match(path, query) {
            return;
        }

        let palette = self.theme.active;
        let current_track = playback.queue.current_track().cloned();

        // The playing track's id IS its stored path, so containment is a
        // plain component-wise prefix check — no store round-trip needed.
        let contains_current = current_track
            .as_ref()
            .is_some_and(|tid| std::path::Path::new(&tid.0).starts_with(path));

        let is_selected = library.selected_folder.as_deref() == Some(path);

        let label = path.file_name().map_or_else(
            || path.to_string_lossy().to_string(),
            |n| n.to_string_lossy().to_string(),
        );

        let folder_track_ids = self.views.folder_subtree_ids(path);

        // Collapse state persists per path in egui memory, exactly like the
        // former CollapsingHeader; roots open when they contain the playing
        // track or the selection (pre-restyle behavior).
        let id = egui::Id::new(("riff_sidebar_folder", path.as_os_str()));
        let mut collapsing =
            CollapsingState::load_with_default_open(ui.ctx(), id, contains_current || is_selected);
        let glyph = if collapsing.is_open() {
            Icon::FolderOpen
        } else {
            Icon::Folder
        };

        // The folder's own cover, if it has one, fills the row's leading art
        // slot and the glyph goes away — art replaces it rather than sitting
        // beside it, so an uncovered row keeps today's geometry exactly.
        let cover = folder_cover_texture(
            &mut self.cover_cache,
            self.covers.as_ref(),
            &self.cover_textures,
            path,
            COVER_THUMB,
        );

        let row = sidebar::tree_row(
            ui,
            &mut self.icons,
            &palette,
            TreeRow {
                indent_level: level,
                icon: cover.is_none().then_some(glyph),
                cover,
                label: &label,
                count: None,
                meta: None,
                favorite: None,
                selected: is_selected,
                now_playing: false,
                playing: false,
                art_slot: true,
            },
        );

        // Same gestures as before the restyle: single click toggles + selects,
        // double click plays the subtree, and the whole-list context menu
        // rides on the row.
        if row.response.clicked() {
            collapsing.toggle(ui);
            library.selected_folder = Some(path.to_path_buf());
        }
        if row.response.double_clicked() {
            play_folder(&folder_track_ids, self.transport.as_ref());
        }
        if !folder_track_ids.is_empty() {
            show_list_context_menu(
                &row.response,
                &self.theme.active,
                self.transport.as_ref(),
                playback,
                &folder_track_ids,
            );
        }
        collapsing.store(ui.ctx());

        collapsing.show_body_unindented(ui, |ui| {
            let children = self.views.folder_children(path);
            for child_path in children.iter() {
                self.render_folder_node(ui, library, playback, child_path, level + 1, query);
            }

            let direct = self.views.folder_direct_tracks(path);
            let tracks = folder_tracks_filtered(&direct, query);

            for track in tracks.iter().copied() {
                let label = label_numbered(track);
                self.interactive_track_row(
                    ui,
                    TrackRowSpec {
                        track,
                        label: &label,
                        indent_level: level + 1,
                    },
                    TrackRowContext {
                        library,
                        playback,
                        current_track: current_track.as_ref(),
                        reorder: None,
                    },
                );
            }
        });

        // The tree drives its own unfold (issue 03). egui's animation manager
        // advances a tween's value whenever it is asked and never schedules the
        // pass that would ask again, so a riff-driven tween that does not say
        // "there is another frame" itself renders as the single frame that
        // started it — a click that looks like a dropped frame. Every surface
        // riff tweens therefore carries its own "ask while not settled"
        // condition, and this is the Folders tree's.
        //
        // The clock is the collapsing state's own openness, not a timer: it is
        // the value the tween is actually moving, so the request stops on the
        // frame the body lands rather than one duration after it started, and
        // retuning the published duration cannot desynchronise the two.
        //
        // "Not settled" is the *open* interval, not `openness < 1.0`. A closed
        // body rests at exactly `0.0`, so the one-sided reading is true for
        // every collapsed folder forever: the sidebar would ask the frame loop
        // for a frame on every pass with the tree shut, trading a missing
        // frame during a tween for a permanently uncapped loop. The bound at
        // zero is what keeps a settled surface — open or shut — silent.
        //
        // The accessor reports `1.0` outright under
        // `Memory::everything_is_visible`, a debug-only flag riff never sets,
        // so a flag-set test pays nothing for this and production cannot take
        // the early out.
        //
        // The unfold duration is not retuned here. It was always going to be
        // what the motion tokens publish; the missing frame request was the
        // whole defect, and a shorter duration would have hidden it rather
        // than fixed it.
        let openness = collapsing.openness(ui.ctx());
        if openness > 0.0 && openness < 1.0 {
            ui.ctx().request_repaint();
        }
    }
}

/// Where the two halves of the Cover Cache meet for a Track row: the cache
/// decides whether this `(identity, box)` needs a request, and the View half
/// answers with the texture to paint — the real Cover, or the shared
/// placeholder tile on a full miss. The one place a row needs both, so a render
/// site asks for a cover texture rather than assembling the decision out of
/// fields it has no business knowing.
///
/// A free function so the browser columns' per-row closures can reach it: those
/// closures already hold a mutable borrow of the session Views, so `&mut self`
/// is not available to them, and the handles they were re-splatting — the
/// marker set, the marker set's LRU order — are now inside the cache.
#[allow(clippy::too_many_arguments)]
pub fn cover_texture_for<S: std::hash::BuildHasher>(
    cache: &mut crate::ui::cover_cache::CoverCache,
    covers: &dyn Covers,
    textures: &mut std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    lru_keys: &mut Vec<CoverCacheKey>,
    ctx: &egui::Context,
    palette: &Palette,
    track_id: &TrackId,
    size: RequestedSize,
) -> egui::TextureHandle {
    cache.want_track(covers, track_id.clone(), PathBuf::from(&track_id.0), size);
    crate::ui::artwork::lookup_cover_texture(
        cache,
        textures,
        lru_keys,
        ctx,
        palette,
        &track_id.0,
        size,
    )
}

/// The Folders tree's half of the same responsibility (ADR 0006): a folder row
/// wants the cover art of the directory it *is*, and gets it in place of the
/// folder glyph. The Cover Cache answers whether that art has arrived; the View
/// half holds the texture for it. A miss — first ask, answer outstanding, or a
/// directory with no cover of its own — is `None`, which is what keeps the row's
/// glyph for this frame, and it does **not** fall back to the generated
/// music-note placeholder the artless track rows get: a folder with no cover of
/// its own is an ordinary folder, not an artless album.
///
/// Which is why this is not [`cover_texture_for`]: a folder row has to be told
/// art exists before it decides between the two, while a Track row paints the
/// placeholder until real pixels land and asks for neither.
pub fn folder_cover_texture<S: std::hash::BuildHasher>(
    cache: &mut crate::ui::cover_cache::CoverCache,
    covers: &dyn Covers,
    textures: &std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    folder: &Path,
    size: RequestedSize,
) -> Option<egui::TextureId> {
    let key = cache.want_folder(covers, folder, size)?;
    textures.get(&key).map(egui::TextureHandle::id)
}

/// Ask the Cover worker to delete every cached Thumbnail, and record that one is
/// outstanding. A press while a clear is still running is ignored rather than
/// queued: the wipe is idempotent, and a second one only delays the answer the
/// first already promised. Returns whether the request went out.
pub fn request_cache_clear(covers: &dyn Covers, in_flight: &mut bool) -> bool {
    if *in_flight {
        return false;
    }
    covers.clear_cache();
    *in_flight = true;
    true
}

/// Flush the View half's texture map for a Thumbnail-cache clear the worker has
/// settled, as [`crate::ui::frame::FrameOutput::cache_clear`] reports it.
///
/// **Why this is the draw half's and not the Frame's:** the map holds
/// `egui::TextureHandle`s, and `FrameOutput` names no egui type — that
/// discipline is the whole point of the frame's interface. The Frame polls the
/// service, clears the in-flight flag, forgets the Cover Cache's arrivals and
/// writes the status line, all of it egui-free; the flush is the one act left,
/// and it happens at the Frame's slot either way.
///
/// The flush happens on SETTLED rather than when the button was pressed, and
/// that ordering matters: at confirm time every visible row would re-request
/// while the wipe was still queued behind those very requests, rebuilding the
/// entries the user had just asked to delete. Settled is the moment the disk is
/// genuinely empty, so it is the moment the screen can be emptied with it. A
/// `Failed` clear leaves every texture in place — nothing was removed, so
/// nothing has to be re-derived.
pub fn flush_cleared_cache<S: std::hash::BuildHasher>(
    outcome: &ClearCacheOutcome,
    textures: &mut std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    lru_keys: &mut Vec<CoverCacheKey>,
) {
    if *outcome == ClearCacheOutcome::Cleared {
        crate::ui::artwork::evict_all_covers(textures, lru_keys);
    }
}

/// Play a folder: start its first track and queue the rest as ONE batch
/// command (allocation plan 4.3), so the queue mutates once under one lock
/// instead of N times. The ids arrive in the store's path order — exactly
/// what the former mirror listing produced. The play/append split maps onto
/// the Transport port's [`crate::app::transport::Transport::play_many`].
fn play_folder(track_ids: &[TrackId], transport: &dyn Transport) {
    let Some(first) = track_ids.first() else {
        return;
    };
    transport.play_many(first.clone(), track_ids[1..].to_vec());
}

/// Tracks directly in a folder, optionally filtered by search query. The
/// listing arrives borrowed from the folder projection's Arc-shared cache;
/// the filter matches against each track's PRECOMPUTED lowercase search
/// text — the same value the store keeps in its `search_text` column, so
/// the per-frame `format!` + `to_lowercase` work is gone (allocation plan
/// 4.2). Collects lightweight references and hoists the lowercased query
/// out of the per-item work.
fn folder_tracks_filtered<'a>(tracks: &'a [Track], query: &str) -> Vec<&'a Track> {
    if query.is_empty() {
        tracks.iter().collect()
    } else {
        let q = query.to_lowercase();
        tracks
            .iter()
            .filter(|t| t.search_text.contains(&q))
            .collect()
    }
}

/// A header row that a right-click can reach.
///
/// [`show_list_context_menu`] hangs its popup off an `egui::Response`, and
/// `Response::context_menu` opens only when that response SENSES a secondary
/// click. A `ui.horizontal` layout's response is `Sense::hover()` — it is a
/// rectangle, not a control — so a menu attached to one can never open, however
/// many items it offers. That is invisible until something right-clicks the
/// header, and then the menu is simply absent.
///
/// Registering the header's own rect as a click-sensing widget under a name
/// unique to this position is what makes it a control for this one purpose.
/// `Response::interact` is NOT enough: it keeps the layout's shared background
/// id, which every other layout also uses, so the interaction is lost. Nothing
/// else reads this response — a primary click on the header still selects
/// nothing, opens nothing, and navigates nowhere, which is the whole point of a
/// header being a pure peek (see [`show_list_context_menu`]).
fn header_response(ui: &egui::Ui, header: &egui::InnerResponse<()>) -> egui::Response {
    ui.interact(
        header.response.rect,
        ui.id().with("whole_list_context_menu"),
        egui::Sense::click(),
    )
}

/// Shared whole-list context menu (playlist/folder headers). Like the track
/// menu it reports intents and lets the host act on them afterwards. The
/// playback session travels with the transport because one of the four items —
/// Shuffle — is the one that is not a transport command at all: it completes its
/// intent in the queue, engaging shuffle there and then playing the batch
/// through this menu's own [`apply_list_menu_intent`] arm. It is the playback
/// session and never the library session, so the two locks are never held
/// together.
///
/// Nothing here SELECTS, OPENS, or NAVIGATES. A header has no readout for a
/// selection to stay consistent with, so right-clicking one is a pure peek: the
/// four items act on the list and the view is left exactly as it was. The
/// entity rows in the Artists, Albums, and Genres Columns, and the Track rows
/// wherever a Track is listed, are the deliberate opposite — they DO select,
/// which is why they attach a different renderer and a different applier.
fn show_list_context_menu(
    response: &egui::Response,
    palette: &Palette,
    transport: &dyn Transport,
    playback: &mut PlaybackSession,
    track_ids: &[TrackId],
) {
    let mut intents = Vec::new();
    response.context_menu(|ui| {
        crate::ui::menu::list_menu(ui, palette, &mut intents);
    });
    for intent in intents {
        apply_list_menu_intent(intent, track_ids, transport, playback);
    }
}

/// The host slots one collection menu's effects answer to — assembled by the
/// attach site and applied by [`apply_collection_menu`], the counterpart of
/// [`TrackMenuHost`] for the Track menu. Nothing in here is reachable while
/// the menu is being painted: the renderer only reports a choice, or the fact
/// that the menu was opened at all.
pub struct CollectionMenuEffects<'a> {
    /// The library session: opening an entity row's menu selects it, so the
    /// menu just opened and the Detail Panel describe the same entity.
    pub library: &'a mut LibrarySession,
    /// The playback session: the Shuffle item's flag lives on its queue, and
    /// the batch intents go to the engine from there.
    pub playback: &'a mut PlaybackSession,
    /// The playback command port the batch intents go through.
    pub transport: &'a dyn Transport,
    /// The Session Views seam, which resolves the row's Track batch at
    /// dispatch time.
    pub views: &'a mut SessionViews,
}

/// Apply one entity row's context-menu report: the selection, then whatever the
/// listener chose.
///
/// `column` is the identity of the Column the row was clicked in — its Section,
/// its depth, its Scroll Memory slot — which the Column stated once and this
/// function reads. A Section root and a Drill Column therefore cannot disagree
/// with a plain click on the same row: it is the same identity, reached through
/// the same two calls.
///
/// The selection is applied FIRST and unconditionally, before the intents are
/// looked at: OPENING the menu is what moves it, so a right-click that chooses
/// nothing still leaves the readout agreeing with the menu on screen. That is
/// also why the selection is an argument here rather than something read back
/// off the click — this whole function is reachable, and provable, without a
/// pointer.
///
/// Each intent then goes through [`apply_list_menu_intent`], the same applier a
/// playlist header, a smart playlist header, and a folder node use, so the four
/// items mean one thing on all six set-of-Tracks surfaces and their effect logic
/// is written once rather than twice.
pub fn apply_collection_menu(
    key: &str,
    intents: &[crate::ui::menu::ListMenuIntent],
    column: ColumnIdentity,
    effects: CollectionMenuEffects<'_>,
) {
    apply_entity_selection(key, column, effects.library);
    for intent in intents {
        let batch = entity_track_ids(key, column, effects.views);
        apply_list_menu_intent(*intent, &batch, effects.transport, effects.playback);
    }
}

/// Apply one emitted whole-list intent, preserving the list's current shape:
/// **Play** starts the first Track and queues the rest behind it as one batch,
/// **Play Next** inserts the whole list in order, **Add to Queue** adds it at
/// the end, and **Shuffle** engages shuffle and then plays the same batch.
/// `playback` is the session the shuffle flag lives on, borrowed only by that
/// one arm.
pub fn apply_list_menu_intent(
    intent: crate::ui::menu::ListMenuIntent,
    track_ids: &[TrackId],
    transport: &dyn Transport,
    playback: &mut PlaybackSession,
) {
    use crate::ui::menu::ListMenuIntent;
    match intent {
        // The collection's Play is the SAME helper the album header's and the
        // Detail Panel's Play use, so a Track menu and a collection menu cannot
        // drift apart: one `play_many` batch, never a command per Track.
        ListMenuIntent::Play => play_album_batch(track_ids, transport),
        ListMenuIntent::PlayNext => {
            for tid in track_ids.iter().rev() {
                transport.play_next(tid.clone());
            }
        }
        // A whole collection queued is ONE `AddMany`, not a per-Track fan-out:
        // the queue mutates once under one lock with one shuffle regeneration.
        // Nothing is played — the current Track carries on — which is the one
        // thing that keeps this apart from `play_many` above.
        ListMenuIntent::AddToQueue => transport.add_many(track_ids.to_vec()),
        // Shuffle is a one-shot that STARTS the collection: engage shuffle,
        // then play the same batch Play would. Shuffle stays on afterwards —
        // the album header's long-standing behaviour, preserved rather than
        // quietly changed. An empty list starts nothing and never so much as
        // turns shuffle on, exactly as the album header's Shuffle does.
        ListMenuIntent::Shuffle => {
            if track_ids.is_empty() {
                return;
            }
            playback.queue.set_shuffle(true);
            play_album_batch(track_ids, transport);
        }
    }
}

/// The shared `mm:ss` time-readout format now lives with the playerbar
/// widgets that render it (Issue 08); re-exported here so existing callers
/// and the test prelude keep their stable path.
pub use crate::ui::playerbar::format_duration;
