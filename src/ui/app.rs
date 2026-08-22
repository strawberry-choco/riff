use crate::app::commands::{LibraryCommand, LibraryUpdate};
use crate::app::cover_resolver::CoverResolver;
use crate::app::projection::{
    BrowsingProjection, FolderProjection, ProjectionKey, SmartPlaylistsProjection,
    TrackListProjection, WINDOW_SIZE,
};
use crate::app::state::{AppState, BrowseMode, LibraryStatus, ViewMode};
use crate::app::store::{
    LibraryMutationStore, LibraryQueryStore, PlaylistStore, SettingsStore, StoreGeneration,
};
use crate::app::traits::{CoverImage, MetadataWriter, TagEdit};
use crate::app::watcher_manager::WatcherManager;
use crate::app::MutexExt;
use crate::domain::{
    Album, Artist, PlaybackCommand, PlaybackState, Playlist, PlaylistId, RepeatMode,
    SmartPlaylistKind, Track, TrackId,
};
use crate::infra::{ImageCoverLoader, LoftyMetadataReader, LoftyMetadataWriter};
use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use elegance::Theme as EleganceTheme;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Request to write metadata tags to a track's file, sent from the UI thread
/// to the background tag-write thread.
struct TagWriteRequest {
    path: PathBuf,
    edit: TagEdit,
    track_id: TrackId,
}

/// Outcome of a background tag write, sent back to the UI thread.
struct TagWriteResult {
    track_id: TrackId,
    path: PathBuf,
    edit: TagEdit,
    outcome: Result<(), String>,
}

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

/// Theme selection state: the elegance light/dark choice plus the previous
/// frame's high-contrast state (REQ-UI-007).
struct ThemeState {
    /// `true` = dark (slate), `false` = light (frost).
    elegance_dark: bool,
    /// Whether the previous frame rendered the high-contrast theme; used to
    /// invalidate the elegance install cache when leaving high contrast.
    was_high_contrast: bool,
}

/// Max entries per cover cache (positive textures and negative results
/// alike); the oldest entries are evicted LRU-style beyond this cap.
const COVER_CACHE_CAP: usize = 50;

/// Insert `key` at the most-recently-used end of an LRU key list: an already
/// present entry is moved to the end (no duplicates), and keys evicted beyond
/// `cap` are returned so the caller can drop their cached payloads. Shared by
/// the positive (texture) and negative (artless-track) cover caches.
pub fn lru_insert(keys: &mut Vec<String>, key: String, cap: usize) -> Vec<String> {
    keys.retain(|k| k != &key);
    keys.push(key);
    let mut evicted = Vec::new();
    while keys.len() > cap {
        evicted.push(keys.remove(0));
    }
    evicted
}

