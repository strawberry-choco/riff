//! The Session Projection serving one Custom Smart Playlist's rows and total.
//!
//! A Custom Smart Playlist is a query, never a stored row list, and the UI
//! reads its bounded windows plus a count through this projection — the
//! existing read discipline (ADR 0002), applied to the bounded
//! `custom_smart_playlist_page` read. The built-in lists' smart projection
//! cannot serve it: that port read hands back a whole unbounded `Vec` per
//! kind, the one read this feature cannot use because there is no per-Rule
//! cap. The window size, the cache bound, and the fetch-first-swap-later
//! degradation are owned by [`WindowedListProjection`] — the ONE house —
//! and are inherited here, never restated.
//!
//! ## Two generations, one stamp
//!
//! A name or tree write moves the **playlist** generation; a Track write
//! moves the **library** generation. ADR 0002's 2026-09-23 amendment records
//! a second two-counter projection as a decision rather than growing
//! `GenerationCache::level` into holding two stamps, and the decided shape
//! is one stamp per cache with the second epoch **baked into the query
//! signature**: the window cache keys on the library generation (its own
//! stamp) and carries the playlist epoch inside
//! [`CustomListKey::playlist_epoch`] — the same hand-off the playlist
//! projection makes with its bundle key. Either counter moving therefore
//! drops rows for free: the library stamp moving drops them as staleness,
//! the playlist epoch moving drops them as a key change.
//!
//! ## The cache key is the query, never a row id
//!
//! Rule and Rule Box ids are store-generated, opaque, and never handed to
//! the frontend, so the key holds the compiled [`RuleTree`]'s **shape**
//! instead — two different trees with the same number of Rules are
//! provably different keys, and nothing here can name a database row.
//!
//! ## Unreadable Rules answer with a reason, not a query
//!
//! [`RuleTree::from_smart_playlist_tree`] is the ONE bridge between the
//! read type's closed enums and the stored-id spelling a bounded read
//! compiles; a non-empty unreadable list converts to nothing. This
//! projection enforces the consequence: such a list issues **no query at
//! all** — it matches nothing — and carries why out through
//! [`CustomListDegraded`] instead of collapsing to a silent zero.
//!
//! ## The draft tree's count is keyed on the tree, not on a generation
//!
//! The editor's live "matches N songs" line counts a tree that is **not
//! saved**: no [`PlaylistId`] to look one up by, and a shape that changes
//! several times inside one library generation. [`CustomPlaylistProjection::count_tree`]
//! serves it over the same bounded Listing Page read (the total alone, `limit 0`),
//! and holds each answer against the **compiled tree's shape** for the
//! library generation it was read at — so a repaint of an unchanged tree,
//! and an edit that brings a Rule back to a value it held before, both come
//! from the hold instead of the store.

use crate::app::projection::windowed_list::WindowedListProjection;
use crate::app::store::{
    GenerationCache, Page, RuleTree, SmartPlaylistTree, StoreError, StoreGeneration, TrackListOrder,
};
use crate::domain::{PlaylistId, Track};
use std::sync::Arc;

/// The query signature a Custom list's cached windows were computed for.
///
/// Every mutable element of the read is here — the compiled tree's shape,
/// the Track list order, the current needle, and the playlist epoch the
/// tree was read at (the library epoch is the cache's own stamp, not a key
/// field: one stamp per cache, per the module doc). A change to any of them
/// retargets the projection and drops the rows in hand even at an unchanged
/// generation, exactly like every other paged listing's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomListKey {
    /// The compiled tree's shape the window was computed for. Rule and Rule
    /// Box ids never reach this key — [`RuleTree`] holds the stored *field*
    /// and *operator* ids and the sibling order, not row identities.
    pub tree: RuleTree,
    /// The listing's shared Track order.
    pub order: TrackListOrder,
    /// The listing's current search needle, `None` for no needle.
    pub needle: Option<String>,
    /// The playlist-generation epoch the tree was read at. Baked into the
    /// signature so a name or tree write moves it and drops rows for free,
    /// while the window cache itself stays single-stamped on the library
    /// generation.
    pub playlist_epoch: u64,
}

