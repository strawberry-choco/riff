//! The **Cover Cache** decision half, ported from the frozen egui frontend
//! (`crates/riff-gui/src/ui/cover_cache.rs`, with the key type and the cap
//! from `ui/artwork.rs`). The plan's N3 wording said to *move* those items
//! out of `artwork.rs` and reuse them; that is superseded here — `riff-gui`
//! is frozen to bug fixes (ADR 0022) and this crate may not name a
//! `riff-gui` edge (the architecture's dependency graph), so the half is
//! PORTED instead: same facts, same caps, no egui type.
//!
//! What the module knows, and nothing else: which Covers are **wanted**, at
//! **which size**, **which are in flight**, and **which have arrived**
//! (`CONTEXT.md`'s term, implemented as written, including the clause that
//! it holds no picture). A decoded Cover is pixels on their way to the View;
//! here the View is the nsh image cache: [`arrival_image`] wraps a decoded
//! cover as an [`ImageSrc`] the caller files into its [`CoverMap`](crate::model::CoverMap), and nsh
//! caches the decoded image by source. The egui View half's texture LRU and
//! byte budget have no direct port — nsh's image cache purges under memory
//! pressure but carries no numeric bound (P0 corrections 10 and 24), so the
//! eviction policy that bounds the map is N6's to build and measure. The
//! decision half's own caps — [`COVER_IN_FLIGHT_CAP`], and the negative
//! verdict path the Cover Service keeps — are kept as-is.
//!
//! Assertable with no engine: see `tests/cover_cache_tests.rs` beside this
//! crate, ported from `crates/riff-gui/tests/cover_cache_tests.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use nsh_core::element::ImageSrc;
use riff_backend::app::cover_service::{Covers, lru_insert};
use riff_backend::app::traits::{DecodedCover, RequestedSize};
use riff_backend::domain::TrackId;

/// Key of the cover cache: one artwork identity at one requested box.
///
/// The identity is a track path — albums and artists resolve through their
/// first track — so it is a `String` rather than a `TrackId`. The size is
/// part of the key because a hero upload and a thumbnail upload of the same
/// track are different pixels, and neither may stand in for the other.
pub type CoverCacheKey = (String, u32, u32);

/// The cache key for one artwork identity at one requested box.
#[must_use]
pub fn cover_cache_key(identity: &str, size: RequestedSize) -> CoverCacheKey {
    (identity.to_string(), size.width, size.height)
}

/// How many requests may be outstanding at once; the oldest ask is dropped
/// LRU-style beyond this cap. Ported unchanged from the egui crate.
pub const COVER_IN_FLIGHT_CAP: usize = 512;

/// The canonical display boxes, one per kind of surface, ported from the
/// egui crate's `artwork.rs` unchanged.
///
/// Surfaces ask for the nearest of these rather than their own exact pixel
/// size: every distinct `(identity, size)` pair is a separate worker job and
/// a separate cache entry, so an unbounded set of boxes would multiply both.
/// The View half's byte budget that bounded the egui map has no port here —
/// nsh's image cache carries no numeric bound, and the eviction policy that
/// replaces it is N6's to build and measure (P0 corrections 10 and 24).
pub const COVER_THUMB: RequestedSize = RequestedSize {
    width: 56,
    height: 56,
};
/// The card box (a grid tile), as the egui store defines it.
pub const COVER_CARD: RequestedSize = RequestedSize {
    width: 200,
    height: 200,
};
/// The hero box (Now Playing), as the egui store defines it.
pub const COVER_HERO: RequestedSize = RequestedSize {
    width: 512,
    height: 512,
};

/// One Cover that has arrived, on its way to the View's map.
///
/// The key is the composite the View writes the entry under, so the View
/// never re-derives it, and the pixels are the worker's decode handed over
/// untouched — wrapping them as an [`ImageSrc`] is the View's work, not this
/// one's.
pub struct CoverArrival {
    /// The `(identity, box)` the View files this Cover under.
    pub key: CoverCacheKey,
    /// RGBA8 exactly as the cover worker decoded it.
    pub cover: DecodedCover,
}

/// Wrap one arrival's pixels as an nsh image source. The buffer is held as
/// it is (`Vec<u8>`, not copied) and nsh caches the decoded image; the
/// caches drop it once no view shows it (reference counting, not a bound —
/// see the module doc for what that leaves for N6).
#[must_use]
pub fn arrival_image(cover: DecodedCover) -> ImageSrc {
    ImageSrc::rgba(cover.width, cover.height, cover.rgba)
}

