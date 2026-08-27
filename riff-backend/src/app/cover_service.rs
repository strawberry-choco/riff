//! The Cover Service: resolves cover art for Tracks off the UI thread,
//! owning request deduplication and the bounded negative cache (ADR 0006).
//!
//! The module splits into two halves wired over unbounded crossbeam channels,
//! exactly like the Tag Edit Service:
//!
//! - [`CoverService`] is the front-end seam the UI holds as
//!   `Box<dyn Covers>`: it fires resolve requests and polls drained results.
//!   It never blocks and never touches disk.
//! - [`CoverWorker`] is the blocking back-end that owns the resolver chain
//!   ([`CoverResolver`](crate::app::cover_resolver::CoverResolver) over the
//!   Metadata Reader + Cover Loader ports), collapses repeated requests for
//!   the same Track into one resolve, and negative-caches artless Tracks
//!   behind a bounded LRU so they stop triggering disk I/O until eviction
//!   makes an eventual retry possible.
//!
//! Threading follows the Audio Engine pattern: nothing here spawns threads.
//! The composition root constructs both halves and runs [`CoverWorker::run`]
//! on its own dedicated thread; the UI keeps only the boxed front-end handle
//! and the egui-bound work (rgba→texture conversion, texture LRU).

use crate::app::cover_resolver::CoverResolver;
use crate::app::traits::{CoverImage, CoverLoader, MetadataReader};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use crate::domain::TrackId;

/// Max entries per cover cache (the UI's positive texture cache and this
/// module's negative cache alike); the oldest entries are evicted LRU-style
/// beyond this cap. Moved here from the UI layer so both caches share ONE
/// capacity discipline instead of two constants drifting apart.
pub const COVER_CACHE_CAP: usize = 50;

/// Insert `key` at the most-recently-used end of an LRU key list: an already
/// present entry is moved to the end (no duplicates), and keys evicted beyond
/// `cap` are returned so the caller can drop their cached payloads. Moved
/// here from the UI layer (generalized over the key type) together with
/// [`COVER_CACHE_CAP`] so the eviction discipline has one implementation.
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
/// fire-and-forget requests, poll drained results. Implemented by the real
/// front-end over channels and by mocks in tests; injected boxed.
pub trait Covers: Send {
    /// Ask for the cover of one Track. Never blocks; duplicates of a track
    /// whose resolve is pending/in-flight are dropped, as are requests for
    /// tracks known to be artless (until their negative-cache entry is
    /// evicted).
    fn request(&self, track_id: TrackId, path: PathBuf);

    /// Drain every completed resolution, oldest first. Non-blocking: an
    /// empty Vec means nothing has finished yet.
    fn poll(&self) -> Vec<(TrackId, Option<CoverImage>)>;
}

/// Front-end of the Cover Service: what the UI holds (boxed as
/// `Box<dyn Covers>`). Pairs with a [`CoverWorker`] over channels.
pub struct CoverService {
    request_tx: Sender<(TrackId, PathBuf)>,
    result_rx: Receiver<(TrackId, Option<CoverImage>)>,
}

impl CoverService {
    /// Wire a matched ([`CoverService`], [`CoverWorker`]) pair over fresh
    /// unbounded channels. The resolver chain is built here from its two
    /// ports — the service owns it. Box the returned service as the UI's
    /// `Box<dyn Covers>` handle and run the worker on its own thread
    /// (`worker.run()`), exactly like the Audio Engine.
    #[must_use]
    pub fn new(
        metadata_reader: Box<dyn MetadataReader>,
        cover_loader: Box<dyn CoverLoader>,
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
                backlog: VecDeque::new(),
                pending: HashSet::new(),
                negative: Vec::new(),
            },
        )
    }
}

impl Covers for CoverService {
    fn request(&self, track_id: TrackId, path: PathBuf) {
        // A send fails only when the worker half is gone (shutting down);
        // dropping the request mirrors every other fire-and-forget sender.
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

/// Blocking back-end of the Cover Service: resolves requested covers one at
/// a time, in submission order, and publishes each result keyed by Track
/// identity. Spawns nothing; the composition root runs it on the dedicated
/// cover thread.
pub struct CoverWorker {
    request_rx: Receiver<(TrackId, PathBuf)>,
    result_tx: Sender<(TrackId, Option<CoverImage>)>,
    resolver: CoverResolver,
    /// Accepted-but-unresolved requests beyond the one being processed, in
    /// arrival order.
    backlog: VecDeque<(TrackId, PathBuf)>,
    /// Tracks with an accepted, unfinished request (queued or resolving):
    /// repeats of these are dropped at intake (request deduplication).
    pending: HashSet<TrackId>,
    /// Artless tracks, most-recently-negative last (bounded LRU): requests
    /// for these are dropped at intake until eviction forgets them.
    negative: Vec<TrackId>,
}

impl CoverWorker {
    /// Block resolving requests until every [`CoverService`] front end is
    /// dropped. Spawns nothing; run this on the dedicated cover thread.
    pub fn run(mut self) {
        while let Some((track_id, path)) = self.next_accepted() {
            let result = match self.resolver.resolve(&path) {
                Ok(resolved) => resolved,
                Err(e) => {
                    // Same collapse as the former inline UI thread: a failed
                    // resolve reports "no cover" for this round.
                    tracing::warn!("Cover resolution failed for {:?}: {}", path, e);
                    None
                }
            };
            // Bookkeeping BEFORE publishing so the state is fully settled by
            // the time the result is observable through `poll`.
            self.pending.remove(&track_id);
            if result.is_none() {
                let _ = lru_insert(&mut self.negative, track_id.clone(), COVER_CACHE_CAP);
            }
            // Requests for this track that raced with the resolve collapse
            // into the single outcome about to be published.
            self.absorb_raced(&track_id);
            let _ = self.result_tx.send((track_id, result));
        }
    }

    /// Intake: serve the oldest acceptable request, dropping duplicates of
    /// pending/in-flight tracks and of negative-cached (artless) tracks
    /// before they can trigger any resolver I/O.
    fn next_accepted(&mut self) -> Option<(TrackId, PathBuf)> {
        loop {
            // Refill the backlog from the channel; block only when it has
            // run dry.
            if self.backlog.is_empty()
                && let Ok(request) = self.request_rx.recv()
            {
                self.backlog.push_back(request);
            }
            // Slurp whatever else is already queued (non-blocking).
            while let Ok(request) = self.request_rx.try_recv() {
                self.backlog.push_back(request);
            }

            let candidate = self.backlog.pop_front()?;
            let suppressed = self.pending.contains(&candidate.0)
                || self.negative.iter().any(|cached| cached == &candidate.0);
            if suppressed {
                continue; // no disk I/O, no result
            }
            self.pending.insert(candidate.0.clone());
            return Some(candidate);
        }
    }

    /// Drain requests that arrived while `just_resolved` was in flight:
    /// repeats of it are dropped (their answer is the outcome being
    /// published), everything else waits in the backlog.
    fn absorb_raced(&mut self, just_resolved: &TrackId) {
        while let Ok((track_id, path)) = self.request_rx.try_recv() {
            if &track_id != just_resolved {
                self.backlog.push_back((track_id, path));
            }
        }
    }
}
