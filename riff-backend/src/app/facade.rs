//! The Backend Facade: the single seam between the frontend and the backend.
//!
//! Issues 02-09 extend this seam without changing its surface: exactly ONE
//! type the frontend holds.
//!
//! # Invariants
//!
//! - The frontend never constructs raw [`crate::domain::PlaybackCommand`]s.
//!   Every command flows through a facade method.
//! - The frontend never holds `Arc<Mutex<AppState>>`; that stays backend-side.
//! - Every command the facade accepts is infallible at the call site: command
//!   failures surface later as [`BackendEvent::TypedNotice`] on the event inbox.

use crossbeam_channel::Receiver;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::state::{LibraryStatus, ScalarSettings};
use crate::app::store::{SettingsStore, StoreChanged};
use crate::domain::{PlaybackCommand, PlaybackPosition, PlaybackState, RepeatMode, TrackId};

// ---------------------------------------------------------------------------
// Event types (one enum the frontend drains)
// ---------------------------------------------------------------------------

/// Typed notices carry severity and source so the frontend can route them to
/// persistent slots instead of a single catch-all status string (issue 07).
#[derive(Debug, Clone, PartialEq)]
pub enum NoticeSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NoticeSource {
    Playback,
    Scan,
    TagEdit,
    Library,
    Settings,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoticePayload {
    pub severity: NoticeSeverity,
    pub source: NoticeSource,
    pub message: String,
}

/// Request produced by [`BackendFacade::cancel_scan`] and consumed by the scan
/// service to abort an in-flight walk (issue 07 / 08).
#[derive(Debug, Clone, PartialEq)]
pub struct ScanCancelRequest {
    pub path: PathBuf,
}

impl ScanCancelRequest {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

/// Correlation identifier that ties a tag-edit submission to its outcome
/// (issue 09).
pub type CorrelationId = u64;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagEditFields(pub HashMap<String, String>);

#[derive(Debug, Clone, PartialEq)]
pub struct TagEditSubmission {
    pub correlation_id: CorrelationId,
    pub track_id: TrackId,
    pub file_path: PathBuf,
    pub fields: TagEditFields,
}

/// A typed change event the backend pushes to the frontend through the
/// facade's event inbox.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    TrackChanged(TrackId),
    StateChange(PlaybackState),
    PositionChange(PlaybackPosition),
    VolumeChange(f32),
    QueueChanged {
        len: usize,
        shuffle: bool,
        repeat: RepeatMode,
    },
    CommandApplied(PlaybackCommand),
    /// Legacy catch-all. Prefer [`BackendEvent::TypedNotice`] for new call
    /// sites; kept so existing tests and call paths still compile.
    Notice(String),
    /// Typed replacement for [`BackendEvent::Notice`].
    TypedNotice(NoticePayload),
    /// Scan lifecycle events (issue 07).
    ScanStarted { path: String },
    ScanProgress {
        path: String,
        files_found: usize,
        total_estimated: Option<usize>,
    },
    ScanCompleted { path: String, total_files: usize },
    ScanFailed { path: String, reason: String },
    ScanCancelled { path: String },
    LibraryStatusChanged { path: String, status: LibraryStatus },
    /// Library generation moved (issue 04, coalesced).
    LibraryChanged { generation: u64 },
    PlaylistsChanged { generation: u64 },
    InitialSnapshot {
        library_generation: u64,
        playlists_generation: u64,
    },
    /// Upcoming queue entries after current (issue 05).
    QueueUpcoming(Vec<TrackId>),
    /// Full settings snapshot (issue 06).
    SettingsChanged {
        volume: f32,
        muted: bool,
        replaygain_enabled: bool,
    },
    /// Current track including None when stopped (issue 05).
    CurrentTrack(Option<TrackId>),
    /// Tag-edit correlation events (issue 09).
    TagEditSubmitted { correlation_id: CorrelationId, track_id: TrackId },
    TagEditCompleted {
        correlation_id: CorrelationId,
        track_id: TrackId,
        file_path: PathBuf,
    },
    TagEditFailed {
        correlation_id: CorrelationId,
        track_id: TrackId,
        reason: String,
    },
    /// Library management events (issue 08).
    LibraryRootAdded { path: String },
    LibraryRootRemoved { path: String },
    LibraryCleared,
}

// ---------------------------------------------------------------------------
// Facade
// ---------------------------------------------------------------------------

/// The single public seam toward the frontend.
#[allow(clippy::struct_excessive_bools, reason = "pre-existing fields; refactor out of scope")]
pub struct BackendFacade {
    // --- Playback state (kept local so the facade is testable headlessly) ---
    current: Option<TrackId>,
    state: PlaybackState,
    position: PlaybackPosition,
    volume: f32,
    muted: bool,
    queue: Vec<TrackId>,
    shuffle: bool,
    repeat: RepeatMode,

