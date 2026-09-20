//! The Cover Service: resolves cover art for Tracks off the UI thread,
//! owning request deduplication and the bounded negative cache (ADR 0006).

use crate::app::cover_resolver::CoverResolver;
use crate::infra::ports::{CoverLoader, DecodedCover, MetadataReader, RequestedSize};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use riff_persistence::track::TrackId;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Max entries per cover cache (the UI's positive texture cache and this
/// module's negative cache alike); the oldest entries are evicted LRU-style
/// beyond this cap. Sized to hold comfortably more than a full browser
/// column's visible window, so fast scrolling never thrashes the cache
/// (the artists-root idle-CPU fix).
pub const COVER_CACHE_CAP: usize = 200;

/// How long [`CoverWorker::next_accepted`] waits for the next request before
/// it looks at its stop flag. The worker is idle almost all the time, so the
/// wait must expire: with a blocking `recv` the thread could never be asked
/// to stop without dropping every [`CoverService`] front end first.
const WORKER_POLL: Duration = Duration::from_millis(10);

/// Insert `key` at the most-recently-used end of an LRU key list: an already
/// present entry is moved to the end (no duplicates), and keys evicted beyond
/// `cap` are returned so the caller can drop their cached payloads.
pub fn lru_insert<K: PartialEq>(keys: &mut Vec<K>, key: K, cap: usize) -> Vec<K> {
    keys.retain(|k| *k != key);
    keys.push(key);
    let mut evicted = Vec::new();
    while keys.len() > cap {
        evicted.push(keys.remove(0));
    }
    evicted
}

/// Seam between the UI and the background cover-resolution flow (ADR 0006):
/// fire-and-forget requests, poll drained results.
///
/// A request names the display box it wants, and the result reports it back so
/// the caller can file the pixels under the right key. The worker keeps no
/// decoded cache between requests, so the same subject at two sizes is two
/// independent jobs — a re-read and a re-decode each.
pub trait Covers: Send {
    fn request(&self, track_id: TrackId, path: PathBuf, size: RequestedSize);
    /// Ask for the cover art of a directory *itself* — the Folders tree's row
    /// tile. The identity is the directory path, and the resolution never
    /// opens an audio file, so the Settings "Read embedded artwork" toggle does
    /// not gate this request: it governs tag reads, and a plain image file has
    /// no tags.
    fn request_folder(&self, folder: &Path, size: RequestedSize);
    fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)>;
}

/// Whether embedded artwork may be read, answered fresh for every
/// resolution so the Settings Library pane's "Read embedded artwork"
/// toggle applies immediately (design-handoff issue 12). The composition
/// root binds the Application Store's scalar settings behind this closure.
pub type CoverPolicy = Box<dyn Fn() -> bool + Send>;

/// What a request is for: a track's own file, or a directory's own cover file.
/// The two are resolved by different rules — only a track can carry embedded
/// art — but they share one worker, one in-flight set and one negative cache.
enum CoverSubject {
    Track(PathBuf),
    Folder(PathBuf),
}

/// A request accepted by the worker: what it is for, where to read it, and the
/// box it is wanted at.
type Request = (TrackId, CoverSubject, RequestedSize);

/// A delivered result: the subject's identity, the box it was asked for, and
/// the pixels.
type Resolution = (TrackId, RequestedSize, Option<DecodedCover>);

/// Front-end of the Cover Service.
pub struct CoverService {
    request_tx: Sender<Request>,
    result_rx: Receiver<Resolution>,
}

impl CoverService {
    #[must_use]
    pub fn new(
        metadata_reader: Box<dyn MetadataReader>,
        cover_loader: Box<dyn CoverLoader>,
        policy: CoverPolicy,
        stop: Arc<AtomicBool>,
    ) -> (Self, CoverWorker) {
        let (request_tx, request_rx) = unbounded();
        let (result_tx, result_rx) = unbounded();
        (
            Self {
                request_tx,
                result_rx,
            },
            CoverWorker {
                request_rx,
                result_tx,
                resolver: CoverResolver::new(metadata_reader, cover_loader),
                policy,
                backlog: VecDeque::new(),
                pending: HashSet::new(),
                negative: Vec::new(),
                stop,
            },
        )
    }
}

impl Covers for CoverService {
    fn request(&self, track_id: TrackId, path: PathBuf, size: RequestedSize) {
        let _ = self
            .request_tx
            .send((track_id, CoverSubject::Track(path), size));
    }

