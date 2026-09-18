mod browser_pane;
mod selection_pane;

use crate::ui::chrome::TitleBarAction;
use crate::ui::now_playing::{NowPlayingAction, UpNextEntry};
use crate::ui::playerbar::PlayerBarAction;
use crate::ui::settings::SettingsSection;
use crate::ui::theme;
use eframe::egui;
use riff_backend::app::MutexExt;
use riff_backend::app::Transport;
pub use riff_backend::app::cover_service::{COVER_CACHE_CAP, Covers, lru_insert};
use riff_backend::app::events::BackendEvents;
use riff_backend::app::preferences::Preferences;
use riff_backend::app::scan_service::{ScanOutcome, Scans};
use riff_backend::app::state::{
    BrowseMode, BrowserSelection, LibrarySection, LibrarySession, LibraryStatus, PlaybackSession,
    ViewMode,
};
use riff_backend::app::store::{LibraryMutationStore, PlaylistStore, SettingsStore};
use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
use riff_backend::app::traits::TagEdit;
use riff_backend::app::views::SessionViews;
use riff_backend::app::watcher_manager::WatcherManager;
use riff_backend::domain::{
    PlaybackState, Playlist, PlaylistId, SmartPlaylistKind, Track, TrackId,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
    cover_textures: std::collections::HashMap<String, egui::TextureHandle>,
    cover_lru_keys: Vec<String>,
    /// The Cover Service front end (ADR 0006): sends resolve intent and
    /// yields drained results; dedup and the negative cache live behind it.
    covers: Box<dyn Covers>,
    /// The Tag Edit Service front end (ADR 0006): submits save intent and
    /// yields polled outcomes; the whole save flow lives behind it.
    tag_edits: Box<dyn TagEdits>,
    /// The inline editor's open per-selection draft (tickets 02/03): `Some`
    /// while the Detail Panel is editing the selection; discarded the moment
    /// the selection changes — the draft never outlives its selection.
    inline_draft: Option<crate::ui::selection::TagDraft>,
    /// The one inline Tag Edit currently outstanding, recorded at submit so a
    /// polled outcome can be matched back to the draft (and its file name
    /// shown in the status line) — outcomes themselves carry no identity.
    inline_in_flight: Option<(TrackId, PathBuf)>,
    /// An album batch's outstanding requests (ticket 03), serialized by the
    /// worker and polled in submission order.
    inline_batch_in_flight: Option<BatchInFlight>,
    /// Which read-only smart playlist is open in the library explorer, if any.
    /// Transient UI state (precedent: `tag_edit`); the playlist contents are
    /// re-computed from library data on every frame, so nothing is cached.
    smart_playlist_view: Option<SmartPlaylistKind>,
    /// Which user playlist is open in the library explorer, if any.
    playlist_view: Option<PlaylistId>,
    /// The playing track's id whose cover the Now Playing stage last
    /// requested/rendered; only re-cloned when the track moves.
    now_playing_cover_key: Option<String>,
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
    /// Caller-retained action buffer for the content top bar (issue 06).
    topbar_actions: Vec<crate::ui::topbar::TopBarAction>,
    now_playing_actions: Vec<NowPlayingAction>,
    /// Transient "New Playlist" name prompt (`Some` = open, holds the draft).
    playlist_create_name: Option<String>,
    /// Transient rename prompt: (playlist id, draft name).
    playlist_rename: Option<(PlaylistId, String)>,
    /// Transient Clear Library confirmation (`true` = awaiting confirm).
    /// Grouped with the other transient prompts on `RiffApp`.
    pub(crate) clear_library_confirm: bool,
    /// Ctrl+K request flag (issue 06): one-shot focus request for the global
    /// search field in the content top bar, consumed on the frame it lands.
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
    /// The Backend Events inbox: the observable surface both the Transport
    /// wrapper and the tray thread record dispatched commands onto, and the
    /// inbox the UI drains at the start of every frame.
    backend_events: Arc<Mutex<BackendEvents>>,
    quit_flag: Arc<AtomicBool>,
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
        quit_flag: Arc<AtomicBool>,
        settings_store: Box<dyn SettingsStore>,
        playlist_store: Box<dyn PlaylistStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        views: SessionViews,
        tag_edits: Box<dyn TagEdits>,
        covers: Box<dyn Covers>,
        backend_events: Arc<Mutex<BackendEvents>>,
        #[cfg(not(target_os = "linux"))]
        visibility_listener: crate::ui::window_visibility::VisibilityListener,
    ) -> Self {
        Self {
            playback,
            library,
            transport,
            scans,
            cover_textures: std::collections::HashMap::new(),
            cover_lru_keys: Vec::new(),
            covers,
            tag_edits,
            inline_draft: None,
            inline_in_flight: None,
            inline_batch_in_flight: None,
            smart_playlist_view: None,
            playlist_view: None,
            now_playing_cover_key: None,
            last_title_key: TitleKey::Unset,
            playerbar_readouts: crate::ui::playerbar::SeekReadouts::new(),
            stage_readouts: crate::ui::playerbar::SeekReadouts::new(),
            titlebar_actions: Vec::new(),
            playerbar_actions: Vec::new(),
            topbar_actions: Vec::new(),
            now_playing_actions: Vec::new(),
            playlist_create_name: None,
            playlist_rename: None,
            clear_library_confirm: false,
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
            quit_flag,
            backend_events,
        }
    }

    /// Test-only constructor: the production wiring surface minus everything
    /// the platform supplies.
    ///
    /// It fills in what [`Self::new`] takes from the host — no tray icon, an
    /// empty watcher-manager handle, a fresh quit flag, and a real visibility
    /// channel pair — so a test can build the whole shell over mock ports with
    /// no audio device, no tray, and no Application Store. The returned sender
    /// lets a test push a Show/Hide request through the same path the tray
    /// uses; on Linux there is nothing to drain it, so it is inert there.
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
            Arc::new(AtomicBool::new(false)),
            settings_store,
            playlist_store,
            library_mutations,
            views,
            tag_edits,
            covers,
            backend_events,
            #[cfg(not(target_os = "linux"))]
            visibility_listener,
        );
        (app, visibility_tx)
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
            crate::ui::cover_placeholder::evict_generated(
                &mut self.cover_textures,
                &mut self.cover_lru_keys,
            );
        }
        self.theme.active = palette;
        self.theme.last_applied = Some((dark, high_contrast));
    }

    /// Send cover intent for one track to the Cover Service. The only
    /// UI-side check left is the texture cache (the texture LRU is
    /// UI-owned per the texture boundary); request deduplication and the
    /// negative cache live behind the service seam.
    fn request_cover(&self, track_id: &TrackId, file_path: &Path) {
        request_cover_intent(
            self.cover_textures.contains_key(&track_id.0),
            self.covers.as_ref(),
            track_id.clone(),
            file_path.to_path_buf(),
        );
    }

    /// Drain polled Library Scan outcomes from the service and map them onto
    /// session state exactly as before the extraction: per-root statuses
    /// plus the titlebar scan-status line. The service NEVER touches
    /// `LibrarySession` — this mapping is the UI's whole remaining scan
    /// responsibility (ADR 0006). The watcher observes a scan's end itself
    /// via `is_scanning`, so no relay fires here anymore.
    fn poll_library_updates(&self, library: &mut LibrarySession) {
        for outcome in self.scans.poll() {
            match outcome {
                ScanOutcome::Progress { path, files_found } => {
                    library
                        .library_statuses
                        .insert(path, LibraryStatus::Scanning { files_found });
                    library.scan_status = Some(format!("{files_found} files"));
                }
                ScanOutcome::Complete { path, total_files } => {
                    library
                        .library_statuses
                        .insert(path, LibraryStatus::Scanned(total_files));
                    library.scan_status = Some(format!("Scan complete: {total_files} tracks"));
                    // Scan batches already committed through the store as
                    // they progressed; nothing whole-file remains to save.
                }
                ScanOutcome::Failed { path, reason } => {
                    library.library_statuses.insert(path, LibraryStatus::Idle);
                    library.scan_status = Some(format!("Error: {reason}"));
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
        crate::ui::cover_placeholder::evict_generated(
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
        );
    }

    /// Drain polled Tag Edit outcomes from the service. On [`Saved`] the
    /// inline editor's draft closes (or the album batch tallies), and the
    /// status line reports the saved file; on `Failed` the draft keeps its
    /// inline reason — there is no silent-success path. All outcome
    /// application lives in the free inline outcome functions, which tests
    /// drive.
    fn poll_tag_edit_outcomes(&mut self, library: &mut LibrarySession) {
        while let Some(outcome) = self.tag_edits.poll() {
            // Outcomes carry no identity; the outstanding record captured at
            // submit time routes the outcome to its flow. Only one edit is
            // outstanding at a time: the album batch's record first, then the
            // single-track inline record.
            if self.inline_batch_in_flight.is_some() {
                apply_inline_batch_outcome(
                    outcome,
                    &mut self.inline_draft,
                    &mut self.inline_batch_in_flight,
                    &mut library.scan_status,
                );
            } else if self.inline_in_flight.is_some() {
                apply_inline_tag_edit_outcome(
                    outcome,
                    &mut self.inline_draft,
                    &mut self.inline_in_flight,
                    &mut library.scan_status,
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
            ctx,
        );
    }

    /// Validate the inline editor's draft and submit its requests through the
    /// Tag Edits seam, recording the outstanding record so the polled
    /// outcomes can be applied to the draft (and the status line) later: a
    /// single-Track draft's one request, or an Album draft's dirty-only batch.
    fn submit_inline_tag_edit(&mut self) {
        use crate::ui::selection::DraftKind;
        let Some(ref mut draft) = self.inline_draft else {
            return;
        };
        match draft.kind {
            DraftKind::Track => {
                submit_inline_tag_edit_fields(
                    draft,
                    self.tag_edits.as_ref(),
                    &mut self.inline_in_flight,
                );
            }
            DraftKind::Album => {
                submit_inline_batch_fields(
                    draft,
                    self.tag_edits.as_ref(),
                    &mut self.inline_batch_in_flight,
                );
            }
        }
    }

    /// Open the per-selection draft for the resolved readout: a Track draft
    /// for a track readout, an album batch draft for an album readout —
    /// targets resolved through the Session Views seam (the same source every
    /// readout reads).
    fn open_inline_draft(&mut self, content: &InspectorContent) {
        use crate::ui::selection::TagDraft;
        let tags = content.tags.clone();
        match content.kind {
            InspectorKind::Track => {
                if let Some(track) = content
                    .track_ids
                    .first()
                    .and_then(|id| self.views.selected_track(id))
                {
                    self.inline_draft = Some(TagDraft::for_track(
                        track.id.clone(),
                        track.file_path,
                        &tags,
                    ));
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
                self.inline_draft = Some(TagDraft::for_album(targets, &tags));
            }
            InspectorKind::Artist | InspectorKind::Genre => {}
        }
    }

    /// Resolve a cover texture through the shared cache, touching the LRU
    /// to mark it as recently used. A full miss (no real cover, no cached
    /// placeholder tile) resolves the shared music-icon placeholder tile
    /// into the cache — real art, when it arrives through the poll path,
    /// still wins.
    fn resolve_cover_texture(&mut self, ctx: &egui::Context, key: &str) -> egui::TextureHandle {
        let palette = self.theme.active;
        crate::ui::cover_placeholder::lookup_cover_texture(
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ctx,
            &palette,
            key,
        )
    }

    /// Attach the shared track context menu to `response`. See
    /// [`show_track_context_menu`] for the available actions.
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
        let inline_draft_slot = &mut self.inline_draft;
        let selected_slot = &mut library.selected_track;
        let playlist_store_slot = self.playlist_store.as_mut();
        show_track_context_menu(
            response,
            TrackMenuArgs {
                transport: self.transport.as_ref(),
                track_id,
                track,
                selected_track: selected_slot,
                inline_draft: inline_draft_slot,
                playlists,
                playlist_store: playlist_store_slot,
                remove_from_playlist,
            },
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

        self.request_cover(&track.id, &track.file_path);

        // Every library track row carries a leading cover tile: the real
        // cover when one is cached, otherwise the shared music-icon
        // placeholder — artless tracks read as a uniform tile instead of an
        // empty gap. The request above keeps filling the cache with real art
        // as it lands (the placeholder lives under a separate key).
        let cover = Some(self.resolve_cover_texture(ui.ctx(), &track.id.0).id());

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
                disclosure: None,
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
    /// Per-frame logic that also runs while the window is hidden (eframe 0.34
    /// calls `logic` before every `ui`, and on repaints while hidden). No UI
    /// may be shown here — only state checks and viewport commands.
    ///
    /// Implements close-to-tray on macOS/Windows (REQ-SI-001): an OS close
    /// (X / Alt+F4 / Cmd+Q) is vetoed and the window hides to the tray with
    /// playback continuing. A real quit (the tray Quit sets `quit_flag`) is
    /// always allowed through. On Linux there is no tray, so the default
    /// no-op `logic` applies and closing quits normally.
    #[cfg(not(target_os = "linux"))]
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let hidden = !ctx.input(|i| i.viewport().visible().unwrap_or(true));

        if self.quit_flag.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // While hidden ui() never runs, so keep a slow repaint loop alive to
        // keep observing the tray quit flag and visibility toggles.
        if hidden {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        // Tray Quit while hidden: ui() cannot observe quit_flag, so initiate
        // the close here. (When visible, the same check in ui() does it.)
        // Reconcile frontend-local visibility requests (drained from the tray's
        // own channel, Issue 03) with the real viewport visibility. No backend
        // state is touched — visibility is ephemeral frontend state.
        let want_visible = match self.visibility_listener.drain() {
            Some(m) => m.0,
            None => !hidden,
        };
        if want_visible && hidden {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        } else if !want_visible && !hidden {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // Close-to-tray: veto the OS close request and hide instead (frontend-
        // local; no backend state touched). This only runs when NOT quitting —
        // a quit-initiated close goes through above.
        if !self.quit_flag.load(Ordering::Relaxed) && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.quit_flag.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Clone both Arcs BEFORE locking: the guards borrow `self`, and the
        // whole frame below calls `self.<method>(...)` — exactly how the
        // pre-split frame loop handled `self.state`.
        let playback_arc = self.playback.clone();
        let library_arc = self.library.clone();

        if self.first_frame {
            self.prefs = Preferences::hydrate(
                &playback_arc,
                &library_arc,
                self.settings_store.as_ref(),
                self.transport.as_ref(),
            );
            self.first_frame = false;
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

        // Apply the active theme (REQ-UI-007 accessibility). Done after the
        // first-frame load so a persisted high-contrast choice takes effect on
        // the very first frame. High Contrast is a variant over the active
        // light/dark palette.
        self.apply_theme(ui.ctx(), library.ui_flags.high_contrast);

        // Drain the backend event inbox and route playback-error typed
        // notices to the status line (issue 01 seam fix) — the coordinator
        // no longer writes the library session's status slot directly.
        apply_backend_events(self.drain_backend_events(), &mut library.scan_status);

        self.poll_library_updates(&mut library);
        self.poll_tag_edit_outcomes(&mut library);
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
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let content = crate::ui::chrome::TitleBarContent {
                    scan_status: scan_status.as_deref(),
                    theme_dark: self.theme.dark,
                    advanced_mode: library.ui_flags.advanced_mode,
                    active_nav: crate::ui::chrome::NavDestination::active(
                        library.view_mode,
                        library.browse_mode,
                    ),
                };
                self.titlebar_actions.clear();
                crate::ui::chrome::show_titlebar(
                    ui,
                    &mut self.icons,
                    &self.theme.active,
                    &content,
                    &mut self.titlebar_actions,
                );
                for action in self.titlebar_actions.drain(..) {
                    apply_titlebar_action(action, ui.ctx(), &mut library, &mut self.theme);
                }
            });

        // Left 280px column: the library browser (search, Library/Folders
        // nav, playlists). Shared chrome per the mockup — present on every
        // view; only the main stage switches. The restyled content (issue 07)
        // keeps a 12px inset from the panel edge.
        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| {
                self.render_library_sidebar(ui, &mut library);
            });

        // Content top bar (handoff issue 06): the orange wordmark, the
        // global "Search or jump to…" field, and the list/grid view toggles —
        // a second content strip above the library stage (open decision 4
        // keeps the frameless chrome above it).
        if library.view_mode == ViewMode::Library {
            self.render_top_bar(ui, &mut library);
        }

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
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

// --- Per-frame helpers -------------------------------------------------------

/// Apply one [`crate::ui::chrome::TitleBarAction`] to app state and viewport
/// commands (Issue 06). Window controls route through the same vetoable
/// viewport commands as their issue-04 counterparts, so close-to-tray
/// (REQ-SI-001) keeps working from the custom chrome. Preference changes are
/// session writes only — the frame-end `Preferences` commit persists them.
fn apply_titlebar_action(
    action: crate::ui::chrome::TitleBarAction,
    ctx: &egui::Context,
    library: &mut LibrarySession,
    theme: &mut ThemeState,
) {
    use crate::ui::chrome::{NavDestination, TitleBarAction as Action, WindowControl};
    match action {
        Action::ToggleTheme => theme.dark = !theme.dark,
        Action::ToggleAdvanced => {
            library.ui_flags.advanced_mode = !library.ui_flags.advanced_mode;
        }
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
        Action::Close => ctx.send_viewport_cmd(WindowControl::Close.viewport_command()),
    }
}

/// Apply one [`crate::ui::browser::BrowserAction`] (handoff issue 08) to the
/// library session: the sort toggle and genre chips are session fields; a
/// row selection resolves per section into the [`BrowserSelection`] the
/// stage's root column consumes, landing at level 0 (`select_at(0, …)`) so
/// drill-down restarts from the root. Track rows select through
/// `library.selected_track` (the existing `interactive_track_row` flow), so
/// the All Tracks variant never sets a browser selection. The drill columns
/// (level 1 / 2) go through [`apply_drill_action`].
pub fn apply_browser_action(
    action: crate::ui::browser::BrowserAction,
    library: &mut LibrarySession,
) {
    use riff_backend::app::state::BrowserSelection;
    match action {
        crate::ui::browser::BrowserAction::ToggleSort => {
            library.browser_sort_desc = !library.browser_sort_desc;
        }
        crate::ui::browser::BrowserAction::Select(key) => {
            let selection = match library.library_section {
                LibrarySection::Artists => Some(BrowserSelection::Artist(key)),
                LibrarySection::Genres => Some(BrowserSelection::Genre(key)),
                LibrarySection::Albums => {
                    key.split_once('\u{1f}')
                        .map(|(artist, title)| BrowserSelection::Album {
                            artist: artist.to_owned(),
                            title: title.to_owned(),
                        })
                }
                LibrarySection::AllTracks => None,
            };
            if let Some(selection) = selection {
                library.select_at(0, selection);
            }
        }
    }
}

/// Apply one [`crate::ui::detail::DetailAction`] (handoff issue 09) to the
/// sessions and the store. `album_tracks` is the current album's track ids
/// in store order, resolved by the caller from the Session Views seam —
/// the play batch the header's two actions start. Playback follows the
/// folder-enqueue precedent: one `play_many` batch, never per-track sends.
pub fn apply_detail_action(
    action: crate::ui::detail::DetailAction,
    library: &mut LibrarySession,
    playback: &mut PlaybackSession,
    transport: &dyn Transport,
    library_mutations: &mut dyn LibraryMutationStore,
    album_tracks: &[TrackId],
) {
    use crate::ui::detail::DetailAction as Action;
    match action {
        // A breadcrumb segment at level `i` climbs the drill-down path back
        // to that level: level 0 empties the path (the section root).
        Action::Crumb(index) => library.truncate_path(index),
        // Entity rows no longer render inside the detail column — they are
        // their own columns in the elastic stage — so this action cannot
        // fire from the app. The widget seam keeps the variant for its
        // own contract (tests render rows directly).
        Action::SelectRow(_) => {}
        Action::PlayAll | Action::Shuffle => {
            // An empty album starts nothing — and never re-enables shuffle.
            if album_tracks.is_empty() {
                return;
            }
            if action == Action::Shuffle {
                // Shuffle state lives in the session queue (same as the
                // player bar's toggle); the batch itself is the ordinary
                // play_many.
                playback.queue.set_shuffle(true);
            }
            play_album_batch(album_tracks, transport);
        }
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
    }
}

/// Apply one [`crate::ui::selection::SelectionAction`] (handoff issue 10).
/// `track_ids` is the selection's track ids in store order, resolved by the
/// caller from the Session Views seam — the batch the panel's Play starts
/// (one `play_many` batch, never per-track sends) or the Queue action
/// appends (the context menu's per-track queue precedent). An empty batch
/// starts and queues nothing.
pub fn apply_selection_action(
    action: crate::ui::selection::SelectionAction,
    transport: &dyn Transport,
    track_ids: &[TrackId],
) {
    match action {
        crate::ui::selection::SelectionAction::PlayAlbum => {
            play_album_batch(track_ids, transport);
        }
        crate::ui::selection::SelectionAction::Queue => {
            for tid in track_ids {
                transport.add_to_queue(tid.clone());
            }
        }
        // The inline editor's Save/Cancel/StartEdit intents are consumed by
        // the selection panel before they ever reach this public seam (the
        // app layer owns the draft and the request), so they never carry a
        // track batch — and are never dispatched here.
        crate::ui::selection::SelectionAction::SaveTagEdit
        | crate::ui::selection::SelectionAction::CancelTagEdit
        | crate::ui::selection::SelectionAction::StartEdit => {}
    }
}

/// Apply a drill-down row selection from one of the elastic stage's entity
/// columns at `level` (1 or 2 — deeper than the root's level 0): the row
/// key follows the section's identity convention (`(album artist, title)`
/// composites for album rows, bare names for artist rows), and
/// [`LibrarySession::select_at`] truncates any deeper path entries. The
/// root column's selections keep going through [`apply_browser_action`]
/// (`select_at(0, …)`), so drill-down always restarts from the root.
pub fn apply_drill_action(
    section: LibrarySection,
    level: usize,
    key: String,
    library: &mut LibrarySession,
) {
    let selection = match (section, level) {
        // Album rows: an artist's albums (Artists, level 1) and an
        // artist's albums within a genre (Genres, level 2).
        (LibrarySection::Artists, 1) | (LibrarySection::Genres, 2) => {
            key.split_once('\u{1f}')
                .map(|(artist, title)| BrowserSelection::Album {
                    artist: artist.to_owned(),
                    title: title.to_owned(),
                })
        }
        // Artist rows: the artists carrying a genre (Genres, level 1).
        (LibrarySection::Genres, 1) => Some(BrowserSelection::Artist(key)),
        _ => None,
    };
    if let Some(selection) = selection {
        library.select_at(level, selection);
    }
}

/// Start an album batch: its first track plays, the rest queue behind it —
/// exactly [`play_folder`]'s gesture. An empty album starts nothing (and
/// never re-enables shuffle).
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
    pub breadcrumb: Vec<crate::ui::detail::Crumb>,
    pub header: Option<crate::ui::detail::AlbumHeader>,
    pub tracks: Vec<crate::ui::detail::TrackRow>,
}

/// The kinds of list column the elastic stage renders for
/// [`BrowseMode::Library`] sections. Entity listings below the album level
/// are their own columns now; the Tracks column is the existing
/// `DetailColumn` shape (breadcrumb + album header + track table).
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
    /// The Tracks column: breadcrumb, album header, and track table.
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

/// The elastic stage's column sizing policy (elastic-column spec): non-last
/// list columns keep their preferred [`theme::COLUMN_WIDTH`], the last list
/// column absorbs the remaining width, and the inspector (when visible)
/// takes [`theme::INSPECTOR_WIDTH`] off the top. When the width left for
/// the list columns cannot satisfy the minimum floors ([`theme::COLUMN_MIN_W`]
/// per entity column, [`theme::LAST_COLUMN_MIN_W`] for the last), every
/// column shrinks proportionally to its floor — the stage never introduces
/// horizontal scrolling, accepting below-floor widths only in extreme narrow
/// windows. Returns one width per list column (the inspector is separate).
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a column count is a small non-negative number"
)]
pub fn column_widths(available: f32, list_columns: usize, inspector: bool) -> Vec<f32> {
    let inspector_w = if inspector {
        theme::INSPECTOR_WIDTH
    } else {
        0.0
    };
    let available_lists = (available - inspector_w).max(0.0);
    if list_columns == 0 {
        return Vec::new();
    }
    // Preferred widths: non-last columns at COLUMN_WIDTH, the last column
    // absorbing the remainder.
    let mut widths = vec![theme::COLUMN_WIDTH; list_columns - 1];
    widths.push(available_lists - theme::COLUMN_WIDTH * (list_columns - 1) as f32);
    if list_columns == 1 {
        // A single column fills the stage; floors do not apply.
        return widths;
    }
    let floors = theme::COLUMN_MIN_W * (list_columns - 1) as f32 + theme::LAST_COLUMN_MIN_W;
    if available_lists < floors {
        // Narrow window: shrink every column proportionally to its floor.
        let scale = available_lists / floors;
        for (i, width) in widths.iter_mut().enumerate() {
            let floor = if i + 1 == list_columns {
                theme::LAST_COLUMN_MIN_W
            } else {
                theme::COLUMN_MIN_W
            };
            *width = floor * scale;
        }
    } else if let Some(last) = widths.last_mut()
        && *last < theme::LAST_COLUMN_MIN_W
    {
        // The remainder is enough for the floors, but the last column's
        // remainder would land below its floor: the non-last columns (which
        // have headroom above their own floors) yield width toward the last
        // column's floor first, then the last column absorbs the rest.
        let non_last_total = theme::COLUMN_WIDTH * (list_columns - 1) as f32;
        let headroom = non_last_total - theme::COLUMN_MIN_W * (list_columns - 1) as f32;
        let give = (theme::LAST_COLUMN_MIN_W - *last).min(headroom);
        *last += give;
        let scale = (available_lists - *last) / non_last_total;
        for width in widths.iter_mut().take(list_columns - 1) {
            *width *= scale;
        }
    }
    widths
}

/// Resolve what the Tracks column renders for the current drill-down path:
/// the breadcrumb trail (section root, then one crumb per path entry), and
/// on the album level the album header plus its track list — genre-scoped
/// in the Genres section. Entity listings below the album level are their
/// own columns in the stage, so this resolver carries no rows.
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

    let root = match library.library_section {
        LibrarySection::Genres => "Genres",
        LibrarySection::Albums => "Albums",
        _ => "Artists",
    };
    let mut content = DetailContent {
        breadcrumb: vec![crate::ui::detail::Crumb {
            label: root.to_string(),
        }],
        ..DetailContent::default()
    };
    for entry in &library.browser_path {
        let label = match entry {
            BrowserSelection::Artist(name) => name.clone(),
            BrowserSelection::Genre(genre) => genre.clone(),
            BrowserSelection::Album { title, .. } => title.clone(),
        };
        content.breadcrumb.push(crate::ui::detail::Crumb { label });
    }
    let Some(BrowserSelection::Album { artist, title }) = library.current_selection() else {
        return content;
    };
    // The album's year comes from its entry in the artist's album table
    // (the store derives it from the first-added track).
    let year = views
        .artist_albums(artist)
        .iter()
        .find(|album| &album.title == title)
        .and_then(|album| album.year);
    content.header = Some(crate::ui::detail::AlbumHeader {
        title: title.clone(),
        subtitle: Some(year.map_or_else(|| artist.clone(), |y| format!("{artist} \u{b7} {y}"))),
    });
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
    /// The selection's track ids in store order — the batch Play and
    /// Add to Queue actions start.
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
    let details = vec![
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
    // table (the same source the detail column's header uses).
    let albums = views.artist_albums(artist);
    let album = albums.iter().find(|album| album.title == title);
    let details = vec![
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

/// Apply one [`crate::ui::topbar::TopBarAction`] (handoff issue 06): the
/// layout lands on the library session — the browser column (issue 08) reads
/// it, and the frame-end `Preferences` commit makes the choice survive
/// restarts.
pub fn apply_top_bar_action(action: crate::ui::topbar::TopBarAction, library: &mut LibrarySession) {
    match action {
        crate::ui::topbar::TopBarAction::SetLayout(layout) => {
            library.browser_layout = layout;
        }
    }
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

    /// Content top bar (handoff issue 06): the orange wordmark, the global
    /// "Search or jump to…" field, and the list/grid view toggles, drawn by
    /// the [`crate::ui::topbar`] widgets inside a 48px top panel. The search
    /// field edits the session's `search_query` directly, so typing filters
    /// the whole library; toggles land on the persisted browser layout via
    /// [`apply_top_bar_action`], and a pending Ctrl+K request focuses the
    /// field on the frame it lands.
    fn render_top_bar(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        egui::Panel::top("top_bar")
            .exact_size(theme::TOPBAR_H)
            .show(ui, |ui| {
                let content = crate::ui::topbar::TopBarContent {
                    layout: library.browser_layout,
                };
                self.topbar_actions.clear();
                let search_response = crate::ui::topbar::show_top_bar(
                    ui,
                    &mut self.icons,
                    &self.theme.active,
                    &mut library.search_query,
                    content,
                    &mut self.topbar_actions,
                );
                // Ctrl+K landed: focus the global search field this frame.
                if self.global_search_focus {
                    search_response.request_focus();
                    self.global_search_focus = false;
                }
            });
        for action in self.topbar_actions.drain(..) {
            apply_top_bar_action(action, library);
        }
    }

    /// Bottom shell strip (Issues 06 + 08): transport, seek row, and volume
    /// at the exact 88px playerbar token height, drawn by the restyled
    /// playerbar widgets. Every reported [`crate::ui::playerbar::
    /// PlayerBarAction`] routes through [`apply_player_bar_action`], so each
    /// control still emits its engine command.
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
            self.request_cover(&id, &file_path);
            cover = Some(self.resolve_cover_texture(ui.ctx(), &id.0));
        }

        // The `{index}/{len}` queue-position label, formatted fresh each
        // frame from the live queue shape.
        let queue_position = format!(
            "{}/{}",
            playback.queue.current_index.map_or(0, |i| i + 1),
            playback.queue.tracks.len()
        );
        self.playerbar_readouts.sync(
            playback.current_position.current,
            playback.current_position.total,
        );
        let content = crate::ui::playerbar::PlayerBarContent {
            cover,
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
        let counts = self.views.sidebar_counts(library.library_paths.len());

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
                    disclosure: None,
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
                disclosure: None,
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
                    disclosure: None,
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
            return;
        }

        // Grid mode (handoff issue 08): the same paged tracks as cover
        // tiles. The list mode below keeps the interactive rows —
        // double-click play, context menus, per-column readouts — until the
        // detail column provides its own actions.
        if library.browser_layout == riff_backend::app::state::BrowserLayout::Grid {
            self.render_flat_grid(ui, library, query, current_track.as_ref());
            return;
        }

        egui::ScrollArea::vertical().show_rows(
            ui,
            crate::ui::sidebar::ROW_H,
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
    }

    /// Render the tracks of a read-only smart playlist. The list reads
    /// through the Session Views seam over store queries (ADR 0002): every
    /// committed mutation bumps the generation, so the next frame
    /// regenerates from committed state — no manual refresh needed.
    fn render_smart_playlist_view(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
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
            show_list_context_menu(&header.response, self.transport.as_ref(), &tids);
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
            crate::ui::sidebar::ROW_H,
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
        playback: &PlaybackSession,
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
            show_list_context_menu(&header.response, self.transport.as_ref(), &valid_ids);
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
            crate::ui::sidebar::ROW_H,
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
    /// via [`commit_playlist_reorder`] (ADR 0002); clicks, double-clicks,
    /// and the shared context menu behave exactly as before.
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

        self.request_cover(&track.id, &track.file_path);

        // Same leading cover tile as the library rows: real art when cached,
        // otherwise the shared music-icon placeholder for artless tracks.
        let cover = Some(self.resolve_cover_texture(ui.ctx(), &track.id.0).id());

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
                disclosure: None,
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
                self.request_cover(&id, &file_path);
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
            .map(|key| self.resolve_cover_texture(ui.ctx(), key));
        self.now_playing_cover_key = cover_key;
        // Text block + Up Next rows formatted straight from the playback
        // projection's resolved tracks each frame; staleness is the
        // projection's job, not the widget layer's.
        let (title, meta_line, details) = match self.views.playback_current() {
            Some(track) => (
                Some(Arc::from(track.metadata.display_title(&track.file_path))),
                Some(Arc::from(format!(
                    "{} - {}",
                    track.metadata.display_artist(),
                    track.metadata.display_album()
                ))),
                crate::ui::now_playing::metadata_details(&track.metadata).map(Arc::from),
            ),
            None => (None, None, None),
        };
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
        playback: &PlaybackSession,
        query: &str,
    ) {
        if library.library_paths.is_empty() {
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
        let lib_paths = library.library_paths.clone();
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
        playback: &PlaybackSession,
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

        let row = sidebar::tree_row(
            ui,
            &mut self.icons,
            &palette,
            TreeRow {
                indent_level: level,
                icon: Some(if collapsing.is_open() {
                    Icon::FolderOpen
                } else {
                    Icon::Folder
                }),
                cover: None,
                label: &label,
                count: None,
                meta: None,
                favorite: None,
                selected: is_selected,
                now_playing: false,
                playing: false,
                disclosure: Some(collapsing.is_open()),
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
            show_list_context_menu(&row.response, self.transport.as_ref(), &folder_track_ids);
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
    }
}

/// Parse an optional numeric tag field; empty input means "leave unset".
fn parse_number(label: &str, raw: &str) -> Result<Option<u32>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        trimmed
            .parse::<u32>()
            .map(Some)
            .map_err(|_| format!("{label} must be a whole number"))
    }
}

/// The UI's whole remaining cover responsibility (ADR 0006): ask the Cover
/// Service for art unless the texture is already in the UI-owned cache.
/// Free function so tests drive the exact production path without a window.
pub fn request_cover_intent(
    texture_cached: bool,
    covers: &dyn Covers,
    track_id: TrackId,
    path: PathBuf,
) {
    if !texture_cached {
        covers.request(track_id, path);
    }
}

/// Apply one polled Tag Edit outcome to the inline editor's draft, its
/// outstanding request record, and the status line — the same code path
/// `poll_tag_edit_outcomes` runs per outcome. `Saved` closes the matching
/// draft and reports the saved file; `Failed` keeps the editor open with the
/// reason inline. Outcomes carry no identity, so the record captured at
/// submit time supplies both the match key and the file name; an outcome
/// with no matching outstanding request is ignored. A draft that has already
/// been discarded (the selection moved) never comes back: the status line
/// still reports the outcome, the editor simply stays gone.
pub fn apply_inline_tag_edit_outcome(
    outcome: TagEditOutcome,
    draft: &mut Option<crate::ui::selection::TagDraft>,
    in_flight: &mut Option<(TrackId, PathBuf)>,
    scan_status: &mut Option<String>,
) {
    let Some((track_id, path)) = in_flight.take() else {
        return;
    };
    match outcome {
        TagEditOutcome::Saved => {
            let name = path.file_name().map_or_else(
                || path.to_string_lossy().to_string(),
                |n| n.to_string_lossy().to_string(),
            );
            *scan_status = Some(format!("Tags saved for {name}"));
            tracing::info!("Tags written for {:?}", path);
            if draft.as_ref().is_some_and(|d| d.track_id == track_id) {
                *draft = None;
            }
        }
        TagEditOutcome::Failed { reason } => {
            tracing::warn!("Tag edit failed for {:?}: {}", path, reason);
            if let Some(d) = draft.as_mut()
                && d.track_id == track_id
            {
                d.error = Some(reason);
                d.saving = false;
            }
        }
    }
}

/// Apply drained backend events to session state (issue 01 seam fix).
/// Playback errors arrive as typed notices with playback source — the
/// coordinator no longer writes the library session's status slot directly —
/// so the UI routes them to the titlebar status line here, preserving the
/// exact visible string. Other event kinds carry no UI state change yet.
pub fn apply_backend_events(
    events: Vec<riff_backend::app::events::BackendEvent>,
    scan_status: &mut Option<String>,
) {
    use riff_backend::app::events::{BackendEvent, NoticeSource};
    for event in events {
        if let BackendEvent::TypedNotice(payload) = event
            && payload.source == NoticeSource::Playback
        {
            *scan_status = Some(payload.message);
        }
    }
}

/// Validate the inline editor's draft and submit one [`TagEditRequest`]
/// through the service seam — the same durable path single-Track saves use
/// today (ADR 0006). Invalid numeric fields keep the draft open with an
/// inline reason and submit nothing; valid fields clear the error, flip the
/// draft into its saving state, and record the outstanding request so its
/// outcome can be matched later. Nothing is ever written without an explicit
/// Save upstream.
pub fn submit_inline_tag_edit_fields(
    draft: &mut crate::ui::selection::TagDraft,
    tag_edits: &dyn TagEdits,
    in_flight: &mut Option<(TrackId, PathBuf)>,
) {
    use crate::ui::selection::TagField;
    match (
        parse_number("Year", &draft.fields[TagField::Year.index()]),
        parse_number("Track number", &draft.fields[TagField::TrackNumber.index()]),
    ) {
        (Ok(year), Ok(track_number)) => {
            draft.error = None;
            draft.saving = true;
            let request = TagEditRequest {
                track_id: draft.track_id.clone(),
                path: draft.path.clone(),
                edit: TagEdit {
                    title: Some(draft.fields[TagField::Title.index()].clone()),
                    artist: Some(draft.fields[TagField::Artist.index()].clone()),
                    album: Some(draft.fields[TagField::Album.index()].clone()),
                    album_artist: Some(draft.fields[TagField::AlbumArtist.index()].clone()),
                    genre: Some(draft.fields[TagField::Genre.index()].clone()),
                    year,
                    track_number,
                    ..Default::default()
                },
            };
            *in_flight = Some((request.track_id.clone(), request.path.clone()));
            tag_edits.submit(request);
        }
        (Err(error), _) | (_, Err(error)) => {
            draft.error = Some(error);
        }
    }
}

/// The album batch's outstanding requests, in submission order — the worker
/// serializes the batch, so polled outcomes arrive in the same order and each
/// lands on the record popped first. The tallies feed the draft's
/// [`crate::ui::selection::BatchStatus`] the Save bar renders.
#[derive(Debug, Clone)]
pub struct BatchInFlight {
    pending: std::collections::VecDeque<(TrackId, PathBuf)>,
    total: usize,
    saved: usize,
    failed: usize,
    first_failure: Option<String>,
}

impl BatchInFlight {
    fn new(pending: std::collections::VecDeque<(TrackId, PathBuf)>) -> Self {
        let total = pending.len();
        Self {
            pending,
            total,
            saved: 0,
            failed: 0,
            first_failure: None,
        }
    }
}

/// Submit the album draft's dirty-only batch: one [`TagEditRequest`] per
/// album Track, carrying only the fields whose typed text differs from the
/// row's originally displayed value (`Some`) — a `(different)` row left with
/// an empty input is untouched, so it is skipped and can never blank a tag
/// on every Track; clearing a shared tag stays a single-Track action. A
/// fully untouched editor submits nothing and writes no files (Save is
/// disabled while nothing is dirty). Invalid numeric fields keep the draft
/// open with an inline reason and submit nothing.
pub fn submit_inline_batch_fields(
    draft: &mut crate::ui::selection::TagDraft,
    tag_edits: &dyn TagEdits,
    batch: &mut Option<BatchInFlight>,
) {
    use crate::ui::selection::TagField;

    if !draft.any_dirty() {
        return;
    }
    // Only a dirty numeric field is parsed: an untouched Year/Track Number
    // stays `None` in every request rather than being rewritten.
    let year = if draft.is_dirty(TagField::Year) {
        match parse_number("Year", &draft.fields[TagField::Year.index()]) {
            Ok(y) => y,
            Err(error) => {
                draft.error = Some(error);
                return;
            }
        }
    } else {
        None
    };
    let track_number = if draft.is_dirty(TagField::TrackNumber) {
        match parse_number("Track number", &draft.fields[TagField::TrackNumber.index()]) {
            Ok(n) => n,
            Err(error) => {
                draft.error = Some(error);
                return;
            }
        }
    } else {
        None
    };
    let value = |field: TagField| -> Option<String> {
        draft
            .is_dirty(field)
            .then(|| draft.fields[field.index()].clone())
    };
    let pending = draft
        .album_tracks
        .iter()
        .map(|(track_id, path)| {
            let request = TagEditRequest {
                track_id: track_id.clone(),
                path: path.clone(),
                edit: TagEdit {
                    title: value(TagField::Title),
                    artist: value(TagField::Artist),
                    album: value(TagField::Album),
                    album_artist: value(TagField::AlbumArtist),
                    genre: value(TagField::Genre),
                    year,
                    track_number,
                    ..Default::default()
                },
            };
            tag_edits.submit(request);
            (track_id.clone(), path.clone())
        })
        .collect::<std::collections::VecDeque<_>>();

    draft.error = None;
    draft.saving = true;
    draft.batch = Some(crate::ui::selection::BatchStatus {
        total: pending.len(),
        saved: 0,
        failed: 0,
        first_failure: None,
    });
    *batch = Some(BatchInFlight::new(pending));
}

/// Apply one polled Tag Edit outcome to the album batch: each outcome lands
/// on the next pending request in submission order (the worker serializes
/// the batch), updating the draft's tallies and the status line — "Tags
/// saved for X" per save, the failure reason per failure, exactly the
/// per-request surface single-Track saves use. When the last outcome lands
/// the draft stops saving, its [`crate::ui::selection::BatchStatus`] turns
/// done, and the Save bar shows the "Saved N of M tracks" summary (orange
/// when any failed). A draft that has already been discarded (the selection
/// moved) never comes back; the status line still reports each outcome.
pub fn apply_inline_batch_outcome(
    outcome: TagEditOutcome,
    draft: &mut Option<crate::ui::selection::TagDraft>,
    batch: &mut Option<BatchInFlight>,
    scan_status: &mut Option<String>,
) {
    let Some(in_flight) = batch.as_mut() else {
        return;
    };
    let Some((_track_id, path)) = in_flight.pending.pop_front() else {
        return;
    };
    match outcome {
        TagEditOutcome::Saved => {
            in_flight.saved += 1;
            let name = path.file_name().map_or_else(
                || path.to_string_lossy().to_string(),
                |n| n.to_string_lossy().to_string(),
            );
            *scan_status = Some(format!("Tags saved for {name}"));
        }
        TagEditOutcome::Failed { reason } => {
            in_flight.failed += 1;
            in_flight.first_failure.get_or_insert(reason.clone());
            *scan_status = Some(reason);
        }
    }
    if let Some(d) = draft.as_mut() {
        d.batch = Some(crate::ui::selection::BatchStatus {
            total: in_flight.total,
            saved: in_flight.saved,
            failed: in_flight.failed,
            first_failure: in_flight.first_failure.clone(),
        });
    }
    if in_flight.pending.is_empty() {
        *batch = None;
        if let Some(d) = draft.as_mut() {
            d.saving = false;
        }
    }
}

/// Whether the open inline draft still belongs to the resolved readout: the
/// draft never outlives its selection — a changed selection discards it so a
/// stale half-typed edit can never leak onto a different Track or Album
/// (ticket 03). An Album draft belongs to the album whose track batch it
/// targets; an Artist/Genre readout never hosts one.
pub fn inline_draft_is_current(
    draft: &crate::ui::selection::TagDraft,
    content: &InspectorContent,
) -> bool {
    use crate::ui::selection::DraftKind;
    match (draft.kind, content.kind) {
        (DraftKind::Track, InspectorKind::Track) => {
            content.track_ids.first() == Some(&draft.track_id)
        }
        (DraftKind::Album, InspectorKind::Album) => {
            let ids: Vec<&TrackId> = draft.album_tracks.iter().map(|(id, _)| id).collect();
            content.track_ids.iter().collect::<Vec<_>>() == ids
        }
        _ => false,
    }
}

/// Consume polled cover results into the UI texture cache: decode + rgba→
/// texture conversion is the egui-bound work that stays on the main thread
/// (the texture boundary, ADR 0006; the cover port hands out still-encoded
/// bytes plus their container format); dedup and negative caching live
/// behind the service seam, so artless results are simply dropped here.
pub fn cache_polled_covers<S: std::hash::BuildHasher>(
    covers: &dyn Covers,
    textures: &mut std::collections::HashMap<String, egui::TextureHandle, S>,
    lru_keys: &mut Vec<String>,
    ctx: &egui::Context,
) {
    for (track_id, cover_image) in covers.poll() {
        let Some(cover_image) = cover_image else {
            continue; // artless: the service negative-caches it
        };
        let image_format = match cover_image.format {
            riff_backend::app::traits::CoverImageFormat::Jpeg => image::ImageFormat::Jpeg,
            riff_backend::app::traits::CoverImageFormat::Png => image::ImageFormat::Png,
        };
        let Ok(decoded) = image::load_from_memory_with_format(&cover_image.data, image_format)
            .map_err(|e| tracing::warn!("Failed to decode cover for {}: {e}", track_id.0))
        else {
            continue;
        };
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        let color_image =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        let texture = ctx.load_texture(&track_id.0, color_image, egui::TextureOptions::default());
        textures.insert(track_id.0.clone(), texture);
        for old in lru_insert(lru_keys, track_id.0, COVER_CACHE_CAP) {
            textures.remove(&old);
        }
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

/// Arguments for the shared track context menu, grouped into one value to
/// keep the call sites readable.
struct TrackMenuArgs<'a> {
    transport: &'a dyn Transport,
    track_id: &'a TrackId,
    /// The track itself; `None` (e.g. a playlist entry whose file is missing)
    /// suppresses playback actions and "Edit Tags".
    track: Option<&'a Track>,
    /// The selection slot: the "Edit Tags" item selects the Track before the
    /// inline editor opens, so the readout follows the entry point.
    selected_track: &'a mut Option<TrackId>,
    /// The inline editor's draft slot: "Edit Tags" opens the per-selection
    /// draft for that track (the retired modal's entry point, now un-gated).
    inline_draft: &'a mut Option<crate::ui::selection::TagDraft>,
    /// The seam's `Arc`'d playlist snapshot, cloned out before rendering —
    /// it only names the "Add to Playlist" targets; mutations commit through
    /// the store and the projection invalidates itself.
    playlists: Arc<[Playlist]>,
    /// The Application Store's playlists section: entry mutations commit
    /// through it as one immediate durable transaction.
    playlist_store: &'a mut dyn PlaylistStore,
    /// When `Some`, adds a "Remove from Playlist" action for that playlist.
    remove_from_playlist: Option<&'a PlaylistId>,
}

/// Shared track context menu: play / play next / add to queue, "Add to
/// Playlist", optional "Remove from Playlist", and "Edit Tags" — the inline
/// editor's entry point, available for every Track (REQ-UI-006 revoked).
/// Queue actions are suppressed when the file is missing (`track` is
/// `None`).
fn show_track_context_menu(response: &egui::Response, args: TrackMenuArgs<'_>) {
    let TrackMenuArgs {
        transport,
        track_id,
        track,
        selected_track,
        inline_draft,
        playlists,
        playlist_store,
        remove_from_playlist,
    } = args;
    let tid = track_id.clone();
    let playable = track.is_some();
    // The inline editor opens for any track, gated or not (Issue 04).
    let edit_track = track.cloned();
    let remove_pid = remove_from_playlist.cloned();
    let playlist_options: Vec<(PlaylistId, String)> = playlists
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    response.context_menu(move |ui| {
        if playable {
            if ui.button("Play").clicked() {
                transport.play(tid.clone());
                ui.close();
            }
            if ui.button("Play Next").clicked() {
                transport.play_next(tid.clone());
                ui.close();
            }
            if ui.button("Add to Queue").clicked() {
                transport.add_to_queue(tid.clone());
                ui.close();
            }
            add_to_playlist_menu(ui, &playlist_options, playlist_store, &tid);
        }
        if let Some(ref pid) = remove_pid
            && ui.button("Remove from Playlist").clicked() {
                // One immediate durable transaction; the committed mutation
                // bumps the playlist generation, so the seam's next read
                // reflects the removal with zero caller action (ADR 0002).
                if let Err(e) = playlist_store.remove_playlist_entries(pid, &tid) {
                    tracing::warn!("Failed to remove playlist entry: {e}");
                }
                ui.close();
            }
        // "Edit Tags" is the inline editor's entry point: it selects the
        // Track (so the Detail Panel shows its readout) and opens the
        // per-selection draft focused on the first tag field (Issue 04).
        if let Some(ref t) = edit_track
            && ui
                .button("Edit Tags")
                .on_hover_text(
                    "Edit this track's tags (title, artist, album, and more). Changes are written to the file on Save.",
                )
                .clicked()
            {
                *selected_track = Some(tid.clone());
                let rows = tag_rows(std::slice::from_ref(t));
                *inline_draft = Some(crate::ui::selection::TagDraft::for_track(
                    t.id.clone(),
                    t.file_path.clone(),
                    &rows,
                ));
                if let Some(draft) = inline_draft.as_mut() {
                    draft.focus_first = true;
                }
                ui.close();
            }
    });
}

/// Shared whole-list context menu (playlist/folder headers): Play (first
/// track, then queue the rest), Play Next, and Append to Queue.
fn show_list_context_menu(
    response: &egui::Response,
    transport: &dyn Transport,
    track_ids: &[TrackId],
) {
    let tids = track_ids.to_vec();
    response.context_menu(move |ui| {
        if ui.button("Play").clicked() {
            if let Some(first) = tids.first() {
                transport.play(first.clone());
                for tid in &tids[1..] {
                    transport.add_to_queue(tid.clone());
                }
            }
            ui.close();
        }
        if ui.button("Play Next").clicked() {
            for tid in tids.iter().rev() {
                transport.play_next(tid.clone());
            }
            ui.close();
        }
        if ui.button("Append to Queue").clicked() {
            for tid in &tids {
                transport.add_to_queue(tid.clone());
            }
            ui.close();
        }
    });
}

/// "Add to Playlist" submenu shared by the track context menus (Task 4.2).
/// Clicking a playlist appends the track (exact duplicates ignored) as one
/// immediate durable transaction, so the change survives a restart. Takes
/// only the precomputed options and the store: the committed append bumps
/// the playlist generation, so the seam's next read reflects it with zero
/// caller action — nothing to patch or clear here.
fn add_to_playlist_menu(
    ui: &mut egui::Ui,
    playlist_options: &[(PlaylistId, String)],
    store: &mut dyn PlaylistStore,
    track_id: &TrackId,
) {
    ui.menu_button("Add to Playlist", |ui| {
        if playlist_options.is_empty() {
            ui.label("No playlists yet");
            return;
        }
        for (pid, pname) in playlist_options {
            if ui.button(pname).clicked() {
                if let Err(e) = store.add_playlist_entry(pid, track_id) {
                    tracing::warn!("Failed to add playlist entry: {e}");
                }
                ui.close();
            }
        }
    });
}

/// The shared `mm:ss` time-readout format now lives with the playerbar
/// widgets that render it (Issue 08); re-exported here so existing callers
/// and the test prelude keep their stable path.
pub use crate::ui::playerbar::format_duration;