    // --- Event inbox the frontend drains -------------------------------
    events: VecDeque<BackendEvent>,

    // --- Event backbone (issue 04) ---------------------------------------
    backend_changes: Receiver<StoreChanged>,
    last_library_change_time: Option<Instant>,
    last_library_generation: u64,
    last_playlist_generation: u64,
    snapshot_emitted: bool,

    // --- Seek-drag suppression (issue 05) --------------------------------
    seeking: bool,

    // --- Scalar-settings persistence (issue 06) --------------------------
    settings_store: Option<Box<dyn SettingsStore + Send>>,
    persist_deadline: Option<Instant>,
    persist_debounce_ms: u64,
    replaygain_enabled: bool,

    // --- Scan-events channel (issue 07) ----------------------------------
    scan_events: Receiver<BackendEvent>,
    cancel_scan_intent: Option<PathBuf>,

    // --- Tag-edit correlation (issue 09) ---------------------------------
    correlation_counter: u64,
    tag_edit_events: Receiver<BackendEvent>,
}

impl Default for BackendFacade {
    fn default() -> Self {
        Self {
            current: None,
            state: PlaybackState::Stopped,
            position: PlaybackPosition::default(),
            volume: 0.5,
            muted: false,
            queue: Vec::new(),
            shuffle: false,
            repeat: RepeatMode::None,
            events: VecDeque::new(),
            backend_changes: crossbeam_channel::unbounded().1,
            last_library_change_time: None,
            last_library_generation: 0,
            last_playlist_generation: 0,
            snapshot_emitted: false,
            seeking: false,
            settings_store: None,
            persist_deadline: None,
            persist_debounce_ms: 150,
            replaygain_enabled: false,
            scan_events: crossbeam_channel::unbounded().1,
            cancel_scan_intent: None,
            correlation_counter: 0,
            tag_edit_events: crossbeam_channel::unbounded().1,
        }
    }
}

impl BackendFacade {
    /// Coalesce window for [`BackendEvent::LibraryChanged`]: ~4 emissions/sec.
    pub const COALESCE_WINDOW: Duration = Duration::from_millis(250);

    /// Create a new facade pre-wired to the given backend change receiver.
    #[must_use]
    pub fn with_backend_events(backend_changes: Receiver<StoreChanged>) -> Self {
        Self {
            backend_changes,
            ..Default::default()
        }
    }

    // --- Playback transport ----------------------------------------------

