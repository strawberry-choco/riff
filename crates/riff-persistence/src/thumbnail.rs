//! The Thumbnail cache: the port and the two types that cross it.
//!
//! A **Thumbnail** (see `CONTEXT.md`) is a Cover reduced to a fixed pixel box
//! for display — derived, display-only, and never a source for a larger size.
//! This module is the contract for keeping those reductions on disk between
//! runs; the file-backed adapter lives in `riff-infra`.
//!
//! Like the rest of this crate it is `std`-only and implements nothing.

use crate::errors::StoreError;
use std::path::Path;

/// The display box a Thumbnail is rendered for.
///
/// Part of a cache entry's identity: a Cover has one cached Thumbnail per box,
/// and they are not interchangeable — a 56×56 reduction may never stand in for
/// a 512×512 one.
///
/// This is deliberately its own type rather than `riff-library`'s
/// `RequestedSize`: that type lives above this crate, and naming it here would
/// invert the dependency chain. `riff-library` maps its own size onto this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailBox {
    pub width: u32,
    pub height: u32,
}

/// An encoded Thumbnail, ready to decode.
///
/// Deliberately *not* pixels: the cache stores what the `riff-infra` cover
/// loader can decode, so the decode keeps exactly one implementation and this
/// crate never names an image format. The bytes are a lossy container (JPEG for
/// an opaque Cover, a lossless one when the Cover carries alpha), which is safe
/// here and only here because a Thumbnail is **terminal** — nothing is ever
/// derived from it, so there is no second-generation degradation to compound.
///
/// `width`/`height` are the stored image's own pixel size, which is not always
/// `box_`: a source smaller than the requested box is never blown up, so a
/// 40×40 Cover requested at 56×56 is stored as 40×40 under a 56×56 entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedThumbnail {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The persistent Thumbnail cache.
///
/// **Why there is no content hash and no LRU — deliberately.** The keyed input
/// is the cover *Source's* path plus its `(mtime, len)`, gathered in one `stat`;
/// hashing the source *bytes* instead would mean reading the whole library to
/// decide what to re-decode, which contradicts the reason the cache exists. The
/// `stat` comparison is what makes an entry current, so the hash only has to
/// distribute filenames, and a 128-bit one over the path is enough that a
/// collision cannot serve one Track's Cover for another's. Keying on the Source
/// rather than the Track is what collapses the twelve Tracks of one album onto a
/// single stored ladder.
///
/// There is no eviction because the entry name carries the fingerprint: when a
/// source changes, the old entry is simply no longer addressed, so correctness
/// needs no policy on top of it. Growth is bounded by the number of distinct
/// cover Sources ever seen and is reclaimed by [`ThumbnailCache::clear`] — one
/// user action, not a background policy.
pub trait ThumbnailCache: Send + Sync {
    /// The encoded Thumbnail for this source and box, if one is cached and still
    /// current. `None` means miss — either absent, or the source changed.
    fn load(&self, source: &Path, box_: ThumbnailBox) -> Option<EncodedThumbnail>;

    /// Persist an encoded Thumbnail. A failure is never fatal to the caller: a
    /// cache that cannot be written costs a re-decode, nothing more.
    fn store(
        &self,
        source: &Path,
        box_: ThumbnailBox,
        thumbnail: &EncodedThumbnail,
    ) -> Result<(), StoreError>;

    /// Delete every cached Thumbnail. The only reclaim there is.
    fn clear(&self) -> Result<(), StoreError>;
}
