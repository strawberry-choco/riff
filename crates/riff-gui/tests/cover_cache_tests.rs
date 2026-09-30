//! The Cover Cache's protocol, asserted at its own interface.
//!
//! Like the Scroll Memory and the Column dispatch beside it, this module is
//! pure, deterministic and in-process — and unlike everything else in the
//! artwork path it holds **no picture**. Nothing here builds an `egui::Context`,
//! a window, or a kittest harness: a Cover is wanted at a box, is in flight, or
//! has arrived, and those are facts about a cache key rather than about a
//! texture. The egui half — the texture map, its LRU order and the byte budget
//! that bounds them — is asserted from the workspace-root suite, where the
//! Context it needs can be built.
//!
//! Four of these assertions used to sit in `tests/ui_tests.rs`, where the only
//! way to reach the decision was a free function that took the texture map, the
//! marker set and the marker's own LRU list as separate arguments. They moved
//! here when the cache took ownership of them, and the egui-only trimmings they
//! carried with them (a hero *texture* standing in for a cached entry) were
//! replaced by the fact they were really about: a Cover that has *arrived*.

use riff_backend::app::cover_service::{ClearCacheOutcome, Covers};
use riff_backend::app::traits::{DecodedCover, RequestedSize};
use riff_backend::domain::TrackId;
use riff_gui::ui::artwork::{COVER_HERO, COVER_IN_FLIGHT_CAP, COVER_THUMB, cover_cache_key};
use riff_gui::ui::cover_cache::CoverCache;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One request as the Cover Service records it: the Track, the file it resolves
/// through, and the box it wants.
type Request = (TrackId, PathBuf, RequestedSize);

/// One delivered answer: the Track, the box it answers, and its pixels or
/// `None` for an artless verdict.
type Answer = (TrackId, RequestedSize, Option<DecodedCover>);

/// Recording [`Covers`] fake: captures every request intent and serves scripted
/// answers, one poll at a time. `Clone` is a second inspection handle over the
/// same state, so a test can settle through one and read the record through
/// either.
#[derive(Clone, Default)]
struct RecordingCovers {
    requested: Arc<Mutex<Vec<Request>>>,
    answers: Arc<Mutex<Vec<Answer>>>,
}

impl RecordingCovers {
    fn new() -> Self {
        Self::default()
    }

    /// Queue one answer for the next `settle`.
    fn serve(&self, track_id: TrackId, size: RequestedSize, cover: Option<DecodedCover>) {
        self.answers
            .lock()
            .expect("unpoisoned")
            .push((track_id, size, cover));
    }

    /// Every request that reached the service, in order.
    fn requested(&self) -> Vec<Request> {
        self.requested.lock().expect("unpoisoned").clone()
    }
}

impl Covers for RecordingCovers {
    fn request(&self, track_id: TrackId, path: PathBuf, size: RequestedSize) {
        self.requested
            .lock()
            .expect("unpoisoned")
            .push((track_id, path, size));
    }

    /// A folder request lands in the same recording: its identity IS its
    /// directory path, so the two are told apart by what a caller asks for and
    /// not by which method arrived.
    fn request_folder(&self, folder: &std::path::Path, size: RequestedSize) {
        let folder = folder.to_path_buf();
        self.requested.lock().expect("unpoisoned").push((
            TrackId::from_path(&folder),
            folder,
            size,
        ));
    }

    fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
        std::mem::take(&mut self.answers.lock().expect("unpoisoned"))
    }

    fn clear_cache(&self) {}

    fn poll_cache_clear(&self) -> Option<ClearCacheOutcome> {
        None
    }
}

/// One opaque cover, big enough that its size can be asserted.
fn cover(px: u32) -> DecodedCover {
    DecodedCover {
        rgba: vec![7u8; (px * px * 4) as usize],
        width: px,
        height: px,
    }
}

#[test]
fn a_first_ask_sends_the_identity_the_box_and_the_path_and_marks_that_key() {
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    let id = TrackId("/music/one.mp3".to_string());
    let path = PathBuf::from("/music/one.mp3");

    assert!(
        cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB),
        "nothing is known about this cover yet, so the ask goes out"
    );
    assert_eq!(
        covers.requested(),
        vec![(id.clone(), path, COVER_THUMB)],
        "the service is asked for the track, at the box the row paints at"
    );
    assert_eq!(
        cache.in_flight().len(),
        1,
        "one outstanding ask, one marker"
    );
    assert!(
        cache
            .in_flight()
            .contains(&cover_cache_key(&id.0, COVER_THUMB)),
        "the marker is the composite the entry is written under — identity AND box"
    );
}