impl CustomListKey {
    /// The inert signature a projection starts with, before any list is
    /// opened. No real query can match it: a stored tree always holds at
    /// least one Rule (the save refuses anything else), so its compiled
    /// shape is never the empty box spelled here with an empty combine word.
    fn never_opened() -> Self {
        Self {
            tree: RuleTree::Box {
                combine: String::new(),
                children: Vec::new(),
            },
            order: TrackListOrder::default(),
            needle: None,
            playlist_epoch: 0,
        }
    }
}

/// A Custom list read that could not answer fresh: the value to draw plus
/// the reason it is not fresh.
///
/// This is the reason channel the typed poll outcomes (`ScanOutcome::Failed`,
/// `ClearCacheOutcome`, `PassReport.failed`) already use for writes —
/// "surfaced as an outcome instead of dying in a log line" — applied to the
/// synchronous Custom-list reads whose surfaces must show *why* they are
/// showing a stale or empty answer. `Ok(value)` means fresh, no reason;
/// `Err(CustomListDegraded { value, reason })` means `value` is the last
/// good rows/total in hand for this query (or the default when none is) and
/// `reason` is listener-facing text. Every other `SessionViews` read keeps
/// answering through `degrade` with no reason channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomListDegraded<T> {
    /// The value to draw: the last good answer for this query, or the
    /// default (`0`, `None`) when nothing good is in hand.
    pub value: T,
    /// Why the read could not answer fresh. Listener-facing text.
    pub reason: String,
}

/// Why a tree with unreadable rows issues no query at all.
fn unreadable_reason(tree: &SmartPlaylistTree) -> String {
    let unreadable = tree.unreadable().len();
    format!(
        "This Custom Smart Playlist holds {unreadable} rule{} riff cannot read, so it matches nothing",
        if unreadable == 1 { "" } else { "s" }
    )
}

/// The reason a listing gives when the list itself is gone.
fn gone_reason(id: &PlaylistId) -> String {
    format!("Custom Smart Playlist {id:?} is gone, so its listing matches nothing")
}

/// The bounded Listing Page reader both Custom reads share: the port read's
/// shape, taking the compiled tree, the order, the needle, and the window
/// the `WindowedListProjection` house asks for.
type PageReader<'a> = &'a mut dyn FnMut(
    &RuleTree,
    TrackListOrder,
    Option<&str>,
    usize,
    usize,
) -> Result<Page<Track>, StoreError>;

/// The bounded window-plus-count projection over Custom Smart Playlist
/// listings, plus the generation-cached tree read those rows compile from
/// and the shape-keyed hold behind the editor's live count.
///
/// One instance serves whichever list is open: retargeting happens through
/// [`CustomListKey`] on every call, so switching lists, sorts, or needles
/// drops rows even at an unchanged generation. The tree lives in a
/// [`GenerationCache`] keyed by list id on the playlist generation — read
/// once per list per epoch, never per frame — and the playlist epoch it was
/// read at rides into the window key (module doc, "Two generations, one
/// stamp").
pub struct CustomPlaylistProjection {
    /// `id`'s last read tree, stamped at the playlist epoch. A cached
    /// absence (the inner `Option` being `None`) is a loaded answer: a list
    /// that is gone answers `None` without requerying per frame.
    trees: GenerationCache<PlaylistId, Option<SmartPlaylistTree>>,
    /// The bounded windows and total, keyed by the whole query signature,
    /// stamped at the library generation.
    windows: WindowedListProjection<CustomListKey, Track>,
    /// The draft trees' held match counts, one bundle per library
    /// generation, keyed inside the bundle on the compiled shape
    /// ([`Self::count_tree`]). Its own cache because it is its own question:
    /// a draft has no list id to key the trees cache on and no playlist
    /// generation to stamp, and its shape changes several times inside one
    /// library generation, which is exactly what a per-generation hold
    /// serves.
    tree_counts: GenerationCache<(), TreeCounts>,
    /// Which list the last successful tree load answered — [`Self::cached_tree`]
    /// only serves a tree to the id it was loaded for; a failed load leaves
    /// the previous entry untouched (the `GenerationCache` invariant).
    loaded_for: Option<PlaylistId>,
    /// Which list last reached the window reads — the failed-tree-load
    /// fallback only serves rows in hand when they belong to the same list
    /// the caller is asking about.
    queried_for: Option<PlaylistId>,
}

