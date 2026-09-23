//! The Library Scan Service: walks a library root off the UI thread and
//! commits what it finds into the Application Store as ONE durable batch at
//! a time, reporting progress as pollable outcomes (ADR 0006 pattern).
//!
//! The module splits into two halves wired over unbounded crossbeam channels,
//! exactly like the Tag Edit and Cover services:
//!
//! - [`ScanService`] is the front-end seam the UI holds boxed as
//!   `Box<dyn Scans>`: it requests scans, cancels the running one, and polls
//!   drained [`ScanOutcome`]s. It never blocks and never touches the Library Session. It is cheaply shareable (internal `Arc`), so the
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

use crate::app::MutexExt;
use crate::app::scan::build_tracks;
use crate::app::store::{FullScanSummary, LibraryMutationStore, LibraryQueryStore};
use crate::app::traits::MetadataReader;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use riff_persistence::track::{METADATA_VERSION, TrackId};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Tracks per durable commit. Every batch commits to the Application Store
/// as ONE immediate transaction first — an interrupted (or cancelled) scan
/// keeps all committed batches (spec user story 3).
pub const SCAN_BATCH_SIZE: usize = 10;

/// How long [`ScanWorker::run`] waits for the next request before it looks at
/// its stop flag. The worker is idle almost all the time, so the wait must
/// expire: with a blocking `recv` the thread could never be asked to stop
/// without dropping every [`ScanService`] front end first.
const WORKER_POLL: Duration = Duration::from_millis(10);

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
    /// Engine.
    ///
    /// Both flags are caller-created. `cancel_flag` is shared with the real
    /// scanner the composition root closes over in `walk`, so one cancellation
    /// reaches the walk and the batch loop alike. `stop` is the Composition
    /// Root's shutdown request: a different lever from cancellation, because
    /// it ends the worker itself (after the scan in flight has settled) rather
    /// than aborting the scan running on it.
    #[must_use]
    pub fn new(
        reader: Box<dyn MetadataReader + Send>,
        queries: Box<dyn LibraryQueryStore + Send>,
        mutations: Box<dyn LibraryMutationStore + Send>,
        cancel_flag: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
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
                stop,
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
        // dropping the request mirrors every other fire-and-forget sender.
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
    /// Cooperative stop request from the Composition Root; see [`Self::run`].
    stop: Arc<AtomicBool>,
}

/// How one directory scan ended, before mapping onto published outcomes.
enum ScanEnd {
    /// Walk finished; `files` is the total discovered count and `errors`
    /// the discovered files that could not be indexed (design-handoff
    /// issue 12).
    Completed { files: usize, errors: usize },
    /// Cancelled between batches: committed batches stay, nothing publishes.
    Cancelled,
    /// A store commit failed; the scan aborted with the reason.
    Failed(String),
}

