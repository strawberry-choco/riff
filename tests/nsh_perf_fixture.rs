//! The N16 perf fixture. Ignored by default: it seeds a 50k-track mock
//! library and 500 covers, then prints the numbers the rewrite plan's perf
//! protocol records (cold listing read, a warm 300-row window, and the cover
//! memory held). It prints; it asserts no budget, because the budget is the
//! live app's and is recorded in the evidence log, not a unit threshold.
//!
//! Run with:
//!
//! ```bash
//! cargo test -p riff-tests nsh_perf_fixture -- --ignored --nocapture
//! ```

use std::time::Instant;

use nsh_core::element::ImageSrc;
use riff_backend::app::state::TrackSort;
use riff_backend::app::views::SessionViews;
use riff_backend::domain::TrackId;

use crate::mocks::MockLibraryQueryStore;
use crate::test_utils::create_test_track_with_metadata;

/// How many tracks the fixture seeds.
const TRACKS: usize = 50_000;
/// How many covers the fixture builds.
const COVERS: usize = 500;
/// The cover box the thumbnail path uses.
const COVER_BOX: u32 = 56;
/// The window the scroll probe reads.
const WINDOW: usize = 300;

fn seeded_tracks() -> Vec<riff_backend::domain::Track> {
    (0..TRACKS)
        .map(|at| {
            create_test_track_with_metadata(
                &format!("/music/perf/{at:05}.wav"),
                &format!("/music/perf/{at:05}.wav"),
                &format!("Artist {:03}", at / 100),
                &format!("Track {at:05}"),
                &format!("Album {:03}", at / 10),
            )
        })
        .collect()
}

#[test]
#[ignore = "seeds 50k tracks and 500 covers; run with --ignored --nocapture"]
fn the_50k_fixture_prints_its_numbers() {
    let build = Instant::now();
    let tracks = seeded_tracks();
    let library: std::collections::HashMap<TrackId, _> =
        tracks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    let mut views = SessionViews::new(
        Box::new(MockLibraryQueryStore {
            library,
            flat: tracks,
            library_counts: riff_persistence::store::LibraryCounts {
                tracks: TRACKS,
                artists: TRACKS / 100,
                albums: TRACKS / 10,
                genres: 7,
            },
            ..MockLibraryQueryStore::default()
        }),
        Box::new(crate::mocks::MockPlaylistStore::default()),
    );
    println!("fixture build: {} ms", build.elapsed().as_millis());

    // Cold listing read: the count, then the first page.
    let cold = Instant::now();
    let total = views.track_count("", TrackSort::NumberAsc);
    let first = views.track_row("", TrackSort::NumberAsc, 0);
    println!(
        "cold count+first row ({total} tracks, first {}): {} ms",
        first.is_some(),
        cold.elapsed().as_millis()
    );

    // A warm window: 300 sequential row reads, the prefetch handler's shape.
    let warm = Instant::now();
    let rows = (0..WINDOW)
        .filter_map(|at| views.track_row("", TrackSort::NumberAsc, at))
        .count();
    let per_row = warm.elapsed().as_secs_f64() * 1000.0 / WINDOW as f64;
    println!("warm window: {rows} rows, {per_row:.3} ms/row");

    // The cover memory the View half would hold at the thumbnail box.
    let covers = Instant::now();
    let bytes = usize::try_from(COVER_BOX).unwrap_or(56).pow(2) * 4;
    let held: Vec<ImageSrc> = (0..COVERS)
        .map(|at| {
            ImageSrc::rgba(
                COVER_BOX,
                COVER_BOX,
                vec![u8::try_from(at % 256).unwrap_or(0); bytes],
            )
        })
        .collect();
    println!(
        "covers: {} held, {:.1} MB RGBA, {} ms to build",
        held.len(),
        (held.len() * bytes) as f64 / (1024.0 * 1024.0),
        covers.elapsed().as_millis()
    );

    // RSS is a live-process number: the in-process mock cannot read it
    // without an OS dependency, so the evidence log records the running
    // app's RSS beside these numbers.
    println!("rss: record from the live app's evidence log (same fixture)");
}
