//! The Thumbnail cache's on-disk format, as a pure function.
//!
//! ```text
//! covers/<ab>/<fnv1a128(source_path)>-<mtime_nanos>-<len>-<W>x<H>.img
//! ```
//!
//! There is no index — no sqlite table, no sidecar, no Application Store
//! change. The filename *is* the fingerprint, so a lookup is one `stat` of the
//! source, one `open` of the name derived from it, and nothing to read, parse,
//! lock, migrate or corrupt.

use riff_persistence::thumbnail::ThumbnailBox;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// FNV-1a 128-bit offset basis — the value an empty input hashes to.
const FNV1A_128_OFFSET_BASIS: u128 = 0x6c16_2768_36a5_0a17_2be9_10d6_96f6_a663;

/// FNV-1a 128-bit prime: `2^104 + 2^8 + 0x3b`.
const FNV1A_128_PRIME: u128 = 0x0000_0100_0000_0000_0000_0000_0000_013b;

/// The 128-bit FNV-1a hash of `bytes`, std-only.
///
/// **Why this hash and not a content hash, deliberately.** The cached
/// Thumbnail is *derived* data: the source file is the truth, and a stale or
/// wrong entry is a rendering bug, not a data-loss bug — so the key has to be
/// cheap, and `blake3` or `sha2` over the file body is not. Hashing the source
/// bytes to decide what to re-decode would read the whole library, which
/// contradicts the feature this cache exists to avoid. 128 bits rather than 64
/// so a birthday collision cannot serve one Track's Cover for another's across
/// a million-entry library; `DefaultHasher` is excluded because it is
/// documented as unstable across Rust releases, and a silent change to it
/// would invalidate every entry on a toolchain bump.
///
/// There is likewise **no LRU and no eviction here**: the entry is named for
/// the source's `(mtime, len)`, so a changed source is a different name and the
/// stale one becomes unreachable rather than wrong. Growth is bounded by the
/// number of distinct cover Sources ever seen and is reclaimed only by the
/// Settings action, not by size policy.
#[must_use]
pub fn fnv1a128(bytes: &[u8]) -> u128 {
    let mut hash = FNV1A_128_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(FNV1A_128_PRIME);
    }
    hash
}

/// What the cache knows about a cover Source: the two numbers one `stat`
/// returns, and the whole of the invalidation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFingerprint {
    pub mtime_nanos: u128,
    pub len: u64,
}

impl SourceFingerprint {
    /// One `stat` of `source`. `None` when the source cannot be stated — an
    /// absent source has nothing cached and nothing to derive a name from, so
    /// the caller treats it as a miss and resolves normally.
    #[must_use]
    pub fn of(source: &Path) -> Option<Self> {
        let metadata = source.metadata().ok()?;
        let mtime_nanos = match metadata
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
        {
            Ok(since) => since.as_nanos(),
            // Pre-epoch mtimes are a FAT/exFAT anomaly. Folding them down into
            // the far end of the range keeps one name per distinct time, and
            // keeps them from colliding with the post-epoch values a real clock
            // produces.
            Err(before) => u128::MAX - before.duration().as_nanos(),
        };
        Some(Self {
            mtime_nanos,
            len: metadata.len(),
        })
    }
}

/// The name `source`'s Thumbnail at `box_` is stored under, below `root`.
///
/// The hash is over the path's *text*, lossily converted, because that is
/// already how Track identity is derived in this app (`TrackId::from_path` on
/// `to_string_lossy`) — so two paths that differ only in bytes that are not
/// valid UTF-8 were never distinct identities to begin with.
#[must_use]
pub fn cache_path(
    root: &Path,
    source: &Path,
    fingerprint: &SourceFingerprint,
    box_: ThumbnailBox,
) -> PathBuf {
    let hash = fnv1a128(source.as_os_str().to_string_lossy().as_bytes());
    let entry = format!(
        "{hash:032x}-{}-{}-{}x{}.img",
        fingerprint.mtime_nanos, fingerprint.len, box_.width, box_.height
    );
    // 256 fanout directories: a flat cache of a million names is slow to
    // enumerate, and on Windows it is slow to enumerate worst of all.
    root.join(format!("{:02x}", (hash >> 120) as u8))
        .join(entry)
}
