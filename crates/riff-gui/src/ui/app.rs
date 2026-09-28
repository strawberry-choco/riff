mod browser_pane;
mod library_picker;
mod selection_pane;
mod tag_editor;

pub use library_picker::register_library_path;
pub use tag_editor::InlineTagEditor;

use crate::ui::chrome::TitleBarAction;
use crate::ui::now_playing::{NowPlayingAction, UpNextEntry};
use crate::ui::playerbar::PlayerBarAction;
use crate::ui::settings::SettingsSection;
use crate::ui::theme::{self, Palette};
// Ungated, unlike the visibility *channel* fields: `CUSTOM_TITLEBAR_CLOSE` is
// compiled on every platform (it is the pure data `close_resolution` hands
// back, which is deliberately ungated so the close decision stays assertable
// from the Linux/Windows CI machines), so the type it is declared with must be
// in scope on Linux too. Gating this import with the channel would leave the
// const unnameable there.
use crate::ui::window_visibility::VisibilityMessage;
use eframe::egui;
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
use riff_backend::app::scan_service::{ScanOutcome, Scans};
use riff_backend::app::state::{
    BrowseMode, BrowserSelection, LibrarySection, LibrarySession, LibraryStatus, PlaybackSession,
    ViewMode,
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
pub(crate) struct ThemeState {
    /// `true` = dark (mockup palette), `false` = light (derived per ADR 0004).
    dark: bool,
    /// The resolved palette currently installed on the context (Issue 03):
    /// view code reads its semantic slots instead of hardcoding colors, so
    /// every themed surface follows the active palette (ADR 0004).
    pub(crate) active: theme::Palette,
    /// The `(dark, high_contrast)` pair currently installed on the context,
    /// or `None` before the first install.
    last_applied: Option<(bool, bool)>,
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
    cover_lru_keys: Vec<CoverCacheKey>,
    /// Cover requests this frame has sent and not yet been answered, keyed exactly
    /// as the texture map is. Without it every repaint of a row whose art has not
    /// landed re-enqueues a request onto an unbounded channel — invisible when the
    /// answer comes back in one heartbeat, and a real allocation plus a `PathBuf`
    /// and `String` clone per frame per row when it does not.
    cover_in_flight: std::collections::HashSet<CoverCacheKey>,
    /// The LRU order of [`Self::cover_in_flight`], kept beside it the same way
    /// `cover_lru_keys` tracks `cover_textures`.
    cover_in_flight_keys: Vec<CoverCacheKey>,
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
    first_frame: bool,
    pub(crate) watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
    /// The Application Store's settings section. `Preferences` reads it on
    /// the first frame and diff-commits the sessions back at frame end, so
    /// preferences survive restarts through the store.
    pub(crate) settings_store: Box<dyn SettingsStore>,
    /// The Settings round-trip owner: hydrates the stored Settings into the
    /// sessions on launch and diff-commits session changes back at frame
    /// end, so a preference change is durable by construction.
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        playback: Arc<Mutex<PlaybackSession>>,
        library: Arc<Mutex<LibrarySession>>,
        transport: Box<dyn Transport>,
        scans: Box<dyn Scans>,
        watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
        #[cfg(not(target_os = "linux"))] tray_icon: Option<tray_icon::TrayIcon>,
        settings_store: Box<dyn SettingsStore>,
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
            cover_in_flight: std::collections::HashSet::new(),
            cover_in_flight_keys: Vec::new(),
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
            first_frame: true,
            watcher_manager,
            settings_store,
            prefs: Preferences::default(),
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

        let app = Self::new(
            playback,
            library,
            transport,
            scans,
            Arc::new(Mutex::new(None)),
            #[cfg(not(target_os = "linux"))]
            None,
            settings_store,
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

    /// Load the persisted preferences from the first frame's session
    /// snapshots, exactly once. Runs before anything reads [`Self::prefs`] —
    /// and before the theme is applied — so a persisted choice (High Contrast
    /// in particular) takes effect on the very first frame instead of the
    /// second. Guarded by [`Self::first_frame`], which it clears.
    fn hydrate_prefs_once(
        &mut self,
        playback: &Arc<Mutex<PlaybackSession>>,
        library: &Arc<Mutex<LibrarySession>>,
    ) {
        self.prefs = Preferences::hydrate(
            playback,
            library,
            self.settings_store.as_ref(),
            self.transport.as_ref(),
        );
        self.first_frame = false;
    }

    /// Apply the active theme to the context (REQ-UI-007, Issue 01). The
    /// palette is resolved from the token module — dark (mockup) or light
    /// (derived per ADR 0004), with High Contrast as a token-set variant over
    /// the base — and installed globally. Installation happens once at init
    /// and again only when the selection changes, not every frame. The
    /// resolved palette is kept on [`ThemeState`] so view code can style
    /// itself from the active tokens (Issue 03).
    fn apply_theme(&mut self, ctx: &egui::Context, high_contrast: bool) {
        let dark = self.theme.dark;
        if self.theme.last_applied == Some((dark, high_contrast)) {
            return;
        }

        let palette = theme::resolve(dark, high_contrast);
        theme::install(ctx, &palette);
        // A palette-family flip invalidates the placeholder tile: its well
        // and glyph colours were derived for the old family's tokens, so it
        // re-renders under the new one on its next lookup.
        if self.theme.active.dark != palette.dark {
            crate::ui::artwork::evict_generated(&mut self.cover_textures, &mut self.cover_lru_keys);
        }
        self.theme.active = palette;
        self.theme.last_applied = Some((dark, high_contrast));
    }

    /// Send cover intent for one track to the Cover Service. The only
    /// UI-side check left is the texture cache (the texture LRU is
    /// UI-owned per the texture boundary); request deduplication and the
    /// negative cache live behind the service seam.
    fn request_cover(&mut self, track_id: &TrackId, file_path: &Path, size: RequestedSize) {
        request_cover_intent(
            &self.cover_textures,
            &mut self.cover_in_flight,
            &mut self.cover_in_flight_keys,
            self.covers.as_ref(),
            track_id.clone(),
            file_path.to_path_buf(),
            size,
        );
    }

    /// Drain polled Library Scan outcomes from the service and report each
    /// root's Readiness through the [`LibraryPaths`] slot, exactly as before
    /// the extraction — the scan worker writes *through* the module instead of
    /// reaching into the session — plus the titlebar scan-status line. The
    /// service NEVER touches `LibrarySession` (ADR 0006). The watcher observes
    /// a scan's end itself via `is_scanning`, so no relay fires here anymore.
    fn poll_library_updates(&mut self, library: &mut LibrarySession) {
        use riff_backend::app::events::NoticeSeverity;
        for outcome in self.scans.poll() {
            match outcome {
                ScanOutcome::Progress { path, files_found } => {
                    library
                        .library_paths
                        .report_readiness(&path, LibraryStatus::Scanning { files_found });
                    self.feedback
                        .set_scan(format!("{files_found} files"), NoticeSeverity::Info);
                }
                ScanOutcome::Complete { path, total_files } => {
                    library
                        .library_paths
                        .report_readiness(&path, LibraryStatus::Scanned(total_files));
                    self.feedback.set_scan(
                        format!("Scan complete: {total_files} tracks"),
                        NoticeSeverity::Info,
                    );
                    // Scan batches already committed through the store as
                    // they progressed; nothing whole-file remains to save.
                }
                ScanOutcome::Failed { path, reason } => {
                    library
                        .library_paths
                        .report_readiness(&path, LibraryStatus::Idle);
                    // A failed scan carries an Error severity and a Rescan
                    // recovery intent through to the paint boundary.
                    self.feedback.put(crate::ui::feedback::Feedback {
                        severity: NoticeSeverity::Error,
                        source: riff_backend::app::events::NoticeSource::Scan,
                        message: format!("Error: {reason}"),
                        recovery: Some(crate::ui::feedback::Recovery::Rescan),
                    });
                }
            }
        }
    }

    fn poll_watchers(&self) {
        if let Some(ref mut mgr) = *self.watcher_manager.lock_or_recover() {
            mgr.poll();
        }
    }

    /// Drop the shared placeholder tile from the texture cache. For
    /// sibling modules (`ui::settings`): the artwork-policy toggle uses it
    /// so tracks resolved as artless under the old policy re-resolve.
    pub(crate) fn evict_generated_covers(&mut self) {
        crate::ui::artwork::evict_generated(&mut self.cover_textures, &mut self.cover_lru_keys);
        // The markers go with them. A row that asked under the old policy has an
        // outstanding request whose answer is about to be wrong for the new one, and
        // leaving it marked would suppress the re-ask that the eviction above exists
        // to cause.
        self.cover_in_flight.clear();
        self.cover_in_flight_keys.clear();
    }

    /// Begin a Thumbnail-cache clear for `ui::settings`, the sibling of
    /// [`Self::evict_generated_covers`] in the same pane. The wipe runs on the cover
    /// worker and [`Self::poll_cache_clear_outcome`] reports it once it settles, so
    /// the frame only ever says "clearing" and never waits.
    pub(crate) fn request_thumbnail_cache_clear(&mut self) {
        if request_cache_clear(self.covers.as_ref(), &mut self.clear_cache_in_flight) {
            self.feedback.set_library(
                "Clearing the thumbnail cache\u{2026}".to_string(),
                riff_backend::app::events::NoticeSeverity::Info,
            );
        }
    }

    /// Drain the settled outcome of a Thumbnail-cache clear and report it on the
    /// status line the rest of the Library pane already uses. A cache that cannot
    /// be cleared is an inconvenience, not a data-loss event, so this is one line
    /// in the feedback board — never a modal.
    /// Drain every background service's outstanding results into the feedback
    /// board, in the order the status line is later composed from it. The three
    /// drains are one step because their sequence relative to
    /// `feedback.display_message()` is load-bearing: a slot filled after the
    /// compose is a frame late.
    fn drain_background_outcomes(&mut self, library: &mut LibrarySession) {
        self.poll_library_updates(library);
        self.tag_editor.poll_outcomes(&mut self.feedback);
        self.poll_cache_clear_outcome();
    }

    fn poll_cache_clear_outcome(&mut self) {
        let Some(outcome) = settle_cache_clear(
            self.covers.as_ref(),
            &mut self.clear_cache_in_flight,
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
        ) else {
            return;
        };
        match outcome {
            ClearCacheOutcome::Cleared => {
                self.feedback.set_library(
                    "Thumbnail cache cleared. Covers rebuild as you browse.".to_string(),
                    riff_backend::app::events::NoticeSeverity::Info,
                );
            }
            ClearCacheOutcome::Failed { reason } => {
                tracing::warn!("Failed to clear the Thumbnail cache: {reason}");
                self.feedback.set_library(
                    "Failed to clear the Thumbnail cache \u{2014} nothing was changed.".to_string(),
                    riff_backend::app::events::NoticeSeverity::Error,
                );
            }
        }
    }

    /// Consume polled cover results into the UI texture cache: rgba→texture
    /// conversion is the egui-bound work that stays on the main thread;
    /// every other caching concern lives in the service.
    fn update_cover_cache(&mut self, ctx: &egui::Context) {
        cache_polled_covers(
            self.covers.as_ref(),
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            &mut self.cover_in_flight,
            &mut self.cover_in_flight_keys,
            ctx,
        );
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
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ctx,
            &palette,
            identity,
            size,
        )
    }

    /// Attach the shared track context menu to `response`. See
    /// [`show_track_context_menu`] for the available actions.
    ///
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
    /// The menu's props are read off the row's own `track` here, which is what
    /// makes one answer per row: the Favourite item's label flips on
    /// `track.favorite`, and a listing may hold a mix of Favourited and
    /// un-Favourited rows. The Favourite item's own write is the SAME durable
    /// setter the heart's is — `library_mutations.set_track_favorite` — so the
    /// two are one change with two doors.
    fn attach_track_menu(
        &mut self,
        response: &egui::Response,
        library: &mut LibrarySession,
        track_id: &TrackId,
        track: Option<&Track>,
        remove_from_playlist: Option<&PlaylistId>,
    ) {
        // Arc clone out of the seam first: no `&self.views` borrow may live
        // across widget rendering.
        let playlists = self.views.playlists();
        let options: Vec<(PlaylistId, String)> = playlists
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        let palette = self.theme.active;
        let open = {
            let mut effects = TrackMenuEffects {
                track_id,
                track,
                selected_track: &mut library.selected_track,
                tag_editor: &mut self.tag_editor,
                transport: self.transport.as_ref(),
                playlist_store: self.playlist_store.as_mut(),
                library_mutations: self.library_mutations.as_mut(),
                remove_from_playlist,
            };
            show_track_context_menu(response, &palette, &options, &mut effects)
        };
        // The selection is the OPENING's effect, applied through its own
        // pointer-free applier and not through the item path above: a
        // right-click that opens the menu and is then dismissed chose nothing
        // and has still selected. The effects bag is released first because it
        // holds the very slot this writes.
        apply_track_menu_open(
            track_id,
            if open {
                TrackMenuOpen::Opened
            } else {
                TrackMenuOpen::NotOpened
            },
            &mut library.selected_track,
        );
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
    fn render_track_row(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
        track: &Track,
        current_track: Option<&TrackId>,
        remove_from_playlist: Option<&PlaylistId>,
    ) {
        self.interactive_track_row(
            ui,
            library,
            playback,
            track,
            current_track,
            remove_from_playlist,
            &label_artist_title(track),
            0,
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
    #[allow(clippy::too_many_arguments)]
    fn interactive_track_row(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
        track: &Track,
        current_track: Option<&TrackId>,
        remove_from_playlist: Option<&PlaylistId>,
        label: &str,
        indent_level: usize,
    ) {
        use crate::ui::sidebar::{self, TreeRow};
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
            library,
            &track.id,
            Some(track),
            remove_from_playlist,
        );
    }

    /// Apply the frame's titlebar actions. Close is resolved here rather than
    /// in [`apply_titlebar_action`], because this method owns `self`: on
    /// macOS/Windows the custom X follows the persisted "Quit on close"
    /// preference — by default it hides through the frontend-local
    /// [`VisibilityMessage(false)`] visibility channel (applied by `logic()`
    /// one frame later), and only when the preference is on does it send a
    /// real `Close`. On Linux there is no tray, so the X always really closes.
    ///
    /// This is NOT where a macOS close is resolved, and it must not become
    /// one: the native branch's window controls are the system's traffic
    /// lights, and `chrome.rs` never calls `draw_caption_controls` on
    /// `ChromeMode::NativeTrafficLights` (the caption code is `cfg`'d out
    /// there), so `TitleBarAction::Close` is unreachable on macOS — the red
    /// button's close is resolved by [`close_resolution`] in `ui()`'s native
    /// block instead. Unifying these two paths would reintroduce the
    /// tray-Quit cancellation bug, because this one resolves a `Close` through
    /// the preference alone and cannot see the quit intent.
    fn apply_titlebar_actions(&mut self, ctx: &egui::Context, library: &mut LibrarySession) {
        for action in self.titlebar_actions.drain(..) {
            if action == TitleBarAction::Close {
                #[cfg(not(target_os = "linux"))]
                {
                    if library.ui_flags.close_quits_app {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        let _ = self.visibility_tx.send(CUSTOM_TITLEBAR_CLOSE);
                    }
                }
                #[cfg(target_os = "linux")]
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                apply_titlebar_action(action, ctx, library, &mut self.theme);
            }
        }
    }

    /// Resolve the macOS native close request (the red traffic light) for the
    /// frame `ui()` is building, and act on it.
    ///
    /// Called as the first thing `ui()` does, because eframe reads
    /// `close_requested()` at the top of the frame and quits unless THAT
    /// frame's viewport output carries [`egui::ViewportCommand::CancelClose`]
    /// — so the decision cannot be deferred, and nothing else in the frame may
    /// get to queue a competing close first.
    ///
    /// The event carries no provenance (see [`CloseIntent`]): the tray's Quit
    /// enqueues the same `Close` the red light produces. The quit flag is the
    /// one fact that separates them, and the tray stores it before enqueueing —
    /// see the ordering note in `tray.rs`. The load is `Acquire` to pair with
    /// that `Release` store; the correctness argument rests on that pair, not
    /// on any third-party crate's internal mutex.
    ///
    /// A no-op when eframe reported no close request, and on Windows/Linux
    /// (not compiled) — their frameless OS close always quits.
    #[cfg(target_os = "macos")]
    fn resolve_native_close(&mut self, ui: &egui::Ui, library: &LibrarySession) {
        if !ui.input(|i| i.viewport().close_requested()) {
            return;
        }
        let intent = if self.quit_flag.load(std::sync::atomic::Ordering::Acquire) {
            CloseIntent::Quit
        } else {
            CloseIntent::WindowClose
        };
        if let Some((cancel, hide)) = close_resolution(intent, library.ui_flags.close_quits_app) {
            ui.ctx().send_viewport_cmd(cancel);
            let _ = self.visibility_tx.send(hide);
        }
    }

    /// Drain every pending [`BackendEvents::events`] for this frame.
    ///
    /// Called at the start of the frame so any dispatch recorded by the tray
    /// thread or by a transport between frames is observable before the UI
    /// renders. The frontend renders from the real engine updates on the
    /// playback session; this seam's events are the observability surface
    /// that proves every dispatch path (mouse/keyboard/tray) flows through
    /// one recorded Transport.
    pub fn drain_backend_events(&self) -> Vec<riff_backend::app::events::BackendEvent> {
        use std::sync::PoisonError;
        self.backend_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events()
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
    /// "Quit on close" preference, in `ui()`'s native block, so that a window
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

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Clone both Arcs BEFORE locking: the guards borrow `self`, and the
        // whole frame below calls `self.<method>(...)` — exactly how the
        // pre-split frame loop handled `self.state`.
        let playback_arc = self.playback.clone();
        let library_arc = self.library.clone();

        if self.first_frame {
            self.hydrate_prefs_once(&playback_arc, &library_arc);
        }

        // Snapshot playback first (lock → clone → drop), then take the
        // library guard. The engine and coordinator write playback state on
        // their own threads, so the frame renders from a plain clone and
        // writes back only the UI-owned fields at frame end — a whole
        // session replace here would clobber engine-written position and
        // traversal index. The library session is UI-owned, so its guard is
        // held live for the whole frame.
        let mut playback = playback_arc.lock_or_recover().clone();
        let mut library = library_arc.lock_or_recover();

        // Native close request (macOS traffic lights) — resolved first, in the
        // very frame eframe reported it in, because eframe quits unless THAT
        // frame's output carries the cancel. See `resolve_native_close`.
        #[cfg(target_os = "macos")]
        self.resolve_native_close(ui, &library);

        // Apply the active theme (REQ-UI-007 accessibility). Done after the
        // first-frame load so a persisted high-contrast choice takes effect on
        // the very first frame. High Contrast is a variant over the active
        // light/dark palette.
        self.apply_theme(ui.ctx(), library.ui_flags.high_contrast);

        // Drain the backend event inbox: route playback-error typed notices
        // to the status line (issue 01 seam fix) — the coordinator no longer
        // writes the library session's status slot directly — and fold any
        // Library-generation move into the Scroll Memory, so a committed
        // rescan turns every Section slot's fingerprint stale.
        let events = self.drain_backend_events();
        self.scroll_memory.note_backend_events(&events);
        apply_backend_events(events, &mut self.feedback);

        self.drain_background_outcomes(&mut library);
        // Compose the titlebar status line from the independent source slots,
        // so a Library Scan update cannot erase a live playback error or a Tag
        // Edit outcome (issue 11). The composed message feeds the existing
        // `scan_status` line — same placement, same copy.
        library.scan_status = self.feedback.display_message();
        self.update_cover_cache(ui.ctx());
        self.poll_watchers();

        handle_keyboard_shortcuts(
            ui.ctx(),
            &playback,
            &mut self.global_search_focus,
            self.transport.as_ref(),
        );

        // Update window title and tray tooltip (REQ-SI-001). Both are
        // compared against the last-pushed identity first and only rebuilt
        // when the playing track moves; the tooltip shows "Artist - Title"
        // for the current track, else "riff".
        self.update_window_title(ui.ctx(), &playback);

        // --- SHELL (Issue 06): unified Panel API at exact token dimensions ---
        //
        // Top 56px strip: the frameless titlebar (issue 04, ADR 0005)
        // merged with the former top bar — wordmark, scan status, the
        // theme/Now Playing/Settings/Advanced controls, and the custom
        // minimize/close buttons over a full-width drag region.
        let scan_status = library.scan_status.clone();
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
                        crate::ui::chrome::measured_traffic_lights_width(frame),
                        ui.ctx().zoom_factor(),
                    ),
                };
                self.titlebar_actions.clear();
                let search_response = crate::ui::chrome::show_titlebar(
                    ui,
                    &mut self.icons,
                    &self.theme.active,
                    &content,
                    &mut library.search_query,
                    &mut self.titlebar_actions,
                );
                // Ctrl+K landed: focus the titlebar search field this frame.
                if self.global_search_focus {
                    search_response.request_focus();
                    self.global_search_focus = false;
                }
                self.apply_titlebar_actions(ui.ctx(), &mut library);
            });

        // Left 280px column: the library browser (search, Library/Folders
        // nav, playlists). Shared chrome per the mockup — present on every
        // view; only the main stage switches. The restyled content (issue 07)
        // keeps a 12px inset from the panel edge, and the panel carries the
        // surface token itself rather than inheriting a default.
        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(12))
                    .fill(self.theme.active.surface),
            )
            .show(ui, |ui| {
                self.render_library_sidebar(ui, &mut library);
            });

        // Bottom 88px strip: transport + progress + volume.
        self.render_control_bar(ui, &mut library, &mut playback);

        // --- MAIN STAGE: exactly one View visible at a time ---
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.theme.active.background))
            .show(ui, |ui| match library.view_mode {
                ViewMode::Library => {
                    // The elastic column stage (elastic-column spec): the
                    // section-driven column sequence plus the collapsible
                    // inspector replace the fixed three-pane explorer.
                    self.render_elastic_stage(ui, &mut library, &mut playback);
                }
                ViewMode::NowPlaying => self.show_now_playing_view(ui, &mut library, &playback),
                ViewMode::Settings => {
                    self.show_settings_view(ui, &mut library, &mut playback);
                }
            });

        // --- WRITE BACK: the library guard is still live here, so the
        // frame-end Preferences commit sees the frame's playback snapshot
        // (volume, mute, replay-gain, shuffle, repeat) together with the
        // library session's preference fields, and lands any drift in the
        // store — durability by construction, no per-handler call sites.
        // Then the guard is dropped and only the UI-owned playback fields
        // are written back: the engine and coordinator own `playback_state`,
        // `current_position`, and the queue's traversal state, so a
        // whole-session replace here would clobber their work between
        // frames.
        self.prefs
            .commit_if_changed(&playback, &library, self.settings_store.as_mut());
        drop(library);
        {
            let mut live = self.playback.lock_or_recover();
            live.current_volume = playback.current_volume;
            live.muted = playback.muted;
            live.replaygain_enabled = playback.replaygain_enabled;
            live.queue.set_shuffle(playback.queue.shuffle);
            live.queue.repeat = playback.queue.repeat;
        }
        // The end-of-frame tick keeps visible frames responsive (seek
        // readouts, the playing row). A window hidden to the tray schedules no
        // repaints — the tray wakes the loop on demand — so gate it on
        // `window_hidden`: without the gate, egui keeps re-requesting repaints
        // forever on a window it still believes is visible (eframe 0.35 keeps
        // calling `ui` for a hidden window). Linux has no hidden state, so it
        // keeps the unconditional tick.
        #[cfg(not(target_os = "linux"))]
        if !self.window_hidden {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        #[cfg(target_os = "linux")]
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
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
/// enqueues the close and [`RiffApp`] loads in `ui()`'s native close block.
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

/// Apply one [`crate::ui::chrome::TitleBarAction`] to app state and viewport
/// commands (Issue 06). Minimize/maximize apply their viewport commands here;
/// Close is handled by the caller (`ui()`), which owns the visibility channel
/// the custom X's hide travels on (macOS/Windows) or sends the real close
/// (Linux). Preference changes are session writes only — the frame-end
/// `Preferences` commit persists them.
fn apply_titlebar_action(
    action: crate::ui::chrome::TitleBarAction,
    ctx: &egui::Context,
    library: &mut LibrarySession,
    theme: &mut ThemeState,
) {
    use crate::ui::chrome::{NavDestination, TitleBarAction as Action, WindowControl};
    match action {
        Action::ToggleTheme => theme.dark = !theme.dark,
        Action::ToggleNowPlaying => {
            // Now Playing replaces the active view; leaving it returns to the
            // Library view (resolved navigation gap).
            library.view_mode = match library.view_mode {
                ViewMode::Library | ViewMode::Settings => ViewMode::NowPlaying,
                ViewMode::NowPlaying => ViewMode::Library,
            };
        }
        Action::GoSettings => {
            NavDestination::Settings.apply(&mut library.view_mode, &mut library.browse_mode);
        }
        Action::Minimize => ctx.send_viewport_cmd(WindowControl::Minimize.viewport_command()),
        Action::ToggleMaximize => {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        // Close is deliberately not handled here: the caller (`ui()`'s action
        // drain) resolves it first — hide via the visibility channel on
        // macOS/Windows, real close on Linux — so it can never reach this
        // match. The arm exists to keep the match exhaustive.
        Action::Close => {}
    }
}

/// Apply one [`crate::ui::browser::BrowserAction`] (handoff issue 08) to the
/// library session: the sort toggle is a session field; a row selection
/// resolves per section into the [`BrowserSelection`] the stage's root column
/// consumes, landing at level 0 (`select_at(0, …)`) so drill-down restarts from
/// the root. Track rows select through `library.selected_track` (the existing
/// `interactive_track_row` flow), so the All Tracks variant never sets a
/// browser selection. The drill columns (level 1 / 2) go through
/// [`apply_drill_action`].
///
/// A [`BrowserAction::ContextMenu`](crate::ui::browser::BrowserAction::ContextMenu)
/// is answered by [`apply_collection_menu`], which needs the playback session,
/// the transport, and the Session Views seam as well as the library — this
/// function holds only the library, so the section roots route that variant
/// there instead of here. The arm exists because the match must stay
/// exhaustive, and it is never reached from the app.
pub fn apply_browser_action(
    action: crate::ui::browser::BrowserAction,
    library: &mut LibrarySession,
) {
    match action {
        crate::ui::browser::BrowserAction::ToggleSort => {
            library.browser_sort_desc = !library.browser_sort_desc;
        }
        crate::ui::browser::BrowserAction::Select(key) => {
            let section = library.library_section;
            apply_entity_selection(&key, section, 0, library);
        }
        crate::ui::browser::BrowserAction::ContextMenu { .. } => {}
    }
}

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
/// Shared by the section roots' [`apply_browser_action`] and the drill columns'
/// [`apply_drill_action`] so a click and a right-click on the same row are
/// literally the same line of code.
fn apply_entity_selection(
    key: &str,
    section: LibrarySection,
    level: usize,
    library: &mut LibrarySession,
) {
    if let Some(row) = entity_row(key, section, level) {
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
    section: LibrarySection,
    level: usize,
    views: &mut SessionViews,
) -> Vec<TrackId> {
    match entity_row(key, section, level) {
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
pub fn apply_detail_action(
    action: crate::ui::detail::DetailAction,
    library: &mut LibrarySession,
    transport: &dyn Transport,
    library_mutations: &mut dyn LibraryMutationStore,
) {
    use crate::ui::detail::DetailAction as Action;
    // Two arms below are deliberate no-ops for reasons of their own, so the
    // identical bodies are the point rather than a merge waiting to happen.
    #[expect(
        clippy::match_same_arms,
        reason = "each no-op arm carries its own reason for being unreachable here"
    )]
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
        // A Track row's menu report CANNOT be answered here: choosing an item
        // needs the Playlist Store and the Inline Tag Editor, which are not
        // this applier's parameters, and re-using the Track menu's own applier
        // means reaching the same two appliers every other Track row reaches.
        // The one call site that can receive this report — the Tracks column —
        // intercepts it before it gets here, which is why this arm is a
        // documented no-op rather than a second dispatch of its own.
        Action::TrackMenu { .. } => {}
        Action::SetFavorite { key, favorite } => {
            if let Err(e) = library_mutations.set_track_favorite(&TrackId(key.clone()), favorite) {
                tracing::warn!("Failed to commit the favorite flag for {key}: {e}");
            }
        }
    }
}

/// Apply a drill-down row selection from one of the elastic stage's entity
/// columns at `level` (1 or 2 — deeper than the root's level 0): the row
/// key follows the section's identity convention (`(album artist, title)`
/// composites for album rows, bare names for artist rows), and
/// [`LibrarySession::select_at`] truncates any deeper path entries. The
/// root column's selections keep going through [`apply_browser_action`]
/// (`select_at(0, …)`), so drill-down always restarts from the root. Both
/// reach [`apply_entity_selection`], which is the one place the key's
/// meaning is decided.
pub fn apply_drill_action(
    section: LibrarySection,
    level: usize,
    key: String,
    library: &mut LibrarySession,
) {
    apply_entity_selection(&key, section, level, library);
}

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

/// Apply one [`crate::ui::now_playing::NowPlayingAction`] (Issue 10). Close
/// ALWAYS lands on the Library View: Now Playing is a mode that replaces the
/// active View (resolved navigation gaps), so there is no prior view to
/// restore — closing from anywhere returns to the Library. Transport actions
/// pass straight through to the Transport port; seek targets re-clamp
/// against the live track duration exactly like the playerbar's.
pub fn apply_now_playing_action(
    action: crate::ui::now_playing::NowPlayingAction,
    library: &mut LibrarySession,
    playback: &PlaybackSession,
    transport: &dyn Transport,
) {
    use crate::ui::now_playing::NowPlayingAction as Action;
    match action {
        Action::Close => library.view_mode = ViewMode::Library,
        Action::PlayNext(track_id) => transport.play_next(track_id),
        Action::Seek(duration) => transport.seek(playback, duration.as_secs_f32()),
    }
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

/// Apply one restyled player-bar action (Issue 08) through the SAME engine
/// intents and state paths the pre-restyle controls used. Transport actions
/// pass straight through to the Transport port; the port's mutators complete
/// the intent on the session themselves — `set_volume` clamps and stores the
/// slider value, `toggle_mute` flips the flag, `toggle_shuffle`/
/// `toggle_repeat` flip the queue state — and send the engine exactly what
/// it needs, so a muted app never emits sound. Seek targets re-clamp against
/// the live track duration inside the adapter. Preference changes are
/// session writes only — the frame-end `Preferences` commit persists them.
pub fn apply_player_bar_action(
    action: crate::ui::playerbar::PlayerBarAction,
    library: &mut LibrarySession,
    playback: &mut PlaybackSession,
    transport: &dyn Transport,
) {
    use crate::ui::playerbar::PlayerBarAction as Action;
    match action {
        Action::Previous => transport.previous(),
        Action::Pause => transport.pause(),
        Action::Resume => transport.resume(),
        Action::PlaySelected => {
            // Pre-restyle behavior: with nothing selected, play does nothing.
            if let Some(selected) = library.selected_track.clone() {
                transport.play(selected);
            }
        }
        Action::Next => transport.next(),
        Action::Stop => transport.stop(),
        Action::Seek(target) => transport.seek(playback, target.as_secs_f32()),
        Action::SetVolume(volume) => {
            // While muted the slider still edits current_volume, but the
            // engine keeps receiving 0 until unmuted.
            transport.set_volume(playback, volume);
        }
        Action::ToggleMute => {
            // Muting never moves the volume slider — it only zeroes the
            // effective volume sent to the engine; unmuting restores it.
            transport.toggle_mute(playback);
        }
        Action::ToggleShuffle => transport.toggle_shuffle(playback),
        Action::ToggleRepeat => transport.toggle_repeat(playback),
        Action::ToggleQueue => {
            // The queue panel is session state, not persisted (issue 13).
            library.queue_open = !library.queue_open;
        }
        Action::ToggleExpanded => {
            // The enlarged player view IS the Now Playing mode (issue 13):
            // same routing as the titlebar's Now Playing toggle, and purely
            // view state — playback keeps running untouched.
            library.view_mode = match library.view_mode {
                ViewMode::Library | ViewMode::Settings => ViewMode::NowPlaying,
                ViewMode::NowPlaying => ViewMode::Library,
            };
        }
        Action::PlayNext(track_id) => transport.play_next(track_id),
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
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)) {
        *global_search_focus = true;
    }
    if !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
    {
        let playing = playback.playback_state == PlaybackState::Playing;
        if playing {
            transport.pause();
        } else {
            transport.resume();
        }
    }
}

/// Push the window title and tray tooltip for the current track (REQ-SI-001).
/// Both derive from one identity — the current `TrackId` — which is compared
/// against the last push FIRST: steady-state frames send no viewport command
/// and format nothing. The key exists to avoid repeating OS viewport
/// commands, not for staleness; the current Track resolves through the
/// Session Views seam over the store's `get_track` query — never the
/// in-memory mirror.
/// Last identity pushed to the window title / tray tooltip. `Unset`
/// distinguishes "nothing pushed yet" from "pushed while nothing plays" so
/// the very first frame always pushes once.
#[derive(Default)]
enum TitleKey {
    #[default]
    Unset,
    Set(Option<TrackId>),
}

impl RiffApp {
    fn update_window_title(&mut self, ctx: &egui::Context, playback: &PlaybackSession) {
        self.views
            .sync_playback(&playback.queue, crate::ui::now_playing::UP_NEXT_LIMIT);
        let current_id = self.views.playback_current().map(|t| &t.id);
        let unchanged = match &self.last_title_key {
            TitleKey::Set(id) => id.as_ref() == current_id,
            TitleKey::Unset => false,
        };
        if unchanged {
            return;
        }

        // Cold path: the playing track moved — both strings are rebuilt and
        // pushed exactly once per identity change.
        let (tooltip, title) = match self.views.playback_current() {
            Some(track) => {
                let tooltip = format!(
                    "{} - {}",
                    track.metadata.display_artist(),
                    track.metadata.display_title(&track.file_path)
                );
                let title = format!("{tooltip} \u{2014} riff");
                (tooltip, title)
            }
            None => ("riff".to_owned(), "riff".to_owned()),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        self.last_title_key = TitleKey::Set(current_id.cloned());

        #[cfg(not(target_os = "linux"))]
        {
            if self.last_tray_tooltip != tooltip {
                if let Some(ref tray) = self.tray_icon {
                    crate::ui::tray::update_tooltip(tray, &tooltip);
                }
                self.last_tray_tooltip = tooltip;
            }
        }
        #[cfg(target_os = "linux")]
        {
            let _ = tooltip;
        }
    }

    /// Bottom shell strip (Issues 06 + 08): transport, seek row, and volume
    /// at the exact 88px playerbar token height, drawn by the restyled
    /// playerbar widgets. Every reported [`crate::ui::playerbar::
    /// PlayerBarAction`] routes through [`apply_player_bar_action`], so each
    /// control still emits its engine command. Also stashes this frame's
    /// `(title, meta_line)` handout in [`Self::now_playing_labels`] for the
    /// Now Playing stage.
    fn render_control_bar(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
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
            let up_next = crate::ui::now_playing::up_next_entries(
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
        for action in self.playerbar_actions.drain(..) {
            apply_player_bar_action(action, library, playback, self.transport.as_ref());
        }
        self.now_playing_labels = (title, meta_line);
    }
}

// --- Helper methods factored out to avoid borrow conflicts ---
impl RiffApp {
    /// Sidebar content (design-handoff issue 07): the flat, always-visible
    /// sectioned nav — LIBRARY, SMART LISTS, PLAYLISTS — every row carrying a
    /// live count from the counts read model, with the Add-folder /
    /// "Last scan X ago" footer pinned to the panel's bottom. Draws inside
    /// the shell's fixed-width sidebar panel, which is shared chrome present
    /// on every view; only the main stage switches. Every nav click lands on
    /// the library view (clearing any active search) so exactly one browser
    /// variant is visible after it — the variant itself renders in the
    /// browser column pane (issue 08), not here.
    fn render_library_sidebar(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
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
        self.render_library_rows(
            ui,
            library,
            &counts,
            library_section_live,
            folder_section_live,
        );
        ui.add_space(8.0);

        // --- SMART LISTS -------------------------------------------------------
        // Four core lists are always visible (no Advanced gate); Never Played
        // and Lost Gems relocate behind Advanced mode (relocated, not
        // deleted — handoff issue 07 / open decision 2).
        self.render_smart_list_rows(ui, library, &counts);

        // --- PLAYLISTS ---------------------------------------------------------
        // User playlists (Task 4.2): named, editable lists persisted in the
        // Application Store, with their existing create/rename/delete/reorder
        // flows. Always visible.
        self.render_playlists_section(ui, library);

        // --- FOOTER (pinned to the panel's bottom) ------------------------------
        egui::Panel::bottom("sidebar_footer")
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.render_sidebar_footer(ui, library);
            });
    }

    /// The LIBRARY section's five rows (design-handoff issue 07): All Tracks,
    /// Artists, Albums, Genres, and Folders, each with its live count.
    /// Clicking one lands on the library view in that section (Folders
    /// switches the browse mode instead) and closes any opened list.
    fn render_library_rows(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        counts: &riff_backend::app::views::SidebarCounts,
        library_section_live: bool,
        folder_section_live: bool,
    ) {
        use crate::ui::sidebar::{self, TreeRow};

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
                library.view_mode = ViewMode::Library;
                library.browse_mode = BrowseMode::Library;
                library.library_section = section;
                // Section (or browse-mode) navigation resets the drill-down
                // path: the new section starts at its root listing.
                library.reset_browser_path();
                library.search_query.clear();
                self.smart_playlist_view = None;
                self.playlist_view = None;
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
            library.view_mode = ViewMode::Library;
            library.browse_mode = BrowseMode::Folders;
            // A browse-mode switch resets the drill-down path along with the
            // section change.
            library.reset_browser_path();
            library.search_query.clear();
            self.smart_playlist_view = None;
            self.playlist_view = None;
        }
    }

    /// The SMART LISTS section's rows (design-handoff issue 07): the four
    /// core lists always, Never Played / Lost Gems behind Advanced mode.
    /// Clicking one opens it over the library view. The section header is
    /// clickable: a chevron pinned to the header's right edge (the same
    /// slot the Playlists "+" button uses) folds the section away, and the
    /// collapsed state is a persisted UI flag
    /// (`UiFlags::smart_lists_collapsed`) restored on launch through the
    /// scalar settings round-trip.
    fn render_smart_list_rows(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        counts: &riff_backend::app::views::SidebarCounts,
    ) {
        use crate::ui::icons::Icon;
        use crate::ui::sidebar::{self, TreeRow};

        let palette = self.theme.active;
        let collapsed = library.ui_flags.smart_lists_collapsed;

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
                    library.ui_flags.smart_lists_collapsed = !collapsed;
                    // Folding the section away also closes any smart list it
                    // opened: with the rows gone there is no other way back
                    // to that view.
                    self.smart_playlist_view = None;
                }
                response.on_hover_text(label);
            });
        });

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
                library.view_mode = ViewMode::Library;
                library.browse_mode = BrowseMode::Library;
                // Opening a smart list leaves the browser's drill-down path
                // behind: the listing replaces the column stage.
                library.reset_browser_path();
                library.search_query.clear();
                self.smart_playlist_view = Some(kind);
                self.playlist_view = None;
            }
        }
    }

    /// The sidebar footer's action side: the stamp text comes from the
    /// last-scan read model (issue 05) formatted by
    /// [`sidebar::format_last_scan_ago`]; Add folder routes through the
    /// EXISTING add-library-path flow.
    fn render_sidebar_footer(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        use crate::ui::sidebar;

        let stamp = self.views.last_scan().map(|scan| {
            let elapsed = scan.elapsed().unwrap_or_default();
            format!("Last scan {}", sidebar::format_last_scan_ago(elapsed))
        });
        if sidebar::sidebar_footer(ui, &mut self.icons, &self.theme.active, stamp.as_deref()) {
            self.add_folder_from_sidebar(library);
        }
    }

    /// Add a library root from the sidebar footer: the native folder picker
    /// everywhere except Linux, where the text-input row renders beneath the
    /// Settings stage — so the footer lands there before opening it.
    fn add_folder_from_sidebar(&mut self, library: &mut LibrarySession) {
        #[cfg(target_os = "linux")]
        {
            crate::ui::chrome::NavDestination::Settings
                .apply(&mut library.view_mode, &mut library.browse_mode);
            self.settings_show_input = true;
            self.settings_path_error = None;
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.add_library_via_platform_picker(library);
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
        // (bounded store windows via `SessionViews::track_list`), smart
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
        // after committed mutations. The seam owns the window math, the
        // count reads, and the torn-count recount; this view only maps row
        // indices to pages.
        let current_track = playback.queue.current_track().cloned();

        // Anchor read: sizes the row range with the authoritative total.
        let first_page = self.views.track_list(query, 0);

        // ---- Scroll Memory (scroll-memory spec, issue 01) ----
        // The All Tracks flat list is the first Section slot: it declares its
        // slot to the Scroll Memory, which hands back the salt and the start
        // offset (the saved one when the fingerprint matches, zero on a stale
        // slot or none saved yet), and the visit token that records the actual
        // offset back under the same content identity.
        let (control, visit) = self.scroll_memory.begin_section(
            riff_backend::app::state::LibrarySection::AllTracks,
            query,
            false,
        );
        if first_page.total == 0 {
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
            first_page.total,
            |ui, row_range| {
                let mut page: Option<riff_backend::app::views::TrackListPage> = None;
                for i in row_range {
                    // Refetch only when the row leaves the page in hand; the
                    // seam serves repeat windows from cache.
                    if page.as_ref().is_none_or(|p| p.start + p.rows.len() <= i) {
                        page = Some(self.views.track_list(query, i));
                    }
                    let page = page.as_ref().expect("page fetched above");
                    if let Some(track) = page.rows.get(i - page.start) {
                        self.render_track_row(
                            ui,
                            library,
                            playback,
                            track,
                            current_track.as_ref(),
                            None,
                        );
                    }
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
        // with whole-list actions mirroring the album/folder header menu.
        let header = ui.horizontal(|ui| {
            ui.heading(kind.display_name());
            ui.weak(format!("({} tracks, read-only)", tracks.len()));
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

        egui::ScrollArea::vertical().show_rows(
            ui,
            theme::geometry::sidebar::ROW_H,
            tracks.len(),
            |ui, row_range| {
                for i in row_range {
                    if let Some(track) = tracks.get(i) {
                        self.render_track_row(
                            ui,
                            library,
                            playback,
                            track,
                            current_track.as_ref(),
                            None,
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
    fn render_playlists_section(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
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
                    self.playlist_create_name = Some(String::new());
                    self.playlist_rename = None;
                }
            });
        });

        self.render_playlist_create_prompt(ui);

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
                let id = playlists[index].id.clone();
                apply_playlist_row_action(
                    action,
                    &id,
                    self.playlist_store.as_mut(),
                    &mut self.views,
                    PlaylistPromptSlots {
                        view: &mut self.playlist_view,
                        smart_view: &mut self.smart_playlist_view,
                        rename: &mut self.playlist_rename,
                        create_name: &mut self.playlist_create_name,
                    },
                );
                // The section is shared chrome on every view: opening a
                // playlist lands on the library view so its listing is on
                // screen, and clears any search filter over the results.
                if action == crate::ui::sidebar::PlaylistRowAction::Open {
                    library.view_mode = ViewMode::Library;
                    library.browse_mode = BrowseMode::Library;
                    // Opening a playlist leaves the browser's drill-down path
                    // behind: the listing replaces the column stage.
                    library.reset_browser_path();
                    library.search_query.clear();
                }
            }

            self.render_playlist_rename_prompt(ui, &playlists[index].id);
        }
    }

    /// The inline "New Playlist" name prompt while it is open.
    fn render_playlist_create_prompt(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = self.playlist_create_name.as_mut() else {
            return;
        };
        // The pure widget seam (golden-image gap audit P1-7) reports the
        // outcome; the store flow below is unchanged.
        let outcome = crate::ui::prompts::playlist_create_prompt(ui, draft);
        let confirm = outcome == Some(crate::ui::prompts::PromptOutcome::Confirm);
        let cancel = outcome == Some(crate::ui::prompts::PromptOutcome::Cancel);
        if confirm {
            let name = self.playlist_create_name.take().unwrap_or_default();
            let name = name.trim().to_string();
            if !name.is_empty() {
                match self.playlist_store.create_playlist(&name, &[]) {
                    Ok(id) => {
                        // The committed create bumps the playlist generation;
                        // the seam's next read lists the new playlist.
                        self.playlist_view = Some(id);
                    }
                    Err(e) => tracing::warn!("Failed to create playlist: {e}"),
                }
            }
        } else if cancel {
            self.playlist_create_name = None;
        }
    }

    /// The inline rename prompt for one playlist while it is open. Addressed
    /// by playlist id so fresh frames never clone it.
    fn render_playlist_rename_prompt(&mut self, ui: &mut egui::Ui, pid: &PlaylistId) {
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
        let outcome = crate::ui::prompts::playlist_rename_prompt(ui, draft);
        let confirm = outcome == Some(crate::ui::prompts::PromptOutcome::Confirm);
        let cancel = outcome == Some(crate::ui::prompts::PromptOutcome::Cancel);
        if confirm {
            if let Some((rid, draft)) = self.playlist_rename.take() {
                // Same Store flow as before the restyle: trim, rename as one
                // durable transaction. The seam's next read reflects the new
                // name on its own (ADR 0002).
                commit_playlist_rename(self.playlist_store.as_mut(), &rid, &draft);
            }
        } else if cancel {
            self.playlist_rename = None;
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
        // mirroring the smart-playlist header menu.
        let header = ui.horizontal(|ui| {
            ui.heading(playlist_name);
            ui.weak(format!("({track_count} tracks)"));
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

        // One row per entry: the store-resolved track (if any) plus the
        // final playability verdict (Library-known AND file exists on disk).
        egui::ScrollArea::vertical().show_rows(
            ui,
            theme::geometry::sidebar::ROW_H,
            entries.len(),
            |ui, row_range| {
                for i in row_range {
                    if let Some(entry) = entries.get(i) {
                        self.render_playlist_entry(
                            ui,
                            library,
                            playback,
                            playlist_id,
                            entry,
                            current_track.as_ref(),
                            i,
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
    #[allow(clippy::too_many_arguments)]
    fn render_playlist_entry(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
        playlist_id: &PlaylistId,
        entry: &(TrackId, Option<Track>, bool),
        current_track: Option<&TrackId>,
        index: usize,
    ) {
        use std::path::PathBuf;
        let (tid, track, valid) = entry;
        if *valid && let Some(t) = track {
            self.render_reorderable_playlist_row(
                ui,
                library,
                playback,
                t,
                current_track,
                playlist_id,
                index,
            );
            return;
        }

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
            self.attach_track_menu(&response, library, tid, None, Some(playlist_id));
        });
    }

    /// One drag-reorderable row of [`Self::render_playlist_view`] (Issue
    /// 12): the standard interactive track row wrapped in egui's built-in
    /// drag-and-drop support ([`sidebar::reorderable_row`]). Releasing a row
    /// on another persists the new order through the [`PlaylistStore`] port
    /// via [`commit_playlist_reorder`] (ADR 0002); clicks and double-clicks
    /// behave exactly as before.
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
    #[allow(clippy::too_many_arguments)]
    fn render_reorderable_playlist_row(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
        track: &Track,
        current_track: Option<&TrackId>,
        playlist_id: &PlaylistId,
        index: usize,
    ) {
        use crate::ui::sidebar::{self, TreeRow};

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

        let outcome = sidebar::reorderable_row(
            ui,
            &mut self.icons,
            &self.theme.active,
            egui::Id::new(("riff_playlist_entry", &playlist_id.0, index)),
            index,
            TreeRow {
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
            },
        );
        let favorite_toggled = outcome.favorite_toggled;
        let response = outcome.response;
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
        if let Some(from) = outcome.drop_from {
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
            library,
            &track.id,
            Some(track),
            Some(playlist_id),
        );
    }

    /// The restyled Now Playing stage (Issue 10): the 240px cover with its
    /// extra-large radius and brand glow, the 3xl title, the meta line, the
    /// in-view seek row, and the Up Next queue rows in Playback Queue order.
    /// Draws through the pure widget seam in
    /// [`crate::ui::now_playing::show_now_playing`]; every reported action
    /// routes through [`apply_now_playing_action`], so Close always lands on
    /// the Library View and the transport still emits engine commands.
    fn show_now_playing_view(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
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
        let up_next: Arc<[UpNextEntry]> = crate::ui::now_playing::up_next_entries(
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
        for action in self.now_playing_actions.drain(..) {
            apply_now_playing_action(action, library, playback, self.transport.as_ref());
        }
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
        let cover = folder_cover_intent(
            &self.cover_textures,
            &mut self.cover_in_flight,
            &mut self.cover_in_flight_keys,
            self.covers.as_ref(),
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
                    library,
                    playback,
                    track,
                    current_track.as_ref(),
                    None,
                    &label,
                    level + 1,
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

/// The UI's whole remaining cover responsibility (ADR 0006): ask the Cover
/// Service for art unless that track at that exact box is already in the
/// UI-owned cache. Free function so tests drive the exact production path
/// without a window.
///
/// The cache check is made here rather than by the caller because the key it
/// checks is the same composite the cache is written under: a track cached at
/// hero size is still a miss at thumbnail size.
pub fn request_cover_intent<S: std::hash::BuildHasher>(
    textures: &std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    in_flight: &mut std::collections::HashSet<CoverCacheKey, S>,
    in_flight_keys: &mut Vec<CoverCacheKey>,
    covers: &dyn Covers,
    track_id: TrackId,
    path: PathBuf,
    size: RequestedSize,
) {
    let key = cover_cache_key(&track_id.0, size);
    if textures.contains_key(&key) || in_flight.contains(&key) {
        return;
    }
    mark_in_flight(in_flight, in_flight_keys, key);
    covers.request(track_id, path, size);
}

/// Record that a `(identity, box)` has an outstanding request, keeping the marker
/// set bounded by the same LRU discipline as the texture map.
///
/// The marker is dropped by [`cache_polled_covers`] when the answer arrives, so
/// overflowing the cap can only ever forget a *recent* ask — and the cost of
/// forgetting is one re-request, not a wrong image. Both structures are updated
/// together because a key left in the list but not the set would be re-marked
/// forever, and one left in the set but not the list could never be evicted.
fn mark_in_flight<S: std::hash::BuildHasher>(
    in_flight: &mut std::collections::HashSet<CoverCacheKey, S>,
    in_flight_keys: &mut Vec<CoverCacheKey>,
    key: CoverCacheKey,
) {
    for evicted in lru_insert(in_flight_keys, key.clone(), COVER_IN_FLIGHT_CAP) {
        in_flight.remove(&evicted);
    }
    in_flight.insert(key);
}

/// The Folders tree's half of the same responsibility (ADR 0006): a folder row
/// wants the cover art of the directory it *is*, and gets it in place of the
/// folder glyph. Free function like [`request_cover_intent`] so tests drive the
/// production path without a window.
///
/// A cache hit is the texture to paint; a miss sends the request and answers
/// `None`, which is what keeps the row's glyph for this frame. The miss does
/// **not** fall back to the generated music-note placeholder the artless track
/// rows get — a folder with no cover of its own is an ordinary folder, not an
/// artless album.
///
/// The identity is the directory path, and the service files the decoded result
/// under exactly that key, so [`cache_polled_covers`] delivers folder art with
/// no further plumbing.
pub fn folder_cover_intent<S: std::hash::BuildHasher>(
    textures: &std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    in_flight: &mut std::collections::HashSet<CoverCacheKey, S>,
    in_flight_keys: &mut Vec<CoverCacheKey>,
    covers: &dyn Covers,
    folder: &Path,
    size: RequestedSize,
) -> Option<egui::TextureId> {
    let identity = folder.to_string_lossy().to_string();
    let key = cover_cache_key(&identity, size);
    if let Some(texture) = textures.get(&key) {
        return Some(texture.id());
    }
    if !in_flight.contains(&key) {
        mark_in_flight(in_flight, in_flight_keys, key);
        covers.request_folder(folder, size);
    }
    None
}

/// Apply drained backend events to the structured feedback board (issue 11).
/// Playback errors arrive as typed notices stamped with playback source and
/// error severity; each is folded into its source's persistent slot so it
/// survives alongside — not overwritten by — Library Scan progress. Other event
/// kinds carry no UI feedback yet.
pub fn apply_backend_events(
    events: Vec<riff_backend::app::events::BackendEvent>,
    feedback: &mut crate::ui::feedback::FeedbackBoard,
) {
    use crate::ui::feedback::Feedback;
    use riff_backend::app::events::BackendEvent;
    for event in events {
        if let BackendEvent::TypedNotice(payload) = event {
            feedback.put(Feedback::from_notice(&payload, None));
        }
    }
}

/// Consume polled cover results into the UI texture cache: wrapping already
/// decoded pixels and uploading is the egui-bound work that stays on the main
/// thread (the texture boundary, ADR 0006). The decode itself happened on the
/// cover worker thread behind the port, so a frame can never block on it;
/// dedup and negative caching live behind the service seam, so artless
/// results are simply dropped here.
pub fn cache_polled_covers<S: std::hash::BuildHasher>(
    covers: &dyn Covers,
    textures: &mut std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    lru_keys: &mut Vec<CoverCacheKey>,
    in_flight: &mut std::collections::HashSet<CoverCacheKey, S>,
    in_flight_keys: &mut Vec<CoverCacheKey>,
    ctx: &egui::Context,
) {
    for (track_id, size, cover) in covers.poll() {
        // Every delivered answer is terminal, `None` included: an artless row that
        // kept its marker would never ask again, and a row whose art appears later
        // would stay blank for the session. Cleared before the `continue` below for
        // exactly that reason.
        let key = cover_cache_key(&track_id.0, size);
        in_flight.remove(&key);
        in_flight_keys.retain(|existing| existing != &key);

        let Some(cover) = cover else {
            continue; // artless: the service negative-caches it
        };
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [cover.width as usize, cover.height as usize],
            &cover.rgba,
        );
        let texture = ctx.load_texture(&track_id.0, color_image, egui::TextureOptions::default());
        textures.insert(key.clone(), texture);
        for old in lru_insert(lru_keys, key, COVER_CACHE_CAP) {
            textures.remove(&old);
        }
        crate::ui::artwork::enforce_texture_byte_budget(textures, lru_keys);
    }
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

/// Drain the settled outcome of a Thumbnail-cache clear, and flush the texture map
/// with it.
///
/// The flush happens here rather than when the button was pressed, and the order
/// matters: at confirm time every visible row would re-request while the wipe was
/// still queued behind those very requests, rebuilding the entries the user had
/// just asked to delete. Settled is the moment the disk is genuinely empty, so it
/// is the moment the screen can be emptied with it. A `Failed` clear leaves every
/// texture in place — nothing was removed, so nothing has to be re-derived.
pub fn settle_cache_clear<S: std::hash::BuildHasher>(
    covers: &dyn Covers,
    in_flight: &mut bool,
    textures: &mut std::collections::HashMap<CoverCacheKey, egui::TextureHandle, S>,
    lru_keys: &mut Vec<CoverCacheKey>,
) -> Option<ClearCacheOutcome> {
    if !*in_flight {
        return None;
    }
    let outcome = covers.poll_cache_clear()?;
    *in_flight = false;
    if outcome == ClearCacheOutcome::Cleared {
        crate::ui::artwork::evict_all_covers(textures, lru_keys);
    }
    Some(outcome)
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

/// The host slots one Track menu's intents answer to: everything an emitted
/// intent may act on, and nothing the menu may reach while it renders
/// (component-layer issue 15).
pub struct TrackMenuEffects<'a> {
    /// The Track the menu was attached to — the subject of every intent.
    pub track_id: &'a TrackId,
    /// The track itself; `None` (e.g. a playlist entry whose file is missing)
    /// suppresses playback actions, "Edit Tags", and the Favourite item.
    pub track: Option<&'a Track>,
    /// The selection slot: the "Edit Tags" intent selects the Track before the
    /// inline editor opens, so the readout follows the entry point.
    pub selected_track: &'a mut Option<TrackId>,
    /// The inline editor's controller: "Edit Tags" opens the per-selection
    /// draft for that track through it (the retired modal's entry point, now
    /// un-gated).
    pub tag_editor: &'a mut InlineTagEditor,
    /// The playback command port the queue intents go through.
    pub transport: &'a dyn Transport,
    /// The Application Store's playlists section: entry mutations commit
    /// through it as one immediate durable transaction.
    pub playlist_store: &'a mut dyn PlaylistStore,
    /// The Application Store's library section, and the Favourite flag's only
    /// durable home. The Favourite item writes through the SAME port method the
    /// row's heart does ([`LibraryMutationStore::set_track_favorite`]), so the
    /// menu is a second path to one change rather than a second way of storing
    /// it — which is the whole reason it is here and not a variant of the
    /// Playlist Store above.
    pub library_mutations: &'a mut dyn LibraryMutationStore,
    /// When `Some`, the row belongs to that Playlist, so the menu offers the
    /// removal and this is the playlist it removes from.
    pub remove_from_playlist: Option<&'a PlaylistId>,
}

/// Shared track context menu. The rows and their meanings belong to
/// [`crate::ui::menu`]; this only decides which rows exist, renders them inside
/// egui's popup, and hands each emitted intent to [`apply_track_menu_intent`] —
/// so no Transport command, store write, or editor draft can happen while the
/// menu is being painted.
///
/// The props are read off `effects` per row, which is why every Track-row
/// surface can answer one way for a Favourited Track and the other way for an
/// un-Favourited one in the same frame without any of them knowing about the
/// others.
///
/// Returns whether the popup was open on this frame, which is the menu's only
/// selection-carrying report: the row's identity, and the fact that its menu
/// opened, apart from anything the listener chose from it. `false` means the row
/// was merely painted. The popup is egui's own transient one, keyed by the row's
/// response identity rather than by any index, so a virtualized listing keeps no
/// per-row open state — and a row materialized only because it was on screen is
/// all the report needs to say.
fn show_track_context_menu(
    response: &egui::Response,
    palette: &Palette,
    playlists: &[(PlaylistId, String)],
    effects: &mut TrackMenuEffects<'_>,
) -> bool {
    let menu = crate::ui::menu::TrackMenu {
        playable: effects.track.is_some(),
        editable: effects.track.is_some(),
        // The row's OWN flag, read off the Track the host resolved for it: the
        // item's label flips on this, and a listing can hold a mix of
        // Favourited and un-Favourited rows. A missing-file entry has no Track
        // to read, and is offered no Favourite in the first place.
        favorite: effects.track.is_some_and(|track| track.favorite),
        playlists,
        remove_from_playlist: effects.remove_from_playlist.is_some(),
    };
    let mut intents = Vec::new();
    let open = response
        .context_menu(|ui| {
            crate::ui::menu::track_menu(ui, palette, &menu, &mut intents);
        })
        .is_some();
    for intent in intents {
        apply_track_menu_intent(intent, effects);
    }
    open
}

/// Apply one emitted Track-menu intent. Every effect in the app's track menus
/// starts here, in answer to a row the listener actually activated.
pub fn apply_track_menu_intent(
    intent: crate::ui::menu::TrackMenuIntent,
    effects: &mut TrackMenuEffects<'_>,
) {
    use crate::ui::menu::TrackMenuIntent;
    let track_id = effects.track_id.clone();
    match intent {
        TrackMenuIntent::Play => effects.transport.play(track_id),
        TrackMenuIntent::PlayNext => effects.transport.play_next(track_id),
        TrackMenuIntent::AddToQueue => effects.transport.add_to_queue(track_id),
        TrackMenuIntent::AddToPlaylist(playlist) => {
            // One immediate durable transaction; the committed mutation bumps
            // the playlist generation, so the seam's next read reflects it with
            // zero caller action (ADR 0002).
            if let Err(e) = effects
                .playlist_store
                .add_playlist_entry(&playlist, &track_id)
            {
                tracing::warn!("Failed to add playlist entry: {e}");
            }
        }
        TrackMenuIntent::RemoveFromPlaylist => {
            let Some(playlist) = effects.remove_from_playlist else {
                return;
            };
            if let Err(e) = effects
                .playlist_store
                .remove_playlist_entries(playlist, &track_id)
            {
                tracing::warn!("Failed to remove playlist entry: {e}");
            }
        }
        // The Favourite item commits the flag the menu said it would, through
        // the one durable setter the row's heart also uses: one immediate
        // transaction, the committed mutation bumps the library generation so
        // the projections re-resolve the row on the next frame with zero caller
        // action (ADR 0002). Nothing else moves — no transport command, no
        // playlist entry, no tag draft — because a Favourite is a library fact
        // and nothing else. A failed commit changes nothing and is logged, the
        // same as the heart's.
        TrackMenuIntent::SetFavorite(favorite) => {
            if let Err(e) = effects
                .library_mutations
                .set_track_favorite(&track_id, favorite)
            {
                tracing::warn!("Failed to commit the favorite flag for {}: {e}", track_id.0);
            }
        }
        // "Edit Tags" is the inline editor's entry point: it selects the Track
        // (so the Detail Panel shows its readout) and opens the per-selection
        // draft focused on the first tag field (Issue 04).
        TrackMenuIntent::EditTags => {
            let Some(track) = effects.track else {
                return;
            };
            *effects.selected_track = Some(track_id);
            let rows = tag_rows(std::slice::from_ref(track));
            effects
                .tag_editor
                .open_track(track.id.clone(), track.file_path.clone(), &rows);
            // The entry point focuses the first tag field (Issue 04): the
            // one-shot flag is consumed the frame it lands.
            if let Some(draft) = effects.tag_editor.draft_mut() {
                draft.focus_first = true;
            }
        }
    }
}

/// What one Track row's menu reported besides the items the listener chose:
/// whether the popup was open on this frame.
///
/// A report in its own right rather than a flag among [`TrackMenuEffects`],
/// because those are the slots an intent acts THROUGH while this is the menu
/// itself acting. OPENING is a different event from CHOOSING, and they are kept
/// apart on purpose: the selection is the open's, so a right-click that opens a
/// menu and is then dismissed has still selected — which would be false if the
/// effect rode along on an item choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackMenuOpen {
    /// The row's popup was open on this frame — with nothing chosen yet, or
    /// with a choice [`apply_track_menu_intent`] has already answered.
    Opened,
    /// The row was painted and nothing opened on it: the state a list of Track
    /// rows spends almost all of its frames in, and the one that must move
    /// nothing at all.
    NotOpened,
}

/// Apply one Track row's menu-open report: OPENING the menu makes that Track the
/// selected Track, and that is all the open does.
///
/// Two deliberate shapes here.
///
/// The selection is an argument rather than something read back off the click,
/// which is what makes the whole step reachable — and provable — with no pointer
/// in the test: the row was right-clicked a long way from here, and everything
/// that click caused is spelled out in these two values. It is the same reason
/// the entity rows' selection is an argument to [`apply_collection_menu`].
///
/// And it is a free function over plain borrows rather than a method on
/// [`TrackMenuEffects`], for the same reason plus one: the effects bag is the
/// menu's item path, and an open with no item chosen has no item path to run.
/// Putting this in the bag would have meant either a slot nothing reads or a
/// constructor that stands in for a pointer the test does not have.
///
/// A `NotOpened` report moves nothing — neither the selection it would have
/// written nor the one already there. The report moves the selection; it never
/// owns it.
pub fn apply_track_menu_open(
    track_id: &TrackId,
    open: TrackMenuOpen,
    selected_track: &mut Option<TrackId>,
) {
    if matches!(open, TrackMenuOpen::Opened) {
        *selected_track = Some(track_id.clone());
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
/// [`TrackMenuEffects`] for the Track menu. Nothing in here is reachable while
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
/// `section` and `level` say which entity the row's `key` names, and the caller
/// supplies them because only the caller knows them — a section root passes the
/// session's own section and level 0, which is exactly what a plain click on
/// the same row passes, while a drill column passes the section and depth it
/// rendered with. Neither can disagree with itself about which row was clicked.
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
    section: LibrarySection,
    level: usize,
    effects: CollectionMenuEffects<'_>,
) {
    apply_entity_selection(key, section, level, effects.library);
    for intent in intents {
        let batch = entity_track_ids(key, section, level, effects.views);
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
