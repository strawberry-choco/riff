//! `FileThumbnailCache` — the Thumbnail cache's file-backed adapter, over a
//! scratch root. Ticket 02 of `.scratch/thumbnail-cache/issues/`.
//!
//! The adapter treats the encoded bytes as opaque (choosing a container is
//! `03`'s job); what this suite owns is the disk contract — one `stat` plus one
//! `open` on a hit, a miss on any change, an atomic write, and a clear that
//! really removes the tree.

use super::adapter_tests::png_fixture;
use riff_infra::media::thumbnail_path::{SourceFingerprint, cache_path};
use riff_infra::media::thumbnail_store::FileThumbnailCache;
use riff_persistence::thumbnail::{EncodedThumbnail, ThumbnailBox, ThumbnailCache};
use std::path::PathBuf;

/// A 200×200 box — one of the three canonical ones, and big enough that a
/// round-tripped entry cannot be confused with another rung.
const CARD: ThumbnailBox = ThumbnailBox {
    width: 200,
    height: 200,
};

/// A scratch root with a real cover Source inside it, so `SourceFingerprint::of`
/// has a file to `stat`. Dropping the holder removes both.
struct Scratch {
    _dir: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let source = dir.path().join("cover.jpg");
        std::fs::write(&source, b"the bytes of a real cover image").expect("source is writable");
        let root = dir.path().join("covers");
        Self {
            _dir: dir,
            root,
            source,
        }
    }

    fn cache(&self) -> FileThumbnailCache {
        FileThumbnailCache::with_root(self.root.clone())
    }
}

/// A store followed by a load is the whole feature: the same encoded bytes and
/// the same pixel dimensions come back, and they live in the fanout directory
/// `01` derived rather than somewhere the adapter invented.
#[test]
fn a_stored_thumbnail_reads_back_the_same_bytes_and_dimensions() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let stored = EncodedThumbnail {
        bytes: png_fixture(200, 200, [9, 8, 7, 255]),
        width: 200,
        height: 200,
    };

    cache
        .store(&scratch.source, CARD, &stored)
        .expect("a writable root must accept an entry");

    let read_back = cache
        .load(&scratch.source, CARD)
        .expect("the entry just stored must be found by one stat and one open");
    assert_eq!(read_back.bytes, stored.bytes);
    assert_eq!(
        (read_back.width, read_back.height),
        (200, 200),
        "the stored pixel size survives, not just the bytes"
    );

    let expected = cache_path(
        &scratch.root,
        &scratch.source,
        &SourceFingerprint::of(&scratch.source).expect("the source exists"),
        CARD,
    );
    assert!(
        expected.parent().unwrap().is_dir(),
        "the fanout directory is created by the write, not pre-existing"
    );
    assert!(expected.is_file(), "the entry is at the derived path");
}

/// `01`'s deferred no-upscale pin, at the seam where it can actually fail.
///
/// A Cover smaller than the box it was asked for is stored at its own pixel
/// size under the *requested* name, because deriving the name from the result
/// would need a header read on every hit to work out which rung the entry is.
/// The name and the pixels therefore disagree, deliberately — and a reader must
/// get the pixels, not the name.
#[test]
fn an_entry_smaller_than_its_box_reports_its_own_pixels_not_the_box() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let thumb = ThumbnailBox {
        width: 56,
        height: 56,
    };

    cache
        .store(
            &scratch.source,
            thumb,
            &EncodedThumbnail {
                bytes: png_fixture(40, 40, [1, 2, 3, 255]),
                width: 40,
                height: 40,
            },
        )
        .expect("a writable root must accept an entry");

    let read_back = cache
        .load(&scratch.source, thumb)
        .expect("the requested box names the entry, and the entry exists");
    assert_eq!(
        (read_back.width, read_back.height),
        (40, 40),
        "the stored image's own size, not the 56x56 it was asked for"
    );

    let entry = cache_path(
        &scratch.root,
        &scratch.source,
        &SourceFingerprint::of(&scratch.source).expect("the source exists"),
        thumb,
    );
    assert!(
        entry
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-56x56.img"),
        "the name carries the requested box while the bytes carry 40x40 pixels"
    );
}