impl ScanWorker {
    /// Process requested scans until a stop request arrives or every
    /// [`ScanService`] front end is dropped. Spawns nothing; run this on the
    /// dedicated scan thread.
    ///
    /// The receive polls on [`WORKER_POLL`] instead of blocking so the stop
    /// flag can be observed while no scan is requested — the worker's normal
    /// state. A stop is honored between scans, never inside one: the scan in
    /// flight ends through the ordinary cancel path (see
    /// [`Self::scan_one`]), which is what keeps its committed batches.
    pub fn run(mut self) {
        while !self.stop.load(Ordering::Relaxed) {
            match self.request_rx.recv_timeout(WORKER_POLL) {
                Ok(path) => {
                    // A request dequeued in the same instant as the stop must
                    // not reset the cancel flag and start a fresh full scan
                    // that shutdown would then have to wait out.
                    if self.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    self.scan_one(path);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
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
            ScanEnd::Completed { files, errors } => {
                // A completed full scan records its summary (design-handoff
                // issues 05 and 12) so the sidebar footer and the Settings
                // Library pane can answer "when did I last scan, and what
                // did it see". The batches are already committed, so a
                // record failure only warns: it must not turn a landed scan
                // into a reported failure.
                let summary = FullScanSummary {
                    at: SystemTime::now(),
                    files,
                    errors,
                };
                if let Err(e) = self.mutations.record_full_scan_completed(summary) {
                    tracing::warn!("Failed to record the last-scan summary: {e}");
                }
                // Stamped only here, on the completed branch: a cancelled or
                // failed scan leaves the store behind so the next scan
                // finishes the re-read. Before `Complete` publishes, so an
                // observer of the outcome never sees a lagging version for a
                // scan that already earned the stamp.
                if let Err(e) = self.mutations.stamp_metadata_version(METADATA_VERSION) {
                    tracing::warn!("Failed to stamp the metadata version: {e}");
                }
                let _ = self.outcome_tx.send(ScanOutcome::Complete {
                    path: path.clone(),
                    total_files: files,
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
    /// session generation so Session Projections refetch. Returns the total
    /// discovered file count plus how many of those could not be indexed
    /// (design-handoff issue 12).
    fn scan_directory(&mut self, path: &Path) -> ScanEnd {
        let files = (self.walk)(path);
        let total = files.len();
        let mut errors = 0usize;

        // Whether this store's metadata was written under the shape the binary
        // reads. A store behind the binary re-reads every known path for one
        // scan so a tag added since its last full scan (an album's ReplayGain,
        // say) reaches a library indexed before that column existed; the
        // completed scan stamps the version forward, which is what bounds the
        // re-read. Read ONCE here, never per path — the filter already costs
        // one indexed lookup per file, and a row fetch per batch would be a
        // regression on a large library.
        let metadata_is_current = match self.queries.metadata_version() {
            Ok(version) => version >= METADATA_VERSION,
            Err(e) => {
                // Fail the same way the per-path check below does: re-read.
                // An unanswerable version is not evidence the store is current.
                tracing::warn!("Metadata version check failed: {e}");
                false
            }
        };

        for (i, chunk) in files.chunks(SCAN_BATCH_SIZE).enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                return ScanEnd::Cancelled;
            }

            let processed = i * SCAN_BATCH_SIZE + chunk.len();

            // Skip paths the store already knows so rescans don't re-read
            // unchanged metadata. One indexed primary-key lookup per path —
            // cheap next to the tag I/O it saves, and the worker stays off
            // the Library Session entirely. That saving is only safe while the
            // store is current; see `metadata_is_current` above.
            let fresh_paths: Vec<PathBuf> = if metadata_is_current {
                chunk
                    .iter()
                    .filter(|p| match self.queries.get_track(&TrackId::from_path(p)) {
                        Ok(known) => known.is_none(),
                        Err(e) => {
                            // Fail open: when the check fails, scan the path
                            // anyway — the store upsert is idempotent and
                            // preserves play history.
                            tracing::warn!("Freshness check failed for {p:?}: {e}");
                            true
                        }
                    })
                    .cloned()
                    .collect()
            } else {
                chunk.to_vec()
            };

            if !fresh_paths.is_empty() {
                // Per-file read failures are skipped inside `build_tracks`,
                // so a scan never aborts on one bad file; they are counted
                // as the scan's error tally instead.
                let fresh_count = fresh_paths.len();
                let tracks = build_tracks(fresh_paths, self.reader.as_ref());
                errors += fresh_count - tracks.len();
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

        ScanEnd::Completed {
            files: total,
            errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::errors::LibraryError;
    use crate::app::store::{LibraryCounts, LibraryMutationStore, LibraryQueryStore, StoreError};
    use crate::app::traits::{AudioFormatInfo, MetadataReader};
    use crate::domain::{Album, Artist, CoverSource, GenreCount, SmartPlaylistKind};
    use riff_persistence::track::{Track, TrackId, TrackMetadata};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    fn make_track(id: &str) -> Track {
        Track {
            id: TrackId(id.to_string()),
            file_path: PathBuf::from(id),
            metadata: TrackMetadata::default(),
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
            favorite: false,
            search_text: String::new(),
        }
    }

    struct MockReader;
    impl MetadataReader for MockReader {
        fn read_all(
            &self,
            _path: &Path,
        ) -> Result<(TrackMetadata, Duration, CoverSource, AudioFormatInfo), LibraryError> {
            Ok((
                TrackMetadata::default(),
                Duration::from_mins(3),
                CoverSource::None,
                AudioFormatInfo {
                    sample_rate: 44100,
                    channels: 2,
                },
            ))
        }

        fn read_cover_source(&self, _path: &Path) -> Result<CoverSource, LibraryError> {
            Ok(CoverSource::None)
        }
    }

    struct MockQueries {
        known: Arc<Mutex<HashSet<PathBuf>>>,
    }
    impl LibraryQueryStore for MockQueries {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            Ok(self
                .known
                .lock()
                .unwrap()
                .contains(&PathBuf::from(&id.0))
                .then(|| make_track(&id.0)))
        }

        // Current, so these tests keep exercising the skipping branch; the
        // lagging branch has its own fixtures.
        fn metadata_version(&self) -> Result<u32, StoreError> {
            Ok(METADATA_VERSION)
        }

        fn tracks_page(
            &self,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Track>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }
        fn library_counts(&self) -> Result<LibraryCounts, StoreError> {
            Ok(LibraryCounts::default())
        }
        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            Ok(Vec::new())
        }
        fn search_page(
            &self,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Track>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }
        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            Ok(Vec::new())
        }
        fn artist_albums(&self, _artist: &str) -> Result<Vec<Album>, StoreError> {
            Ok(Vec::new())
        }
        fn album_tracks(
            &self,
            _album_artist: &str,
            _title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }
        fn folder_has_audio(&self, _folder: &Path) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn folder_has_search_match(
            &self,
            _folder: &Path,
            _query: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn track_ids_in_folder_tree(&self, _folder: &Path) -> Result<Vec<TrackId>, StoreError> {
            Ok(Vec::new())
        }
        fn tracks_in_folder(&self, _folder: &Path) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn folder_track_count(&self, _folder: &Path) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn last_full_scan(&self) -> Result<Option<FullScanSummary>, StoreError> {
            Ok(None)
        }
        fn subdirs_with_audio(&self, _folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
            Ok(Vec::new())
        }
        fn smart_playlist(
            &self,
            _kind: SmartPlaylistKind,
            _limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError> {
            Ok(Vec::new())
        }

        fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError> {
            Ok(Vec::new())
        }

        fn artists_in_genre(&self, _genre: &str) -> Result<Vec<Artist>, StoreError> {
            Ok(Vec::new())
        }

        fn artist_albums_in_genre(
            &self,
            _artist: &str,
            _genre: &str,
        ) -> Result<Vec<Album>, StoreError> {
            Ok(Vec::new())
        }

        fn album_tracks_in_genre(
            &self,
            _album_artist: &str,
            _album_title: &str,
            _genre: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn hit_albums_page(
            &self,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Album>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }
        fn hit_artists_page(
            &self,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Artist>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }
        fn album_hit_tracks(
            &self,
            _album_artist: &str,
            _album_title: &str,
            _query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }
        fn album_is_name_hit(
            &self,
            _album_artist: &str,
            _album_title: &str,
            _query: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn hit_albums_in_genre(
            &self,
            _genre: &str,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<Album>, StoreError> {
            Ok(Vec::new())
        }
        fn hit_artists_in_genre(
            &self,
            _genre: &str,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<Artist>, StoreError> {
            Ok(Vec::new())
        }
        fn album_hit_tracks_in_genre(
            &self,
            _album_artist: &str,
            _album_title: &str,
            _genre: &str,
            _query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }
        fn hit_genre_counts(&self, _query: &str) -> Result<Vec<GenreCount>, StoreError> {
            Ok(Vec::new())
        }

        fn artists_page(
            &self,
            _direction: crate::app::store::SortDirection,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Artist>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }

        fn albums_page(
            &self,
            _direction: crate::app::store::SortDirection,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Album>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }

        fn genres_page(
            &self,
            _direction: crate::app::store::SortDirection,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<GenreCount>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }

        fn artists_in_genre_page(
            &self,
            _genre: &str,
            _direction: crate::app::store::SortDirection,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Artist>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }

        fn artist_albums_in_genre_page(
            &self,
            _artist: &str,
            _genre: &str,
            _direction: crate::app::store::SortDirection,
            _offset: usize,
            _limit: usize,
        ) -> Result<crate::app::store::Page<Album>, StoreError> {
            Ok(crate::app::store::Page::new(0, Vec::new(), 0))
        }
    }

    struct MockMutations;
    impl LibraryMutationStore for MockMutations {
        fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, StoreError> {
            Ok(0)
        }
        fn stamp_metadata_version(&mut self, _version: u32) -> Result<(), StoreError> {
            Ok(())
        }
        fn record_track_played(
            &mut self,
            _id: &TrackId,
            _played_at: SystemTime,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn apply_tag_refresh(&mut self, _track: &Track) -> Result<(), StoreError> {
            Ok(())
        }
        fn set_track_favorite(
            &mut self,
            _id: &TrackId,
            _favorite: bool,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn remove_library_path(&mut self, _root: &Path) -> Result<usize, StoreError> {
            Ok(0)
        }
        fn clear_library(&mut self) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn record_full_scan_completed(
            &mut self,
            _summary: FullScanSummary,
        ) -> Result<(), StoreError> {
            Ok(())
        }
    }

    #[allow(
        clippy::type_complexity,
        reason = "mirrors the walk-closure parameter type of ScanService::new"
    )]
    fn make_walk() -> Box<dyn Fn(&Path) -> Vec<PathBuf> + Send> {
        Box::new(|_path: &Path| vec![PathBuf::from("track1.flac"), PathBuf::from("track2.flac")])
    }

    #[test]
    fn scan_service_requests_scan() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (service, _worker) = ScanService::new(
            Box::new(MockReader),
            Box::new(MockQueries {
                known: Arc::new(Mutex::new(HashSet::new())),
            }),
            Box::new(MockMutations),
            Arc::clone(&cancel),
            Arc::new(AtomicBool::new(false)),
            make_walk(),
        );
        let path = PathBuf::from("/music");
        service.request(path.clone());
        assert!(service.is_scanning(&path));
    }

    #[test]
    fn scan_service_cancel_clears_active() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (service, _worker) = ScanService::new(
            Box::new(MockReader),
            Box::new(MockQueries {
                known: Arc::new(Mutex::new(HashSet::new())),
            }),
            Box::new(MockMutations),
            Arc::clone(&cancel),
            Arc::new(AtomicBool::new(false)),
            make_walk(),
        );
        let path = PathBuf::from("/music");
        service.request(path.clone());
        service.cancel();
        assert!(cancel.load(Ordering::Relaxed));
    }
}