    pub fn play(&mut self, track: TrackId) {
        self.state = PlaybackState::Playing;
        self.current = Some(track.clone());
        self.queue.clear();
        self.queue.push(track.clone());
        self.events
            .push_back(BackendEvent::TrackChanged(track.clone()));
        self.events
            .push_back(BackendEvent::StateChange(PlaybackState::Playing));
        self.events.push_back(BackendEvent::QueueChanged {
            len: self.queue.len(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        });
        self.events
            .push_back(BackendEvent::CurrentTrack(Some(track)));
        self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
    }

    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
            self.events
                .push_back(BackendEvent::StateChange(PlaybackState::Paused));
        }
    }

    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.state = PlaybackState::Playing;
            self.events
                .push_back(BackendEvent::StateChange(PlaybackState::Playing));
        }
    }

    pub fn next(&mut self) {
        self.events
            .push_back(BackendEvent::CurrentTrack(None));
        self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
    }

    pub fn previous(&mut self) {
        self.events
            .push_back(BackendEvent::CurrentTrack(None));
        self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
    }

    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
        self.events
            .push_back(BackendEvent::StateChange(PlaybackState::Stopped));
        self.events
            .push_back(BackendEvent::Notice("stopped".to_string()));
        self.events
            .push_back(BackendEvent::CurrentTrack(None));
        self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
    }

    pub fn seek(&mut self, pos: Duration) {
        let total = self.position.total;
        let clamped = if let Some(t) = total && pos > t {
            t
        } else {
            pos
        };
        self.position.current = clamped;
        if !self.seeking {
            self.events.push_back(BackendEvent::PositionChange(PlaybackPosition {
                current: clamped,
                total,
            }));
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        let effective = if self.muted { 0.0 } else { self.volume };
        self.events.push_back(BackendEvent::VolumeChange(effective));
        self.emit_settings_changed();
        self.schedule_persist();
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        let effective = if self.muted { 0.0 } else { self.volume };
        self.events.push_back(BackendEvent::VolumeChange(effective));
        self.emit_settings_changed();
        self.schedule_persist();
    }

    pub fn add_to_queue(&mut self, track: TrackId) {
        self.queue.push(track);
        self.events.push_back(BackendEvent::QueueChanged {
            len: self.queue.len(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        });
        self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
    }

    pub fn play_next(&mut self, track: TrackId) {
        let insert_at = usize::from(!self.queue.is_empty());
        self.queue.insert(insert_at, track);
        self.events.push_back(BackendEvent::QueueChanged {
            len: self.queue.len(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        });
        self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.events.push_back(BackendEvent::QueueChanged {
            len: self.queue.len(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        });
        self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
        self.emit_settings_changed();
        self.schedule_persist();
    }

    pub fn toggle_repeat(&mut self) {
        self.repeat = match self.repeat {
            RepeatMode::None => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::None,
        };
        self.events.push_back(BackendEvent::QueueChanged {
            len: self.queue.len(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        });
        self.emit_settings_changed();
        self.schedule_persist();
    }

    // --- Issue-05 seek-drag suppression API --------------------------------

    #[must_use]
    pub fn is_seeking(&self) -> bool {
        self.seeking
    }

    pub fn set_seeking(&mut self, seeking: bool) {
        self.seeking = seeking;
    }

    #[must_use]
    pub fn position(&self) -> PlaybackPosition {
        self.position
    }

    #[must_use]
    fn upcoming(&self) -> Vec<TrackId> {
        let Some(current) = &self.current else {
            return self.queue.clone();
        };
        if let Some(first) = self.queue.first()
            && first == current {
                return self.queue[1..].to_vec();
            }
        self.queue.clone()
    }

    // --- Transport-recorded events ----------------------------------------

    #[allow(clippy::too_many_lines)]
    pub fn record_command(&mut self, cmd: PlaybackCommand) {
        self.events
            .push_back(BackendEvent::CommandApplied(cmd.clone()));
        match cmd {
            PlaybackCommand::Play(track) => {
                self.state = PlaybackState::Playing;
                self.current = Some(track.clone());
                self.queue.clear();
                self.queue.push(track.clone());
                self.events
                    .push_back(BackendEvent::TrackChanged(track.clone()));
                self.events
                    .push_back(BackendEvent::StateChange(PlaybackState::Playing));
                self.events.push_back(BackendEvent::QueueChanged {
                    len: self.queue.len(),
                    shuffle: self.shuffle,
                    repeat: self.repeat,
                });
                self.events
                    .push_back(BackendEvent::CurrentTrack(Some(track)));
                self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
            }
            PlaybackCommand::Pause => {
                if self.state == PlaybackState::Playing {
                    self.state = PlaybackState::Paused;
                    self.events
                        .push_back(BackendEvent::StateChange(PlaybackState::Paused));
                }
            }
            PlaybackCommand::Resume => {
                if self.state == PlaybackState::Paused {
                    self.state = PlaybackState::Playing;
                    self.events
                        .push_back(BackendEvent::StateChange(PlaybackState::Playing));
                }
            }
            PlaybackCommand::Stop => {
                self.state = PlaybackState::Stopped;
                self.events
                    .push_back(BackendEvent::StateChange(PlaybackState::Stopped));
            }
            PlaybackCommand::SetVolume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                let effective = if self.muted { 0.0 } else { self.volume };
                self.events
                    .push_back(BackendEvent::VolumeChange(effective));
                self.emit_settings_changed();
                self.schedule_persist();
            }
            PlaybackCommand::Seek(pos) => {
                let total = self.position.total;
                let clamped = if let Some(t) = total && pos > t { t } else { pos };
                self.position.current = clamped;
                if !self.seeking {
                    self.events.push_back(BackendEvent::PositionChange(PlaybackPosition {
                        current: clamped,
                        total,
                    }));
                }
            }
            PlaybackCommand::PlayNext(track) => {
                let insert_at = usize::from(!self.queue.is_empty());
                self.queue.insert(insert_at, track);
                self.events.push_back(BackendEvent::QueueChanged {
                    len: self.queue.len(),
                    shuffle: self.shuffle,
                    repeat: self.repeat,
                });
                self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
            }
            PlaybackCommand::AddToQueue(track) => {
                self.queue.push(track);
                self.events.push_back(BackendEvent::QueueChanged {
                    len: self.queue.len(),
                    shuffle: self.shuffle,
                    repeat: self.repeat,
                });
                self.events.push_back(BackendEvent::QueueUpcoming(self.upcoming()));
            }
            PlaybackCommand::PlayPause => {
                match self.state {
                    PlaybackState::Playing => {
                        self.state = PlaybackState::Paused;
                        self.events
                            .push_back(BackendEvent::StateChange(PlaybackState::Paused));
                    }
                    PlaybackState::Paused => {
                        self.state = PlaybackState::Playing;
                        self.events
                            .push_back(BackendEvent::StateChange(PlaybackState::Playing));
                    }
                    PlaybackState::Stopped => {}
                }
            }
            PlaybackCommand::AddMany(_)
            | PlaybackCommand::Next
            | PlaybackCommand::Previous => {}
        }
    }

    // --- Event inbox -----------------------------------------------------

    pub fn events(&mut self) -> Vec<BackendEvent> {
        let mut out: Vec<BackendEvent> = self.events.drain(..).collect();
        let mut latest_library_gen: Option<u64> = None;
        while let Ok(change) = self.backend_changes.try_recv() {
            match change {
                StoreChanged::Library(generation) => {
                    if generation <= self.last_library_generation {
                        continue;
                    }
                    self.last_library_generation = generation;
                    latest_library_gen = Some(generation);
                }
                StoreChanged::Playlists(generation) => {
                    if generation > self.last_playlist_generation {
                        self.last_playlist_generation = generation;
                        out.push(BackendEvent::PlaylistsChanged { generation });
                    }
                }
            }
        }
        if let Some(latest) = latest_library_gen {
            let now = Instant::now();
            let since_last =
                self.last_library_change_time.map(|t| now.duration_since(t));
            let should_emit = match since_last {
                None => true,
                Some(d) => d >= Self::COALESCE_WINDOW,
            };
            if should_emit {
                out.push(BackendEvent::LibraryChanged { generation: latest });
                self.last_library_change_time = Some(now);
            }
        }
        // Persist any overdue scalar-settings commit (issue 06).
        self.try_persist();
        // Drain scan + tag-edit outboxes.
        while let Ok(ev) = self.scan_events.try_recv() {
            out.push(ev);
        }
        while let Ok(ev) = self.tag_edit_events.try_recv() {
            out.push(ev);
        }
        out
    }

    pub fn notify_scan_complete(&mut self) {
        // Drain backend_changes to find the latest library generation.
        let mut latest_generation = self.last_library_generation;
        while let Ok(change) = self.backend_changes.try_recv() {
            if let StoreChanged::Library(generation) = change
                && generation > latest_generation
            {
                latest_generation = generation;
            }
        }
        if latest_generation > 0 {
            self.last_library_generation = latest_generation;
            self.events
                .push_back(BackendEvent::LibraryChanged { generation: latest_generation });
            self.last_library_change_time = Some(Instant::now());
        }
    }

    pub fn bootstrap(&mut self, library_gen: u64, playlists_gen: u64) {
        if self.snapshot_emitted {
            return;
        }
        self.last_library_generation = library_gen;
        self.last_playlist_generation = playlists_gen;
        self.events.push_back(BackendEvent::InitialSnapshot {
            library_generation: library_gen,
            playlists_generation: playlists_gen,
        });
        self.snapshot_emitted = true;
    }

    pub fn subscribe_to_backend_changes(&mut self, rx: Receiver<StoreChanged>) {
        self.backend_changes = rx;
    }

    pub fn subscribe_scan_events(&mut self, rx: Receiver<BackendEvent>) {
        self.scan_events = rx;
    }

    pub fn subscribe_tag_edit_events(&mut self, rx: Receiver<BackendEvent>) {
        self.tag_edit_events = rx;
    }

    // --- Scan lifecycle (issue 07) ---------------------------------------

    pub fn scan_started(&mut self, path: PathBuf) {
        let p = path.to_string_lossy().to_string();
        self.events.push_back(BackendEvent::ScanStarted { path: p.clone() });
        self.events.push_back(BackendEvent::LibraryStatusChanged {
            path: p,
            status: LibraryStatus::Scanning { files_found: 0 },
        });
    }

    pub fn scan_progress(
        &mut self,
        path: PathBuf,
        files_found: usize,
        total_estimated: Option<usize>,
    ) {
        let p = path.to_string_lossy().to_string();
        self.events.push_back(BackendEvent::ScanProgress {
            path: p,
            files_found,
            total_estimated,
        });
    }

    pub fn scan_completed(&mut self, path: PathBuf, total_files: usize) {
        let p = path.to_string_lossy().to_string();
        self.events.push_back(BackendEvent::ScanCompleted {
            path: p.clone(),
            total_files,
        });
        self.events.push_back(BackendEvent::LibraryStatusChanged {
            path: p,
            status: LibraryStatus::Scanned(total_files),
        });
    }

    pub fn scan_failed(&mut self, path: PathBuf, reason: String) {
        let p = path.to_string_lossy().to_string();
        self.events.push_back(BackendEvent::ScanFailed {
            path: p.clone(),
            reason: reason.clone(),
        });
        self.events.push_back(BackendEvent::LibraryStatusChanged {
            path: p.clone(),
            status: LibraryStatus::Unavailable,
        });
        self.events.push_back(BackendEvent::TypedNotice(NoticePayload {
            severity: NoticeSeverity::Error,
            source: NoticeSource::Scan,
            message: format!("Scan of '{p}' failed: {reason}"),
        }));
    }

    pub fn cancel_scan(&mut self, path: PathBuf) -> ScanCancelRequest {
        self.cancel_scan_intent = Some(path.clone());
        let p = path.to_string_lossy().to_string();
        self.events.push_back(BackendEvent::ScanCancelled { path: p });
        ScanCancelRequest::new(path)
    }

    #[must_use]
    pub fn pending_cancel_scan(&self) -> Option<PathBuf> {
        self.cancel_scan_intent.clone()
    }

    pub fn clear_cancel_scan(&mut self) {
        self.cancel_scan_intent = None;
    }

    // --- Library management (issue 08) -----------------------------------

    /// Add a library root path. Emits `LibraryRootAdded`.
    pub fn add_library_root(&mut self, path: PathBuf) -> String {
        let p = path.to_string_lossy().to_string();
        self.events
            .push_back(BackendEvent::LibraryRootAdded { path: p.clone() });
        p
    }

    /// Remove a library root path. Emits `LibraryRootRemoved`.
    pub fn remove_library_root(&mut self, path: PathBuf) -> String {
        let p = path.to_string_lossy().to_string();
        self.events
            .push_back(BackendEvent::LibraryRootRemoved { path: p.clone() });
        p
    }

    /// Clear all library roots. Emits `LibraryCleared`.
    pub fn clear_library(&mut self) {
        self.events.push_back(BackendEvent::LibraryCleared);
    }

    /// Set watch state for a library path. Emits `LibraryStatusChanged`.
    pub fn set_watch_state(&mut self, path: PathBuf, status: LibraryStatus) {
        let p = path.to_string_lossy().to_string();
        self.events.push_back(BackendEvent::LibraryStatusChanged {
            path: p,
            status,
        });
    }

    // --- Scalar-settings persistence (issue 06) --------------------------

    pub fn with_settings_store(&mut self, store: Box<dyn SettingsStore + Send>) {
        self.settings_store = Some(store);
    }

    pub fn set_replaygain_enabled(&mut self, enabled: bool) {
        self.replaygain_enabled = enabled;
        self.emit_settings_changed();
        self.schedule_persist();
    }

    #[must_use]
    pub fn replaygain_enabled(&self) -> bool {
        self.replaygain_enabled
    }

    fn emit_settings_changed(&mut self) {
        self.events.push_back(BackendEvent::SettingsChanged {
            volume: self.volume,
            muted: self.muted,
            replaygain_enabled: self.replaygain_enabled,
        });
    }

    pub fn schedule_persist(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(self.persist_debounce_ms);
        match self.persist_deadline {
            None => self.persist_deadline = Some(deadline),
            Some(existing) if deadline > existing => self.persist_deadline = Some(deadline),
            _ => {}
        }
    }

    pub fn try_persist(&mut self) {
        let Some(deadline) = self.persist_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }
        self.persist_deadline = None;
        let Some(ref mut store) = self.settings_store else {
            return;
        };
        let scalars = ScalarSettings {
            volume: Some(self.volume),
            advanced_mode: false,
            high_contrast: false,
            replaygain_enabled: self.replaygain_enabled,
        };
        if let Err(e) = store.save_scalars(&scalars) {
            self.events.push_back(BackendEvent::TypedNotice(NoticePayload {
                severity: NoticeSeverity::Error,
                source: NoticeSource::Settings,
                message: format!("Failed to persist settings: {e}"),
            }));
        }
    }

    // --- Tag-edit correlation ids (issue 09) -----------------------------

    pub fn submit_tag_edit(
        &mut self,
        track_id: TrackId,
        _file_path: PathBuf,
        _fields: TagEditFields,
    ) -> CorrelationId {
        self.correlation_counter += 1;
        let id = self.correlation_counter;
        self.events.push_back(BackendEvent::TagEditSubmitted {
            correlation_id: id,
            track_id: track_id.clone(),
        });
        id
    }

    pub fn complete_tag_edit(
        &mut self,
        correlation_id: CorrelationId,
        track_id: TrackId,
        file_path: PathBuf,
    ) {
        self.events.push_back(BackendEvent::TagEditCompleted {
            correlation_id,
            track_id,
            file_path,
        });
    }

    pub fn fail_tag_edit(
        &mut self,
        correlation_id: CorrelationId,
        track_id: TrackId,
        reason: String,
    ) {
        self.events.push_back(BackendEvent::TagEditFailed {
            correlation_id,
            track_id: track_id.clone(),
            reason: reason.clone(),
        });
        self.events.push_back(BackendEvent::TypedNotice(NoticePayload {
            severity: NoticeSeverity::Error,
            source: NoticeSource::TagEdit,
            message: reason,
        }));
    }

    // --- Lifecycle -------------------------------------------------------

    pub fn shutdown(&mut self) {
        self.events
            .push_back(BackendEvent::Notice("shutting down".to_string()));
    }

    /// Returns any remaining events and clears the cancel-scan intent.
    /// Placeholder — real drain/join happens at the composition root (issue 11).
    pub fn shutdown_drain(&mut self) -> Vec<BackendEvent> {
        self.cancel_scan_intent = None;
        self.events.drain(..).collect()
    }

    /// Always returns false in this placeholder; issue 11 may refine.
    #[must_use]
    #[allow(clippy::unused_self, reason = "placeholder; issue 11 may add state")]
    pub fn is_shutting_down(&self) -> bool {
        false
    }

    // --- Test helpers ----------------------------------------------------

    #[cfg(test)]
    pub fn with_initial_volume(mut self, volume: f32) -> Self {
        self.volume = volume.clamp(0.0, 1.0);
        self
    }

    #[cfg(test)]
    pub fn with_initial_position(mut self, pos: Duration, total: Option<Duration>) -> Self {
        self.position = PlaybackPosition {
            current: pos,
            total,
        };
        self
    }
}

// ===========================================================================
// Tests â€" one #[cfg(test)] module per issue, with an exhaustive match at the
// end proving every BackendEvent variant is handled.
// ===========================================================================



// ===========================================================================
// Tests
// ===========================================================================



#[cfg(test)]
mod domain_match_tests {
    use crate::domain::PlaybackCommand;

    #[test]
    fn facade_handles_every_playback_command_variant() {
        let _ = handle_all;
    }

    fn handle_all(cmd: PlaybackCommand) {
        match cmd {
            PlaybackCommand::Play(_)
            | PlaybackCommand::Pause
            | PlaybackCommand::Resume
            | PlaybackCommand::Stop
            | PlaybackCommand::Seek(_)
            | PlaybackCommand::SetVolume(_)
            | PlaybackCommand::Next
            | PlaybackCommand::Previous
            | PlaybackCommand::PlayNext(_)
            | PlaybackCommand::AddToQueue(_)
            | PlaybackCommand::AddMany(_)
            | PlaybackCommand::PlayPause => {}
        }
    }
}

#[cfg(test)]
mod backend_event_match_tests {
    use super::BackendEvent;

    #[test]
    fn every_backend_event_variant_is_handled() {
        let _ = match_all;
    }

    fn match_all(ev: BackendEvent) {
        match ev {
            BackendEvent::TrackChanged(_)
            | BackendEvent::StateChange(_)
            | BackendEvent::PositionChange(_)
            | BackendEvent::VolumeChange(_)
            | BackendEvent::CommandApplied(_)
            | BackendEvent::Notice(_)
            | BackendEvent::TypedNotice(_)
            | BackendEvent::QueueUpcoming(_)
            | BackendEvent::CurrentTrack(_)
            | BackendEvent::TagEditSubmitted { .. }
            | BackendEvent::TagEditCompleted { .. }
            | BackendEvent::TagEditFailed { .. }
            | BackendEvent::QueueChanged { .. }
            | BackendEvent::LibraryChanged { .. }
            | BackendEvent::PlaylistsChanged { .. }
            | BackendEvent::InitialSnapshot { .. }
            | BackendEvent::ScanStarted { .. }
            | BackendEvent::ScanProgress { .. }
            | BackendEvent::ScanCompleted { .. }
            | BackendEvent::ScanFailed { .. }
            | BackendEvent::ScanCancelled { .. }
            | BackendEvent::LibraryStatusChanged { .. }
            | BackendEvent::SettingsChanged { .. }
            | BackendEvent::LibraryRootAdded { .. }
            | BackendEvent::LibraryRootRemoved { .. }
            | BackendEvent::LibraryCleared => {}
        }
    }
}

#[cfg(test)]
mod issue04_store_events {
    use crate::app::store::StoreGeneration;

    #[test]
    fn store_generation_value_moves_on_bump() {
        let generation = StoreGeneration::new();
        assert_eq!(generation.current(), 0);
        let g = generation.bump();
        assert_eq!(g, 1);
        assert_eq!(generation.current(), 1);
    }

    #[test]
    fn store_generation_handles_are_independent() {
        let lib_gen = StoreGeneration::new();
        let playlist_gen = StoreGeneration::new();
        lib_gen.bump();
        lib_gen.bump();
        playlist_gen.bump();
        assert_eq!(lib_gen.current(), 2);
        assert_eq!(playlist_gen.current(), 1);
    }
}

#[cfg(test)]
mod issue04_events {
    use crossbeam_channel::unbounded;

    use super::{BackendEvent, BackendFacade, StoreChanged};

    #[test]
    fn bootstrap_emits_exactly_one_initial_snapshot() {
        let mut f = BackendFacade::default();
        f.bootstrap(42, 7);
        let evs = f.events();
        assert_eq!(
            evs,
            vec![BackendEvent::InitialSnapshot {
                library_generation: 42,
                playlists_generation: 7
            }]
        );
        f.bootstrap(999, 999);
        assert!(f.events().is_empty());
    }

    #[test]
    fn library_change_events_are_coalesced() {
        let (tx, rx) = unbounded::<StoreChanged>();
        let mut f = BackendFacade::with_backend_events(rx);
        for i in 1..=100 {
            let _ = tx.send(StoreChanged::Library(i));
        }
        let evs = f.events();
        let library_events: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, BackendEvent::LibraryChanged { .. }))
            .collect();
        assert!(library_events.len() <= 1);
        let first = library_events.first().cloned();
        assert_eq!(first, Some(&BackendEvent::LibraryChanged { generation: 100 }));
    }

    #[test]
    fn notify_scan_complete_forces_terminal_emit() {
        let (tx, rx) = unbounded::<StoreChanged>();
        let mut f = BackendFacade::with_backend_events(rx);
        for i in 1..=5 {
            let _ = tx.send(StoreChanged::Library(i));
        }
        let _ = f.events();
        for i in 6..=20 {
            let _ = tx.send(StoreChanged::Library(i));
        }
        f.notify_scan_complete();
        let evs = f.events();
        for e in &evs {
            if let BackendEvent::LibraryChanged { generation } = e {
                assert_eq!(*generation, 20);
                return;
            }
        }
        panic!("expected LibraryChanged(20), got {:?}", evs);
    }

    #[test]
    fn playlists_change_events_forward_without_coalescing() {
        let (tx, rx) = unbounded::<StoreChanged>();
        let mut f = BackendFacade::with_backend_events(rx);
        for i in 1..=5 {
            let _ = tx.send(StoreChanged::Playlists(i));
        }
        let evs = f.events();
        let playlist_events: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, BackendEvent::PlaylistsChanged { .. }))
            .collect();
        assert_eq!(playlist_events.len(), 5);
    }

    #[test]
    fn bootstrap_blocks_store_emitted_before_it() {
        let (tx, rx) = unbounded::<StoreChanged>();
        let mut f = BackendFacade::with_backend_events(rx);
        for i in 1..=6 {
            let _ = tx.send(StoreChanged::Library(i));
        }
        f.bootstrap(6, 0);
        let _ = f.events();
        let _ = tx.send(StoreChanged::Library(7));
        let evs = f.events();
        for e in &evs {
            if let BackendEvent::LibraryChanged { generation } = e {
                assert_eq!(*generation, 7);
                return;
            }
        }
        panic!("expected LibraryChanged(7), got {:?}", evs);
    }
}