    fn request_folder(&self, folder: &Path, size: RequestedSize) {
        let folder = folder.to_path_buf();
        let identity = TrackId::from_path(&folder);
        let _ = self
            .request_tx
            .send((identity, CoverSubject::Folder(folder), size));
    }

    fn poll(&self) -> Vec<Resolution> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
            results.push(result);
        }
        results
    }
}

/// Blocking back-end of the Cover Service.
pub struct CoverWorker {
    request_rx: Receiver<Request>,
    result_tx: Sender<Resolution>,
    resolver: CoverResolver,
    /// The read-embedded-artwork policy, evaluated fresh per resolution.
    policy: CoverPolicy,
    backlog: VecDeque<Request>,
    /// In-flight jobs, keyed by identity *and* size: the same subject wanted at
    /// two sizes is two jobs, because nothing decoded is retained between them.
    pending: HashSet<(TrackId, RequestedSize)>,
    /// Subjects resolved as having no artwork. Keyed by identity alone: an
    /// artless subject is artless at every size, so one entry suppresses all of
    /// them.
    ///
    /// Tracks and folders share this set safely because their identities can
    /// never be equal: a track identity is always a file's path and a folder
    /// identity always a directory's, and one filesystem holds at most one of
    /// those under any given name.
    negative: Vec<TrackId>,
    /// Cooperative stop request from the Composition Root; see
    /// [`Self::next_accepted`].
    stop: Arc<AtomicBool>,
}

impl CoverWorker {
    /// Resolve covers until the request channel closes or the Composition
    /// Root asks the worker to stop. Spawns nothing; run this on the
    /// dedicated cover thread.
    pub fn run(mut self) {
        while let Some((identity, subject, size)) = self.next_accepted() {
            // The embedded-art policy is consulted on the track arm only: a
            // directory's cover is a plain image file, so there is no tag read
            // for the toggle to gate.
            let (result, path) = match subject {
                CoverSubject::Track(path) => {
                    let read_embedded = (self.policy)();
                    let result = self.resolver.resolve(&path, read_embedded, size);
                    (result, path)
                }
                CoverSubject::Folder(path) => {
                    let result = self.resolver.resolve_folder(&path, size);
                    (result, path)
                }
            };
            let result = match result {
                Ok(resolved) => resolved,
                Err(e) => {
                    tracing::warn!("Cover resolution failed for {:?}: {}", path, e);
                    None
                }
            };
            self.pending.remove(&(identity.clone(), size));
            if result.is_none() {
                let _ = lru_insert(&mut self.negative, identity.clone(), COVER_CACHE_CAP);
            }
            self.absorb_raced((&identity, size));
            let _ = self.result_tx.send((identity, size, result));
        }
    }

    /// The next request that survives dedup, or `None` when the worker has
    /// been asked to stop or the request channel closed.
    ///
    /// The wait for a first request polls on [`WORKER_POLL`] rather than
    /// blocking: an idle worker is the normal case, and the expiry is the
    /// only moment it can notice a stop request. A tick that finds nothing
    /// leaves the backlog, the pending set, and the negative cache exactly as
    /// they were — the dedup semantics below are unchanged.
    fn next_accepted(&mut self) -> Option<Request> {
        loop {
            if self.backlog.is_empty() {
                match self.request_rx.recv_timeout(WORKER_POLL) {
                    Ok(request) => self.backlog.push_back(request),
                    Err(RecvTimeoutError::Timeout) => {
                        if self.stop.load(Ordering::Relaxed) {
                            return None;
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return None,
                }
            }
            while let Ok(request) = self.request_rx.try_recv() {
                self.backlog.push_back(request);
            }

            let candidate = self.backlog.pop_front()?;
            let suppressed = self.pending.contains(&(candidate.0.clone(), candidate.2))
                || self.negative.iter().any(|cached| cached == &candidate.0);
            if suppressed {
                continue;
            }
            self.pending.insert((candidate.0.clone(), candidate.2));
            return Some(candidate);
        }
    }

    /// Drop requests the worker raced with the resolve that just completed: an
    /// identical `(identity, size)` duplicate was already served by it. A
    /// different size for the same subject is a separate job and stays queued.
    fn absorb_raced(&mut self, just_resolved: (&TrackId, RequestedSize)) {
        while let Ok(request) = self.request_rx.try_recv() {
            if (&request.0, request.2) != just_resolved {
                self.backlog.push_back(request);
            }
        }
    }
}
