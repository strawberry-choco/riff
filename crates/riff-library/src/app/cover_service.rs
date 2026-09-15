//! The Cover Service: resolves cover art for Tracks off the UI thread,
//! owning request deduplication and the bounded negative cache (ADR 0006).

use crate::app::cover_resolver::CoverResolver;
use crate::infra::ports::{CoverImage, CoverLoader, MetadataReader};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use riff_persistence::track::TrackId;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Max entries per cover cache (the UI's positive texture cache and this
/// module's negative cache alike); the oldest entries are evicted LRU-style
/// beyond this cap.
pub const COVER_CACHE_CAP: usize = 50;

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
pub trait Covers: Send {
    fn request(&self, track_id: TrackId, path: PathBuf);
    fn poll(&self) -> Vec<(TrackId, Option<CoverImage>)>;
}

/// Whether embedded artwork may be read, answered fresh for every
/// resolution so the Settings Library pane's "Read embedded artwork"
/// toggle applies immediately (design-handoff issue 12). The composition
/// root binds the Application Store's scalar settings behind this closure.
pub type CoverPolicy = Box<dyn Fn() -> bool + Send>;

/// Front-end of the Cover Service.
pub struct CoverService {
    request_tx: Sender<(TrackId, PathBuf)>,
    result_rx: Receiver<(TrackId, Option<CoverImage>)>,
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
    fn request(&self, track_id: TrackId, path: PathBuf) {
        let _ = self.request_tx.send((track_id, path));
    }

    fn poll(&self) -> Vec<(TrackId, Option<CoverImage>)> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
            results.push(result);
        }
        results
    }
}

/// Blocking back-end of the Cover Service.
pub struct CoverWorker {
    request_rx: Receiver<(TrackId, PathBuf)>,
    result_tx: Sender<(TrackId, Option<CoverImage>)>,
    resolver: CoverResolver,
    /// The read-embedded-artwork policy, evaluated fresh per resolution.
    policy: CoverPolicy,
    backlog: VecDeque<(TrackId, PathBuf)>,
    pending: HashSet<TrackId>,
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
        while let Some((track_id, path)) = self.next_accepted() {
            let read_embedded = (self.policy)();
            let result = match self.resolver.resolve(&path, read_embedded) {
                Ok(resolved) => resolved,
                Err(e) => {
                    tracing::warn!("Cover resolution failed for {:?}: {}", path, e);
                    None
                }
            };
            self.pending.remove(&track_id);
            if result.is_none() {
                let _ = lru_insert(&mut self.negative, track_id.clone(), COVER_CACHE_CAP);
            }
            self.absorb_raced(&track_id);
            let _ = self.result_tx.send((track_id, result));
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
    fn next_accepted(&mut self) -> Option<(TrackId, PathBuf)> {
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
            let suppressed = self.pending.contains(&candidate.0)
                || self.negative.iter().any(|cached| cached == &candidate.0);
            if suppressed {
                continue;
            }
            self.pending.insert(candidate.0.clone());
            return Some(candidate);
        }
    }

    fn absorb_raced(&mut self, just_resolved: &TrackId) {
        while let Ok((track_id, path)) = self.request_rx.try_recv() {
            if &track_id != just_resolved {
                self.backlog.push_back((track_id, path));
            }
        }
    }
}