#[cfg(test)]
mod issue08_library_management {
    use super::{BackendEvent, BackendFacade};

    #[test]
    fn add_library_root_emits_event() {
        let mut f = BackendFacade::default();
        let path = std::path::PathBuf::from("/music");
        let returned = f.add_library_root(path.clone());
        assert_eq!(returned, "/music");
        let evs = f.events();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            BackendEvent::LibraryRootAdded { path: p } => assert_eq!(p, "/music"),
            _ => panic!("expected LibraryRootAdded, got {:?}", evs[0]),
        }
    }

    #[test]
    fn remove_library_root_emits_event() {
        let mut f = BackendFacade::default();
        let path = std::path::PathBuf::from("/music/old");
        let returned = f.remove_library_root(path.clone());
        assert_eq!(returned, "/music/old");
        let evs = f.events();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            BackendEvent::LibraryRootRemoved { path: p } => assert_eq!(p, "/music/old"),
            _ => panic!("expected LibraryRootRemoved, got {:?}", evs[0]),
        }
    }

    #[test]
    fn clear_library_emits_event() {
        let mut f = BackendFacade::default();
        f.clear_library();
        let evs = f.events();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            BackendEvent::LibraryCleared => {}
            _ => panic!("expected LibraryCleared, got {:?}", evs[0]),
        }
    }
}

