use crate::app::MutexExt;
use crate::app::commands::{LibraryCommand, LibraryUpdate};
// The cover-cache capacity discipline lives in the app layer next to the
// Cover Service's negative cache, so both caches share ONE constant and ONE
// LRU implementation (spec: no second constant scheme).
use crate::app::cover_service::Covers;
pub use crate::app::cover_service::{COVER_CACHE_CAP, lru_insert};
use crate::app::state::{AppState, BrowseMode, LibraryStatus, ViewMode};
use crate::app::store::{LibraryMutationStore, PlaylistStore, SettingsStore};
use crate::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
use crate::app::traits::TagEdit;
use crate::app::views::SessionViews;
use crate::app::watcher_manager::WatcherManager;
use crate::domain::{
    Album, Artist, PlaybackCommand, PlaybackState, Playlist, PlaylistId, SmartPlaylistKind, Track,
    TrackId,
};
use crate::ui::chrome::TitleBarAction;
use crate::ui::now_playing::{NowPlayingAction, UpNextEntry};
use crate::ui::playerbar::PlayerBarAction;
use crate::ui::theme;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Transient UI state for the "Edit Tags" modal. Lives on `RiffApp` (not
/// `AppState`), following the `settings_text_input` precedent. Public so the
/// pre-fill contract (REQ-ML-008) is testable; only constructed by the UI.
pub struct TagEditState {
    pub track_id: TrackId,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: String,
    pub track_number: String,
    pub error: Option<String>,
    pub saving: bool,
}

impl TagEditState {
    /// Pre-populate the editable fields from the track's current metadata.
    pub fn from_track(track: &Track) -> Self {
        Self {
            track_id: track.id.clone(),
            path: track.file_path.clone(),
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
            error: None,
            saving: false,
        }
    }
}

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

/// The lazily computed display labels for one track (allocation plan 2.1):
/// each variant is formatted at most once per store generation and then
/// handed out as an `Arc` clone. `None` means "not computed yet".
#[derive(Default)]
struct CachedTrackLabels {
    /// `"Artist - Title"` — flat list, search, playlists, window title.
    artist_title: Option<Arc<str>>,
    /// `"N. Title"` — album and folder tree rows.
    numbered: Option<Arc<str>>,
}

/// The cached Up Next rows for the Now Playing stage (allocation plan 2.2),
/// keyed by the store generation plus the exact Up Next window contents:
/// queue labels only change when the playback projection's window does.
struct UpNextLabels {
    generation: u64,
    /// The window ids the entries were built from; compared by reference
    /// against the live projection window so fresh frames allocate nothing.
    ids: Vec<TrackId>,
    entries: Arc<[UpNextEntry]>,
}

/// The Now Playing stage's text block (allocation plan 2.2), keyed by
/// current `TrackId` + generation: title, `"Artist - Album"` meta line, and
/// the joined details line.
struct NpTextCache {
    generation: u64,
    id: Option<TrackId>,
    title: Option<Arc<str>>,
    meta_line: Option<Arc<str>>,
    details: NpDetails,
}

/// Tri-state for the cached details line: computed-absent (the track has no
/// detail fields, or no track is playing) or the line itself.
enum NpDetails {
    Absent,
    Line(Arc<str>),
}

impl NpDetails {
    fn from_option(line: Option<String>) -> Self {
        match line {
            Some(line) => Self::Line(Arc::from(line)),
            None => Self::Absent,
        }
    }
}

/// The Now Playing stage's per-frame text handout — `Arc` clones out of
/// [`NpTextCache`], so fresh frames bump refcounts instead of formatting.
struct NpText {
    title: Option<Arc<str>>,
    meta_line: Option<Arc<str>>,
    details: Option<Arc<str>>,
}

pub struct RiffApp {
    pub state: Arc<Mutex<AppState>>,
    command_sender: Option<Sender<PlaybackCommand>>,
    library_command_sender: Option<Sender<LibraryCommand>>,
    library_update_rx: Option<Receiver<LibraryUpdate>>,
    cover_textures: std::collections::HashMap<String, egui::TextureHandle>,
    cover_lru_keys: Vec<String>,
    /// The Cover Service front end (ADR 0006): sends resolve intent and
    /// yields drained results; dedup and the negative cache live behind it.
    covers: Box<dyn Covers>,
    /// The Tag Edit Service front end (ADR 0006): submits save intent and
    /// yields polled outcomes; the whole save flow lives behind it.
    tag_edits: Box<dyn TagEdits>,
    /// The one Tag Edit currently outstanding, recorded at submit time so a
    /// polled outcome can be matched back to its modal (and its file name
    /// shown in the status line) — outcomes themselves carry no identity.
    tag_edit_in_flight: Option<(TrackId, PathBuf)>,
    tag_edit: Option<TagEditState>,
    /// Which read-only smart playlist is open in the library explorer, if any.
    /// Transient UI state (precedent: `tag_edit`); the playlist contents are
    /// re-computed from library data on every frame, so nothing is cached.
    smart_playlist_view: Option<SmartPlaylistKind>,
    /// Which user playlist is open in the library explorer, if any.
    playlist_view: Option<PlaylistId>,
    /// Store generation the [`Self::row_labels`] cache was filled at; a
    /// mismatch clears it (allocation plan 2.1).
    row_label_generation: u64,
    /// Per-track display labels, computed at most once per store generation
    /// and handed out as `Arc` clones — kills the per-row `format!` class.
    row_labels: HashMap<TrackId, CachedTrackLabels>,
    /// Sidebar playlist-row labels ("Name (count)"), keyed by playlist id
    /// and rebuilt only when the row's name or entry count moves. Entries
    /// for deleted playlists linger until their id is reused — bounded by
    /// playlists created this session and never looked up.
    playlist_labels: HashMap<PlaylistId, (String, usize, Arc<str>)>,
    /// The Now Playing stage's cached text block — see [`NpTextCache`].
    np_text_cache: Option<NpTextCache>,
    /// The playing track's id whose cover the Now Playing stage last
    /// requested/rendered; only re-cloned when the track moves.
    now_playing_cover_key: Option<String>,
    /// The cached Up Next rows for the Now Playing stage — see
    /// [`UpNextLabels`].
    up_next_cache: Option<UpNextLabels>,
    /// The (current `TrackId`, generation) pair the window title and tray
    /// tooltip were last pushed for (allocation plan 2.3): both are only
    /// rebuilt when this identity moves.
    last_title_key: Option<(Option<TrackId>, u64)>,
    /// The `{index}/{len}` queue-position label, rebuilt only when either
    /// component moves (allocation plan 2.4).
    queue_position_cache: (Option<usize>, usize, Option<Arc<str>>),
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
    /// Transient "New Playlist" name prompt (`Some` = open, holds the draft).
    playlist_create_name: Option<String>,
    /// Transient rename prompt: (playlist id, draft name).
    playlist_rename: Option<(PlaylistId, String)>,
    /// Transient Clear Library confirmation (`true` = awaiting confirm).
    /// Grouped with the other transient prompts on `RiffApp`.
    pub(crate) clear_library_confirm: bool,
    search_focus: bool,
    first_frame: bool,
    pub(crate) watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
    /// The Application Store's settings section. The UI reads settings from
    /// it on the first frame and writes every preference change straight
    /// back, so preferences survive restarts through the store.
    pub(crate) settings_store: Box<dyn SettingsStore>,
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
    /// The Session Views facade (ADR 0002): every store-backed read the UI
    /// renders — flat list, search, browsing, folders, smart playlists, and
    /// the playback-side slots — goes through it. It owns the five bounded
    /// Session Projections, the Library query port, and the session-local
    /// generation counter, so view code never touches staleness handling or
    /// store-error fallbacks.
    pub(crate) views: SessionViews,
    pub(crate) theme: ThemeState,
    /// Vendored-glyph texture cache for the shell's icon controls (Issue 06).
    pub(crate) icons: crate::ui::icons::IconCache,
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
    quit_flag: Arc<AtomicBool>,
}

impl RiffApp {
    /// Composition-root constructor: the main thread wires every dependency
    /// by hand, so the parameter count is the wiring surface itself.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Arc<Mutex<AppState>>,
        command_sender: Sender<PlaybackCommand>,
        library_command_sender: Sender<LibraryCommand>,
        library_update_rx: Receiver<LibraryUpdate>,
        watcher_manager: Arc<Mutex<Option<WatcherManager>>>,
        #[cfg(not(target_os = "linux"))] tray_icon: Option<tray_icon::TrayIcon>,
        quit_flag: Arc<AtomicBool>,
        settings_store: Box<dyn SettingsStore>,
        playlist_store: Box<dyn PlaylistStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        views: SessionViews,
        tag_edits: Box<dyn TagEdits>,
        covers: Box<dyn Covers>,
    ) -> Self {
        Self {
            state,
            command_sender: Some(command_sender),
            library_command_sender: Some(library_command_sender),
            library_update_rx: Some(library_update_rx),
            cover_textures: std::collections::HashMap::new(),
            cover_lru_keys: Vec::new(),
            covers,
            tag_edits,
            tag_edit_in_flight: None,
            tag_edit: None,
            smart_playlist_view: None,
            playlist_view: None,
            row_label_generation: 0,
            row_labels: HashMap::new(),
            playlist_labels: HashMap::new(),
            np_text_cache: None,
            now_playing_cover_key: None,
            up_next_cache: None,
            last_title_key: None,
            queue_position_cache: (None, 0, None),
            playerbar_readouts: crate::ui::playerbar::SeekReadouts::new(),
            stage_readouts: crate::ui::playerbar::SeekReadouts::new(),
            titlebar_actions: Vec::new(),
            playerbar_actions: Vec::new(),
            now_playing_actions: Vec::new(),
            playlist_create_name: None,
            playlist_rename: None,
            clear_library_confirm: false,
            search_focus: false,
            first_frame: true,
            watcher_manager,
            settings_store,
            playlist_store,
            library_mutations,
            views,
            theme: ThemeState {
                dark: true, // dark (mockup palette) by default
                active: theme::Palette::dark(),
                last_applied: None,
            },
            icons: crate::ui::icons::IconCache::new(),
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
            quit_flag,
        }
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

    fn poll_library_updates(&self, state: &mut AppState) {
        if let Some(ref rx) = self.library_update_rx {
            while let Ok(update) = rx.try_recv() {
                match update {
                    LibraryUpdate::Progress {
                        path,
                        files_found,
                        current_dir,
                    } => {
                        state
                            .library_statuses
                            .insert(path, LibraryStatus::Scanning { files_found });
                        state.scan_status = Some(format!("{files_found} files, {current_dir}"));
                    }
                    LibraryUpdate::Complete { path, total_files } => {
                        state
                            .library_statuses
                            .insert(path.clone(), LibraryStatus::Scanned(total_files));
                        state.scan_status = Some(format!("Scan complete: {total_files} tracks"));
                        // Scan batches already committed through the store as
                        // they progressed; nothing whole-file remains to save.
                        if let Some(ref mut mgr) = *self.watcher_manager.lock_or_recover() {
                            mgr.mark_scan_complete(&path);
                        }
                    }
                    LibraryUpdate::Error { path, message } => {
                        state.library_statuses.insert(path, LibraryStatus::Idle);
                        state.scan_status = Some(format!("Error: {message}"));
                    }
                }
            }
        }
    }

    fn poll_watchers(&self) {
        if let Some(ref mut mgr) = *self.watcher_manager.lock_or_recover() {
            mgr.poll();
        }
    }

    /// Drain polled Tag Edit outcomes from the service. On [`Saved`] the
    /// matching open modal closes and the status line reports the saved
    /// file; on `Failed` the dialog stays open with the reason — there is
    /// no silent-success path. All outcome application lives in the free
    /// [`apply_tag_edit_outcome`], which is what tests drive.
    fn poll_tag_edit_outcomes(&mut self, state: &mut AppState) {
        while let Some(outcome) = self.tag_edits.poll() {
            apply_tag_edit_outcome(
                outcome,
                &mut self.tag_edit,
                &mut self.tag_edit_in_flight,
                &mut state.scan_status,
            );
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

    /// Render the "Edit Tags" modal while `self.tag_edit` is open. Writing
    /// only happens on an explicit Save click; Cancel (or the window close
    /// button) discards the edits.
    fn show_tag_edit_modal(&mut self, ctx: &egui::Context) {
        // Read the active palette's error token before the modal state takes
        // its mutable borrow (Issue 03: no hardcoded colors in view code).
        let error_color = self.theme.active.error;
        let Some(tag_edit) = self.tag_edit.as_mut() else {
            return;
        };

        let mut open = true;
        let mut save_clicked = false;
        let mut cancel_clicked = false;
        // Escape closes the modal (REQ-UI-007 keyboard navigation), matching
        // the window close button and Cancel.
        let escape_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        egui::Window::new("Edit Tags")
            .id(egui::Id::new("tag_edit_modal"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(tag_edit.path.to_string_lossy()).weak());
                ui.separator();
                egui::Grid::new("tag_edit_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Title");
                        ui.add(
                            egui::TextEdit::singleline(&mut tag_edit.title).desired_width(280.0),
                        );
                        ui.end_row();
                        ui.label("Artist");
                        ui.add(
                            egui::TextEdit::singleline(&mut tag_edit.artist).desired_width(280.0),
                        );
                        ui.end_row();
                        ui.label("Album");
                        ui.add(
                            egui::TextEdit::singleline(&mut tag_edit.album).desired_width(280.0),
                        );
                        ui.end_row();
                        ui.label("Album Artist");
                        ui.add(
                            egui::TextEdit::singleline(&mut tag_edit.album_artist)
                                .desired_width(280.0),
                        );
                        ui.end_row();
                        ui.label("Genre");
                        ui.add(
                            egui::TextEdit::singleline(&mut tag_edit.genre).desired_width(280.0),
                        );
                        ui.end_row();
                        ui.label("Year");
                        ui.add(egui::TextEdit::singleline(&mut tag_edit.year).desired_width(80.0));
                        ui.end_row();
                        ui.label("Track Number");
                        ui.add(
                            egui::TextEdit::singleline(&mut tag_edit.track_number)
                                .desired_width(80.0),
                        );
                        ui.end_row();
                    });

                if let Some(ref error) = tag_edit.error {
                    ui.colored_label(error_color, error);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!tag_edit.saving, egui::Button::new("Save"))
                        .clicked()
                    {
                        save_clicked = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_clicked = true;
                    }
                    if tag_edit.saving {
                        ui.spinner();
                    }
                });
            });

        if !open || cancel_clicked || escape_pressed {
            self.tag_edit = None;
            return;
        }

        if save_clicked {
            self.submit_tag_edit();
        }
    }

    /// Validate the modal fields and submit a [`TagEditRequest`] to the Tag
    /// Edit Service. Invalid numeric fields keep the modal open with an
    /// error; nothing is ever written without an explicit Save. The whole
    /// flow lives in the free [`submit_tag_edit_fields`], which is what
    /// tests drive.
    fn submit_tag_edit(&mut self) {
        if let Some(ref mut tag_edit) = self.tag_edit {
            submit_tag_edit_fields(
                tag_edit,
                self.tag_edits.as_ref(),
                &mut self.tag_edit_in_flight,
            );
        }
    }

    /// Get a cover texture, touching the LRU to mark it as recently used.
    fn get_cover_texture(&mut self, key: &str) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.cover_textures.get(key) {
            // Move to end (most recently used) and return a clone.
            self.cover_lru_keys.retain(|k| k != key);
            self.cover_lru_keys.push(key.to_string());
            Some(tex.clone())
        } else {
            None
        }
    }

    /// Attach the shared track context menu to `response`. See
    /// [`show_track_context_menu`] for the available actions.
    fn attach_track_menu(
        &mut self,
        response: &egui::Response,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        track_id: &TrackId,
        track: Option<&Track>,
        remove_from_playlist: Option<&PlaylistId>,
    ) {
        let advanced = state.ui_flags.advanced_mode;
        // Arc clone out of the seam first: no `&self.views` borrow may live
        // across widget rendering.
        let playlists = self.views.playlists();
        let tag_edit_slot = &mut self.tag_edit;
        let playlist_store_slot = self.playlist_store.as_mut();
        show_track_context_menu(
            response,
            TrackMenuArgs {
                cmd,
                track_id,
                track,
                tag_edit: tag_edit_slot,
                advanced,
                playlists,
                playlist_store: playlist_store_slot,
                remove_from_playlist,
            },
        );
    }

    // --- Per-frame label caches (allocation plan 2.1/2.2) ------------------

    /// Drop the per-track label cache when the store generation moved: any
    /// committed mutation may have changed metadata, so labels recompute
    /// lazily on their next request.
    fn ensure_label_generation(&mut self) {
        let generation = self.views.generation();
        if self.row_label_generation != generation {
            self.row_labels.clear();
            self.row_label_generation = generation;
        }
    }

    /// Fill-or-fetch one lazily computed label slot for `track_id`. Fresh
    /// frames hand out an `Arc` clone; only the first request per
    /// (track, generation) formats.
    fn slot_label(
        &mut self,
        track_id: &TrackId,
        pick: impl Fn(&CachedTrackLabels) -> &Option<Arc<str>>,
        set: impl Fn(&mut CachedTrackLabels, Arc<str>),
        make: impl FnOnce() -> String,
    ) -> Arc<str> {
        self.ensure_label_generation();
        if let Some(labels) = self.row_labels.get(track_id)
            && let Some(label) = pick(labels)
        {
            return Arc::clone(label);
        }
        let label: Arc<str> = Arc::from(make());
        set(
            self.row_labels.entry(track_id.clone()).or_default(),
            Arc::clone(&label),
        );
        label
    }

    /// `"Artist - Title"` for one track, cached per generation.
    fn label_artist_title(&mut self, track: &Track) -> Arc<str> {
        self.slot_label(
            &track.id,
            |labels| &labels.artist_title,
            |labels, value| labels.artist_title = Some(value),
            || {
                format!(
                    "{} - {}",
                    track.metadata.display_artist(),
                    track.metadata.display_title(&track.file_path)
                )
            },
        )
    }

    /// `"N. Title"` for one track (album/folder tree rows), cached per
    /// generation.
    fn label_numbered(&mut self, track: &Track) -> Arc<str> {
        self.slot_label(
            &track.id,
            |labels| &labels.numbered,
            |labels, value| labels.numbered = Some(value),
            || {
                format!(
                    "{}. {}",
                    track.metadata.track_number.unwrap_or(0),
                    track.metadata.display_title(&track.file_path)
                )
            },
        )
    }

    /// The sidebar playlist row's painted label (`"Name (count)"`), rebuilt
    /// only while the row's name or entry count differs from the cache.
    fn playlist_label(&mut self, id: &PlaylistId, name: &str, count: usize) -> Arc<str> {
        if let Some((cached_name, cached_count, label)) = self.playlist_labels.get(id)
            && cached_name == name
            && *cached_count == count
        {
            return Arc::clone(label);
        }
        let label: Arc<str> = Arc::from(format!("{name} ({count})"));
        self.playlist_labels
            .insert(id.clone(), (name.to_owned(), count, Arc::clone(&label)));
        label
    }

    /// The Now Playing stage's text block (title, meta line, details line),
    /// keyed by current `TrackId` + generation and handed out as `Arc`
    /// clones — allocation plan 2.2.
    fn now_playing_text(&mut self) -> NpText {
        let generation = self.views.generation();
        let current = self.views.playback_current();
        let fresh = self.np_text_cache.as_ref().is_some_and(|cache| {
            cache.generation == generation && cache.id.as_ref() == current.map(|t| &t.id)
        });
        if !fresh {
            // Cold path: the playing track or the generation moved.
            let id = current.map(|t| t.id.clone());
            let (title, meta_line, details) = match self.views.playback_current() {
                Some(track) => (
                    Some(Arc::from(track.metadata.display_title(&track.file_path))),
                    Some(Arc::from(format!(
                        "{} - {}",
                        track.metadata.display_artist(),
                        track.metadata.display_album()
                    ))),
                    NpDetails::from_option(crate::ui::now_playing::metadata_details(
                        &track.metadata,
                    )),
                ),
                None => (None, None, NpDetails::Absent),
            };
            self.np_text_cache = Some(NpTextCache {
                generation,
                id,
                title,
                meta_line,
                details,
            });
        }
        let cache = self.np_text_cache.as_ref().expect("np text just ensured");
        NpText {
            title: cache.title.clone(),
            meta_line: cache.meta_line.clone(),
            details: match &cache.details {
                NpDetails::Line(line) => Some(Arc::clone(line)),
                NpDetails::Absent => None,
            },
        }
    }

    /// The Up Next rows for the Now Playing stage, cached against the store
    /// generation plus the exact window contents (allocation plan 2.2):
    /// queue labels only change when the playback projection's window does,
    /// and the comparison walks it by reference so fresh frames allocate
    /// nothing.
    fn up_next_entries_cached(&mut self, limit: usize) -> Arc<[UpNextEntry]> {
        let generation = self.views.generation();
        let fresh = {
            let window = self.views.playback_up_next();
            let window = &window[..window.len().min(limit)];
            match &self.up_next_cache {
                Some(cache) => {
                    cache.generation == generation
                        && cache.ids.len() == window.len()
                        && cache
                            .ids
                            .iter()
                            .zip(window.iter())
                            .all(|(cached, track)| cached == &track.id)
                }
                None => false,
            }
        };
        if !fresh {
            let window = self.views.playback_up_next();
            let entries: Vec<UpNextEntry> = crate::ui::now_playing::up_next_entries(window, limit);
            let ids: Vec<TrackId> = window
                .iter()
                .take(limit)
                .map(|track| track.id.clone())
                .collect();
            self.up_next_cache = Some(UpNextLabels {
                generation,
                ids,
                entries: entries.into(),
            });
        }
        Arc::clone(
            &self
                .up_next_cache
                .as_ref()
                .expect("up next just ensured")
                .entries,
        )
    }

    /// The playerbar's `{index}/{len}` queue-position label, rebuilt only
    /// when either component moves (allocation plan 2.4).
    fn queue_position_label(&mut self, index: Option<usize>, len: usize) -> Arc<str> {
        let (cached_index, cached_len, label) = &self.queue_position_cache;
        if *cached_index == index
            && *cached_len == len
            && let Some(label) = label
        {
            return Arc::clone(label);
        }
        let label: Arc<str> = Arc::from(format!("{}/{}", index.map_or(0, |i| i + 1), len));
        self.queue_position_cache = (index, len, Some(Arc::clone(&label)));
        label
    }

    /// One library-list track row (Issue 07): a 40px tree row with the
    /// animated equalizer indicator on the now-playing row, click/double-click
    /// handling, and the shared context menu.
    fn render_track_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        track: &Track,
        current_track: Option<&TrackId>,
        remove_from_playlist: Option<&PlaylistId>,
    ) {
        let label = self.label_artist_title(track);
        self.interactive_track_row(
            ui,
            state,
            cmd,
            track,
            current_track,
            remove_from_playlist,
            &label,
            0,
        );
    }

    /// Shared clickable track row behind every track listing: restyled 40px
    /// tree row + selection/play/context-menu wiring. `label` lets callers
    /// keep their display formats ("Artist - Title", "01. Title") — both
    /// served from the per-generation label cache as `Arc` clones.
    #[allow(clippy::too_many_arguments)]
    fn interactive_track_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        track: &Track,
        current_track: Option<&TrackId>,
        remove_from_playlist: Option<&PlaylistId>,
        label: &str,
        indent_level: usize,
    ) {
        use crate::ui::sidebar::{self, TreeRow};

        let is_selected = state.selected_track.as_ref() == Some(&track.id);
        let is_current = current_track == Some(&track.id);
        let playing = state.playback_state == PlaybackState::Playing;

        self.request_cover(&track.id, &track.file_path);

        let response = sidebar::tree_row(
            ui,
            &mut self.icons,
            &self.theme.active,
            TreeRow {
                indent_level,
                icon: None,
                label,
                selected: is_selected,
                now_playing: is_current,
                playing: is_current && playing,
                disclosure: None,
            },
        );
        if response.clicked() {
            state.selected_track = Some(track.id.clone());
        }
        if response.double_clicked() {
            state.selected_track = Some(track.id.clone());
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Play(track.id.clone()));
            }
        }
        self.attach_track_menu(
            &response,
            state,
            cmd,
            &track.id,
            Some(track),
            remove_from_playlist,
        );
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

        // While hidden ui() never runs, so keep a slow repaint loop alive to
        // keep observing the tray quit flag and visibility toggles.
        if hidden {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        // Tray Quit while hidden: ui() cannot observe quit_flag, so initiate
        // the close here. (When visible, the same check in ui() does it.)
        if self.quit_flag.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Reconcile AppState::window_visible (flipped by the tray's
        // ToggleVisibility command, handled in the audio engine thread) with
        // the real viewport visibility, in both directions.
        let want_visible = self.state.lock_or_recover().window_visible;
        if want_visible && hidden {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        } else if !want_visible && !hidden {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // Close-to-tray: veto the OS close request and hide instead. This only
        // runs when NOT quitting — a quit-initiated close goes through above.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.state.lock_or_recover().window_visible = false;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let cmd = self.command_sender.clone();
        let lib_cmd = self.library_command_sender.clone();
        let state_arc = self.state.clone();

        let mut state = state_arc.lock_or_recover();

        if self.quit_flag.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if self.first_frame {
            load_persisted_state(&mut state, self.settings_store.as_ref(), cmd.as_ref());
            self.first_frame = false;
        }

        // Apply the active theme (REQ-UI-007 accessibility). Done after the
        // first-frame load so a persisted high-contrast choice takes effect on
        // the very first frame. High Contrast is a variant over the active
        // light/dark palette.
        self.apply_theme(ui.ctx(), state.ui_flags.high_contrast);

        self.poll_library_updates(&mut state);
        self.poll_tag_edit_outcomes(&mut state);
        self.update_cover_cache(ui.ctx());
        self.poll_watchers();

        handle_keyboard_shortcuts(ui.ctx(), &mut state, &mut self.search_focus, cmd.as_ref());

        // Update window title and tray tooltip (REQ-SI-001). Both are
        // compared against the last-pushed identity first and only rebuilt
        // when the playing track or the store generation moves (allocation
        // plan 2.3); the tooltip shows "Artist - Title" for the current
        // track, else "riff".
        self.update_window_title(ui.ctx(), &state);

        // --- SHELL (Issue 06): unified Panel API at exact token dimensions ---
        //
        // Top 56px strip: the frameless titlebar (issue 04, ADR 0005)
        // merged with the former top bar — wordmark, scan status, the
        // theme/Now Playing/Settings/Advanced controls, and the custom
        // minimize/close buttons over a full-width drag region.
        let scan_status = state.scan_status.clone();
        egui::Panel::top("titlebar")
            .exact_size(theme::TITLEBAR_H)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let content = crate::ui::chrome::TitleBarContent {
                    scan_status: scan_status.as_deref(),
                    theme_dark: self.theme.dark,
                    advanced_mode: state.ui_flags.advanced_mode,
                    active_nav: crate::ui::chrome::NavDestination::active(
                        state.view_mode,
                        state.browse_mode,
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
                    apply_titlebar_action(
                        action,
                        ui.ctx(),
                        &mut state,
                        &mut self.theme,
                        self.settings_store.as_mut(),
                    );
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
                self.render_library_sidebar(ui, &mut state, cmd.as_ref());
            });

        // Bottom 88px strip: transport + progress + volume.
        self.render_control_bar(ui, &mut state, cmd.as_ref());

        // --- MAIN STAGE: exactly one View visible at a time ---
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.theme.active.background))
            .show(ui, |ui| match state.view_mode {
                ViewMode::Library => self.render_track_details_panel(ui, &mut state),
                ViewMode::NowPlaying => self.show_now_playing_view(ui, &mut state, cmd.as_ref()),
                ViewMode::Settings => {
                    self.show_settings_view(ui, &mut state, lib_cmd.as_ref());
                }
            });

        // --- EDIT TAGS MODAL ---
        self.show_tag_edit_modal(ui.ctx());

        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

// --- Per-frame helpers -------------------------------------------------------

/// Apply one [`crate::ui::chrome::TitleBarAction`] to app state and viewport
/// commands (Issue 06). Window controls route through the same vetoable
/// viewport commands as their issue-04 counterparts, so close-to-tray
/// (REQ-SI-001) keeps working from the custom chrome.
fn apply_titlebar_action(
    action: crate::ui::chrome::TitleBarAction,
    ctx: &egui::Context,
    state: &mut AppState,
    theme: &mut ThemeState,
    store: &mut dyn SettingsStore,
) {
    use crate::ui::chrome::{NavDestination, TitleBarAction as Action, WindowControl};
    match action {
        Action::ToggleTheme => theme.dark = !theme.dark,
        Action::ToggleAdvanced => {
            state.ui_flags.advanced_mode = !state.ui_flags.advanced_mode;
            persist_scalars(store, state);
        }
        Action::ToggleNowPlaying => {
            // Now Playing replaces the active view; leaving it returns to the
            // Library view (resolved navigation gap).
            state.view_mode = match state.view_mode {
                ViewMode::Library | ViewMode::Settings => ViewMode::NowPlaying,
                ViewMode::NowPlaying => ViewMode::Library,
            };
        }
        Action::GoSettings => {
            NavDestination::Settings.apply(&mut state.view_mode, &mut state.browse_mode);
        }
        Action::Minimize => ctx.send_viewport_cmd(WindowControl::Minimize.viewport_command()),
        Action::Close => ctx.send_viewport_cmd(WindowControl::Close.viewport_command()),
    }
}

/// Apply one [`crate::ui::now_playing::NowPlayingAction`] (Issue 10). Close
/// ALWAYS lands on the Library View: Now Playing is a mode that replaces the
/// active View (resolved navigation gaps), so there is no prior view to
/// restore — closing from anywhere returns to the Library. Transport actions
/// pass straight through to the command channel; seek targets re-clamp
/// against the live track duration exactly like the playerbar's.
pub fn apply_now_playing_action(
    action: crate::ui::now_playing::NowPlayingAction,
    state: &mut AppState,
    cmd: Option<&Sender<PlaybackCommand>>,
) {
    use crate::ui::now_playing::NowPlayingAction as Action;
    match action {
        Action::Close => state.view_mode = ViewMode::Library,
        Action::PlayNext(track_id) => {
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::PlayNext(track_id));
            }
        }
        Action::Seek(duration) => {
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Seek(clamp_seek(
                    duration.as_secs_f32(),
                    state.current_position.total,
                )));
            }
        }
    }
}