pub struct RiffApp {
    pub state: Arc<Mutex<AppState>>,
    command_sender: Option<Sender<PlaybackCommand>>,
    library_command_sender: Option<Sender<LibraryCommand>>,
    library_update_rx: Option<Receiver<LibraryUpdate>>,
    cover_textures: std::collections::HashMap<String, egui::TextureHandle>,
    cover_lru_keys: Vec<String>,
    /// Tracks (by key) whose cover resolve came back empty, so
    /// `request_cover` does not re-enqueue the same disk/tag I/O every
    /// frame. Same LRU discipline as `cover_lru_keys`.
    cover_negative_keys: Vec<String>,
    cover_request_tx: Option<Sender<(TrackId, PathBuf)>>,
    cover_response_rx: Receiver<(String, Option<CoverImage>)>,
    tag_write_request_tx: Option<Sender<TagWriteRequest>>,
    tag_write_result_rx: Receiver<TagWriteResult>,
    tag_edit: Option<TagEditState>,
    /// Which read-only smart playlist is open in the library explorer, if any.
    /// Transient UI state (precedent: `tag_edit`); the playlist contents are
    /// re-computed from library data on every frame, so nothing is cached.
    smart_playlist_view: Option<SmartPlaylistKind>,
    /// Which user playlist is open in the library explorer, if any.
    playlist_view: Option<PlaylistId>,
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
    /// commits through it as one immediate durable transaction; the in-memory
    /// `state.playlists` list is only a Session Projection refreshed from the
    /// store after each committed change.
    pub(crate) playlist_store: Box<dyn PlaylistStore>,
    /// The Application Store's Library collection query port: the flat list
    /// and search box fetch bounded windows through it, and startup hydrates
    /// the transitional in-memory mirror from it.
    pub(crate) library_queries: Box<dyn LibraryQueryStore>,
    /// The Application Store's Library collection mutation port: committed
    /// metadata changes (e.g. tag edits) persist through it as one durable
    /// transaction per batch.
    pub(crate) library_mutations: Box<dyn LibraryMutationStore>,
    /// Session-local generation counter bumped after each committed store
    /// mutation; projections compare against it to know when to refetch
    /// (ADR 0002).
    pub(crate) store_generation: StoreGeneration,
    /// Bounded Session Projection serving the flat list and search box
    /// (ADR 0003). Never authoritative; invalidated by generation bumps.
    tracks_projection: TrackListProjection,
    /// Session Projection serving the artist/album browsing views: artists
    /// A–Z, per-artist albums, and per-album tracks, all fetched from the
    /// store through [`Self::library_queries`] and invalidated by generation
    /// bumps. Never authoritative.
    browsing_projection: BrowsingProjection,
    /// Session Projection serving the folder-tree views: subtree probes,
    /// subtree search matches, subtree track ids, direct track listings,
    /// and child directories, all fetched from the store through
    /// [`Self::library_queries`] and invalidated by generation bumps.
    folder_projection: FolderProjection,
    /// Session Projection serving the read-only smart playlists: one
    /// computed list per kind, fetched from the store through
    /// [`Self::library_queries`] and invalidated by generation bumps, so a
    /// finished play or scan regenerates them on the next frame.
    smart_playlists_projection: SmartPlaylistsProjection,
    theme: ThemeState,
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
        library_queries: Box<dyn LibraryQueryStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        store_generation: StoreGeneration,
    ) -> Self {
        let resolver = CoverResolver::new(
            Box::new(LoftyMetadataReader::new()),
            Box::new(ImageCoverLoader::new()),
        );

        let (cover_tx, cover_rx_inner): (Sender<(TrackId, PathBuf)>, _) = unbounded();
        let (response_tx, response_rx): (Sender<(String, Option<CoverImage>)>, _) = unbounded();

        std::thread::spawn(move || {
            while let Ok((track_id, path)) = cover_rx_inner.recv() {
                let result = match resolver.resolve(&path) {
                    Ok(val) => val,
                    Err(e) => {
                        tracing::warn!("Cover resolution failed for {:?}: {}", path, e);
                        None
                    }
                };
                let _ = response_tx.send((track_id.0.clone(), result));
            }
        });

        // Tag-write background thread: owns the lofty-based writer, processes
        // write requests off the UI thread, and reports outcomes back.
        let writer = Box::new(LoftyMetadataWriter::new());
        let (tag_write_tx, tag_write_rx): (Sender<TagWriteRequest>, Receiver<TagWriteRequest>) =
            unbounded();
        let (tag_result_tx, tag_result_rx): (Sender<TagWriteResult>, Receiver<TagWriteResult>) =
            unbounded();

        std::thread::spawn(move || {
            while let Ok(request) = tag_write_rx.recv() {
                let outcome = match writer.write_metadata(&request.path, &request.edit) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        tracing::warn!("Tag write failed for {:?}: {}", request.path, e);
                        Err(e.to_string())
                    }
                };
                let _ = tag_result_tx.send(TagWriteResult {
                    track_id: request.track_id,
                    path: request.path,
                    edit: request.edit,
                    outcome,
                });
            }
        });

        Self {
            state,
            command_sender: Some(command_sender),
            library_command_sender: Some(library_command_sender),
            library_update_rx: Some(library_update_rx),
            cover_textures: std::collections::HashMap::new(),
            cover_lru_keys: Vec::new(),
            cover_negative_keys: Vec::new(),
            cover_request_tx: Some(cover_tx),
            cover_response_rx: response_rx,
            tag_write_request_tx: Some(tag_write_tx),
            tag_write_result_rx: tag_result_rx,
            tag_edit: None,
            smart_playlist_view: None,
            playlist_view: None,
            playlist_create_name: None,
            playlist_rename: None,
            clear_library_confirm: false,
            search_focus: false,
            first_frame: true,
            watcher_manager,
            settings_store,
            playlist_store,
            library_queries,
            library_mutations,
            store_generation,
            tracks_projection: TrackListProjection::new(ProjectionKey::Flat),
            browsing_projection: BrowsingProjection::new(),
            folder_projection: FolderProjection::new(),
            smart_playlists_projection: SmartPlaylistsProjection::new(),
            theme: ThemeState {
                elegance_dark: true, // dark (slate) by default
                was_high_contrast: false,
            },
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

    /// Apply the active theme to the context (REQ-UI-007). When `high_contrast`
    /// is set, the high-contrast visuals override the elegance light/dark
    /// theme. When leaving high contrast, the elegance theme's install cache is
    /// invalidated first: `EleganceTheme::install` skips re-applying the style
    /// if it believes its theme is already installed, which would otherwise
    /// leave the high-contrast palette stuck after the toggle is turned off.
    fn apply_theme(&mut self, ctx: &egui::Context, high_contrast: bool) {
        if high_contrast {
            ctx.set_visuals(high_contrast_visuals());
            self.theme.was_high_contrast = true;
            return;
        }

        if self.theme.was_high_contrast {
            // Break elegance's "already installed" cache by installing the
            // opposite theme first, so the correct install below actually runs.
            if self.theme.elegance_dark {
                EleganceTheme::frost().install(ctx);
            } else {
                EleganceTheme::slate().install(ctx);
            }
            self.theme.was_high_contrast = false;
        }

        if self.theme.elegance_dark {
            EleganceTheme::slate().install(ctx);
        } else {
            EleganceTheme::frost().install(ctx);
        }
    }

    fn request_cover(&self, track_id: &TrackId, file_path: &Path) {
        let key = &track_id.0;
        if !self.cover_textures.contains_key(key) && !self.cover_negative_keys.contains(key) {
            if let Some(ref tx) = self.cover_request_tx {
                let _ = tx.send((track_id.clone(), file_path.to_path_buf()));
            }
        }
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

    /// Drain completed tag writes from the background thread. On success the
    /// edited metadata lands in the mirror and persists through the store's
    /// targeted tag-refresh flow as one durable transaction (history
    /// preserved, album year/genre re-derived). On failure the error is
    /// surfaced in the open modal, which stays open.
    fn poll_tag_write_results(&mut self, state: &mut AppState) {
        while let Ok(result) = self.tag_write_result_rx.try_recv() {
            match result.outcome {
                Ok(()) => {
                    let mut edited: Option<Track> = None;
                    if let Some(track) = state.library.tracks.get_mut(&result.track_id) {
                        result.edit.apply_to(&mut track.metadata);
                        edited = Some(track.clone());
                    }
                    if let Some(track) = edited {
                        if let Err(e) = self.library_mutations.apply_tag_refresh(&track) {
                            tracing::error!(
                                "Failed to persist tag edit for {:?}: {e}",
                                result.path
                            );
                        } else {
                            self.store_generation.bump();
                        }
                    }
                    if self
                        .tag_edit
                        .as_ref()
                        .is_some_and(|te| te.track_id == result.track_id)
                    {
                        self.tag_edit = None;
                    }
                    let name = result.path.file_name().map_or_else(
                        || result.path.to_string_lossy().to_string(),
                        |n| n.to_string_lossy().to_string(),
                    );
                    state.scan_status = Some(format!("Tags saved for {name}"));
                    tracing::info!("Tags written for {:?}", result.path);
                }
                Err(message) => {
                    tracing::warn!("Tag write failed for {:?}: {}", result.path, message);
                    if let Some(ref mut tag_edit) = self.tag_edit {
                        if tag_edit.track_id == result.track_id {
                            tag_edit.error = Some(message);
                            tag_edit.saving = false;
                        }
                    }
                }
            }
        }
    }

    fn update_cover_cache(&mut self, ctx: &egui::Context) {
        while let Ok((key, cover)) = self.cover_response_rx.try_recv() {
            if let Some(cover_image) = cover {
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [cover_image.width as usize, cover_image.height as usize],
                    &cover_image.rgba,
                );
                let texture = ctx.load_texture(&key, color_image, egui::TextureOptions::default());
                self.cover_textures.insert(key.clone(), texture);
                for old in lru_insert(&mut self.cover_lru_keys, key.clone(), COVER_CACHE_CAP) {
                    self.cover_textures.remove(&old);
                }
                // Art arrived after a negative result (or an eviction
                // retry): drop any stale negative entry.
                self.cover_negative_keys.retain(|k| k != &key);
            } else {
                // Negative cache: remember artless tracks so `request_cover`
                // stops re-enqueueing a resolve request (disk/tag I/O) every
                // frame. Eviction allows an eventual retry, e.g. once art is
                // added and the cache cycles.
                lru_insert(&mut self.cover_negative_keys, key, COVER_CACHE_CAP);
            }
        }
    }

    /// Render the "Edit Tags" modal while `self.tag_edit` is open. Writing
    /// only happens on an explicit Save click; Cancel (or the window close
    /// button) discards the edits.
    fn show_tag_edit_modal(&mut self, ctx: &egui::Context) {
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
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), error);
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

    /// Validate the modal fields and send a [`TagWriteRequest`] to the
    /// background tag-write thread. Invalid numeric fields keep the modal
    /// open with an error; nothing is ever written without an explicit Save.
    fn submit_tag_edit(&mut self) {
        let Some(tag_edit) = self.tag_edit.as_mut() else {
            return;
        };

        let request = match (
            parse_number("Year", &tag_edit.year),
            parse_number("Track number", &tag_edit.track_number),
        ) {
            (Ok(year), Ok(track_number)) => {
                tag_edit.error = None;
                tag_edit.saving = true;
                Some(TagWriteRequest {
                    path: tag_edit.path.clone(),
                    track_id: tag_edit.track_id.clone(),
                    edit: TagEdit {
                        title: Some(tag_edit.title.clone()),
                        artist: Some(tag_edit.artist.clone()),
                        album: Some(tag_edit.album.clone()),
                        album_artist: Some(tag_edit.album_artist.clone()),
                        genre: Some(tag_edit.genre.clone()),
                        year,
                        track_number,
                    },
                })
            }
            (Err(error), _) | (_, Err(error)) => {
                tag_edit.error = Some(error);
                None
            }
        };

        if let Some(request) = request {
            if let Some(ref tx) = self.tag_write_request_tx {
                let _ = tx.send(request);
            }
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
        let playlists_slot = &mut state.playlists;
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
                playlists: playlists_slot,
                playlist_store: playlist_store_slot,
                remove_from_playlist,
            },
        );
    }

    /// One library-list track row: playing indicator, selectable label,
    /// click/double-click handling, and the shared context menu.
    fn render_track_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        track: &Track,
        current_track: Option<&TrackId>,
        remove_from_playlist: Option<&PlaylistId>,
    ) {
        let is_selected = state.selected_track.as_ref() == Some(&track.id);
        let is_playing = current_track == Some(&track.id);

        self.request_cover(&track.id, &track.file_path);

        ui.horizontal(|ui| {
            ui.set_min_height(20.0);
            if is_playing {
                ui.label("\u{25B6}");
            }
            let label = format!(
                "{} - {}",
                track.metadata.display_artist(),
                track.metadata.display_title(&track.file_path)
            );
            let response = ui.selectable_label(is_selected, label);
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
        });
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

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let cmd = self.command_sender.clone();
        let lib_cmd = self.library_command_sender.clone();
        let state_arc = self.state.clone();

        let mut state = state_arc.lock_or_recover();

        if self.quit_flag.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if self.first_frame {
            load_persisted_state(
                &mut state,
                self.settings_store.as_ref(),
                self.playlist_store.as_ref(),
                self.library_queries.as_ref(),
                cmd.as_ref(),
            );
            self.first_frame = false;
        }

        // Apply the active theme (REQ-UI-007 accessibility). Done after the
        // first-frame load so a persisted high-contrast choice takes effect on
        // the very first frame. High contrast overrides the elegance theme.
        self.apply_theme(ui.ctx(), state.ui_flags.high_contrast);

        self.poll_library_updates(&mut state);
        self.poll_tag_write_results(&mut state);
        self.update_cover_cache(ui.ctx());
        self.poll_watchers();

        handle_keyboard_shortcuts(ui.ctx(), &mut state, &mut self.search_focus, cmd.as_ref());

        // Update window title and tray tooltip (REQ-SI-001). The tooltip shows
        // "Artist - Title" for the current track, else "riff".
        #[cfg(not(target_os = "linux"))]
        {
            let tray_tooltip = update_window_title(ui.ctx(), &state);
            if self.last_tray_tooltip != tray_tooltip {
                if let Some(ref tray) = self.tray_icon {
                    crate::ui::tray::update_tooltip(tray, &tray_tooltip);
                }
                self.last_tray_tooltip = tray_tooltip;
            }
        }
        #[cfg(target_os = "linux")]
        {
            update_window_title(ui.ctx(), &state);
        }

        let theme = &mut self.theme;
        Self::render_top_bar(theme, ui, &mut state, self.settings_store.as_mut());
        self.render_control_bar(ui, &mut state, cmd.as_ref());

        // --- MAIN CONTENT ---
        match state.view_mode {
            ViewMode::Library => self.show_library_view(ui, &mut state, cmd.as_ref()),
            ViewMode::NowPlaying => self.show_now_playing_view(ui, &mut state, cmd.as_ref()),
            ViewMode::Settings => {
                self.show_settings_view(ui, &mut state, lib_cmd.as_ref(), frame);
            }
        }

        // --- EDIT TAGS MODAL ---
        self.show_tag_edit_modal(ui.ctx());

        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

// --- Per-frame helpers -------------------------------------------------------

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

/// Refresh the playlists Session Projection from the Application Store.
/// Failures are logged; the previous projection stands until the next
/// successful load.
fn reload_playlists(store: &dyn PlaylistStore, state: &mut AppState) {
    match store.load_playlists() {
        Ok(playlists) => state.playlists = playlists,
        Err(e) => tracing::warn!("Failed to load playlists from the store: {e}"),
    }
}

/// First-frame restore. The library hydrates from the Application Store
/// through the [`LibraryQueryStore`] port into the transitional in-memory
/// mirror that still serves views not yet migrated to bounded store queries;
/// playlists hydrate through the [`PlaylistStore`] port and every user
/// preference from the typed settings tables via [`SettingsStore`]. The
/// legacy JSON cache is never read or written. Public so the hydration
/// contract is testable headlessly.
pub fn load_persisted_state(
    state: &mut AppState,
    store: &dyn SettingsStore,
    playlist_store: &dyn PlaylistStore,
    library_queries: &dyn LibraryQueryStore,
    cmd: Option<&Sender<PlaybackCommand>>,
) {
    match library_queries.load_collection() {
        Ok(collection) => {
            let mut library = crate::app::library_manager::LibraryManager::new();
            // The snapshot arrives in the exact shapes the former JSON cache
            // carried: albums keyed by the "album artist - title" composite,
            // artists listing their album keys in first-added order.
            for artist in collection.artists {
                library.artists.insert(artist.name.clone(), artist);
            }
            for album in collection.albums {
                let key = format!("{} - {}", album.artist, album.title);
                library.albums.insert(key, album);
            }
            for track in collection.tracks.into_values() {
                library.tracks.insert(track.id.clone(), track);
            }
            state.library = library;
        }
        Err(e) => {
            tracing::warn!("Failed to hydrate the library from the store: {e}");
            // Surface WHY the library is empty: silence looks like data loss.
            state.scan_status =
                Some("Library could not be loaded from the store \u{2014} it will be rebuilt on the next scan.".to_string());
        }
    }
    // Playlists are user data in the Application Store, so they survive a
    // Clear Library (which wipes collection data only).
    reload_playlists(playlist_store, state);

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

/// Send the window title for the current track and return the tray tooltip
/// text (REQ-SI-001).
fn update_window_title(ctx: &egui::Context, state: &AppState) -> String {
    let mut tooltip = "riff".to_owned();
    if let Some(track_id) = state.queue.current_track() {
        if let Some(track) = state.library.get_track(track_id) {
            let artist = track.metadata.display_artist();
            let title = track.metadata.display_title(&track.file_path);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "{artist} - {title} \u{2014} riff"
            )));
            tooltip = format!("{artist} - {title}");
        }
    } else {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("riff".to_owned()));
    }
    tooltip
}