/// The two no-query faults [`CustomPlaylistProjection::query`] names, each
/// with its listener-facing reason.
enum NoQuery {
    /// The tree read itself failed; no fresh query can be compiled. The
    /// listing may still serve its last good rows beside the reason.
    TreeLoad(String),
    /// The tree holds an Unreadable Rule — by rule, no query, and the
    /// answer is empty, never a stale window for some other tree.
    Unreadable(String),
}

/// How many distinct draft-tree shapes one library generation's counts are
/// held for before the oldest leaves.
///
/// The hold exists so a repaint and an edit-back cost no store read; a
/// listener writing a long rule set can commit more shapes than this inside
/// one generation, and each shape beyond the bound is simply re-read when it
/// comes back — correct, just not free. The bound keeps the hold small the
/// way every other cache here is bounded, and it is deliberately not the
/// window house's numbers: those count row windows, this counts whole-tree
/// answers.
const TREE_COUNT_HOLD_CAP: usize = 8;

/// The match counts held for one library generation, each against the
/// compiled tree shape that produced it.
///
/// A `Vec` rather than a map because a compiled tree is a compound value
/// [`RuleTree`] does not hash; the hold is bounded, and a linear scan over
/// a handful of small trees costs far less than the store read it saves.
#[derive(Default)]
struct TreeCounts {
    /// The held `(shape, count)` pairs, oldest first.
    held: Vec<(RuleTree, usize)>,
}

impl TreeCounts {
    /// The count held for `shape`, or `None` when this generation has not
    /// answered this shape yet.
    fn count(&self, shape: &RuleTree) -> Option<usize> {
        self.held
            .iter()
            .find(|(held, _)| held == shape)
            .map(|(_, count)| *count)
    }

    /// Hold `count` as the answer for `shape`, replacing an earlier answer
    /// for the same shape and letting the oldest shape leave at the bound.
    fn hold(&mut self, shape: RuleTree, count: usize) {
        if let Some(slot) = self.held.iter_mut().find(|(held, _)| held == &shape) {
            slot.1 = count;
            return;
        }
        if self.held.len() == TREE_COUNT_HOLD_CAP {
            self.held.remove(0);
        }
        self.held.push((shape, count));
    }
}

impl CustomPlaylistProjection {
    /// Start the projection observing both session counters: the playlist
    /// generation for the tree read, the library generation for the rows.
    #[must_use]
    pub fn new(playlist_generation: StoreGeneration, library_generation: StoreGeneration) -> Self {
        Self {
            trees: GenerationCache::new(playlist_generation),
            windows: WindowedListProjection::new(
                library_generation.clone(),
                CustomListKey::never_opened(),
            ),
            tree_counts: GenerationCache::new(library_generation),
            loaded_for: None,
            queried_for: None,
        }
    }

    /// The Custom list's total. Its own read: asks the bounded Listing Page
    /// for the total alone — offset 0, **limit 0**, the count sentinel every
    /// paged seam read uses; a count read that passed a real window would
    /// silently land in the window-call path — through the same cache as
    /// [`Self::row`].
    ///
    /// `load_tree` is the store's tree read and `read_page` the bounded
    /// Listing Page read; this projection compiles the tree (refusing an
    /// unreadable one before any query is issued) and keys the window cache
    /// on the whole query signature.
    ///
    /// # Errors
    /// `Err` carries a reason and the value to draw: the last good total
    /// when a transient tree-load failure interrupted a read of the list in
    /// hand, and `0` when the list is gone, its tree holds an Unreadable
    /// Rule (no query is ever issued for that), or nothing is in hand.
    pub fn count(
        &mut self,
        id: &PlaylistId,
        order: TrackListOrder,
        needle: Option<&str>,
        load_tree: &mut dyn FnMut(&PlaylistId) -> Result<Option<SmartPlaylistTree>, StoreError>,
        read_page: PageReader<'_>,
    ) -> Result<usize, CustomListDegraded<usize>> {
        let key = match self.query(id, order, needle, load_tree) {
            Ok(Some(key)) => key,
            Ok(None) => {
                return Err(CustomListDegraded {
                    value: 0,
                    reason: gone_reason(id),
                });
            }
            Err(no_query) => {
                let (last_good_allowed, reason) = match no_query {
                    NoQuery::TreeLoad(reason) => (true, reason),
                    NoQuery::Unreadable(reason) => (false, reason),
                };
                let value = if last_good_allowed && self.queried_for.as_ref() == Some(id) {
                    self.windows.cached_count().unwrap_or(0)
                } else {
                    0
                };
                return Err(CustomListDegraded { value, reason });
            }
        };
        self.queried_for = Some(id.clone());
        let total = self.windows.count(key.clone(), &mut || {
            read_page(&key.tree, key.order, needle, 0, 0).map(|page| page.total())
        });
        match total {
            Ok(total) => Ok(total),
            Err(error) => {
                tracing::warn!("Failed to count the Custom Smart Playlist listing: {error}");
                Err(CustomListDegraded {
                    value: self.windows.cached_count().unwrap_or(0),
                    reason: error.to_string(),
                })
            }
        }
    }