/// Commit `state`'s scalar preferences as one small durable store
/// transaction; failures are logged, the in-memory change stands.
fn persist_scalars(store: &mut dyn SettingsStore, state: &AppState) {
    let scalars = crate::app::state::ScalarSettings {
        volume: Some(state.current_volume),
        advanced_mode: state.ui_flags.advanced_mode,
        high_contrast: state.ui_flags.high_contrast,
        replaygain_enabled: state.replaygain_enabled,
    };
    if let Err(e) = store.save_scalars(&scalars) {
        tracing::warn!("Failed to save settings: {e}");
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
            crate::app::playlist_manager::reorder_tracks(&playlist.tracks, from, to)
        });
    let Some(new_order) = new_order else {
        return;
    };
    if let Err(e) = store.reorder_playlist_entries(id, &new_order) {
        tracing::warn!("Failed to reorder playlist entries: {e}");
    }
}

/// Apply one restyled player-bar action (Issue 08) through the SAME engine
/// commands and state paths the pre-restyle controls used. Transport actions
/// pass straight through to the command channel; volume routes through
/// [`AppState::effective_volume`] so a muted app never emits sound; seek
/// targets re-clamp against the live track duration.
pub fn apply_player_bar_action(
    action: crate::ui::playerbar::PlayerBarAction,
    state: &mut AppState,
    cmd: Option<&Sender<PlaybackCommand>>,
    store: &mut dyn SettingsStore,
) {
    use crate::ui::playerbar::PlayerBarAction as Action;
    let send = |command: PlaybackCommand| {
        if let Some(s) = cmd {
            let _ = s.send(command);
        }
    };
    match action {
        Action::Previous => send(PlaybackCommand::Previous),
        Action::Pause => send(PlaybackCommand::Pause),
        Action::Resume => send(PlaybackCommand::Resume),
        Action::PlaySelected => {
            // Pre-restyle behavior: with nothing selected, play does nothing.
            if let Some(selected) = state.selected_track.clone() {
                send(PlaybackCommand::Play(selected));
            }
        }
        Action::Next => send(PlaybackCommand::Next),
        Action::Stop => send(PlaybackCommand::Stop),
        Action::Seek(target) => {
            let secs = target.as_secs_f32();
            send(PlaybackCommand::Seek(clamp_seek(
                secs,
                state.current_position.total,
            )));
        }
        Action::SetVolume(volume) => {
            state.current_volume = volume;
            persist_scalars(store, state);
            // While muted the slider still edits current_volume, but the
            // engine keeps receiving 0 until unmuted.
            send(PlaybackCommand::SetVolume(state.effective_volume()));
        }
        Action::ToggleMute => {
            // Muting never moves the volume slider — it only zeroes the
            // effective volume sent to the engine; unmuting restores it.
            state.muted = !state.muted;
            send(PlaybackCommand::SetVolume(state.effective_volume()));
        }
        Action::ToggleShuffle => {
            let was = state.queue.shuffle;
            state.queue.set_shuffle(!was);
        }
        Action::ToggleRepeat => state.queue.toggle_repeat(),
    }
}

