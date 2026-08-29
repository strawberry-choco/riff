//! The Library Scan Service: walks a library root off the UI thread and
//! commits what it finds into the Application Store as ONE durable batch at
//! a time, reporting progress as pollable outcomes (ADR 0006 pattern).
//!
//! The module splits into two halves wired over unbounded crossbeam channels,
//! exactly like the Tag Edit and Cover services:
//!
//! - [`ScanService`] is the front-end seam the UI holds boxed as
//!   `Box<dyn Scans>`: it requests scans, cancels the running one, and polls
//!   drained [`ScanOutcome`]s. It never blocks on scan work and never
//!   touches the Library Session. It is cheaply shareable (internal `Arc`), so the
//!   watcher thread holds its own clone and answers "is this root currently
//!   scanning" from the same per-path state — the single source of truth.
//! - [`ScanWorker`] is the blocking back-end that owns the entire Library
//!   Scan flow: the filesystem walk (injected as a closure so this module
//!   never names infrastructure types), the store-backed freshness filter
//!   with fail-open through the Library query port, Track construction via
//!   [`crate::app::scan::build_tracks`], ~10-track durable batch commits
//!   through the Library mutation port (whose adapter bumps the session
//!   generation per ADR 0002), cancellation, and per-path scan-state
//!   tracking.
//!
//! Threading follows the Audio Engine pattern: nothing here spawns threads.
//! The composition root constructs both halves — binding the real scanner
//! into the walk closure over the same cancel flag it hands to [`new`] — and
//! runs [`ScanWorker::run`] on its own dedicated thread.

use crate::app::scan::build_tracks;
use crate::app::store::{LibraryMutationStore, LibraryQueryStore};
use crate::app::traits::MetadataReader;
use crate::domain::TrackId;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::app::MutexExt;

/// Tracks per durable commit. Every batch commits to the Application Store
/// as ONE immediate transaction first — an interrupted (or cancelled) scan
/// keeps all committed batches (spec user story 3).
const SCAN_BATCH_SIZE: usize = 10;

/// The filesystem walk, injected as a plain closure so the app layer stays
/// free of infrastructure types: the composition root binds the real scanner
/// over the same cancel flag the service cancels through. Infallible by
/// design — the infra scanner skips unreadable entries instead of failing,
/// so an unavailable root simply yields nothing to commit.
type Walk = Box<dyn Fn(&Path) -> Vec<PathBuf> + Send>;

/// One progress report from a running Library Scan: cumulative count of
/// files processed for the scanned root so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    /// A batch was processed (and any fresh tracks in it committed).
    Progress {
        /// The root being scanned.
        path: PathBuf,
        /// Files processed so far, cumulative.
        files_found: usize,
    },
    /// The whole walk finished; every batch committed.
    Complete {
        /// The root that was scanned.
        path: PathBuf,
        /// Total files discovered under the root.
        total_files: usize,
    },
    /// The scan did NOT land: a store-commit failed mid-scan. Surfaced as an
    /// outcome instead of dying in a log line, so the UI can tell the user
    /// the collection was not updated. Committed batches from earlier in the
    /// scan stay (durability is per batch).
    Failed {
        /// The root whose scan failed.
        path: PathBuf,
        /// Human-readable reason, fit for the status line.
        reason: String,
    },
}

/// Seam between the UI/watcher and the background Library Scan flow (ADR
/// 0006): request scans, cancel, poll outcomes, query per-path state.
/// Implemented by the real front-end over channels and by mocks in tests;
/// injected boxed into the UI and shared by clone with the watcher.
pub trait Scans: Send {
    /// Ask for a scan of one directory root. Never blocks. Ignored when that
    /// exact path already has a scan in flight — the serial worker would
    /// otherwise redo identical work.
    fn request(&self, path: PathBuf);

    /// Cancel whichever scan is currently running (the worker is serial, so
    /// one global flag is unambiguous). Committed batches stay; no
    /// `Complete` outcome is published for a cancelled scan.
    fn cancel(&self);