    /// Row `index` of the Custom list, served from the bounded window cache
    /// [`WindowedListProjection`] owns — window size and cache bound
    /// inherited, never restated. `Ok(None)` means the listing has no row at
    /// that index.
    ///
    /// # Errors
    /// `Err` carries a reason and the value to draw: the last good row for
    /// this query when a transient tree-load failure or a failed store read
    /// interrupted a listing already in hand, and `None` when the list is
    /// gone or its tree holds an Unreadable Rule (no query is ever issued
    /// for that).
    pub fn row(
        &mut self,
        id: &PlaylistId,
        order: TrackListOrder,
        needle: Option<&str>,
        index: usize,
        load_tree: &mut dyn FnMut(&PlaylistId) -> Result<Option<SmartPlaylistTree>, StoreError>,
        read_page: PageReader<'_>,
    ) -> Result<Option<Arc<Track>>, CustomListDegraded<Option<Arc<Track>>>> {
        let key = match self.query(id, order, needle, load_tree) {
            Ok(Some(key)) => key,
            Ok(None) => {
                return Err(CustomListDegraded {
                    value: None,
                    reason: gone_reason(id),
                });
            }
            Err(no_query) => {
                let (last_good_allowed, reason) = match no_query {
                    NoQuery::TreeLoad(reason) => (true, reason),
                    NoQuery::Unreadable(reason) => (false, reason),
                };
                let value = if last_good_allowed && self.queried_for.as_ref() == Some(id) {
                    self.windows.cached_row(index)
                } else {
                    None
                };
                return Err(CustomListDegraded { value, reason });
            }
        };
        self.queried_for = Some(id.clone());
        let row = self.windows.row(key.clone(), index, &mut |offset, limit| {
            read_page(&key.tree, key.order, needle, offset, limit)
        });
        match row {
            Ok(row) => Ok(row),
            Err(error) => {
                tracing::warn!("Failed to refresh the Custom Smart Playlist listing: {error}");
                Err(CustomListDegraded {
                    value: self.windows.cached_row(index),
                    reason: error.to_string(),
                })
            }
        }
    }

    /// The Custom list's tree as the store read it — cached per list id per
    /// playlist epoch, so a repaint costs no store read and a name or tree
    /// write (which moves that epoch) refetches. `Ok(None)` is a cached
    /// absence: the list is gone, and that answer stays cheap.
    ///
    /// # Errors
    /// `Err` carries the last tree loaded for `id` (a cold failure answers
    /// `None`) beside the reason, on a failed store read.
    pub fn tree(
        &mut self,
        id: &PlaylistId,
        load_tree: &mut dyn FnMut(&PlaylistId) -> Result<Option<SmartPlaylistTree>, StoreError>,
    ) -> Result<Option<SmartPlaylistTree>, CustomListDegraded<Option<SmartPlaylistTree>>> {
        let epoch = self.trees.observe();
        if self.trees.holds(epoch, id) {
            self.loaded_for = Some(id.clone());
            return Ok(self.trees.peek().cloned().flatten());
        }
        match load_tree(id) {
            Ok(loaded) => {
                self.trees.slot(epoch, id).clone_from(&loaded);
                self.loaded_for = Some(id.clone());
                Ok(loaded)
            }
            Err(error) => {
                tracing::warn!("Failed to load the Custom Smart Playlist's rule tree: {error}");
                Err(CustomListDegraded {
                    value: self.cached_tree(id),
                    reason: error.to_string(),
                })
            }
        }
    }