impl RiffApp {
    fn render_top_bar(
        theme: &mut ThemeState,
        ui: &mut egui::Ui,
        state: &mut AppState,
        settings_store: &mut dyn SettingsStore,
    ) {
        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("riff");

                if let Some(ref status) = state.scan_status {
                    ui.label(status);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let theme_icon = if theme.elegance_dark {
                        "\u{2600}"
                    } else {
                        "\u{1F319}"
                    };
                    if ui
                        .button(theme_icon)
                        .on_hover_text("Toggle light or dark theme")
                        .clicked()
                    {
                        theme.elegance_dark = !theme.elegance_dark;
                    }
                    if ui.button("\u{2699}").on_hover_text("Settings").clicked() {
                        state.view_mode = ViewMode::Settings;
                    }
                    if ui
                        .button("\u{1F3B5}")
                        .on_hover_text("Now Playing")
                        .clicked()
                    {
                        state.view_mode = match state.view_mode {
                            ViewMode::Library => ViewMode::NowPlaying,
                            ViewMode::NowPlaying | ViewMode::Settings => ViewMode::Library,
                        };
                    }
                    // Progressive disclosure toggle (REQ-UI-006): the simple→
                    // advanced path. Persisted so the choice survives restarts.
                    let advanced_label = if state.ui_flags.advanced_mode {
                        "Advanced: On"
                    } else {
                        "Advanced: Off"
                    };
                    if ui
                        .button(advanced_label)
                        .on_hover_text(
                            "Reveals power features: tag editing, smart playlists, \
                         and extra transport controls (stop, repeat).",
                        )
                        .clicked()
                    {
                        state.ui_flags.advanced_mode = !state.ui_flags.advanced_mode;
                        persist_scalars(settings_store, state);
                    }
                });
            });
        });
    }
}

