//! The **Cover Cache** — what the application knows about a Cover.
//!
//! Four facts, and nothing else: which Covers are **wanted**, **at which
//! size**, **which are in flight**, and **which have arrived**. That is the
//! term `CONTEXT.md` defines, implemented as written — *including* the clause
//! that it holds no picture. A decoded Cover is pixels on their way to the
//! View, and [`CoverArrival`] carries them exactly as far as the View's
//! doorway; this module never becomes a texture, and it names no egui type at
//! all. The texture map, its LRU order and the byte budget that bounds them
//! belong to the View half in [`crate::ui::artwork`], and the two are separate
//! facts which neither answers for the other.
//!
//! Why the split is here rather than inside the frame: a row that repaints
//! before its answer lands re-asks every frame unless something remembers the
//! ask, and six render sites each re-derived that answer by hand — the
//! composite key, the marker set, the marker set's own bound, the request — out
//! of the four fields they had to be handed alongside the texture map. Owning
//! it here makes "a Track cached at hero size is still a MISS at thumbnail
//! size" a property of one module instead of a fact every caller has to know,
//! and it is assertable with no `egui::Context` at all (see
//! `crates/riff-gui/tests/cover_cache_tests.rs`).
//!
//! The one coupling the split cannot avoid runs the other way: only the View
//! half writes the texture map, so only it can know which arrivals the count
//! cap and the byte budget dropped — and it reports them through
//! [`CoverCache::forget`]. Without that report the two halves would drift, and
//! a row whose art was evicted would wait forever for an answer nobody is going
//! to send.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use riff_backend::app::cover_service::{Covers, lru_insert};
use riff_backend::app::traits::{DecodedCover, RequestedSize};
use riff_backend::domain::TrackId;

use super::artwork::{COVER_IN_FLIGHT_CAP, CoverCacheKey, cover_cache_key};

/// One Cover that has arrived, on its way to the View's texture map.
///
/// The key is the composite the View writes the entry under, so the View never
/// re-derives it, and the pixels are the worker's decode handed over
/// untouched — wrapping and uploading them is the View's work, not this one's.
pub struct CoverArrival {
    /// The `(identity, box)` the View files this Cover under.
    pub key: CoverCacheKey,
    /// RGBA8 exactly as the cover worker decoded it.
    pub cover: DecodedCover,
}

/// What the application knows about a Cover, with no picture in it.
///
/// Owned by the frame (as `RiffApp::cover_cache`) and reached through its
/// methods: `want_track` / `want_folder` decide whether a request goes out,
/// `settle` answers what arrived, and the `forget_*` calls are the View half's
/// report of what it dropped. No call site reaches inside.
pub struct CoverCache {
    /// The `(identity, box)` pairs whose Cover has arrived — the View's
    /// entries, remembered here so a repaint of a settled row asks for
    /// nothing. An **artless** answer records nothing: the row stays a miss at
    /// that box and the service's negative verdict is what answers the re-ask.
    ///
    /// Bounded by construction rather than by a constant of its own: an entry
    /// enters here only when the View files it, and leaves when the View
    /// reports it dropped.
    arrived: HashSet<CoverCacheKey>,
    /// Cover requests this frame has sent and not yet been answered, keyed
    /// exactly as `arrived` is. Without it every repaint of a row whose art has
    /// not landed re-enqueues a request onto an unbounded channel — invisible
    /// when the answer comes back in one heartbeat, and a real allocation plus a
    /// `PathBuf` and `String` clone per frame per row when it does not.
    in_flight: HashSet<CoverCacheKey>,
    /// The LRU order of [`Self::in_flight`], kept beside it the same way the
    /// View's `cover_lru_keys` tracks the texture map.
    in_flight_keys: Vec<CoverCacheKey>,
}

impl CoverCache {
    /// A cache that knows nothing: every Cover is yet to be wanted.
    pub fn new() -> Self {
        Self {
            arrived: HashSet::new(),
            in_flight: HashSet::new(),
            in_flight_keys: Vec::new(),
        }
    }

    /// The outstanding markers. Read-only: a caller can see what is in flight
    /// but cannot invent a marker the cap would then have to bound.
    #[must_use]
    pub fn in_flight(&self) -> &HashSet<CoverCacheKey> {
        &self.in_flight
    }

    /// The arrivals the View is currently holding. Read-only, and the mirror of
    /// the View's texture map rather than a second opinion about it.
    #[must_use]
    pub fn arrived(&self) -> &HashSet<CoverCacheKey> {
        &self.arrived
    }

    /// Ask for one Track's Cover at `size`, unless this `(identity, box)` has
    /// already arrived or is already in flight. Returns whether the request
    /// went out.
    ///
    /// The cache check is made HERE rather than by the caller because the key
    /// it checks is the same composite the entry is written under: a Track
    /// cached at hero size is still a **miss** at thumbnail size. Answering it
    /// needs the arrived set and the key space together, and those are the
    /// cache's own facts — a render site knows which box it is painting, not
    /// what the other boxes hold.
    pub fn want_track(
        &mut self,
        covers: &dyn Covers,
        track_id: TrackId,
        path: PathBuf,
        size: RequestedSize,
    ) -> bool {
        let key = cover_cache_key(&track_id.0, size);
        if self.arrived.contains(&key) || self.in_flight.contains(&key) {
            return false;
        }
        self.mark_in_flight(key);
        covers.request(track_id, path, size);
        true
    }