/// The fingerprint is the whole invalidation policy, so a Source whose `stat`
/// moved must simply not be found — not found stale, not found and repaired.
/// Each of the three below is a different way a Source stops matching the name
/// its Thumbnail was stored under.
#[test]
fn a_thumbnail_is_not_found_once_the_source_has_been_replaced() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let old = EncodedThumbnail {
        bytes: png_fixture(64, 64, [1, 1, 1, 255]),
        width: 64,
        height: 64,
    };
    cache
        .store(&scratch.source, CARD, &old)
        .expect("a writable root must accept an entry");
    assert!(cache.load(&scratch.source, CARD).is_some());

    // A different length moves `len`, which alone changes the derived name.
    std::fs::write(
        &scratch.source,
        b"a longer replacement cover image entirely",
    )
    .unwrap();
    assert!(
        cache.load(&scratch.source, CARD).is_none(),
        "a resized source is a different entry"
    );

    // Same length, different bytes: only `mtime` can tell these apart, so an
    // in-place edit must miss too. The pause is what keeps this honest on a
    // filesystem with coarse mtime granularity (exFAT and FAT32 tick in
    // seconds) — without it the two writes could share a timestamp and the
    // name would not have moved for a reason the app cannot control.
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let before = SourceFingerprint::of(&scratch.source).expect("the source exists");
    let mut edited = std::fs::read(&scratch.source).unwrap();
    edited[0] ^= 0xff;
    std::fs::write(&scratch.source, &edited).unwrap();
    let after = SourceFingerprint::of(&scratch.source).expect("the source exists");
    assert_eq!(
        before.len, after.len,
        "the edit kept the length, so only mtime can distinguish these two states"
    );
    assert_ne!(
        before.mtime_nanos, after.mtime_nanos,
        "if this fires, this filesystem's mtime is coarser than the pause allows — \
         the in-place-edit case below cannot be proven here"
    );
    assert!(
        cache.load(&scratch.source, CARD).is_none(),
        "an in-place edit at the same length is caught by mtime alone"
    );
}

#[test]
fn a_thumbnail_is_not_found_once_the_source_is_gone() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    cache
        .store(
            &scratch.source,
            CARD,
            &EncodedThumbnail {
                bytes: png_fixture(64, 64, [2, 2, 2, 255]),
                width: 64,
                height: 64,
            },
        )
        .expect("a writable root must accept an entry");

    std::fs::remove_file(&scratch.source).unwrap();
    assert!(
        cache.load(&scratch.source, CARD).is_none(),
        "an absent source has no fingerprint to derive a name from — a miss, never an error"
    );
}

/// The three canonical boxes are three separate rungs, and one may never
/// answer for another: a Thumbnail is never a source for a larger Thumbnail.
#[test]
fn a_thumbnail_stored_at_one_box_is_not_found_at_another() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let thumb = ThumbnailBox {
        width: 56,
        height: 56,
    };
    let hero = ThumbnailBox {
        width: 512,
        height: 512,
    };
    cache
        .store(
            &scratch.source,
            thumb,
            &EncodedThumbnail {
                bytes: png_fixture(56, 56, [3, 3, 3, 255]),
                width: 56,
                height: 56,
            },
        )
        .expect("a writable root must accept an entry");

    assert!(cache.load(&scratch.source, thumb).is_some());
    assert!(
        cache.load(&scratch.source, hero).is_none(),
        "a rung that was never stored cannot be served by a smaller one"
    );
}

/// Clear is the only reclaim the design has, so it has to really remove the
/// tree — and it has to leave the cache usable afterwards, because the app keeps
/// running and repopulates on the next request.
#[test]
fn clearing_removes_every_entry_and_the_cache_works_again() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let thumbnail = EncodedThumbnail {
        bytes: png_fixture(64, 64, [4, 4, 4, 255]),
        width: 64,
        height: 64,
    };
    cache.store(&scratch.source, CARD, &thumbnail).unwrap();

    cache.clear().expect("a populated root must clear");
    assert!(
        !scratch.root.exists(),
        "the whole tree is gone, not just its entries"
    );
    assert!(cache.load(&scratch.source, CARD).is_none());

    cache
        .store(&scratch.source, CARD, &thumbnail)
        .expect("the root is recreated lazily by the next write");
    assert_eq!(
        cache.load(&scratch.source, CARD).unwrap().bytes,
        thumbnail.bytes
    );
}