/// First-frame restore. Every user preference hydrates from the typed
/// settings tables via [`SettingsStore`]; the Library collection and the
/// user playlists need no hydration step at all — every view reads them
/// live through the [`SessionViews`] seam, which serves the Application
/// Store directly on its first call. The legacy JSON cache is never read
/// or written. Public so the restore contract is testable headlessly.
pub fn load_persisted_state(
    state: &mut AppState,
    store: &dyn SettingsStore,
    cmd: Option<&Sender<PlaybackCommand>>,
) {
    let settings = match store.load_settings() {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!("Failed to load settings from the store: {e}");
            crate::app::store::Settings::default()
        }
    };

    if !settings.library_paths.is_empty() {
        for path in &settings.library_paths {
            let status = if path.exists() {
                LibraryStatus::Idle
            } else {
                LibraryStatus::Unavailable
            };
            state.library_statuses.insert(path.clone(), status);
        }
        state.library_paths = settings.library_paths;
    }

    state.watch_states = settings.watch_states;

    if let Some(vol) = settings.scalars.volume {
        state.current_volume = vol;
        if let Some(s) = cmd {
            // Route through effective_volume so a muted app (once mute
            // state is restored) never emits sound at startup.
            let _ = s.send(PlaybackCommand::SetVolume(state.effective_volume()));
        }
    }

    state.ui_flags.advanced_mode = settings.scalars.advanced_mode;
    state.ui_flags.high_contrast = settings.scalars.high_contrast;
    state.replaygain_enabled = settings.scalars.replaygain_enabled;
}

