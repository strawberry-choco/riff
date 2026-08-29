//! Playlist entry validity helpers. User playlists themselves are user data
//! in the Application Store: every mutation commits through the
//! [`crate::app::store::PlaylistStore`] port as one immediate durable
//! transaction, and [`crate::app::store::PlaylistStore::load_playlist_entries`]
//! resolves each entry together with its Library validity (SQL LEFT JOIN).
//!
//! These helpers decide, at read time, which entries can play: an entry is
//! valid when its track is known to the Application Store's Library AND its
//! file still exists on disk. Dangling entries stay listed (flagged invalid
//! in the UI) and resolve again once the referenced files return.

use riff_persistence::store::PlaylistEntry;
use riff_persistence::track::TrackId;

/// A playlist entry is valid when the track is known to the Application
/// Store's Library (the store's LEFT JOIN validity flag) AND its file still
/// exists on disk. Moved or deleted files make entries invalid — they are
/// flagged in the UI and skipped for playback, never a crash.
pub fn track_is_valid(entry: &PlaylistEntry) -> bool {
    entry
        .track
        .as_ref()
        .is_some_and(|track| track.file_path.exists())
}

/// The playable entries of a resolved playlist-entry list, in playlist
/// order, ready to be loaded into the playback queue. Invalid entries are
/// skipped.
pub fn valid_tracks(entries: &[PlaylistEntry]) -> Vec<TrackId> {
    entries
        .iter()
        .filter(|entry| track_is_valid(entry))
        .map(|entry| entry.id.clone())
        .collect()
}

// --- Drag-reorder math ----------------------------------------------------------

/// The new entry order after a drag-and-drop gesture: remove the entry at
/// `from` and reinsert it at `to`, shifting everything between to close and
/// open the gaps. Returns `None` when either index is out of bounds or the
/// entry was dropped back onto its own slot (a no-op), so callers never
/// commit an empty gesture.
#[must_use]
pub fn reorder_tracks(tracks: &[TrackId], from: usize, to: usize) -> Option<Vec<TrackId>> {
    if from == to || from >= tracks.len() || to >= tracks.len() {
        return None;
    }
    let mut reordered = tracks.to_vec();
    let dragged = reordered.remove(from);
    reordered.insert(to, dragged);
    Some(reordered)
}

// --- Playlist Manager Service ---------------------------------------------------

use crate::app::errors::LibraryError;
use crate::app::traits::{MetadataReader, MetadataWriter};
use crossbeam_channel::{Receiver, Sender, unbounded};
use riff_persistence::store::{LibraryMutationStore, LibraryQueryStore};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// Tracks per durable commit.
#[allow(
    dead_code,
    reason = "service pair kept for the capability seam; the
composition root commits playlist mutations directly"
)]
const PLAYLIST_BATCH_SIZE: usize = 10;

/// One progress report from a running Playlist operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistOutcome {
    /// A batch was processed.
    Progress {
        /// The playlist being processed.
        playlist_id: riff_persistence::playlist::PlaylistId,
        /// Entries processed so far.
        entries_processed: usize,
    },
    /// The operation finished.
    Complete {
        /// The playlist that was processed.
        playlist_id: riff_persistence::playlist::PlaylistId,
        /// Total entries processed.
        total_entries: usize,
    },
    /// The operation failed.
    Failed {
        /// The playlist whose operation failed.
        playlist_id: riff_persistence::playlist::PlaylistId,
        /// Human-readable reason.
        reason: String,
    },
}

/// Seam between the UI and the background Playlist flow.
pub trait Playlists: Send {
    /// Create a new playlist.
    fn create(&self, name: String) -> Result<riff_persistence::playlist::PlaylistId, LibraryError>;
    /// Rename a playlist.
    fn rename(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        name: String,
    ) -> Result<(), LibraryError>;
    /// Delete a playlist.
    fn delete(&self, id: &riff_persistence::playlist::PlaylistId) -> Result<(), LibraryError>;
    /// Add entries to a playlist.
    fn add_entries(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        tracks: Vec<TrackId>,
    ) -> Result<(), LibraryError>;
    /// Remove entries from a playlist.
    fn remove_entries(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        tracks: &[TrackId],
    ) -> Result<(), LibraryError>;
    /// Reorder playlist entries.
    fn reorder(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        from: usize,
        to: usize,
    ) -> Result<(), LibraryError>;
}

