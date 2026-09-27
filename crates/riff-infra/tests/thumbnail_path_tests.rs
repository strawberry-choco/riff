//! The on-disk format of the Thumbnail cache, as a pure function: no IO, no
//! clock, no filesystem — just the name a `(source, fingerprint, box)` triple
//! maps to. Ticket 01 of `.scratch/thumbnail-cache/issues/`.

use riff_infra::media::thumbnail_path::fnv1a128;
use riff_infra::media::thumbnail_path::{SourceFingerprint, cache_path};
use riff_persistence::thumbnail::ThumbnailBox;
use std::path::Path;

/// Reference values from an independent implementation of FNV-1a over 128 bits
/// (offset basis `6c16276836a50a172be910d696f6a663`, prime
/// `2^104 + 2^8 + 0x3b`, each byte XOR-ed in before the multiply). That harness
/// was itself checked against the published 32- and 64-bit FNV-1a vectors for
/// `"a"` and `"foobar"` before these literals were taken from it.
///
/// They are literals from outside this crate on purpose: a refactor that
/// silently changed the algorithm — a different prime, `fnv1` instead of
/// `fnv1a`, a widened offset — would rename every cached entry and look like a
/// cold cache forever.
#[test]
fn fnv1a128_matches_the_reference_vectors() {
    assert_eq!(
        format!("{:032x}", fnv1a128(b"")),
        "6c16276836a50a172be910d696f6a663",
        "the empty input is the offset basis itself — no round was run"
    );
    assert_eq!(
        format!("{:032x}", fnv1a128(b"a")),
        "f5e87f3b3d136a8307c7b80bc17e4476"
    );
    assert_eq!(
        format!("{:032x}", fnv1a128(b"foobar")),
        "4244adb6bb1c60fa1c4baf511c444d5a"
    );
    assert_eq!(
        format!("{:032x}", fnv1a128(b"/music/album/cover.jpg")),
        "d7eea370dd81cb1454dbd5e2f4b8cce6",
        "the hash is taken over the source path's bytes, which is the cache key"
    );
}

/// Two different inputs must not land on the same 128-bit value: the hash is
/// the whole of the entry's identity, so a collision would serve one album's
/// art under another's name.
#[test]
fn fnv1a128_separates_paths_that_differ_by_one_byte() {
    let a = fnv1a128(b"/music/a/cover.jpg");
    let b = fnv1a128(b"/music/b/cover.jpg");
    assert_ne!(a, b);
    assert_ne!(fnv1a128(b"cover.jpg"), fnv1a128(b"cover.jpeg"));
}

/// The entry name, as a whole. Every part of it is load-bearing: the hash is
/// the source's identity, the two numbers after it are the source's `stat`, and
/// the box is what the caller asked to display — so one `stat` of the source
/// plus one `open` of this name is the entire lookup, with no index to read.
#[test]
fn cache_path_builds_the_documented_entry_name() {
    let root = Path::new("/app-data/covers");
    let source = Path::new("/music/album/cover.jpg");
    let fingerprint = SourceFingerprint {
        mtime_nanos: 1_700_000_000_123_456_789,
        len: 4_096,
    };
    let path = cache_path(
        root,
        source,
        &fingerprint,
        ThumbnailBox {
            width: 200,
            height: 200,
        },
    );

    // `d7eea370…` is the hash pinned by the reference-vector test above, not
    // recomputed here.
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("d7eea370dd81cb1454dbd5e2f4b8cce6-1700000000123456789-4096-200x200.img"),
        "the filename is the fingerprint: hash, source mtime, source length, requested box"
    );
    assert_eq!(
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str()),
        Some("d7"),
        "the fanout directory is the hash's first two hex chars"
    );
    assert!(path.starts_with(root));
}