    /// The Folders tree's half of the same responsibility: a folder row wants
    /// the cover art of the directory it *is*, and gets it in place of the
    /// folder glyph. Returns the key the View already holds that art under, or
    /// `None` when this frame must keep the glyph — which is the case on the
    /// first ask, on every repaint while the answer is outstanding, and for a
    /// directory that turns out to have no cover of its own.
    ///
    /// The miss does **not** fall back to the generated music-note placeholder
    /// the artless Track rows get — a folder with no cover of its own is an
    /// ordinary folder, not an artless album. Which is why this asks for
    /// itself, against the same marker set, and reads the arrival back: the
    /// Track callers resolve their texture through the View half either way,
    /// while a folder row has to be told whether art exists before it decides
    /// what to paint.
    ///
    /// The identity is the directory path, and the service files the decoded
    /// result under exactly that key, so the arrival needs no further plumbing.
    pub fn want_folder(
        &mut self,
        covers: &dyn Covers,
        folder: &Path,
        size: RequestedSize,
    ) -> Option<CoverCacheKey> {
        let key = cover_cache_key(&folder.to_string_lossy(), size);
        if self.arrived.contains(&key) {
            return Some(key);
        }
        if !self.in_flight.contains(&key) {
            self.mark_in_flight(key);
            covers.request_folder(folder, size);
        }
        None
    }

    /// Drain the Cover Service, drop the marker for every delivered answer, and
    /// hand back the Covers that carry pixels.
    ///
    /// Every delivered answer is terminal, `None` included: an artless row that
    /// kept its marker would never ask again, and a row whose art appears later
    /// would stay blank for the session. The markers are therefore cleared
    /// before the artless answers are filtered out, for exactly that reason.
    pub fn settle(&mut self, covers: &dyn Covers) -> Vec<CoverArrival> {
        let mut arrivals = Vec::new();
        for (track_id, size, cover) in covers.poll() {
            let key = cover_cache_key(&track_id.0, size);
            self.unmark_in_flight(&key);
            let Some(cover) = cover else {
                continue; // artless: the service negative-caches it
            };
            self.arrived.insert(key.clone());
            arrivals.push(CoverArrival { key, cover });
        }
        arrivals
    }

    /// The View half dropped these entries — the count cap or the byte budget
    /// made room and took them — so they are no longer arrived, and a row that
    /// asks again gets a request.
    ///
    /// Reported by the only thing that can report it: the texture map has one
    /// writer, and it is not this module.
    pub fn forget(&mut self, keys: impl IntoIterator<Item = CoverCacheKey>) {
        for key in keys {
            self.arrived.remove(&key);
        }
    }

    /// The View half was emptied — Settings → Library → "Clear Thumbnail cache"
    /// settled, so nothing is arrived any more.
    ///
    /// Deliberately not a `forget_in_flight` as well: a request still on its
    /// way is still answered, and its arrival is exactly what rebuilds the
    /// entry the user just asked to be re-derived.
    pub fn forget_arrivals(&mut self) {
        self.arrived.clear();
    }

    /// Drop every outstanding marker, so every row asks again on the next
    /// repaint. The artwork-policy toggle's other half: a request that went out
    /// under the old policy has an answer that is about to be wrong for the new
    /// one, and keeping its marker would suppress the re-ask that the eviction
    /// exists to cause.
    pub fn forget_in_flight(&mut self) {
        self.in_flight.clear();
        self.in_flight_keys.clear();
    }

    /// Record that a `(identity, box)` has an outstanding request, keeping the
    /// marker set bounded by the same LRU discipline as the View's texture map.
    ///
    /// The marker is dropped by [`Self::settle`] when the answer arrives, so
    /// overflowing the cap can only ever forget a *recent* ask — and the cost
    /// of forgetting is one re-request, not a wrong image. Both structures are
    /// updated together because a key left in the list but not the set would be
    /// re-marked forever, and one left in the set but not the list could never
    /// be evicted.
    fn mark_in_flight(&mut self, key: CoverCacheKey) {
        for evicted in lru_insert(&mut self.in_flight_keys, key.clone(), COVER_IN_FLIGHT_CAP) {
            self.in_flight.remove(&evicted);
        }
        self.in_flight.insert(key);
    }

    /// The other half of [`Self::mark_in_flight`]'s pairing rule: the answer
    /// arrived, so the key leaves the set and the list together.
    fn unmark_in_flight(&mut self, key: &CoverCacheKey) {
        self.in_flight.remove(key);
        self.in_flight_keys.retain(|existing| existing != key);
    }
}

impl Default for CoverCache {
    fn default() -> Self {
        Self::new()
    }
}