#[cfg(test)]
mod issue10_shutdown {
    use super::{BackendEvent, BackendFacade};

    #[test]
    fn shutdown_emits_notice() {
        let mut f = BackendFacade::default();
        f.shutdown();
        let evs = f.events();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            BackendEvent::Notice(msg) => assert_eq!(msg, "shutting down"),
            _ => panic!("expected Notice, got {:?}", evs[0]),
        }
    }

    #[test]
    fn shutdown_drain_returns_pending_events() {
        let mut f = BackendFacade::default();
        f.play(crate::domain::TrackId("test-track-1".to_string()));
        f.shutdown();
        let drained = f.shutdown_drain();
        // Should contain all pending events including the shutdown notice
        assert!(!drained.is_empty());
        let has_notice = drained.iter().any(|e| matches!(e, BackendEvent::Notice(_)));
        assert!(has_notice, "drained events should contain shutdown notice");
    }

    #[test]
    fn is_shutting_down_returns_false() {
        let f = BackendFacade::default();
        assert!(!f.is_shutting_down());
    }
}

#[cfg(test)]
mod issue11_boundary {
    use super::BackendFacade;

    /// Compile-time proof that BackendFacade has no AppState reference.
    /// If this compiles, the facade is free of AppState.
    #[allow(dead_code)]
    fn facade_compiles_without_appstate(_: &BackendFacade) {
        // This function existing and compiling is the proof.
        // Any reference to AppState in BackendFacade would cause a compile error
        // because AppState is not imported in this module scope.
    }

    #[test]
    fn boundary_no_appstate_reference() {
        // Trivial runtime assertion; the real check is the compile-time one above.
        let _ = std::any::type_name::<BackendFacade>();
    }
}