#[test]
fn forty_asks_while_one_is_outstanding_are_still_one_request() {
    // The repaint bug the in-flight set exists for: a row whose Cover has not
    // landed asked again on *every* frame, against an unbounded channel. One
    // request per outstanding `(identity, box)` is the whole fix, and the
    // marker has to survive until the answer arrives.
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    let id = TrackId("/music/slow.mp3".to_string());
    let path = PathBuf::from("/music/slow.mp3");

    assert!(
        cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB),
        "the first repaint asks"
    );
    for _frame in 0..39 {
        assert!(
            !cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB),
            "a suppressed ask says so"
        );
    }
    assert_eq!(
        covers.requested().len(),
        1,
        "forty repaints of one outstanding request are still one request"
    );

    // A different box for the same row is a different job, marker or not.
    assert!(
        cache.want_track(&covers, id.clone(), path.clone(), COVER_HERO),
        "a hero ask is its own job and goes out beside the thumbnail one"
    );
    assert_eq!(covers.requested().len(), 2);
}

#[test]
fn a_cover_that_arrived_at_hero_size_is_a_miss_for_a_thumbnail_request() {
    // The interesting part of the key. The composite is
    // `(identity, width, height)`, so a hero arrival and a thumbnail request
    // are different facts: reusing the hero pixels would draw the wrong
    // resolution and, worse, never fetch the right ones.
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    let id = TrackId("/music/art.mp3".to_string());
    let path = PathBuf::from("/music/art.mp3");

    assert!(cache.want_track(&covers, id.clone(), path.clone(), COVER_HERO));
    covers.serve(id.clone(), COVER_HERO, Some(cover(2)));
    let arrivals = cache.settle(&covers);
    assert_eq!(
        arrivals.iter().map(|a| &a.key).collect::<Vec<_>>(),
        vec![&cover_cache_key(&id.0, COVER_HERO)],
        "the arrival is filed under the box it was asked for"
    );

    assert!(
        cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB),
        "the hero arrival must not stand in for the thumbnail this row wants"
    );
    assert!(
        !cache.want_track(&covers, id.clone(), path.clone(), COVER_HERO),
        "and the hero box it DID arrive at is a hit, or the row would re-decode it"
    );
    assert_eq!(
        covers.requested().len(),
        2,
        "exactly the hero ask and the thumbnail ask — never a third"
    );
}

#[test]
fn an_answer_that_arrives_drops_the_marker_and_its_pixels_come_back_to_the_view() {
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    let id = TrackId("/music/land.mp3".to_string());
    let path = PathBuf::from("/music/land.mp3");

    assert!(cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB));
    assert!(
        !cache.in_flight().is_empty(),
        "the ask is marked while it is out"
    );

    covers.serve(id.clone(), COVER_THUMB, Some(cover(3)));
    let arrivals = cache.settle(&covers);
    assert_eq!(
        arrivals.len(),
        1,
        "one answer, one arrival for the View to upload"
    );
    assert_eq!(
        (arrivals[0].cover.width, arrivals[0].cover.height),
        (3, 3),
        "and the pixels arrive untouched — the decode happened on the worker"
    );
    assert!(
        cache.in_flight().is_empty(),
        "the answer is terminal, so the marker goes"
    );

    assert!(
        !cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB),
        "a cover that has arrived is never asked for again"
    );
    assert_eq!(covers.requested().len(), 1, "the settle asked for nothing");
}

#[test]
fn an_artless_answer_records_no_arrival_so_the_row_can_ask_again() {
    // `None` is a terminal outcome too. A row that asked once and turned out to
    // be artless must be free to ask again — otherwise a single artless answer
    // silences that `(identity, box)` for the rest of the session. What answers
    // the re-ask cheaply is the service's negative verdict, not the cache.
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    let id = TrackId("/music/artless.mp3".to_string());
    let path = PathBuf::from("/music/artless.mp3");

    assert!(cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB));
    assert_eq!(cache.in_flight().len(), 1);

    covers.serve(id.clone(), COVER_THUMB, None);
    assert!(
        cache.settle(&covers).is_empty(),
        "an artless answer has no pixels to hand the View"
    );
    assert!(
        cache.in_flight().is_empty(),
        "but it is still an answer, so the marker goes"
    );
    assert!(
        cache.arrived().is_empty(),
        "and nothing is recorded as arrived"
    );

    assert!(
        cache.want_track(&covers, id.clone(), path, COVER_THUMB),
        "so the row is free to ask again"
    );
    assert_eq!(covers.requested().len(), 2);
}

#[test]
fn the_in_flight_set_stays_within_its_cap() {
    // This set grows per `(identity, box)` rather than per cached texture, so
    // without its own cap it would be a slow leak across a scrolled library.
    // Bounded, and the evicted entries are the *oldest* markers, which cost one
    // re-request each and nothing worse.
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();

    for index in 0..3_000 {
        let path = PathBuf::from(format!("/music/{index:04}.mp3"));
        assert!(cache.want_track(&covers, TrackId::from_path(&path), path, COVER_THUMB));
    }
    assert_eq!(
        covers.requested().len(),
        3_000,
        "every distinct row still gets its request; the cap bounds the marker, not the asking"
    );
    assert_eq!(
        cache.in_flight().len(),
        COVER_IN_FLIGHT_CAP,
        "the marker set is held at the cap across a whole scrolled library"
    );
}