/// Front-end of the Playlist Manager Service.
#[derive(Clone)]
#[allow(
    dead_code,
    reason = "service pair kept for the capability seam; the
composition root commits playlist mutations directly"
)]
pub struct PlaylistManager {
    request_tx: Sender<PlaylistRequest>,
    outcome_rx: Receiver<PlaylistOutcome>,
    active: Arc<Mutex<HashSet<riff_persistence::playlist::PlaylistId>>>,
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "service pair kept for the capability seam; the
composition root commits playlist mutations directly"
)]
enum PlaylistRequest {
    Create(String),
    Rename(riff_persistence::playlist::PlaylistId, String),
    Delete(riff_persistence::playlist::PlaylistId),
    AddEntries(riff_persistence::playlist::PlaylistId, Vec<TrackId>),
    RemoveEntries(riff_persistence::playlist::PlaylistId, Vec<TrackId>),
    Reorder(riff_persistence::playlist::PlaylistId, usize, usize),
}

impl PlaylistManager {
    #[must_use]
    pub fn new(
        reader: Box<dyn MetadataReader + Send>,
        writer: Box<dyn MetadataWriter + Send>,
        queries: Box<dyn LibraryQueryStore + Send>,
        mutations: Box<dyn LibraryMutationStore + Send>,
        cancel_flag: Arc<AtomicBool>,
    ) -> (Self, PlaylistManagerWorker) {
        let (request_tx, request_rx) = unbounded();
        let (outcome_tx, outcome_rx) = unbounded();
        let active = Arc::new(Mutex::new(HashSet::new()));
        (
            Self {
                request_tx,
                outcome_rx,
                active: Arc::clone(&active),
                cancel: Arc::clone(&cancel_flag),
            },
            PlaylistManagerWorker {
                request_rx,
                outcome_tx,
                active,
                cancel: cancel_flag,
                reader,
                writer,
                queries,
                mutations,
            },
        )
    }
}

impl Playlists for PlaylistManager {
    fn create(&self, name: String) -> Result<riff_persistence::playlist::PlaylistId, LibraryError> {
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let _ = self.request_tx.send(PlaylistRequest::Create(name));
        rx.recv().unwrap()
    }

    fn rename(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        name: String,
    ) -> Result<(), LibraryError> {
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let _ = self
            .request_tx
            .send(PlaylistRequest::Rename(id.clone(), name));
        rx.recv().unwrap()
    }

    fn delete(&self, id: &riff_persistence::playlist::PlaylistId) -> Result<(), LibraryError> {
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let _ = self.request_tx.send(PlaylistRequest::Delete(id.clone()));
        rx.recv().unwrap()
    }

    fn add_entries(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        tracks: Vec<TrackId>,
    ) -> Result<(), LibraryError> {
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let _ = self
            .request_tx
            .send(PlaylistRequest::AddEntries(id.clone(), tracks));
        rx.recv().unwrap()
    }

    fn remove_entries(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        tracks: &[TrackId],
    ) -> Result<(), LibraryError> {
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let _ = self
            .request_tx
            .send(PlaylistRequest::RemoveEntries(id.clone(), tracks.to_vec()));
        rx.recv().unwrap()
    }

    fn reorder(
        &self,
        id: &riff_persistence::playlist::PlaylistId,
        from: usize,
        to: usize,
    ) -> Result<(), LibraryError> {
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let _ = self
            .request_tx
            .send(PlaylistRequest::Reorder(id.clone(), from, to));
        rx.recv().unwrap()
    }
}

/// Background worker for playlist operations.
#[allow(
    dead_code,
    reason = "service pair kept for the capability seam; the
composition root commits playlist mutations directly"
)]
pub struct PlaylistManagerWorker {
    request_rx: Receiver<PlaylistRequest>,
    outcome_tx: Sender<PlaylistOutcome>,
    active: Arc<Mutex<HashSet<riff_persistence::playlist::PlaylistId>>>,
    cancel: Arc<AtomicBool>,
    reader: Box<dyn crate::app::traits::MetadataReader + Send>,
    writer: Box<dyn crate::app::traits::MetadataWriter + Send>,
    queries: Box<dyn crate::app::store::LibraryQueryStore + Send>,
    mutations: Box<dyn crate::app::store::LibraryMutationStore + Send>,
}

impl PlaylistManagerWorker {
    pub fn run(self) {
        while let Ok(_req) = self.request_rx.recv() {
            // Process request
        }
    }
}