/// There is no automatic eviction, so debris from a crash mid-write would be
/// permanent. Sweeping stale partial files on write is the one concession.
#[test]
fn a_partial_write_left_by_a_crash_is_swept_by_the_next_store() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    cache
        .store(
            &scratch.source,
            CARD,
            &EncodedThumbnail {
                bytes: png_fixture(64, 64, [5, 5, 5, 255]),
                width: 64,
                height: 64,
            },
        )
        .unwrap();

    let dir = cache_path(
        &scratch.root,
        &scratch.source,
        &SourceFingerprint::of(&scratch.source).unwrap(),
        CARD,
    )
    .parent()
    .unwrap()
    .to_path_buf();
    let debris = dir.join("00000000000000000000000000000000-1-1-56x56.tmp");
    std::fs::write(&debris, b"half an entry").unwrap();

    cache
        .store(
            &scratch.source,
            CARD,
            &EncodedThumbnail {
                bytes: png_fixture(64, 64, [6, 6, 6, 255]),
                width: 64,
                height: 64,
            },
        )
        .unwrap();

    assert!(!debris.exists(), "the sweep removes the crashed write");
    assert_eq!(
        dir.read_dir()
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
            .count(),
        0,
        "no partial file survives a write, including this write's own temporary"
    );
}

/// A cache that cannot be written must not fail the resolution, so the adapter
/// has to report the failure *and* leave nothing half-committed behind it. The
/// fanout path is occupied by a regular file here rather than made read-only, so
/// the same failure is reachable on Windows as on Unix.
#[test]
fn a_write_that_cannot_land_reports_the_failure_and_leaves_no_entry() {
    let scratch = Scratch::new();
    let fingerprint = SourceFingerprint::of(&scratch.source).unwrap();
    let entry = cache_path(&scratch.root, &scratch.source, &fingerprint, CARD);
    let dir = entry.parent().unwrap();
    std::fs::create_dir_all(&scratch.root).unwrap();
    std::fs::write(dir, b"not a directory at all").unwrap();

    let cache = scratch.cache();
    let result = cache.store(
        &scratch.source,
        CARD,
        &EncodedThumbnail {
            bytes: png_fixture(64, 64, [7, 7, 7, 255]),
            width: 64,
            height: 64,
        },
    );
    assert!(
        result.is_err(),
        "the caller is told, so it can log and carry on"
    );
    assert!(!entry.exists(), "no committed entry");
    assert!(
        !entry.with_extension("tmp").exists(),
        "and no partial entry either"
    );
}

/// The literal case `02` names: an unwritable root. Unix-only, because setting
/// permissions is; the portable shape of the same failure is tested above.
#[cfg(unix)]
#[test]
fn a_read_only_root_reports_the_failure_instead_of_panicking() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = Scratch::new();
    std::fs::create_dir_all(&scratch.root).unwrap();
    std::fs::set_permissions(&scratch.root, std::fs::Permissions::from_mode(0o500))
        .expect("the scratch root must become read-only");

    let cache = scratch.cache();
    let result = cache.store(
        &scratch.source,
        CARD,
        &EncodedThumbnail {
            bytes: png_fixture(64, 64, [8, 8, 8, 255]),
            width: 64,
            height: 64,
        },
    );
    assert!(
        result.is_err(),
        "a full or read-only disk is a failed store"
    );
    assert!(
        cache.load(&scratch.source, CARD).is_none(),
        "and still only ever a miss on the way back"
    );

    std::fs::set_permissions(&scratch.root, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// `clear` is a user action, and pressing it on a cache that was never written
/// must succeed rather than report a fault.
#[test]
fn clearing_an_absent_root_is_not_an_error() {
    let scratch = Scratch::new();
    assert!(!scratch.root.exists());
    scratch
        .cache()
        .clear()
        .expect("already-empty is already clear");
}