#[test]
fn an_arrival_the_view_dropped_is_a_miss_again() {
    // The cache and the texture map are separate facts and neither answers for
    // the other — so the View half is the only thing that can say what it
    // evicted, and it says so here. Without the report a scrolled-past row
    // would sit on a placeholder waiting for an answer nobody will send.
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    let id = TrackId("/music/evicted.mp3".to_string());
    let path = PathBuf::from("/music/evicted.mp3");

    assert!(cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB));
    covers.serve(id.clone(), COVER_THUMB, Some(cover(2)));
    let arrivals = cache.settle(&covers);
    let evicted: Vec<_> = arrivals.into_iter().map(|arrival| arrival.key).collect();
    assert!(
        !cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB),
        "while the View holds the texture, the row asks for nothing"
    );

    cache.forget(evicted);
    assert!(
        cache.want_track(&covers, id.clone(), path, COVER_THUMB),
        "once the View has dropped it, the row asks again"
    );
    assert_eq!(covers.requested().len(), 2);
}

#[test]
fn a_cleared_view_makes_every_arrival_a_miss_again() {
    // Settings → Library → "Clear Thumbnail cache" empties the whole texture
    // map when it settles, so the cache forgets every arrival with it.
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    for index in 0..2 {
        let id = TrackId(format!("/music/wiped{index}.mp3"));
        let path = PathBuf::from(&id.0);
        assert!(cache.want_track(&covers, id.clone(), path, COVER_THUMB));
        covers.serve(id, COVER_THUMB, Some(cover(2)));
    }
    // A third row is still on its way, so the clear has both an arrival to
    // forget and a marker that must survive it.
    let pending = TrackId("/music/wiped2.mp3".to_string());
    assert!(cache.want_track(
        &covers,
        pending.clone(),
        PathBuf::from(&pending.0),
        COVER_THUMB
    ));

    let arrivals = cache.settle(&covers);
    assert_eq!(
        arrivals.len(),
        2,
        "both served covers arrived; the third is out"
    );
    assert_eq!(cache.arrived().len(), 2);
    assert_eq!(
        cache.in_flight().len(),
        1,
        "and one marker is still outstanding"
    );

    cache.forget_arrivals();
    assert!(
        cache.arrived().is_empty(),
        "a cleared View leaves nothing recorded as arrived"
    );
    assert_eq!(
        cache.in_flight().len(),
        1,
        "and a clear does not touch the markers: a request still on its way is answered, \
         and its arrival is what rebuilds the entry"
    );
}

#[test]
fn dropping_every_marker_lets_a_row_ask_again_immediately() {
    // The artwork-policy toggle. A row that asked under the old policy has an
    // outstanding request whose answer is about to be wrong for the new one, and
    // leaving its marker would suppress the re-ask that the eviction exists to
    // cause.
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();
    let id = TrackId("/music/policy.mp3".to_string());
    let path = PathBuf::from("/music/policy.mp3");

    assert!(cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB));
    assert!(!cache.want_track(&covers, id.clone(), path.clone(), COVER_THUMB));

    cache.forget_in_flight();
    assert!(cache.in_flight().is_empty());
    assert!(
        cache.want_track(&covers, id.clone(), path, COVER_THUMB),
        "so the next repaint re-asks under the new policy"
    );
    assert_eq!(covers.requested().len(), 2);
}

#[test]
fn a_folder_row_asks_for_its_own_directory_once_and_paints_it_once_it_has_arrived() {
    let dir = std::path::Path::new("/music/boards");
    let covers = RecordingCovers::new();
    let mut cache = CoverCache::new();

    assert_eq!(
        cache.want_folder(&covers, dir, COVER_THUMB),
        None,
        "a cold folder has nothing to paint, so the row keeps its glyph"
    );
    assert_eq!(
        covers.requested(),
        vec![(TrackId::from_path(dir), dir.to_path_buf(), COVER_THUMB)],
        "the folder row asks for its OWN directory, at the box it paints at"
    );

    // A second ask while the first is out is still one request: the folder
    // request is deduped on the same composite every Track request uses.
    assert_eq!(cache.want_folder(&covers, dir, COVER_THUMB), None);
    assert_eq!(
        covers.requested().len(),
        1,
        "a folder already asked sends nothing"
    );

    let key = cover_cache_key(&dir.to_string_lossy(), COVER_THUMB);
    covers.serve(TrackId::from_path(dir), COVER_THUMB, Some(cover(2)));
    assert_eq!(cache.settle(&covers).len(), 1);

    assert_eq!(
        cache.want_folder(&covers, dir, COVER_THUMB),
        Some(key),
        "the arrival is handed back under the key the View filed it at"
    );
    assert_eq!(
        covers.requested().len(),
        1,
        "a folder already cached at this box sends nothing"
    );
}