#[test]
fn cache_path_is_the_same_name_for_the_same_source_fingerprint_and_box() {
    let fingerprint = SourceFingerprint {
        mtime_nanos: 7,
        len: 11,
    };
    let box_ = ThumbnailBox {
        width: 56,
        height: 56,
    };
    assert_eq!(
        cache_path(
            Path::new("/covers"),
            Path::new("/m/a.mp3"),
            &fingerprint,
            box_
        ),
        cache_path(
            Path::new("/covers"),
            Path::new("/m/a.mp3"),
            &fingerprint,
            box_
        ),
        "a hit must be reachable without remembering anything about the earlier write"
    );
}

/// The fingerprint *is* the invalidation policy: a source whose `stat` moved
/// hashes to a name nobody has written, so the next request misses and rebuilds
/// rather than serving a stale Thumbnail.
#[test]
fn cache_path_moves_when_the_source_mtime_or_length_changes() {
    let at = |mtime: u128, len: u64| SourceFingerprint {
        mtime_nanos: mtime,
        len,
    };
    let box_ = ThumbnailBox {
        width: 56,
        height: 56,
    };
    let base = cache_path(
        Path::new("/covers"),
        Path::new("/m/cover.jpg"),
        &at(1_000, 500),
        box_,
    );
    assert_ne!(
        base,
        cache_path(
            Path::new("/covers"),
            Path::new("/m/cover.jpg"),
            &at(1_001, 500),
            box_
        ),
        "a touched source is a different entry"
    );
    assert_ne!(
        base,
        cache_path(
            Path::new("/covers"),
            Path::new("/m/cover.jpg"),
            &at(1_000, 501),
            box_
        ),
        "a resized source is a different entry"
    );
    assert_ne!(
        base,
        cache_path(
            Path::new("/covers"),
            Path::new("/m/cover.png"),
            &at(1_000, 500),
            box_
        ),
        "a different source file is a different entry"
    );
}

/// Three canonical boxes, three entries: a Thumbnail is never a source for a
/// larger Thumbnail, so the rungs may not share a name.
#[test]
fn cache_path_gives_each_canonical_box_its_own_entry() {
    let fingerprint = SourceFingerprint {
        mtime_nanos: 1,
        len: 2,
    };
    let names: Vec<String> = [(56u32, 56u32), (200, 200), (512, 512)]
        .into_iter()
        .map(|(width, height)| {
            cache_path(
                Path::new("/covers"),
                Path::new("/m/cover.jpg"),
                &fingerprint,
                ThumbnailBox { width, height },
            )
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string()
        })
        .collect();
    assert_eq!(names.len(), 3);
    assert_ne!(names[0], names[1]);
    assert_ne!(names[1], names[2]);
    assert_ne!(names[0], names[2]);
    assert!(
        names[2].ends_with("-512x512.img"),
        "the name carries the *requested* box: a source smaller than the box is \
         stored at its own pixel size under the requested name, because deriving \
         the name from the result would need a header read on every hit"
    );
}

/// Distribution, not just format: 256 fanout directories exist so a million-entry
/// cache is not one million-name directory. If the prefix were constant, or drew
/// from a poorly-mixing part of the hash, the spread collapses.
#[test]
fn cache_path_spreads_sources_across_the_fanout_directories() {
    let fingerprint = SourceFingerprint {
        mtime_nanos: 1,
        len: 1,
    };
    let box_ = ThumbnailBox {
        width: 56,
        height: 56,
    };
    let mut dirs = std::collections::HashSet::new();
    let mut prefixes = std::collections::HashSet::new();
    for index in 0..400 {
        let source = format!("/music/album-{index}/cover.jpg");
        let path = cache_path(Path::new("/covers"), Path::new(&source), &fingerprint, box_);
        let dir = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap()
            .to_string();
        assert!(
            dir.len() == 2 && dir.chars().all(|c| c.is_ascii_hexdigit()),
            "every fanout directory is two hex chars, got {dir:?} for {source}"
        );
        prefixes.insert(dir);
        dirs.insert(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .split('-')
                .next()
                .unwrap()
                .to_string(),
        );
    }
    assert_eq!(dirs.len(), 400, "each source hashes to its own entry");
    assert!(
        prefixes.len() > 200,
        "400 distinct sources should scatter over most of the 256 fanout buckets, got {}",
        prefixes.len()
    );
}