    /// Drain every outcome published so far, oldest first. Non-blocking: an
    /// empty Vec means nothing new has happened.
    fn poll(&self) -> Vec<ScanOutcome>;

    /// Whether a scan of exactly `path` is currently in flight (from request
    /// time until its final bookkeeping). Thread-safe: the watcher thread
    /// queries this while the worker updates it.
    fn is_scanning(&self, path: &Path) -> bool;
}

/// Front-end of the Library Scan Service: what the UI holds (boxed as
/// `Box<dyn Scans>`) and what the watcher thread shares by clone. Pairs
/// with a [`ScanWorker`] over channels plus two small shared cells (the
/// per-path active set and the global cancel flag).
#[derive(Clone)]
pub struct ScanService {
    request_tx: Sender<PathBuf>,
    outcome_rx: Receiver<ScanOutcome>,
    /// Paths with a scan in flight — the single source of truth for
    /// [`Scans::is_scanning`], written by the front-end at request time and
    /// cleared by the worker when the scan's bookkeeping ends.
    active: Arc<Mutex<HashSet<PathBuf>>>,
    /// Global cancel flag, shared with the worker and (via the walk closure)
    /// the infra scanner.
    cancel: Arc<AtomicBool>,
}

impl ScanService {
    /// Wire a matched ([`ScanService`], [`ScanWorker`]) pair over fresh
    /// unbounded channels. Box the returned service as the UI's
    /// `Box<dyn Scans>` handle, share clones with the watcher, and run the
    /// worker on its own thread (`worker.run()`), exactly like the Audio
    /// Engine. `cancel_flag` is caller-created so the composition root can
    /// bind the same flag into the real scanner it closes over in `walk`.
    #[must_use]
    pub fn new(
        reader: Box<dyn MetadataReader + Send>,
        queries: Box<dyn LibraryQueryStore + Send>,
        mutations: Box<dyn LibraryMutationStore + Send>,
        cancel_flag: Arc<AtomicBool>,
        walk: impl Fn(&Path) -> Vec<PathBuf> + Send + 'static,
    ) -> (Self, ScanWorker) {
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
            ScanWorker {
                request_rx,
                outcome_tx,
                active,
                cancel: cancel_flag,
                reader,
                queries,
                mutations,
                walk: Box::new(walk),
            },
        )
    }
}

impl Scans for ScanService {
    fn request(&self, path: PathBuf) {
        // Reserve the per-path slot atomically: a repeat request for a path
        // already scanning is dropped here instead of queueing identical
        // work behind the running scan. Reserving at request time (not at
        // dequeue time) makes `is_scanning` truthful immediately, which is
        // what the watcher's deferral logic keys off.
        if !self.active.lock_or_recover().insert(path.clone()) {
            return;
        }
        // A send fails only when the worker half is gone (shutting down);
        // dropping mirrors every other fire-and-forget sender.
        let _ = self.request_tx.send(path);
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn poll(&self) -> Vec<ScanOutcome> {
        let mut outcomes = Vec::new();
        while let Ok(outcome) = self.outcome_rx.try_recv() {
            outcomes.push(outcome);
        }
        outcomes
    }

    fn is_scanning(&self, path: &Path) -> bool {
        self.active.lock_or_recover().contains(path)
    }
}

/// Blocking back-end of the Library Scan Service: processes requested scans
/// one at a time, in request order, on ONE serial thread — so per-root scan
/// state never overlaps and the single global cancel flag is unambiguous.
/// Spawns nothing; the composition root runs it on the dedicated scan
/// thread.
pub struct ScanWorker {
    request_rx: Receiver<PathBuf>,
    outcome_tx: Sender<ScanOutcome>,
    active: Arc<Mutex<HashSet<PathBuf>>>,
    cancel: Arc<AtomicBool>,
    reader: Box<dyn MetadataReader + Send>,
    queries: Box<dyn LibraryQueryStore + Send>,
    mutations: Box<dyn LibraryMutationStore + Send>,
    walk: Walk,
}

/// How one directory scan ended, before mapping onto published outcomes.
enum ScanEnd {
    /// Walk finished; `usize` is the total file count.
    Completed(usize),
    /// Cancelled between batches: committed batches stay, nothing publishes.
    Cancelled,
    /// A store commit failed; the scan aborted with the reason.
    Failed(String),
}

impl ScanWorker {
    /// Block processing requested scans until every [`ScanService`] front
    /// end is dropped. Spawns nothing; run this on the dedicated scan
    /// thread.
    pub fn run(mut self) {
        while let Ok(path) = self.request_rx.recv() {
            self.scan_one(path);
        }
    }