    /// The last tree loaded for `id`, regardless of epoch — the
    /// stale-but-present fallback every other projection's `cached_*` read
    /// is. Answers `None` for an id no load ever answered, so a failed cold
    /// open cannot hand out another list's tree.
    #[must_use]
    pub fn cached_tree(&self, id: &PlaylistId) -> Option<SmartPlaylistTree> {
        if self.loaded_for.as_ref() == Some(id) {
            self.trees.peek().cloned().flatten()
        } else {
            None
        }
    }

    /// How many Tracks the tree a draft spells currently matches — the live
    /// count behind the editor's "matches N songs" line, read for a tree
    /// rather than a saved list.
    ///
    /// A draft has no [`PlaylistId`] and no playlist generation of its own,
    /// which is why the id-keyed reads above cannot serve it: there is no
    /// stored tree to load and no playlist epoch to stamp, and its shape
    /// changes several times inside one library generation. What there is,
    /// is the tree itself — so this read compiles it through the same ONE
    /// bridge the saved listing uses (an Unreadable Rule means no query at
    /// all, with the listing's own reason) and asks the same bounded Listing
    /// Page for the total alone (`limit 0`, the count sentinel). The answer
    /// is held against the **compiled shape** for the library generation it
    /// was read at, so a repaint of an unchanged tree reuses the hold, and
    /// so does a Rule edited back to a value it held before; a library write
    /// — which is what moves which tracks match — refetches.
    ///
    /// # Errors
    /// `Err` carries the value to draw beside the reason: the last good
    /// count for this exact shape (or `0` when nothing is held), and the
    /// store's refusal verbatim when the engine cannot compute the tree —
    /// which is the same reason the listing shows for the same tree.
    pub fn count_tree(
        &mut self,
        tree: &SmartPlaylistTree,
        read_page: PageReader<'_>,
    ) -> Result<usize, CustomListDegraded<usize>> {
        let Some(shape) = RuleTree::from_smart_playlist_tree(tree) else {
            return Err(CustomListDegraded {
                value: 0,
                reason: unreadable_reason(tree),
            });
        };
        let held_shape = shape.clone();
        let result = self.tree_counts.level(
            &(),
            |held| held.count(&shape),
            || read_page(&shape, TrackListOrder::default(), None, 0, 0).map(|page| page.total()),
            |held, count| {
                held.hold(held_shape, count);
                count
            },
        );
        match result {
            Ok(count) => Ok(count),
            Err(error) => {
                tracing::warn!("Failed to count the tracks a draft Rule tree matches: {error}");
                Err(CustomListDegraded {
                    value: self
                        .tree_counts
                        .peek()
                        .and_then(|held| held.count(&shape))
                        .unwrap_or(0),
                    reason: error.to_string(),
                })
            }
        }
    }

    /// Compile `id`'s tree into the window query signature, or name why the
    /// listing issues no query at all.
    ///
    /// `Ok(None)` is not an error but is not a query either: the list is
    /// gone. An unreadable tree is refused before any store read is asked,
    /// so "matches nothing" carries its reason and never a guessed-at
    /// predicate.
    fn query(
        &mut self,
        id: &PlaylistId,
        order: TrackListOrder,
        needle: Option<&str>,
        load_tree: &mut dyn FnMut(&PlaylistId) -> Result<Option<SmartPlaylistTree>, StoreError>,
    ) -> Result<Option<CustomListKey>, NoQuery> {
        let tree = match self.tree(id, load_tree) {
            Ok(Some(tree)) => tree,
            Ok(None) => return Ok(None),
            Err(degraded) => return Err(NoQuery::TreeLoad(degraded.reason)),
        };
        let Some(compiled) = RuleTree::from_smart_playlist_tree(&tree) else {
            return Err(NoQuery::Unreadable(unreadable_reason(&tree)));
        };
        Ok(Some(CustomListKey {
            tree: compiled,
            order,
            needle: needle.map(str::to_string),
            playlist_epoch: self.trees.observe(),
        }))
    }
}

impl Default for CustomPlaylistProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new(), StoreGeneration::new())
    }
}
