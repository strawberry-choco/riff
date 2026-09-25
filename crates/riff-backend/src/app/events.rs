//! The Backend Events inbox: the app layer's observable surface toward the
//! frontend.
//!
//! The frontend holds exactly one shared `Arc<Mutex<BackendEvents>>`. Two
//! inputs land on the inbox — dispatched `PlaybackCommand`s (recorded by the
//! transports wired at the Composition Root) and the Application Store's
//! `StoreChanged` stream (subscribed at spawn) — plus the coordinator's
//! playback-error notices, stamped with source and severity on drain.
//!
//! # Invariants
//!
//! - The frontend never constructs raw [`crate::domain::PlaybackCommand`]s;
//!   every dispatched command arrives here recorded by a transport.
//! - The inbox holds no session handles; both sessions stay backend-side.
//! - Draining never loses a store change: Library generations coalesce to
//!   the latest, playlist generations forward one event per bump.

use crossbeam_channel::Receiver;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::app::store::StoreChanged;
use crate::domain::PlaybackCommand;

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

/// A typed change event the backend pushes to the frontend through the
/// events inbox.
///
/// This is the surface the inbox actually fills. Library Scan progress and
/// Tag Edit outcomes are not events here: they reach the frontend through the
/// separate polls the frontend drives, and folding those into this inbox is a
/// recorded follow-up of its own, not something this module does.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    /// A playback command as dispatched through a transport.
    CommandApplied(PlaybackCommand),
    /// A notice stamped with source and severity on drain.
    TypedNotice(NoticePayload),
    /// Library generation moved (coalesced to about four emissions/sec).
    LibraryChanged { generation: u64 },
    /// Playlist generation moved (forwarded one event per bump).
    PlaylistsChanged { generation: u64 },
}

// ---------------------------------------------------------------------------
// BackendEvents
// ---------------------------------------------------------------------------

/// The event inbox the frontend drains.
pub struct BackendEvents {
    // --- Event inbox the frontend drains -------------------------------
    events: VecDeque<BackendEvent>,

    // --- Event backbone (issue 04) ---------------------------------------
    backend_changes: Receiver<StoreChanged>,
    last_library_change_time: Option<Instant>,
    last_library_generation: u64,
    last_playlist_generation: u64,

    // --- Playback notices (issue 01 seam fix) ----------------------------
    /// Pre-formatted playback error messages from the Playback Coordinator.
    /// The coordinator sends plain strings (it owns no event types), and
    /// `BackendEvents` stamps each one with playback source and error severity
    /// as a [`BackendEvent::TypedNotice`].
    playback_notices: Receiver<String>,
}

impl Default for BackendEvents {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            backend_changes: crossbeam_channel::unbounded().1,
            last_library_change_time: None,
            last_library_generation: 0,
            last_playlist_generation: 0,
            playback_notices: crossbeam_channel::unbounded().1,
        }
    }
}

impl BackendEvents {
    /// Coalesce window for [`BackendEvent::LibraryChanged`]: ~4 emissions/sec.
    pub const COALESCE_WINDOW: Duration = Duration::from_millis(250);

    /// Record one dispatched playback command onto the inbox. Transport
    /// adapters wired at the Composition Root call this synchronously on
    /// every dispatch path (mouse, keyboard, tray) before the command is
    /// forwarded to the Audio Engine.
    pub fn record_command(&mut self, cmd: PlaybackCommand) {
        self.events.push_back(BackendEvent::CommandApplied(cmd));
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
            let since_last = self.last_library_change_time.map(|t| now.duration_since(t));
            let should_emit = match since_last {
                None => true,
                Some(d) => d >= Self::COALESCE_WINDOW,
            };
            if should_emit {
                out.push(BackendEvent::LibraryChanged { generation: latest });
                self.last_library_change_time = Some(now);
            }
        }
        // Drain playback-error notices into typed notices (issue 01 seam fix).
        while let Ok(message) = self.playback_notices.try_recv() {
            out.push(BackendEvent::TypedNotice(NoticePayload {
                severity: NoticeSeverity::Error,
                source: NoticeSource::Playback,
                message,
            }));
        }
        out
    }

    pub fn subscribe_to_backend_changes(&mut self, rx: Receiver<StoreChanged>) {
        self.backend_changes = rx;
    }

    /// Subscribe the coordinator's playback-error notice channel. Each
    /// drained message becomes a [`BackendEvent::TypedNotice`] with
    /// [`NoticeSource::Playback`] and [`NoticeSeverity::Error`].
    pub fn subscribe_playback_notices(&mut self, rx: Receiver<String>) {
        self.playback_notices = rx;
    }
}

// ===========================================================================
// Tests
// ===========================================================================

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

    use super::{BackendEvent, BackendEvents, StoreChanged};

    #[test]
    fn recorded_commands_surface_as_command_applied() {
        let mut f = BackendEvents::default();
        f.record_command(crate::domain::PlaybackCommand::Play(
            crate::domain::TrackId("a.mp3".to_string()),
        ));
        let evs = f.events();
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], BackendEvent::CommandApplied(_)));
    }

    #[test]
    fn library_change_events_are_coalesced() {
        let (tx, rx) = unbounded::<StoreChanged>();
        let mut f = BackendEvents::default();
        f.subscribe_to_backend_changes(rx);
        for i in 1..=100 {
            let _ = tx.send(StoreChanged::Library(i));
        }
        let evs = f.events();
        let library_events: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, BackendEvent::LibraryChanged { .. }))
            .collect();
        assert!(library_events.len() <= 1);
        let first = library_events.first().copied();
        assert_eq!(
            first,
            Some(&BackendEvent::LibraryChanged { generation: 100 })
        );
    }

    #[test]
    fn playlists_change_events_forward_without_coalescing() {
        let (tx, rx) = unbounded::<StoreChanged>();
        let mut f = BackendEvents::default();
        f.subscribe_to_backend_changes(rx);
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
}

#[cfg(test)]
mod issue01_playback_notices {
    use super::{BackendEvent, BackendEvents, NoticeSeverity, NoticeSource};

    #[test]
    fn playback_notice_surfaces_as_typed_notice_with_playback_source_and_error_severity() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut f = BackendEvents::default();
        f.subscribe_playback_notices(rx);

        tx.send("Playback error: boom".to_string()).unwrap();

        let evs = f.events();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            BackendEvent::TypedNotice(payload) => {
                assert_eq!(payload.severity, NoticeSeverity::Error);
                assert_eq!(payload.source, NoticeSource::Playback);
                assert_eq!(payload.message, "Playback error: boom");
            }
            other => panic!("expected TypedNotice, got {other:?}"),
        }
    }
}