    /// The entire Library Scan flow for one request, collapsed into the
    /// outcomes it publishes (zero or one).
    fn scan_one(&mut self, path: PathBuf) {
        // The global flag belongs to the scan about to run: reset it here so
        // a leftover cancellation from a previous scan cannot kill an
        // unrelated later one (the worker is serial, so one flag suffices).
        self.cancel.store(false, Ordering::Relaxed);

        match self.scan_directory(&path) {
            ScanEnd::Completed(total) => {
                let _ = self.outcome_tx.send(ScanOutcome::Complete {
                    path: path.clone(),
                    total_files: total,
                });
            }
            ScanEnd::Cancelled => {
                // Cancellation is not failure: committed batches survive
                // (durability is per batch) and no Complete is published.
                tracing::info!("Scan of {:?} cancelled", path);
            }
            ScanEnd::Failed(reason) => {
                let _ = self.outcome_tx.send(ScanOutcome::Failed {
                    path: path.clone(),
                    reason,
                });
            }
        }

        // Bookkeeping BEFORE publishing so the state is fully settled by the
        // time the end is observable through `poll`/`is_scanning` (mirrors
        // the Cover Worker): once a Complete is visible, the path is already
        // free for a follow-up request.
        self.active.lock_or_recover().remove(&path);
    }

    /// Walk one directory in ~10-track batches. Every batch commits to the
    /// Application Store as ONE durable transaction first — an interrupted
    /// scan keeps all committed batches — and the mutation adapter bumps the
    /// session generation so Session Projections refetch.
    fn scan_directory(&mut self, path: &Path) -> ScanEnd {
        let files = (self.walk)(path);
        let total = files.len();

        for (i, chunk) in files.chunks(SCAN_BATCH_SIZE).enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                return ScanEnd::Cancelled;
            }

            let processed = i * SCAN_BATCH_SIZE + chunk.len();

            // Skip paths the store already knows so rescans don't re-read
            // unchanged metadata. One indexed primary-key lookup per path —
            // cheap next to the tag I/O it saves, and the worker stays off
            // the Library Session entirely.
            let mut fresh_paths: Vec<PathBuf> = Vec::with_capacity(chunk.len());
            for p in chunk {
                match self.queries.get_track(&TrackId::from_path(p)) {
                    Ok(None) => fresh_paths.push(p.clone()),
                    Ok(Some(_)) => {}
                    Err(e) => {
                        // Fail open: when the check fails, scan the path
                        // anyway — the store upsert is idempotent and
                        // preserves play history.
                        tracing::warn!("Freshness check failed for {p:?}: {e}");
                        fresh_paths.push(p.clone());
                    }
                }
            }

            if !fresh_paths.is_empty() {
                // Per-file read failures are skipped inside `build_tracks`,
                // so a scan never aborts on one bad file.
                let tracks = build_tracks(fresh_paths, self.reader.as_ref());
                if !tracks.is_empty() {
                    // A failed commit aborts the scan and surfaces as a
                    // `Failed` outcome — never a silent success behind a
                    // later Complete.
                    if let Err(e) = self.mutations.apply_scan_batch(&tracks) {
                        tracing::error!("Scan batch failed to commit: {e}");
                        return ScanEnd::Failed(e.to_string());
                    }
                }
            }

            let _ = self.outcome_tx.send(ScanOutcome::Progress {
                path: path.to_path_buf(),
                files_found: processed.min(total),
            });
        }

        ScanEnd::Completed(total)
    }
}