/// What the application knows about a Cover, with no picture in it.
///
/// The bridge model owns one (`UiModel::covers`) and reaches it through its
/// methods: `want_track` / `want_folder` decide whether a request goes out,
/// `settle` answers what arrived, and the `forget_*` calls are the View
/// half's report of what it dropped. No call site reaches inside.
pub struct CoverCache {
    /// The `(identity, box)` pairs whose Cover has arrived — the View's
    /// entries, remembered here so a repaint of a settled row asks for
    /// nothing. An **artless** answer records nothing: the row stays a miss
    /// at that box and the service's negative verdict is what answers the
    /// re-ask.
    ///
    /// Bounded by construction rather than by a constant of its own: an
    /// entry enters here only when the View files it, and leaves when the
    /// View reports it dropped.
    arrived: HashSet<CoverCacheKey>,
    /// Cover requests this session has sent and not yet been answered,
    /// keyed exactly as `arrived` is. Without it every repaint of a row
    /// whose art has not landed re-enqueues a request onto an unbounded
    /// channel — invisible when the answer comes back in one heartbeat, and
    /// a real allocation plus a `PathBuf` and `String` clone per frame per
    /// row when it does not.
    in_flight: HashSet<CoverCacheKey>,
    /// The LRU order of [`Self::in_flight`].
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

    /// The outstanding markers. Read-only: a caller can see what is in
    /// flight but cannot invent a marker the cap would then have to bound.
    #[must_use]
    pub fn in_flight(&self) -> &HashSet<CoverCacheKey> {
        &self.in_flight
    }

    /// The arrivals the View is currently holding. Read-only, and the
    /// mirror of the View's map rather than a second opinion about it.
    #[must_use]
    pub fn arrived(&self) -> &HashSet<CoverCacheKey> {
        &self.arrived
    }

    /// Ask for one Track's Cover at `size`, unless this `(identity, box)`
    /// has already arrived or is already in flight. Returns whether the
    /// request went out.
    ///
    /// The cache check is made HERE rather than by the caller because the
    /// key it checks is the same composite the entry is written under: a
    /// Track cached at hero size is still a **miss** at thumbnail size.
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

    /// The Folders tree's half of the same responsibility: a folder row
    /// wants the cover art of the directory it *is*. Returns the key the
    /// View already holds that art under, or `None` when this caller must
    /// keep its glyph — on the first ask, on every repaint while the answer
    /// is outstanding, and for a directory that turns out to have no cover
    /// of its own. The miss does not fall back to any artless-track
    /// placeholder: a folder with no cover of its own is an ordinary
    /// folder.
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

    /// Drain the Cover Service, drop the marker for every delivered answer,
    /// and hand back the Covers that carry pixels.
    ///
    /// Every delivered answer is terminal, `None` included: an artless row
    /// that kept its marker would never ask again, and a row whose art
    /// appears later would stay blank for the session. The markers are
    /// therefore cleared before the artless answers are filtered out.
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

    /// The View half dropped these entries — N6's eviction made room and
    /// took them — so they are no longer arrived, and a row that asks again
    /// gets a request.
    ///
    /// Reported by the only thing that can report it: the map has one
    /// writer, and it is not this module.
    pub fn forget(&mut self, keys: impl IntoIterator<Item = CoverCacheKey>) {
        for key in keys {
            self.arrived.remove(&key);
        }
    }

    /// The View half was emptied — Settings → Library → "Clear Thumbnail
    /// cache" settled, so nothing is arrived any more.
    ///
    /// Deliberately not a `forget_in_flight` as well: a request still on its
    /// way is still answered, and its arrival is exactly what rebuilds the
    /// entry the user just asked to be re-derived.
    pub fn forget_arrivals(&mut self) {
        self.arrived.clear();
    }

    /// Drop every outstanding marker, so every row asks again on the next
    /// repaint. The artwork-policy toggle's other half: a request that went
    /// out under the old policy has an answer that is about to be wrong for
    /// the new one, and keeping its marker would suppress the re-ask that
    /// the eviction exists to cause.
    pub fn forget_in_flight(&mut self) {
        self.in_flight.clear();
        self.in_flight_keys.clear();
    }

    /// Record that a `(identity, box)` has an outstanding request, keeping
    /// the marker set bounded by the same LRU discipline as the View's map.
    ///
    /// The marker is dropped by [`Self::settle`] when the answer arrives, so
    /// overflowing the cap can only ever forget a *recent* ask — and the
    /// cost of forgetting is one re-request, not a wrong image. Both
    /// structures are updated together because a key left in the list but
    /// not the set would be re-marked forever, and one left in the set but
    /// not the list could never be evicted.
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