impl RiffApp {
    fn render_control_bar(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        egui::Panel::bottom("control_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                Self::render_transport_buttons(ui, state, cmd);
                ui.separator();
                Self::render_progress_row(ui, state, cmd);
                ui.separator();
                self.render_volume_and_mode(ui, state, cmd);
            });
        });
    }

    /// Previous / Stop / Play-Pause / Next buttons. Stop is an advanced
    /// affordance (REQ-UI-006); the minimal bar keeps only prev, play/pause,
    /// and next.
    fn render_transport_buttons(
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        let playing = state.playback_state == PlaybackState::Playing;
        let paused = state.playback_state == PlaybackState::Paused;

        if ui
            .button("\u{23EE}")
            .on_hover_text("Previous track")
            .clicked()
        {
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Previous);
            }
        }
        if state.ui_flags.advanced_mode && ui.button("\u{23F9}").on_hover_text("Stop").clicked() {
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Stop);
            }
        }
        if playing {
            if ui.button("\u{23F8}").on_hover_text("Pause").clicked() {
                if let Some(s) = cmd {
                    let _ = s.send(PlaybackCommand::Pause);
                }
            }
        } else if paused {
            if ui.button("\u{25B6}").on_hover_text("Play").clicked() {
                if let Some(s) = cmd {
                    let _ = s.send(PlaybackCommand::Resume);
                }
            }
        } else if ui.button("\u{25B6}").on_hover_text("Play").clicked() {
            if let Some(ref selected) = state.selected_track {
                if let Some(s) = cmd {
                    let _ = s.send(PlaybackCommand::Play(selected.clone()));
                }
            }
        }
        if ui.button("\u{23ED}").on_hover_text("Next track").clicked() {
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Next);
            }
        }
    }

    /// Position label + clickable/seekable progress bar.
    fn render_progress_row(
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        let progress = state.current_position.total.map_or(0.0, |t| {
            if t.as_secs() > 0 {
                state.current_position.current.as_secs_f32() / t.as_secs_f32()
            } else {
                0.0
            }
        });
        let current_str = format_duration(state.current_position.current);
        let total_str = state
            .current_position
            .total
            .map_or_else(|| "--:--".to_string(), format_duration);
        ui.label(format!("{current_str} / {total_str}"));

        let pr = ui.add(
            egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                .show_percentage()
                .desired_width(200.0),
        );
        if pr.clicked() {
            if let Some(total) = state.current_position.total {
                if let Some(pos) = pr.interact_pointer_pos() {
                    let frac = ((pos.x - pr.rect.min.x) / pr.rect.width()).clamp(0.0, 1.0);
                    if let Some(s) = cmd {
                        let _ = s.send(PlaybackCommand::Seek(std::time::Duration::from_secs_f32(
                            frac * total.as_secs_f32(),
                        )));
                    }
                }
            }
        }
    }

    /// Mute, volume slider, queue position, shuffle and (advanced) repeat.
    fn render_volume_and_mode(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        // Mute toggle (REQ-UI-003-08): a core control, always visible
        // (not gated behind advanced mode). Muting never moves the
        // volume slider — it only zeroes the effective volume sent to
        // the engine; unmuting restores the slider's value.
        let (mute_icon, mute_tip) = if state.muted {
            ("\u{1F507}", "Unmute")
        } else {
            ("\u{1F50A}", "Mute")
        };
        if ui.button(mute_icon).on_hover_text(mute_tip).clicked() {
            state.muted = !state.muted;
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::SetVolume(state.effective_volume()));
            }
        }
        let mut vol = state.current_volume;
        if ui
            .add(egui::Slider::new(&mut vol, 0.0..=1.0))
            .on_hover_text("Volume")
            .changed()
        {
            state.current_volume = vol;
            persist_scalars(self.settings_store.as_mut(), state);
            if let Some(s) = cmd {
                // While muted the slider still edits current_volume,
                // but the engine keeps receiving 0 until unmuted.
                let _ = s.send(PlaybackCommand::SetVolume(state.effective_volume()));
            }
        }
        ui.separator();

        let cidx = state.queue.current_index.map_or(0, |i| i + 1);
        ui.label(format!("{}/{}", cidx, state.queue.tracks.len()));

        let shuff = state.queue.shuffle;
        if ui
            .button(if shuff {
                "\u{1F500}"
            } else {
                "\u{27A1}\u{FE0F}"
            })
            .on_hover_text("Toggle shuffle")
            .clicked()
        {
            state.queue.set_shuffle(!shuff);
        }
        // Repeat is an advanced affordance (REQ-UI-006).
        if state.ui_flags.advanced_mode {
            let rep = match state.queue.repeat {
                RepeatMode::None => "\u{23F9}",
                RepeatMode::All => "\u{1F501}",
                RepeatMode::One => "\u{1F502}",
            };
            if ui
                .button(rep)
                .on_hover_text("Cycles repeat mode: off, repeat all, repeat one.")
                .clicked()
            {
                state.queue.toggle_repeat();
            }
        }
    }
}