/// Global keyboard shortcuts: Ctrl+F focuses search, Space toggles playback.
fn handle_keyboard_shortcuts(
    ctx: &egui::Context,
    state: &mut AppState,
    search_focus: &mut bool,
    cmd: Option<&Sender<PlaybackCommand>>,
) {
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
        *search_focus = true;
    }
    if !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
    {
        let playing = state.playback_state == PlaybackState::Playing;
        if playing {
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Pause);
            }
        } else if let Some(s) = cmd {
            let _ = s.send(PlaybackCommand::Resume);
        }
    }
}

/// Push the window title and tray tooltip for the current track (REQ-SI-001).
/// Both derive from one identity — the current `TrackId` plus the store
/// generation — which is compared against the last push FIRST: steady-state
/// frames send no viewport command and format nothing (allocation plan 2.3,
/// mirroring the `last_tray_tooltip` pattern). The current Track resolves
/// through the Session Views facade over the store's `get_track` query —
/// never the in-memory mirror.
impl RiffApp {
    fn update_window_title(&mut self, ctx: &egui::Context, state: &AppState) {
        self.views
            .sync_playback(&state.queue, crate::ui::now_playing::UP_NEXT_LIMIT);
        let generation = self.views.generation();
        let current_id = self.views.playback_current().map(|t| &t.id);
        let unchanged = self
            .last_title_key
            .as_ref()
            .is_some_and(|(id, cached_gen)| *cached_gen == generation && id.as_ref() == current_id);
        if unchanged {
            return;
        }

        // Cold path: the playing track or the generation moved — both
        // strings are rebuilt and pushed exactly once per identity change,
        // so they format directly instead of touching the row-label cache.
        let id_for_key = current_id.cloned();
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
        self.last_title_key = Some((id_for_key, generation));

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
    /// control still emits its engine command.
    fn render_control_bar(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        // Cover for the current track, served from the LRU texture cache;
        // misses enqueue a background resolve exactly like the other views.
        // The current Track comes from the Session Views facade over the
        // store's `get_track` query — never the in-memory mirror.
        let mut cover = None;
        self.views
            .sync_playback(&state.queue, crate::ui::now_playing::UP_NEXT_LIMIT);
        if let Some(track) = self.views.playback_current() {
            let id = track.id.clone();
            let file_path = track.file_path.clone();
            self.request_cover(&id, &file_path);
            cover = self.get_cover_texture(&id.0);
        }

        let queue_position =
            self.queue_position_label(state.queue.current_index, state.queue.tracks.len());
        self.playerbar_readouts
            .sync(state.current_position.current, state.current_position.total);
        let content = crate::ui::playerbar::PlayerBarContent {
            cover,
            playback: state.playback_state,
            position: state.current_position.current,
            total: state.current_position.total,
            volume: state.current_volume,
            muted: state.muted,
            shuffle: state.queue.shuffle,
            repeat: state.queue.repeat,
            queue_position: &queue_position,
            advanced: state.ui_flags.advanced_mode,
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
        for action in self.playerbar_actions.drain(..) {
            apply_player_bar_action(action, state, cmd, self.settings_store.as_mut());
        }
    }
}

// --- Helper methods factored out to avoid borrow conflicts ---
impl RiffApp {
    /// Sidebar content (Issues 06 + 07): the restyled search box, the
    /// segmented Library/Folders control, and the browser dispatch. Draws
    /// inside the shell's fixed-width sidebar panel, which is shared chrome
    /// present on every view; only the main stage switches. Nav routes
    /// through [`crate::ui::chrome::NavDestination`] so exactly one View is
    /// visible after any click.
    fn render_library_sidebar(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        use crate::ui::chrome::NavDestination;
        use crate::ui::sidebar::{self, SidebarNav};

        let palette = self.theme.active;

        // Search box with focus-ring border (mockup). The response drives the
        // same Ctrl+F request-focus shortcut as before; the clear affordance
        // lives inside the widget.
        let search_response =
            sidebar::search_box(ui, &mut self.icons, &palette, &mut state.search_query);
        if self.search_focus {
            search_response.request_focus();
            self.search_focus = false;
        }
        ui.add_space(10.0);

        // Segmented Library/Folders control: each destination lands on the
        // library view with its browse mode, so clicking one from Settings or
        // Now Playing returns to it.
        let active = NavDestination::active(state.view_mode, state.browse_mode);
        let segment = match active {
            Some(NavDestination::Library) => Some(SidebarNav::Library),
            Some(NavDestination::Folders) => Some(SidebarNav::Folders),
            _ => None,
        };
        if let Some(dest) = sidebar::segmented_nav(ui, &palette, segment) {
            match dest {
                SidebarNav::Library => {
                    NavDestination::Library.apply(&mut state.view_mode, &mut state.browse_mode);
                }
                SidebarNav::Folders => {
                    NavDestination::Folders.apply(&mut state.view_mode, &mut state.browse_mode);
                    self.smart_playlist_view = None;
                    self.playlist_view = None;
                }
            }
        }
        ui.add_space(12.0);

        let query = state.search_query.clone();

        match state.browse_mode {
            BrowseMode::Library => self.render_library_browser(ui, state, cmd, &query),
            BrowseMode::Folders => self.render_folder_tree(ui, state, cmd, &query),
        }
    }

    /// Left-panel content in Library browse mode: the All Tracks / Artists
    /// rows, smart playlists, user playlists, and the results dispatch — all
    /// on the restyled 40px tree rows (Issue 07).
    fn render_library_browser(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        query: &str,
    ) {
        use crate::ui::sidebar::{self, TreeRow};
        let palette = self.theme.active;

        // Existing sub-toggle: All Tracks / Artists, now as tree rows.
        // Selecting either one closes any open smart playlist.
        let no_playlist = self.smart_playlist_view.is_none() && self.playlist_view.is_none();
        let all_tracks = sidebar::tree_row(
            ui,
            &mut self.icons,
            &palette,
            TreeRow {
                indent_level: 0,
                icon: Some(crate::ui::icons::Icon::ListMusic),
                label: "All Tracks",
                selected: !state.ui_flags.show_artists_view && no_playlist,
                now_playing: false,
                playing: false,
                disclosure: None,
            },
        );
        if all_tracks.clicked() {
            state.ui_flags.show_artists_view = false;
            self.smart_playlist_view = None;
            self.playlist_view = None;
        }
        let artists = sidebar::tree_row(
            ui,
            &mut self.icons,
            &palette,
            TreeRow {
                indent_level: 0,
                icon: Some(crate::ui::icons::Icon::Library),
                label: "Artists",
                selected: state.ui_flags.show_artists_view && no_playlist,
                now_playing: false,
                playing: false,
                disclosure: None,
            },
        );
        if artists.clicked() {
            state.ui_flags.show_artists_view = true;
            self.smart_playlist_view = None;
            self.playlist_view = None;
        }
        ui.add_space(8.0);

        // Smart Playlists: four read-only, auto-generated lists
        // derived from local play history. They are virtual, so
        // they never appear while searching. Advanced-only
        // (REQ-UI-006): hidden entirely in the minimal UI.
        if state.ui_flags.advanced_mode && query.is_empty() {
            sidebar::section_header(ui, &palette, "Smart Playlists")
                .on_hover_text("Auto-generated, read-only lists built from your play history.");
            for kind in SmartPlaylistKind::ALL {
                let selected = self.smart_playlist_view == Some(kind);
                let row = sidebar::tree_row(
                    ui,
                    &mut self.icons,
                    &palette,
                    TreeRow {
                        indent_level: 0,
                        icon: Some(crate::ui::icons::Icon::Sparkles),
                        label: kind.display_name(),
                        selected,
                        now_playing: false,
                        playing: false,
                        disclosure: None,
                    },
                );
                if row.clicked() {
                    self.smart_playlist_view = Some(kind);
                    self.playlist_view = None;
                }
            }
            ui.add_space(8.0);
        }

        // User playlists (Task 4.2): named, editable lists persisted in the
        // Application Store. A core feature — always visible (NOT
        // gated behind advanced mode). Hidden while searching, like
        // smart playlists.
        if query.is_empty() {
            self.render_playlists_section(ui);
        }

        // Search gating reads the store's match count (the same query the
        // flat view's projection totals use) — never the in-memory mirror.
        let has_results = query.is_empty() || self.views.search_has_matches(query);
        // A playlist only renders when not searching; search shows
        // matching tracks only. Turning advanced mode off closes
        // any open smart playlist.
        let open_playlist = self
            .smart_playlist_view
            .filter(|_| query.is_empty() && state.ui_flags.advanced_mode);
        let open_user_playlist = self.playlist_view.clone().filter(|_| query.is_empty());

        if !has_results && !query.is_empty() {
            ui.vertical_centered(|ui| {
                ui.label(format!("No tracks found matching '{query}'"));
            });
        } else if let Some(pid) = open_user_playlist {
            self.render_playlist_view(ui, state, cmd, &pid);
        } else if let Some(kind) = open_playlist {
            self.render_smart_playlist_view(ui, state, cmd, kind);
        } else if state.ui_flags.show_artists_view {
            self.render_artist_view(ui, state, cmd, query);
        } else {
            self.render_flat_view(ui, state, cmd, query);
        }
    }

    /// Library-stage content (Issues 06 + 09): selected track metadata +
    /// cover, or the mockup's empty-state hero — the glowing disc circle with
    /// its copy — whenever there are no track details to show.
    fn render_track_details_panel(&mut self, ui: &mut egui::Ui, state: &mut AppState) {
        let palette = self.theme.active;
        let Some(track_id) = state.selected_track.clone() else {
            crate::ui::library::empty_state_hero(ui, &mut self.icons, &palette);
            return;
        };
        // The selected Track resolves through the Session Views facade over
        // the store's `get_track` query (cached until the selection or the
        // generation moves) — never the in-memory mirror. An absent track —
        // unknown to the store, or unreadable right now — renders the empty
        // state.
        let Some(track) = self.views.selected_track(&track_id) else {
            crate::ui::library::empty_state_hero(ui, &mut self.icons, &palette);
            return;
        };
        self.request_cover(&track.id, &track.file_path);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(track.metadata.display_title(&track.file_path));
                ui.label(format!("Artist: {}", track.metadata.display_artist()));
                ui.label(format!("Album: {}", track.metadata.display_album()));
                render_track_meta_labels(ui, &track.metadata, false);
                ui.separator();
                ui.label(format!("File: {}", track.file_path.display()));
            });
            ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                let texture = self.get_cover_texture(&track.id.0);
                cover_art_ui(ui, &palette, texture, COVER_THUMB_SIZE);
            });
        });
    }

    fn render_artist_view(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        query: &str,
    ) {
        // Browsing reads through the Session Views facade over store queries
        // (ADR 0002/0003): artists A–Z straight from the Application Store,
        // each level cached until the next committed mutation bumps the
        // generation. No in-memory mirror involved.
        //
        // Single pass over the borrowed cache: the projection hands back an
        // Arc-shared list, so filtering collects lightweight references
        // instead of cloning Artists, and the lowercased query is hoisted
        // out of the per-item work.
        let artists = self.views.artists();
        let visible: Vec<&Artist> = if query.is_empty() {
            artists.iter().collect()
        } else {
            let q = query.to_lowercase();
            artists
                .iter()
                .filter(|a| a.name.to_lowercase().contains(&q))
                .collect()
        };
        let current_track = state.queue.current_track().cloned();

        // Identity of the playing track's album, used to auto-open exactly
        // the collapsed headers containing it — the same outcome as the
        // former scan of every artist's albums, without loading closed
        // artists' data.
        let current_album: Option<(String, String)> = current_track
            .as_ref()
            .and_then(|tid| self.views.resolve_track(tid))
            .map(|t| {
                (
                    // `display_*` return borrowed `Cow`s now (allocation
                    // plan 4.6); the tuple outlives the track, so own the
                    // strings.
                    t.metadata.display_album_artist().into_owned(),
                    t.metadata.display_album().into_owned(),
                )
            });

        egui::ScrollArea::vertical().show(ui, |ui| {
            for &artist in &visible {
                let artist_has_current = current_album
                    .as_ref()
                    .is_some_and(|(album_artist, _)| album_artist == &artist.name);
                self.render_artist_node(
                    ui,
                    state,
                    cmd,
                    artist,
                    current_album.as_ref(),
                    current_track.as_ref(),
                    artist_has_current,
                    query,
                );
            }
        });
    }

    /// One artist node of the Artists tree (Issue 07): a restyled 40px
    /// collapsible row whose albums nest on the second indent level. The
    /// collapse state persists per artist in egui memory, exactly like the
    /// former `CollapsingHeader`.
    #[allow(clippy::too_many_arguments)]
    fn render_artist_node(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        artist: &Artist,
        current_album: Option<&(String, String)>,
        current_track: Option<&TrackId>,
        artist_has_current: bool,
        query: &str,
    ) {
        use crate::ui::icons::Icon;
        use crate::ui::sidebar::{self, TreeRow};
        use egui::collapsing_header::CollapsingState;

        let palette = self.theme.active;
        let id = egui::Id::new(("riff_sidebar_artist", &artist.name));
        let mut collapsing =
            CollapsingState::load_with_default_open(ui.ctx(), id, artist_has_current);

        let response = sidebar::tree_row(
            ui,
            &mut self.icons,
            &palette,
            TreeRow {
                indent_level: 0,
                icon: Some(Icon::Music),
                label: &artist.name,
                selected: false,
                now_playing: false,
                playing: false,
                disclosure: Some(collapsing.is_open()),
            },
        );
        if response.clicked() {
            collapsing.toggle(ui);
        }
        collapsing.store(ui.ctx());

        if let Some(body) = collapsing.show_body_unindented(ui, |ui| {
            let albums = self.views.artist_albums(&artist.name);

            for album in albums.iter() {
                let album_has_current = current_album.is_some_and(|(album_artist, album_title)| {
                    album_artist == &album.artist && album_title == &album.title
                });
                self.render_album_node(
                    ui,
                    state,
                    cmd,
                    album,
                    current_track,
                    album_has_current,
                    query,
                );
            }
        }) {
            let _ = body;
        }
    }

    /// One album node under an artist (Issue 07): a restyled 40px collapsible
    /// row on the second indent level; its tracks sit on the third.
    #[allow(clippy::too_many_arguments)]
    fn render_album_node(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        album: &Album,
        current_track: Option<&TrackId>,
        album_has_current: bool,
        _query: &str,
    ) {
        use crate::ui::icons::Icon;
        use crate::ui::sidebar::{self, TreeRow};
        use egui::collapsing_header::CollapsingState;

        let palette = self.theme.active;
        let id = egui::Id::new(("riff_sidebar_album", &album.artist, &album.title));
        let mut collapsing =
            CollapsingState::load_with_default_open(ui.ctx(), id, album_has_current);

        let year_str = album.year.map_or(String::new(), |y| format!(" ({y})"));
        let label = format!("{}{year_str}", album.title);

        let response = sidebar::tree_row(
            ui,
            &mut self.icons,
            &palette,
            TreeRow {
                indent_level: 1,
                icon: Some(Icon::Disc),
                label: &label,
                selected: false,
                now_playing: false,
                playing: false,
                disclosure: Some(collapsing.is_open()),
            },
        );
        if response.clicked() {
            collapsing.toggle(ui);
        }
        collapsing.store(ui.ctx());

        collapsing.show_body_unindented(ui, |ui| {
            let tracks = self.views.album_tracks(&album.artist, &album.title);
            self.render_album_track_rows(ui, state, cmd, &tracks, current_track);
        });
    }

    /// Track rows for one album in the Artists view, rendered straight from
    /// the store query results in their canonical order (track number then
    /// filename, missing numbers first). Restyled 40px rows on the third
    /// indent level (Issue 07).
    fn render_album_track_rows(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        tracks: &[Track],
        current_track: Option<&TrackId>,
    ) {
        for track in tracks {
            let label = self.label_numbered(track);
            self.interactive_track_row(ui, state, cmd, track, current_track, None, &label, 2);
        }
    }

    fn render_flat_view(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
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
        // Session Projection behind the Session Views facade (ADR 0003):
        // only visible row windows fetch, invalidated by generation bumps
        // after committed mutations. The facade owns the window math, the
        // count reads, and the torn-count recount; this view only maps row
        // indices to pages.
        let current_track = state.queue.current_track().cloned();

        // Anchor read: sizes the row range with the authoritative total.
        let first_page = self.views.track_list(query, 0);

        egui::ScrollArea::vertical().show_rows(
            ui,
            crate::ui::sidebar::ROW_H,
            first_page.total,
            |ui, row_range| {
                let mut page: Option<crate::app::views::TrackListPage> = None;
                for i in row_range {
                    // Refetch only when the row leaves the page in hand; the
                    // facade serves repeat windows from cache.
                    if page.as_ref().is_none_or(|p| p.start + p.rows.len() <= i) {
                        page = Some(self.views.track_list(query, i));
                    }
                    let page = page.as_ref().expect("page fetched above");
                    if let Some(track) = page.rows.get(i - page.start) {
                        self.render_track_row(ui, state, cmd, track, current_track.as_ref(), None);
                    }
                }
            },
        );
    }

    /// Render the tracks of a read-only smart playlist. The list reads
    /// through the Session Views facade over store queries (ADR 0002): every
    /// committed mutation bumps the generation, so the next frame
    /// regenerates from committed state — no manual refresh needed.
    fn render_smart_playlist_view(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        kind: SmartPlaylistKind,
    ) {
        // Bounded playlists cap at 50 entries; open-ended ones list all.
        let limit = match kind {
            SmartPlaylistKind::RecentlyAdded | SmartPlaylistKind::MostPlayed => 50,
            SmartPlaylistKind::NeverPlayed | SmartPlaylistKind::LostGems => usize::MAX,
        };
        let tracks = self.views.smart_list(kind, limit);
        let current_track = state.queue.current_track().cloned();

        // Header: name + count, clearly read-only (no edit/delete affordances),
        // with whole-list actions mirroring the album/folder header menu.
        let header = ui.horizontal(|ui| {
            ui.heading(kind.display_name());
            ui.weak(format!("({} tracks, read-only)", tracks.len()));
        });
        if !tracks.is_empty() {
            let tids: Vec<TrackId> = tracks.iter().map(|t| t.id.clone()).collect();
            show_list_context_menu(&header.response, cmd, &tids);
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
                        self.render_track_row(ui, state, cmd, track, current_track.as_ref(), None);
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
    fn render_playlists_section(&mut self, ui: &mut egui::Ui) {
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
        // id/name clones. Each row's painted label comes from the
        // generation-aware playlist label cache; ids are only cloned on a
        // click frame.
        let playlists = self.views.playlists();
        for index in 0..playlists.len() {
            let action = {
                let playlist = &playlists[index];
                let selected = self.playlist_view.as_ref() == Some(&playlist.id);
                let label =
                    self.playlist_label(&playlist.id, &playlist.name, playlist.tracks.len());
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
            }

            self.render_playlist_rename_prompt(ui, &playlists[index].id);
        }
    }

    /// The inline "New Playlist" name prompt while it is open.
    fn render_playlist_create_prompt(&mut self, ui: &mut egui::Ui) {
        if self.playlist_create_name.is_none() {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            if let Some(draft) = self.playlist_create_name.as_mut() {
                ui.text_edit_singleline(draft);
                if ui.button("Create").clicked() {
                    confirm = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            }
        });
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
        let mut confirm = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            if let Some((_, draft)) = self.playlist_rename.as_mut() {
                ui.text_edit_singleline(draft);
                if ui.button("Save").clicked() {
                    confirm = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            }
        });
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
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        playlist_id: &PlaylistId,
    ) {
        let playlists = self.views.playlists();
        let Some(playlist) = playlists.iter().find(|p| &p.id == playlist_id) else {
            ui.label("Playlist not found");
            return;
        };
        let playlist_name = &playlist.name;
        let track_count = playlist.tracks.len();

        let current_track = state.queue.current_track().cloned();

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
            show_list_context_menu(&header.response, cmd, &valid_ids);
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
                            state,
                            cmd,
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
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        playlist_id: &PlaylistId,
        entry: &(TrackId, Option<Track>, bool),
        current_track: Option<&TrackId>,
        index: usize,
    ) {
        let (tid, track, valid) = entry;
        if *valid && let Some(t) = track {
            self.render_reorderable_playlist_row(
                ui,
                state,
                cmd,
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
            self.attach_track_menu(&response, state, cmd, tid, None, Some(playlist_id));
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
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        track: &Track,
        current_track: Option<&TrackId>,
        playlist_id: &PlaylistId,
        index: usize,
    ) {
        use crate::ui::sidebar::{self, TreeRow};

        let is_selected = state.selected_track.as_ref() == Some(&track.id);
        let is_current = current_track == Some(&track.id);
        let playing = state.playback_state == PlaybackState::Playing;
        let label = self.label_artist_title(track);

        self.request_cover(&track.id, &track.file_path);

        let outcome = sidebar::reorderable_row(
            ui,
            &mut self.icons,
            &self.theme.active,
            egui::Id::new(("riff_playlist_entry", &playlist_id.0, index)),
            index,
            TreeRow {
                indent_level: 0,
                icon: None,
                label: &label,
                selected: is_selected,
                now_playing: is_current,
                playing: is_current && playing,
                disclosure: None,
            },
        );
        let response = outcome.response;
        if response.clicked() {
            state.selected_track = Some(track.id.clone());
        }
        if response.double_clicked() {
            state.selected_track = Some(track.id.clone());
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Play(track.id.clone()));
            }
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
            state,
            cmd,
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
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        let palette = self.theme.active;

        // Current track + cover from the LRU texture cache; misses enqueue a
        // background resolve exactly like the other views. Both the current
        // Track and the Up Next window come from the Session Views facade
        // over the store's `get_track` query — never the mirror. The cover
        // key is only re-cloned when the playing track moves, so fresh
        // frames allocate nothing here.
        self.views
            .sync_playback(&state.queue, crate::ui::now_playing::UP_NEXT_LIMIT);
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
            .and_then(|key| self.get_cover_texture(key));
        self.now_playing_cover_key = cover_key;
        // Text block + Up Next rows from the generation-keyed caches
        // (allocation plan 2.2): `Arc` clones per frame, strings formatted
        // only when the playing track or the queue window actually moved.
        let np_text = self.now_playing_text();
        let up_next = self.up_next_entries_cached(crate::ui::now_playing::UP_NEXT_LIMIT);

        let content = crate::ui::now_playing::NowPlayingContent {
            cover,
            title: np_text.title,
            meta_line: np_text.meta_line,
            details: np_text.details,
            position: state.current_position.current,
            total: state.current_position.total,
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
            apply_now_playing_action(action, state, cmd);
        }
    }

    fn render_folder_tree(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        query: &str,
    ) {
        if state.library_paths.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label("No library paths configured.");
            });
            return;
        }

        // Folder views read through the Session Views facade over store
        // queries (ADR 0002/0003): escaped prefix matching over stored track
        // paths, cached until the next committed mutation bumps the
        // generation. No in-memory mirror involved.
        let lib_paths = state.library_paths.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for lib_path in &lib_paths {
                if !self.views.folder_has_audio(lib_path) {
                    continue;
                }
                self.render_folder_node(ui, state, cmd, lib_path, 0, query);
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
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        path: &Path,
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
        let current_track = state.queue.current_track().cloned();

        // The playing track's id IS its stored path, so containment is a
        // plain component-wise prefix check — no store round-trip needed.
        let contains_current = current_track
            .as_ref()
            .is_some_and(|tid| std::path::Path::new(&tid.0).starts_with(path));

        let is_selected = state.selected_folder.as_deref() == Some(path);

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

        let response = sidebar::tree_row(
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
                label: &label,
                selected: is_selected,
                now_playing: false,
                playing: false,
                disclosure: Some(collapsing.is_open()),
            },
        );

        // Same gestures as before the restyle: single click toggles + selects,
        // double click plays the subtree, and the whole-list context menu
        // rides on the row.
        if response.clicked() {
            collapsing.toggle(ui);
            state.selected_folder = Some(path.to_path_buf());
        }
        if response.double_clicked() {
            play_folder(&folder_track_ids, cmd);
        }
        if !folder_track_ids.is_empty() {
            show_list_context_menu(&response, cmd, &folder_track_ids);
        }
        collapsing.store(ui.ctx());

        collapsing.show_body_unindented(ui, |ui| {
            let children = self.views.folder_children(path);
            for child_path in children.iter() {
                self.render_folder_node(ui, state, cmd, child_path, level + 1, query);
            }

            let direct = self.views.folder_direct_tracks(path);
            let tracks = folder_tracks_filtered(&direct, query);

            for track in tracks.iter().copied() {
                let label = self.label_numbered(track);
                self.interactive_track_row(
                    ui,
                    state,
                    cmd,
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

/// Parse an optional numeric modal field; empty input means "leave unset".
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

/// Apply one polled Tag Edit outcome to the modal state, the outstanding
/// request record, and the status line — the same code path
/// `poll_tag_edit_outcomes` runs per outcome. `Saved` closes the matching
/// open modal and reports the saved file; `Failed` keeps the dialog open
/// with the reason. Outcomes carry no identity, so the record captured at
/// submit time supplies both the match key and the file name; an outcome
/// with no matching outstanding request is ignored.
pub fn apply_tag_edit_outcome(
    outcome: TagEditOutcome,
    tag_edit: &mut Option<TagEditState>,
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
            if tag_edit.as_ref().is_some_and(|te| te.track_id == track_id) {
                *tag_edit = None;
            }
        }
        TagEditOutcome::Failed { reason } => {
            tracing::warn!("Tag edit failed for {:?}: {}", path, reason);
            if let Some(modal) = tag_edit.as_mut()
                && modal.track_id == track_id
            {
                modal.error = Some(reason);
                modal.saving = false;
            }
        }
    }
}

/// Validate the "Edit Tags" modal fields and submit one [`TagEditRequest`]
/// through the service seam. Invalid numeric fields keep the modal open
/// with an error and submit nothing; valid fields clear the error, flip the
/// modal into its saving state, and record the outstanding request so its
/// outcome can be matched later. Nothing is ever written without an
/// explicit Save upstream.
pub fn submit_tag_edit_fields(
    tag_edit: &mut TagEditState,
    tag_edits: &dyn TagEdits,
    in_flight: &mut Option<(TrackId, PathBuf)>,
) {
    match (
        parse_number("Year", &tag_edit.year),
        parse_number("Track number", &tag_edit.track_number),
    ) {
        (Ok(year), Ok(track_number)) => {
            tag_edit.error = None;
            tag_edit.saving = true;
            let request = TagEditRequest {
                track_id: tag_edit.track_id.clone(),
                path: tag_edit.path.clone(),
                edit: TagEdit {
                    title: Some(tag_edit.title.clone()),
                    artist: Some(tag_edit.artist.clone()),
                    album: Some(tag_edit.album.clone()),
                    album_artist: Some(tag_edit.album_artist.clone()),
                    genre: Some(tag_edit.genre.clone()),
                    year,
                    track_number,
                },
            };
            *in_flight = Some((request.track_id.clone(), request.path.clone()));
            tag_edits.submit(request);
        }
        (Err(error), _) | (_, Err(error)) => {
            tag_edit.error = Some(error);
        }
    }
}

/// Consume polled cover results into the UI texture cache: rgba→texture
/// conversion is the egui-bound work that stays on the main thread (the
/// texture boundary, ADR 0006); dedup and negative caching live behind the
/// service seam, so artless results are simply dropped here.
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
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [cover_image.width as usize, cover_image.height as usize],
            &cover_image.rgba,
        );
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
/// what the former mirror listing produced.
fn play_folder(track_ids: &[TrackId], cmd: Option<&Sender<PlaybackCommand>>) {
    let Some(s) = cmd else { return };
    let Some(first) = track_ids.first() else {
        return;
    };
    let _ = s.send(PlaybackCommand::Play(first.clone()));
    if track_ids.len() > 1 {
        let _ = s.send(PlaybackCommand::AddMany(track_ids[1..].to_vec()));
    }
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

/// Shared metadata label rows (album artist, year, genre, track/disc number).
fn render_track_meta_labels(
    ui: &mut egui::Ui,
    metadata: &crate::domain::TrackMetadata,
    show_disc: bool,
) {
    if let Some(ref aa) = metadata.album_artist
        && *aa != metadata.display_artist()
    {
        ui.label(format!("Album Artist: {aa}"));
    }
    if let Some(y) = metadata.year {
        ui.label(format!("Year: {y}"));
    }
    if let Some(g) = &metadata.genre {
        ui.label(format!("Genre: {g}"));
    }
    if let Some(tn) = metadata.track_number {
        if show_disc {
            ui.label(format!(
                "Track: {} / Disc: {}",
                tn,
                metadata.disc_number.unwrap_or(1)
            ));
        } else {
            ui.label(format!("Track: {tn}"));
        }
    }
}

/// Arguments for the shared track context menu, grouped into one value to
/// keep the call sites readable.
struct TrackMenuArgs<'a> {
    cmd: Option<&'a Sender<PlaybackCommand>>,
    track_id: &'a TrackId,
    /// The track itself; `None` (e.g. a playlist entry whose file is missing)
    /// suppresses playback actions and "Edit Tags".
    track: Option<&'a Track>,
    tag_edit: &'a mut Option<TagEditState>,
    advanced: bool,
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
/// Playlist", optional "Remove from Playlist", and (advanced mode only)
/// "Edit Tags". Queue actions are suppressed when the file is missing
/// (`track` is `None`).
fn show_track_context_menu(response: &egui::Response, args: TrackMenuArgs<'_>) {
    let TrackMenuArgs {
        cmd,
        track_id,
        track,
        tag_edit,
        advanced,
        playlists,
        playlist_store,
        remove_from_playlist,
    } = args;
    let cmd = cmd.cloned();
    let tid = track_id.clone();
    let playable = track.is_some();
    let edit_track = track.filter(|_| advanced).cloned();
    let remove_pid = remove_from_playlist.cloned();
    let playlist_options: Vec<(PlaylistId, String)> = playlists
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    response.context_menu(move |ui| {
        if playable {
            if ui.button("Play").clicked() {
                if let Some(ref s) = cmd {
                    let _ = s.send(PlaybackCommand::Play(tid.clone()));
                }
                ui.close();
            }
            if ui.button("Play Next").clicked() {
                if let Some(ref s) = cmd {
                    let _ = s.send(PlaybackCommand::PlayNext(tid.clone()));
                }
                ui.close();
            }
            if ui.button("Add to Queue").clicked() {
                if let Some(ref s) = cmd {
                    let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
                }
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
        // Edit Tags is advanced-only (REQ-UI-006).
        if let Some(ref t) = edit_track
            && ui
                .button("Edit Tags")
                .on_hover_text(
                    "Edit this track's tags (title, artist, album, and more). Changes are written to the file on Save.",
                )
                .clicked()
            {
                *tag_edit = Some(TagEditState::from_track(t));
                ui.close();
            }
    });
}

/// Shared whole-list context menu (playlist/folder headers): Play (first
/// track, then queue the rest), Play Next, and Append to Queue.
fn show_list_context_menu(
    response: &egui::Response,
    cmd: Option<&Sender<PlaybackCommand>>,
    track_ids: &[TrackId],
) {
    let cmd = cmd.cloned();
    let tids = track_ids.to_vec();
    response.context_menu(move |ui| {
        if ui.button("Play").clicked() {
            if let Some(ref s) = cmd
                && let Some(first) = tids.first()
            {
                let _ = s.send(PlaybackCommand::Play(first.clone()));
                for tid in &tids[1..] {
                    let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
                }
            }
            ui.close();
        }
        if ui.button("Play Next").clicked() {
            if let Some(ref s) = cmd {
                for tid in tids.iter().rev() {
                    let _ = s.send(PlaybackCommand::PlayNext(tid.clone()));
                }
            }
            ui.close();
        }
        if ui.button("Append to Queue").clicked() {
            if let Some(ref s) = cmd {
                for tid in &tids {
                    let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
                }
            }
            ui.close();
        }
    });
}

/// Fixed square size (points) for the cover thumbnail in the library detail
/// pane (REQ-UI-004). The Now Playing cover's size lives with the restyled
/// stage widgets (`now_playing::COVER_SIZE`, Issue 10).
const COVER_THUMB_SIZE: f32 = 200.0;

/// Render cover art inside a fixed `size` x `size` square, or a placeholder
/// (rounded box + note glyph) while no texture is loaded yet. The placeholder
/// reads the active palette's tokens — an empty surface-2 well with a muted
/// ink-3 glyph — so it themes correctly on both palettes (Issue 03). The
/// `SizedTexture` destination rect forces the image into the allotted
/// square, so oversized covers are clamped and can never overflow the layout.
fn cover_art_ui(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    texture: Option<egui::TextureHandle>,
    size: f32,
) {
    if let Some(texture) = texture {
        let sized = egui::load::SizedTexture::new(texture.id(), egui::vec2(size, size));
        ui.add(egui::Image::from_texture(sized).corner_radius(4.0));
    } else {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, palette.surface_2);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "\u{1F3B5}",
            egui::FontId::proportional(size * 0.3),
            palette.ink_3,
        );
    }
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

/// Clamp a seek request (in seconds) into `[0, total]` so a drag past the end
/// of a track seeks to the end rather than beyond it (REQ-UI-005). When the
/// total duration is unknown there is nothing to clamp against, so the seek
/// falls back to the start; non-finite inputs (NaN/infinity) do the same.
pub fn clamp_seek(secs: f32, total: Option<std::time::Duration>) -> std::time::Duration {
    let finite = if secs.is_finite() { secs } else { 0.0 };
    let upper = total.map_or(0.0, |t| t.as_secs_f32());
    std::time::Duration::from_secs_f32(finite.clamp(0.0, upper.max(0.0)))
}
