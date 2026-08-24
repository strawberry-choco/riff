//! The Tag Edit Service: persists one user Tag Edit as ONE durable change —
//! file tags first (source of truth), then the Application Store facts — and
//! reports a single combined outcome (ADR 0006).
//!
//! The module splits into two halves wired over unbounded crossbeam channels:
//!
//! - [`TagEditService`] is the front-end seam the UI holds as
//!   `Box<dyn TagEdits>`: it submits intent and polls outcomes. It never
//!   blocks and never touches disk.
//! - [`TagEditWorker`] is the blocking back-end that owns the entire save
//!   flow: Lofty write via the [`MetadataWriter`](crate::app::traits::MetadataWriter)
//!   port, resolving the edited Track from the Application Store through the
//!   Library query port, applying the edit to the fresh copy, committing
//!   through the Library mutation port's targeted tag-refresh flow (which
//!   bumps the session generation per ADR 0002), and collapsing file-write
//!   and store-commit into one reported [`TagEditOutcome`].
//!
//! Threading follows the Audio Engine pattern: nothing here spawns threads.
//! The composition root constructs both halves and runs
//! [`TagEditWorker::run`] on its own dedicated thread; the UI keeps only the
//! boxed front-end handle.

use crate::app::store::{LibraryMutationStore, LibraryQueryStore};
use crate::app::traits::{MetadataWriter, TagEdit};
use crate::domain::{Track, TrackId};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::path::PathBuf;

/// Failure reason reported when the edited Track no longer resolves from the
/// Application Store (removed mid-edit, or the read failed): nothing is
/// persisted. Exact wording is product behavior (spec user story 2).
const TRACK_NO_LONGER_IN_LIBRARY: &str = "Track is no longer in the library";

/// One requested Tag Edit: the Track's identity (its full file path), the
/// file to write, and the edit itself. Pure application-layer DTO.
#[derive(Debug, Clone)]
pub struct TagEditRequest {
    /// Identity of the edited Track in the Application Store.
    pub track_id: TrackId,
    /// File whose tags the edit writes to.
    pub path: PathBuf,
    /// The requested edit; only `Some` fields are written.
    pub edit: TagEdit,
}

/// The single combined outcome of one submitted Tag Edit: either the whole
/// durable change landed (file tags AND Store facts), or it failed with a
/// human-readable reason. A successful file write with a failed store commit
/// surfaces as [`TagEditOutcome::Failed`], never silent success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagEditOutcome {
    /// The edit persisted to both the file tags and the Application Store.
    Saved,
    /// Nothing durable happened (or the store commit failed); the reason is
    /// fit to show in the still-open Tag Edit dialog.
    Failed { reason: String },
}

/// Seam between the UI and the background Tag Edit save flow (ADR 0006):
/// send intent, poll outcomes. Implemented by the real front-end over
/// channels and by mocks in tests; injected boxed.
pub trait TagEdits: Send {
    /// Enqueue one Tag Edit for background processing. Never blocks.
    fn submit(&self, request: TagEditRequest);

    /// Drain at most one completed outcome, oldest first. Non-blocking:
    /// `None` means nothing has finished yet.
    fn poll(&self) -> Option<TagEditOutcome>;
}

/// Front-end of the Tag Edit Service: what the UI holds (boxed as
/// `Box<dyn TagEdits>`). Pairs with a [`TagEditWorker`] over channels.
pub struct TagEditService {
    request_tx: Sender<TagEditRequest>,
    outcome_rx: Receiver<TagEditOutcome>,
}

impl TagEditService {
    /// Wire a matched ([`TagEditService`], [`TagEditWorker`]) pair over fresh
    /// unbounded channels. Box the returned service as the UI's
    /// `Box<dyn TagEdits>` handle and run the worker on its own thread
    /// (`worker.run()`), exactly like the Audio Engine.
    #[must_use]
    pub fn new(
        writer: Box<dyn MetadataWriter + Send>,
        library_queries: Box<dyn LibraryQueryStore + Send>,
        library_mutations: Box<dyn LibraryMutationStore + Send>,
    ) -> (Self, TagEditWorker) {
        let (request_tx, request_rx) = unbounded();
        let (outcome_tx, outcome_rx) = unbounded();
        (
            Self {
                request_tx,
                outcome_rx,
            },
            TagEditWorker {
                request_rx,
                outcome_tx,
                writer,
                library_queries,
                library_mutations,
            },
        )
    }
}

impl TagEdits for TagEditService {
    fn submit(&self, request: TagEditRequest) {
        // A send fails only when the worker half is gone (shutting down);
        // dropping the request mirrors every other fire-and-forget sender.
        let _ = self.request_tx.send(request);
    }

    fn poll(&self) -> Option<TagEditOutcome> {
        self.outcome_rx.try_recv().ok()
    }
}

/// Blocking back-end of the Tag Edit Service: processes submitted edits one
/// at a time, in submission order, and publishes one combined outcome each.
/// Spawns nothing; the composition root runs it on the dedicated tag-edit
/// thread.
pub struct TagEditWorker {
    request_rx: Receiver<TagEditRequest>,
    outcome_tx: Sender<TagEditOutcome>,
    writer: Box<dyn MetadataWriter + Send>,
    library_queries: Box<dyn LibraryQueryStore + Send>,
    library_mutations: Box<dyn LibraryMutationStore + Send>,
}

impl TagEditWorker {
    /// Block processing submitted edits until every [`TagEditService`] front
    /// end is dropped. Spawns nothing; run this on the dedicated tag-edit
    /// thread.
    pub fn run(mut self) {
        while let Ok(request) = self.request_rx.recv() {
            let outcome = self.process(request);
            let _ = self.outcome_tx.send(outcome);
        }
    }

    /// The entire save flow for one request, collapsed into one outcome:
    /// write the file tags (source of truth), resolve the edited Track from
    /// the Application Store, apply the edit to the fresh copy, commit the
    /// Store facts through the targeted tag-refresh flow.
    fn process(&mut self, request: TagEditRequest) -> TagEditOutcome {
        // File tags first: they are the source of truth. A failed write
        // ends the flow here — nothing is resolved or persisted.
        if let Err(e) = self.writer.write_metadata(&request.path, &request.edit) {
            tracing::warn!("Tag write failed for {:?}: {e}", request.path);
            return TagEditOutcome::Failed {
                reason: e.to_string(),
            };
        }

        // Resolve the edited Track from the Store — the sole authority.
        // Unknown (removed mid-edit, or read failed): nothing to persist.
        let Some(mut track) = self.resolve_track(&request.track_id) else {
            return TagEditOutcome::Failed {
                reason: TRACK_NO_LONGER_IN_LIBRARY.to_string(),
            };
        };

        // Apply the edit to the fresh copy; play history fields are untouched.
        request.edit.apply_to(&mut track.metadata);

        // Persist the Store facts as one durable transaction. A failed
        // commit collapses the whole save into a failure — never a silent
        // success behind an Ok file write.
        if let Err(e) = self.library_mutations.apply_tag_refresh(&track) {
            tracing::error!("Failed to persist tag edit for {:?}: {e}", request.path);
            return TagEditOutcome::Failed {
                reason: e.to_string(),
            };
        }

        TagEditOutcome::Saved
    }

    /// Point read of one Track from the store; read failures collapse to
    /// `None`, like the UI's former `SessionViews::resolve_track`.
    fn resolve_track(&self, id: &TrackId) -> Option<Track> {
        match self.library_queries.get_track(id) {
            Ok(track) => track,
            Err(e) => {
                tracing::warn!("Failed to resolve track {id:?} from the store: {e}");
                None
            }
        }
    }
}