// --- Helper methods factored out to avoid borrow conflicts ---
impl RiffApp {
    fn show_library_view(
        &mut self,
        parent_ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        egui::Panel::left("library_panel").show_inside(parent_ui, |ui| {
            ui.label("Library");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("\u{1F50D}");
                let sr = ui.text_edit_singleline(&mut state.search_query);
                if self.search_focus {
                    sr.request_focus();
                    self.search_focus = false;
                }
                if ui
                    .button("\u{2715}")
                    .on_hover_text("Clear search")
                    .clicked()
                {
                    state.search_query.clear();
                }
            });
            ui.separator();

            // Library / Folders view toggle
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(state.browse_mode == BrowseMode::Library, "Library")
                    .clicked()
                {
                    state.browse_mode = BrowseMode::Library;
                }
                if ui
                    .selectable_label(state.browse_mode == BrowseMode::Folders, "Folders")
                    .clicked()
                {
                    state.browse_mode = BrowseMode::Folders;
                    self.smart_playlist_view = None;
                    self.playlist_view = None;
                }
            });
            ui.separator();

            let query = state.search_query.clone();

            match state.browse_mode {
                BrowseMode::Library => self.render_library_browser(ui, state, cmd, &query),
                BrowseMode::Folders => self.render_folder_tree(ui, state, cmd, &query),
            }
        });

        // Right side: track details + cover
        self.render_track_details_panel(parent_ui, state);
    }

    /// Left-panel content in Library browse mode: the All Tracks / Artists
    /// sub-toggle, smart playlists, user playlists, and the results dispatch.
    fn render_library_browser(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        query: &str,
    ) {
        // Existing sub-toggle: All Tracks / Artists. Selecting
        // either one closes any open smart playlist.
        ui.horizontal(|ui| {
            let no_playlist = self.smart_playlist_view.is_none() && self.playlist_view.is_none();
            if ui
                .selectable_label(
                    !state.ui_flags.show_artists_view && no_playlist,
                    "All Tracks",
                )
                .clicked()
            {
                state.ui_flags.show_artists_view = false;
                self.smart_playlist_view = None;
                self.playlist_view = None;
            }
            if ui
                .selectable_label(state.ui_flags.show_artists_view && no_playlist, "Artists")
                .clicked()
            {
                state.ui_flags.show_artists_view = true;
                self.smart_playlist_view = None;
                self.playlist_view = None;
            }
        });
        ui.separator();

        // Smart Playlists: four read-only, auto-generated lists
        // derived from local play history. They are virtual, so
        // they never appear while searching. Advanced-only
        // (REQ-UI-006): hidden entirely in the minimal UI.
        if state.ui_flags.advanced_mode && query.is_empty() {
            ui.label("Smart Playlists")
                .on_hover_text("Auto-generated, read-only lists built from your play history.");
            for kind in SmartPlaylistKind::ALL {
                let selected = self.smart_playlist_view == Some(kind);
                if ui.selectable_label(selected, kind.display_name()).clicked() {
                    self.smart_playlist_view = Some(kind);
                    self.playlist_view = None;
                }
            }
            ui.separator();
        }

        // User playlists (Task 4.2): named, editable lists persisted in the
        // Application Store. A core feature — always visible (NOT
        // gated behind advanced mode). Hidden while searching, like
        // smart playlists.
        if query.is_empty() {
            self.render_playlists_section(ui, state);
        }

        let has_results = query.is_empty() || !state.library.search(query).is_empty();
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

    /// Right-side detail pane: selected track metadata + cover, or a hint.
    fn render_track_details_panel(&mut self, parent_ui: &mut egui::Ui, state: &mut AppState) {
        egui::CentralPanel::default().show_inside(parent_ui, |ui| {
            let Some(track_id) = state.selected_track.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a track to view details");
                });
                return;
            };
            let Some(track) = state.library.get_track(&track_id) else {
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
                    let path_display = track.file_path.to_string_lossy().to_string();
                    ui.label(format!("File: {path_display}"));
                });
                ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                    let texture = self.get_cover_texture(&track.id.0);
                    cover_art_ui(ui, texture, COVER_THUMB_SIZE);
                });
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
        // Browsing reads through the Session Projection over store queries
        // (ADR 0002/0003): artists A–Z straight from the Application Store,
        // each level cached until the next committed mutation bumps the
        // generation. No in-memory mirror involved.
        let generation = self.store_generation.current();
        let artists: Vec<Artist> = match self
            .browsing_projection
            .artists(generation, &mut || self.library_queries.all_artists())
        {
            Ok(artists) => artists,
            Err(e) => {
                tracing::warn!("Failed to load artists from the store: {e}");
                Vec::new()
            }
        };
        let artists: Vec<Artist> = if query.is_empty() {
            artists
        } else {
            let q = query.to_lowercase();
            artists
                .into_iter()
                .filter(|a| a.name.to_lowercase().contains(&q))
                .collect()
        };
        let current_track = state.queue.current_track().cloned();

        // Identity of the playing track's album, used to auto-open exactly
        // the collapsed headers containing it — the same outcome as the
        // former scan of every artist's albums, without loading closed
        // artists' data.
        let current_album: Option<(String, String)> = current_track.as_ref().and_then(|tid| {
            self.library_queries.get_track(tid).ok().flatten().map(|t| {
                (
                    t.metadata.display_album_artist(),
                    t.metadata.display_album(),
                )
            })
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            for artist in &artists {
                let artist_has_current = current_album
                    .as_ref()
                    .is_some_and(|(album_artist, _)| album_artist == &artist.name);
                egui::CollapsingHeader::new(&artist.name)
                    .default_open(artist_has_current)
                    .show(ui, |ui| {
                        let albums: Vec<Album> = match self.browsing_projection.artist_albums(
                            generation,
                            &artist.name,
                            &mut |a| self.library_queries.artist_albums(a),
                        ) {
                            Ok(albums) => albums,
                            Err(e) => {
                                tracing::warn!("Failed to load albums for {}: {e}", artist.name);
                                return;
                            }
                        };

                        for album in &albums {
                            let album_has_current = current_album.as_ref().is_some_and(
                                |(album_artist, album_title)| {
                                    album_artist == &album.artist && album_title == &album.title
                                },
                            );
                            let year_str = album.year.map_or(String::new(), |y| format!(" ({y})"));
                            egui::CollapsingHeader::new(format!("{}{}", album.title, year_str))
                                .default_open(album_has_current)
                                .show(ui, |ui| {
                                    let tracks: Vec<Track> =
                                        match self.browsing_projection.album_tracks(
                                            generation,
                                            &album.artist,
                                            &album.title,
                                            &mut |a, t| self.library_queries.album_tracks(a, t),
                                        ) {
                                            Ok(tracks) => tracks,
                                            Err(e) => {
                                                tracing::warn!(
                                                    "Failed to load tracks for {}: {e}",
                                                    album.title
                                                );
                                                return;
                                            }
                                        };
                                    self.render_album_track_rows(
                                        ui,
                                        state,
                                        cmd,
                                        &tracks,
                                        current_track.as_ref(),
                                    );
                                });
                        }
                    });
            }
        });
    }

    /// Track rows for one album in the Artists view, rendered straight from
    /// the store query results in their canonical order (track number then
    /// filename, missing numbers first).
    fn render_album_track_rows(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        tracks: &[Track],
        current_track: Option<&TrackId>,
    ) {
        for track in tracks {
            let is_selected = state.selected_track.as_ref() == Some(&track.id);
            let is_current = current_track == Some(&track.id);

            self.request_cover(&track.id, &track.file_path);

            ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                if is_current {
                    ui.label("\u{25B6}");
                }
                let display = format!(
                    "{}. {}",
                    track.metadata.track_number.unwrap_or(0),
                    track.metadata.display_title(&track.file_path)
                );
                let resp = ui.selectable_label(is_selected, display);
                if resp.clicked() {
                    state.selected_track = Some(track.id.clone());
                }
                if resp.double_clicked() {
                    state.selected_track = Some(track.id.clone());
                    if let Some(s) = cmd {
                        let _ = s.send(PlaybackCommand::Play(track.id.clone()));
                    }
                }
                self.attach_track_menu(&resp, state, cmd, &track.id, Some(track), None);
            });
        }
    }

    fn render_flat_view(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        query: &str,
    ) {
        // The flat list and search box are served through the bounded
        // Session Projection over store queries (ADR 0003): only visible row
        // windows fetch, invalidated by generation bumps after committed
        // mutations.
        let key = if query.is_empty() {
            ProjectionKey::Flat
        } else {
            ProjectionKey::Search(query.to_string())
        };
        if self.tracks_projection.key() != &key {
            self.tracks_projection.set_key(key);
        }

        // The authoritative total comes from the store whenever the
        // projection is invalidated; fresh frames reuse the cached count.
        // `total_generation` records when that count was read so the refresh
        // below can detect a mutation landing mid-frame.
        let total_generation = self.store_generation.current();
        let total = if self.tracks_projection.is_fresh(total_generation) {
            self.tracks_projection.total()
        } else if query.is_empty() {
            self.library_queries.track_count().unwrap_or(0)
        } else {
            self.library_queries.search_count(query).unwrap_or(0)
        };

        let current_track = state.queue.current_track().cloned();

        egui::ScrollArea::vertical().show_rows(ui, 22.0, total, |ui, row_range| {
            // Declare exactly the visible windows, then refresh the
            // projection from the store before painting rows.
            let start_window = (row_range.start / WINDOW_SIZE) * WINDOW_SIZE;
            let end_window = (row_range.end / WINDOW_SIZE) * WINDOW_SIZE;
            let mut window = start_window;
            while window <= end_window {
                self.tracks_projection.request_window(window);
                window += WINDOW_SIZE;
            }
            let generation = self.store_generation.current();
            // If a mutation committed between the outer count read and here,
            // recount so the cached total agrees with the refreshed rows;
            // otherwise reuse the outer read (one COUNT query per frame max).
            let effective_total = if generation == total_generation {
                total
            } else if query.is_empty() {
                self.library_queries.track_count().unwrap_or(0)
            } else {
                self.library_queries.search_count(query).unwrap_or(0)
            };
            let _ = self.tracks_projection.refresh(
                generation,
                effective_total,
                &mut |offset, limit| {
                    if query.is_empty() {
                        self.library_queries.tracks_window(offset, limit)
                    } else {
                        self.library_queries.search_window(query, offset, limit)
                    }
                },
            );

            for i in row_range {
                let window_start = (i / WINDOW_SIZE) * WINDOW_SIZE;
                // Clone the row so the projection's shared borrow ends before
                // the mutable render call (one small clone per visible row).
                let row = self
                    .tracks_projection
                    .window(window_start)
                    .and_then(|rows| rows.get(i - window_start))
                    .cloned();
                if let Some(track) = row {
                    self.render_track_row(ui, state, cmd, &track, current_track.as_ref(), None);
                }
            }
        });
    }

    /// Cached smart-playlist computation for one frame generation; failures
    /// render as an empty list with a warning logged.
    fn smart_list_cached(
        &mut self,
        generation: u64,
        kind: SmartPlaylistKind,
        limit: usize,
    ) -> Vec<Track> {
        match self
            .smart_playlists_projection
            .list(generation, kind, limit, &mut |k, l| {
                self.library_queries.smart_playlist(k, l)
            }) {
            Ok(tracks) => tracks,
            Err(e) => {
                tracing::warn!(
                    "Failed to compute smart playlist {}: {e}",
                    kind.display_name()
                );
                Vec::new()
            }
        }
    }

    /// Render the tracks of a read-only smart playlist. The list reads
    /// through the Session Projection over store queries (ADR 0002): every
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
        let generation = self.store_generation.current();
        let tracks = self.smart_list_cached(generation, kind, limit);
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

        egui::ScrollArea::vertical().show_rows(ui, 22.0, tracks.len(), |ui, row_range| {
            for i in row_range {
                if let Some(track) = tracks.get(i) {
                    self.render_track_row(ui, state, cmd, track, current_track.as_ref(), None);
                }
            }
        });
    }

    /// Render the "Playlists" section of the library explorer: the user's
    /// playlists with select / rename / delete affordances, plus the create
    /// and rename prompts. Every mutation commits through the
    /// [`PlaylistStore`] port as one immediate durable transaction.
    fn render_playlists_section(&mut self, ui: &mut egui::Ui, state: &mut AppState) {
        ui.horizontal(|ui| {
            ui.label("Playlists")
                .on_hover_text("Your named playlists, saved across launches.");
            if ui
                .button("\u{2795}")
                .on_hover_text("New Playlist")
                .clicked()
            {
                self.playlist_create_name = Some(String::new());
                self.playlist_rename = None;
            }
        });

        self.render_playlist_create_prompt(ui, state);

        // --- Playlist rows (select / rename / delete) ---
        let summaries: Vec<(PlaylistId, String, usize)> = state
            .playlists
            .iter()
            .map(|p| (p.id.clone(), p.name.clone(), p.tracks.len()))
            .collect();
        for (id, name, count) in summaries {
            let selected = self.playlist_view.as_ref() == Some(&id);
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(selected, format!("{name} ({count})"))
                    .clicked()
                {
                    self.playlist_view = Some(id.clone());
                    self.smart_playlist_view = None;
                }
                if ui
                    .button("\u{270F}")
                    .on_hover_text("Rename playlist")
                    .clicked()
                {
                    self.playlist_rename = Some((id.clone(), name.clone()));
                    self.playlist_create_name = None;
                }
                if ui
                    .button("\u{1F5D1}")
                    .on_hover_text("Delete playlist")
                    .clicked()
                {
                    match self.playlist_store.delete_playlist(&id) {
                        Ok(_) => reload_playlists(self.playlist_store.as_ref(), state),
                        Err(e) => tracing::warn!("Failed to delete playlist: {e}"),
                    }
                    if self.playlist_view.as_ref() == Some(&id) {
                        self.playlist_view = None;
                    }
                }
            });

            self.render_playlist_rename_prompt(ui, state, &id);
        }
        ui.separator();
    }

    /// The inline "New Playlist" name prompt while it is open.
    fn render_playlist_create_prompt(&mut self, ui: &mut egui::Ui, state: &mut AppState) {
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
                        reload_playlists(self.playlist_store.as_ref(), state);
                        self.playlist_view = Some(id);
                    }
                    Err(e) => tracing::warn!("Failed to create playlist: {e}"),
                }
            }
        } else if cancel {
            self.playlist_create_name = None;
        }
    }

    /// The inline rename prompt for one playlist while it is open.
    fn render_playlist_rename_prompt(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        playlist_id: &PlaylistId,
    ) {
        let renaming = self
            .playlist_rename
            .as_ref()
            .is_some_and(|(rid, _)| rid == playlist_id);
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
                let draft = draft.trim().to_string();
                if !draft.is_empty() {
                    match self.playlist_store.rename_playlist(&rid, &draft) {
                        Ok(_) => reload_playlists(self.playlist_store.as_ref(), state),
                        Err(e) => tracing::warn!("Failed to rename playlist: {e}"),
                    }
                }
            }
        } else if cancel {
            self.playlist_rename = None;
        }
    }

    /// Render the tracks of a user playlist, in order. Entries whose files
    /// have been moved or deleted are flagged invalid (dimmed, strikethrough,
    /// "missing" hint) and excluded from playback; valid entries get the
    /// standard track context menu plus "Remove from Playlist".
    fn render_playlist_view(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        playlist_id: &PlaylistId,
    ) {
        let Some(playlist) = state
            .playlists
            .iter()
            .find(|p| &p.id == playlist_id)
            .cloned()
        else {
            ui.label("Playlist not found");
            return;
        };

        let current_track = state.queue.current_track().cloned();
        let valid_ids = crate::app::playlist_manager::valid_tracks(&state.library, &playlist);

        // Header: name + count, with whole-list actions (valid tracks only),
        // mirroring the smart-playlist header menu.
        let header = ui.horizontal(|ui| {
            ui.heading(&playlist.name);
            ui.weak(format!("({} tracks)", playlist.tracks.len()));
        });
        if !valid_ids.is_empty() {
            show_list_context_menu(&header.response, cmd, &valid_ids);
        }
        ui.separator();

        if playlist.tracks.is_empty() {
            ui.vertical_centered(|ui| {
                ui.label("No tracks in this playlist");
                ui.weak("Use a track's context menu \u{2192} Add to Playlist to add tracks.");
            });
            return;
        }

        // Resolve each entry once per frame: the track (if still in the
        // library) and whether its file still exists on disk.
        let entries: Vec<(TrackId, Option<Track>, bool)> = playlist
            .tracks
            .iter()
            .map(|tid| {
                let track = state.library.get_track(tid).cloned();
                let valid = crate::app::playlist_manager::track_is_valid(&state.library, tid);
                (tid.clone(), track, valid)
            })
            .collect();

        egui::ScrollArea::vertical().show_rows(ui, 22.0, entries.len(), |ui, row_range| {
            for i in row_range {
                if let Some(entry) = entries.get(i) {
                    self.render_playlist_entry(
                        ui,
                        state,
                        cmd,
                        playlist_id,
                        entry,
                        current_track.as_ref(),
                    );
                }
            }
        });
    }

    /// One row of [`Self::render_playlist_view`]: a normal track row for
    /// valid entries, a flagged "missing" row otherwise.
    fn render_playlist_entry(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        playlist_id: &PlaylistId,
        entry: &(TrackId, Option<Track>, bool),
        current_track: Option<&TrackId>,
    ) {
        let (tid, track, valid) = entry;
        if *valid {
            if let Some(t) = track {
                self.render_track_row(ui, state, cmd, t, current_track, Some(playlist_id));
                return;
            }
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

    fn show_now_playing_view(
        &mut self,
        parent_ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
    ) {
        egui::CentralPanel::default().show_inside(parent_ui, |ui| {
            ui.vertical_centered(|ui| {
                let Some(track_id) = state.queue.current_track().cloned() else {
                    ui.heading("Nothing Playing");
                    ui.label("Select a track to start playback");
                    return;
                };
                let Some(track) = state.library.get_track(&track_id) else {
                    return;
                };
                self.request_cover(&track.id, &track.file_path);

                let texture = self.get_cover_texture(&track.id.0);
                cover_art_ui(ui, texture, COVER_LARGE_SIZE);

                ui.add_space(10.0);
                ui.heading(track.metadata.display_title(&track.file_path));
                ui.label(format!(
                    "{} - {}",
                    track.metadata.display_artist(),
                    track.metadata.display_album()
                ));
                render_track_meta_labels(ui, &track.metadata, true);

                ui.separator();
                let path_display = track.file_path.to_string_lossy().to_string();
                ui.label(format!("File: {path_display}"));

                // Seekable progress (REQ-UI-005): bound to the live
                // position the engine updates each frame, so the handle
                // advances continuously during playback. Disabled when
                // the total duration is unknown (e.g. streaming input).
                ui.separator();
                render_seek_row(ui, state, cmd);

                ui.separator();
                ui.label("Up Next:");
                render_up_next(ui, state, cmd);
            });
        });
    }

    /// Cached [`LibraryQueryStore::folder_has_audio`] for one frame
    /// generation; failures render as "no audio" with a warning logged.
    fn folder_has_audio_cached(&mut self, generation: u64, path: &Path) -> bool {
        match self
            .folder_projection
            .has_audio(generation, path, &mut |f| {
                self.library_queries.folder_has_audio(f)
            }) {
            Ok(has) => has,
            Err(e) => {
                tracing::warn!("Failed to probe folder {}: {e}", path.display());
                false
            }
        }
    }

    /// Cached subtree search match for one frame generation.
    fn folder_search_match_cached(&mut self, generation: u64, path: &Path, query: &str) -> bool {
        match self
            .folder_projection
            .has_search_match(generation, path, query, &mut |f, q| {
                self.library_queries.folder_has_search_match(f, q)
            }) {
            Ok(has) => has,
            Err(e) => {
                tracing::warn!("Failed to search folder {}: {e}", path.display());
                false
            }
        }
    }

    /// Cached subtree track ids for one frame generation.
    fn folder_subtree_ids_cached(&mut self, generation: u64, path: &Path) -> Vec<TrackId> {
        match self
            .folder_projection
            .subtree_ids(generation, path, &mut |f| {
                self.library_queries.track_ids_in_folder_tree(f)
            }) {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("Failed to list folder tree {}: {e}", path.display());
                Vec::new()
            }
        }
    }

    /// Cached child directories for one frame generation.
    fn folder_children_cached(&mut self, generation: u64, path: &Path) -> Vec<PathBuf> {
        match self.folder_projection.children(generation, path, &mut |f| {
            self.library_queries.subdirs_with_audio(f)
        }) {
            Ok(children) => children,
            Err(e) => {
                tracing::warn!("Failed to list folder children {}: {e}", path.display());
                Vec::new()
            }
        }
    }

    /// Cached direct-track listing for one frame generation.
    fn folder_direct_tracks_cached(&mut self, generation: u64, path: &Path) -> Vec<Track> {
        match self
            .folder_projection
            .direct_tracks(generation, path, &mut |f| {
                self.library_queries.tracks_in_folder(f)
            }) {
            Ok(tracks) => tracks,
            Err(e) => {
                tracing::warn!("Failed to list folder tracks {}: {e}", path.display());
                Vec::new()
            }
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

        // Folder views read through the Session Projection over store
        // queries (ADR 0002/0003): escaped prefix matching over stored track
        // paths, cached until the next committed mutation bumps the
        // generation. No in-memory mirror involved.
        let generation = self.store_generation.current();
        let lib_paths = state.library_paths.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for lib_path in &lib_paths {
                if !self.folder_has_audio_cached(generation, lib_path) {
                    continue;
                }
                self.render_folder_node(ui, state, cmd, generation, lib_path, 0.0, query);
            }
        });
    }

    /// One folder node of the Folders tree. `generation` is passed down so
    /// every node of one frame reads the same snapshot even if a mutation
    /// commits mid-frame.
    #[allow(clippy::too_many_arguments)]
    fn render_folder_node(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut AppState,
        cmd: Option<&Sender<PlaybackCommand>>,
        generation: u64,
        path: &Path,
        indent: f32,
        query: &str,
    ) {
        if !self.folder_has_audio_cached(generation, path) {
            return;
        }

        if !query.is_empty() && !self.folder_search_match_cached(generation, path, query) {
            return;
        }

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
        let header_text = if indent == 0.0 {
            format!("\u{1F4C1} {label}")
        } else {
            label
        };

        let folder_track_ids: Vec<TrackId> = self.folder_subtree_ids_cached(generation, path);

        let header =
            egui::CollapsingHeader::new(header_text).default_open(contains_current || is_selected);

        let header_response = header.show(ui, |ui| {
            let children = self.folder_children_cached(generation, path);
            for child_path in &children {
                self.render_folder_node(
                    ui,
                    state,
                    cmd,
                    generation,
                    child_path,
                    indent + FOLDER_INDENT,
                    query,
                );
            }

            let tracks =
                folder_tracks_filtered(self.folder_direct_tracks_cached(generation, path), query);

            for track in &tracks {
                let is_track_selected = state.selected_track.as_ref() == Some(&track.id);
                let is_current = current_track.as_ref() == Some(&track.id);

                self.request_cover(&track.id, &track.file_path);

                ui.horizontal(|ui| {
                    ui.set_min_height(20.0);
                    if indent > 0.0 {
                        ui.add_space(indent);
                    }
                    if is_current {
                        ui.label("\u{25B6}");
                    }
                    let display = format!(
                        "{}. {}",
                        track.metadata.track_number.unwrap_or(0),
                        track.metadata.display_title(&track.file_path)
                    );
                    let resp = ui.selectable_label(is_track_selected, display);
                    if resp.clicked() {
                        state.selected_track = Some(track.id.clone());
                    }
                    if resp.double_clicked() {
                        state.selected_track = Some(track.id.clone());
                        if let Some(s) = cmd {
                            let _ = s.send(PlaybackCommand::Play(track.id.clone()));
                        }
                    }
                    self.attach_track_menu(&resp, state, cmd, &track.id, Some(track), None);
                });
            }
        });

        if header_response.header_response.clicked() {
            state.selected_folder = Some(path.to_path_buf());
        }

        if header_response.header_response.double_clicked() {
            play_folder(&folder_track_ids, cmd);
        }

        if !folder_track_ids.is_empty() {
            show_list_context_menu(&header_response.header_response, cmd, &folder_track_ids);
        }
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

/// Play a folder: start its first track and queue the rest. The ids arrive
/// in the store's path order — exactly what the former mirror listing
/// produced.
fn play_folder(track_ids: &[TrackId], cmd: Option<&Sender<PlaybackCommand>>) {
    let Some(s) = cmd else { return };
    if let Some(first) = track_ids.first() {
        let _ = s.send(PlaybackCommand::Play(first.clone()));
        for tid in &track_ids[1..] {
            let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
        }
    }
}

/// Tracks directly in a folder, optionally filtered by search query. The
/// listing arrives from the store in canonical order; only the query filter
/// applies here, like before.
fn folder_tracks_filtered(tracks: Vec<Track>, query: &str) -> Vec<Track> {
    if query.is_empty() {
        tracks
    } else {
        let q = query.to_lowercase();
        tracks
            .into_iter()
            .filter(|t| t.metadata.search_text().contains(&q))
            .collect()
    }
}

/// Shared metadata label rows (album artist, year, genre, track/disc number).
fn render_track_meta_labels(
    ui: &mut egui::Ui,
    metadata: &crate::domain::TrackMetadata,
    show_disc: bool,
) {
    if let Some(ref aa) = metadata.album_artist {
        if *aa != metadata.display_artist() {
            ui.label(format!("Album Artist: {aa}"));
        }
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

/// Seekable progress slider row for the Now Playing view (REQ-UI-005).
fn render_seek_row(ui: &mut egui::Ui, state: &AppState, cmd: Option<&Sender<PlaybackCommand>>) {
    let total = state.current_position.total;
    let total_secs = total.map_or(0.0, |t| t.as_secs_f32());
    let mut seek_secs = state
        .current_position
        .current
        .as_secs_f32()
        .clamp(0.0, total_secs.max(1.0));
    ui.horizontal(|ui| {
        ui.label(format_duration(state.current_position.current));
        let slider = ui.add_enabled(
            total.is_some(),
            egui::Slider::new(&mut seek_secs, 0.0..=total_secs.max(1.0)).show_value(false),
        );
        if slider.changed() {
            if let Some(s) = cmd {
                let _ = s.send(PlaybackCommand::Seek(clamp_seek(seek_secs, total)));
            }
        }
        ui.label(total.map_or_else(|| "--:--".to_string(), format_duration));
    });
}

/// "Up Next" queue preview: clicking a track queues it to play NEXT
/// (REQ-UI-005), it does not jump away from the current track.
fn render_up_next(ui: &mut egui::Ui, state: &AppState, cmd: Option<&Sender<PlaybackCommand>>) {
    let upcoming = state.queue.upcoming(5);
    if upcoming.is_empty() {
        ui.label("Queue is empty");
        return;
    }
    for upcoming_tid in upcoming {
        if let Some(t) = state.library.get_track(upcoming_tid) {
            let label = format!("\u{2022} {}", t.metadata.display_title(&t.file_path));
            let tid = upcoming_tid.clone();
            if ui
                .link(label)
                .on_hover_text("Queue this track to play next")
                .clicked()
            {
                if let Some(s) = cmd {
                    let _ = s.send(PlaybackCommand::PlayNext(tid));
                }
            }
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
    playlists: &'a mut Vec<Playlist>,
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
            add_to_playlist_menu(ui, &playlist_options, playlists, playlist_store, &tid);
        }
        if let Some(ref pid) = remove_pid {
            if ui.button("Remove from Playlist").clicked() {
                // One immediate durable transaction; the projection patch
                // mirrors the committed change until the next reload.
                match playlist_store.remove_playlist_entries(pid, &tid) {
                    Ok(true) => {
                        if let Some(playlist) =
                            playlists.iter_mut().find(|p| &p.id == pid)
                        {
                            playlist.tracks.retain(|t| t != &tid);
                        }
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!("Failed to remove playlist entry: {e}"),
                }
                ui.close();
            }
        }
        // Edit Tags is advanced-only (REQ-UI-006).
        if let Some(ref t) = edit_track {
            if ui
                .button("Edit Tags")
                .on_hover_text(
                    "Edit this track's tags (title, artist, album, and more). Changes are written to the file on Save.",
                )
                .clicked()
            {
                *tag_edit = Some(TagEditState::from_track(t));
                ui.close();
            }
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
            if let Some(ref s) = cmd {
                if let Some(first) = tids.first() {
                    let _ = s.send(PlaybackCommand::Play(first.clone()));
                    for tid in &tids[1..] {
                        let _ = s.send(PlaybackCommand::AddToQueue(tid.clone()));
                    }
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
/// pane (REQ-UI-004).
const COVER_THUMB_SIZE: f32 = 200.0;
/// Fixed square size (points) for the large Now Playing cover (REQ-UI-004).
const COVER_LARGE_SIZE: f32 = 300.0;
/// Horizontal indent per level in the Folders tree.
const FOLDER_INDENT: f32 = 16.0;

/// Render cover art inside a fixed `size` x `size` square, or a neutral
/// placeholder (rounded box + note glyph) while no texture is loaded yet.
/// The `SizedTexture` destination rect forces the image into the allotted
/// square, so oversized covers are clamped and can never overflow the layout.
fn cover_art_ui(ui: &mut egui::Ui, texture: Option<egui::TextureHandle>, size: f32) {
    if let Some(texture) = texture {
        let sized = egui::load::SizedTexture::new(texture.id(), egui::vec2(size, size));
        ui.add(egui::Image::from_texture(sized).corner_radius(4.0));
    } else {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_gray(40));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "\u{1F3B5}",
            egui::FontId::proportional(size * 0.3),
            egui::Color32::from_gray(120),
        );
    }
}

/// "Add to Playlist" submenu shared by the track context menus (Task 4.2).
/// Clicking a playlist appends the track (exact duplicates ignored) as one
/// immediate durable transaction, so the change survives a restart. Takes the
/// playlists slice (not the whole `AppState`) so callers can capture a
/// disjoint field and avoid whole-state borrow conflicts.
fn add_to_playlist_menu(
    ui: &mut egui::Ui,
    playlist_options: &[(PlaylistId, String)],
    playlists: &mut [Playlist],
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
                match store.add_playlist_entry(pid, track_id) {
                    Ok(true) => {
                        // Projection patch mirroring the committed append.
                        if let Some(playlist) = playlists.iter_mut().find(|p| &p.id == pid) {
                            playlist.tracks.push(track_id.clone());
                        }
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!("Failed to add playlist entry: {e}"),
                }
                ui.close();
            }
        }
    });
}

pub fn format_duration(duration: std::time::Duration) -> String {
    let mins = duration.as_secs() / 60;
    let secs = duration.as_secs() % 60;
    format!("{mins:02}:{secs:02}")
}

/// Clamp a seek request (in seconds) into `[0, total]` so a drag past the end
/// of a track seeks to the end rather than beyond it (REQ-UI-005). When the
/// total duration is unknown there is nothing to clamp against, so the seek
/// falls back to the start; non-finite inputs (NaN/infinity) do the same.
pub fn clamp_seek(secs: f32, total: Option<std::time::Duration>) -> std::time::Duration {
    let finite = if secs.is_finite() { secs } else { 0.0 };
    let upper = total.map_or(0.0, |t| t.as_secs_f32());
    std::time::Duration::from_secs_f32(finite.clamp(0.0, upper.max(0.0)))
}

/// Build the high-contrast [`egui::Visuals`] used when accessibility
/// high-contrast mode (REQ-UI-007) is enabled.
///
/// It starts from egui's dark visuals and pushes contrast to WCAG-friendly
/// levels: a near-black background, pure-white text, strong light widget
/// borders, and a bright yellow focus/selection stroke so the keyboard-focused
/// element is unmistakable. egui renders `widgets.active` for the focused
/// widget and paints `selection.stroke` on focused/selected elements, so both
/// are given thick, high-contrast strokes here.
pub fn high_contrast_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    let focus = egui::Color32::from_rgb(255, 215, 0); // bright yellow
    let border = egui::Color32::from_gray(200); // strong light border
    let text = egui::Color32::WHITE;
    let panel = egui::Color32::from_gray(10); // near-black panel fill

    v.dark_mode = true;
    v.override_text_color = Some(text);
    v.panel_fill = panel;
    v.window_fill = panel;
    v.window_stroke = egui::Stroke::new(2.0_f32, border);
    v.extreme_bg_color = egui::Color32::BLACK; // text-edit / scroll backgrounds
    v.faint_bg_color = egui::Color32::from_gray(24);
    v.code_bg_color = panel;
    v.hyperlink_color = focus;
    v.warn_fg_color = focus;
    v.error_fg_color = egui::Color32::from_rgb(255, 90, 90);

    // Focus / selection: bright, thick outline so focused widgets stand out.
    v.selection.bg_fill = egui::Color32::from_rgb(90, 70, 0);
    v.selection.stroke = egui::Stroke::new(2.0_f32, focus);

    // Widget states: strong borders and white text throughout; hover and
    // active (which egui also uses for the keyboard-focused widget) get the
    // bright yellow outline.
    v.widgets.noninteractive.weak_bg_fill = panel;
    v.widgets.noninteractive.bg_fill = panel;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.5_f32, border);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, text);

    v.widgets.inactive.weak_bg_fill = egui::Color32::from_gray(30);
    v.widgets.inactive.bg_fill = egui::Color32::from_gray(30);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.5_f32, border);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, text);

    v.widgets.hovered.weak_bg_fill = egui::Color32::from_gray(50);
    v.widgets.hovered.bg_fill = egui::Color32::from_gray(50);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(2.0_f32, focus);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.5_f32, text);

    v.widgets.active.weak_bg_fill = egui::Color32::from_gray(60);
    v.widgets.active.bg_fill = egui::Color32::from_gray(60);
    v.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, focus);
    v.widgets.active.fg_stroke = egui::Stroke::new(2.0_f32, text);

    v.widgets.open.weak_bg_fill = egui::Color32::from_gray(40);
    v.widgets.open.bg_fill = panel;
    v.widgets.open.bg_stroke = egui::Stroke::new(1.5_f32, border);
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, text);

    v
}
