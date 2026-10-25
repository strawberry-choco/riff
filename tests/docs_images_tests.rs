//! README screenshot generator.
//!
//! Renders the real application shell — `RiffApp` over a composed
//! `AppRuntime`, so the actual store, cover service and column browser are in
//! the frame — into PNGs under `docs/images/`. These are the pixels the app
//! paints in a window; only the window is headless.
//!
//! It is `#[ignore]`d: it exists to refresh the README art, not to assert
//! behavior, and CI has no reason to write images.
//!
//! ```bash
//! python3 tools/make-demo-library.py   # build the synthetic library first
//! cargo test -p riff-tests --test integration -- --ignored render_readme
//! ```
//!
//! The library it photographs is synthetic — `tools/make-demo-library.py`
//! writes the tagged FLAC files and the `seed.tsv` beside them under
//! `.scratch/demo-library/`. Nothing here touches a real collection, and the
//! seed file is gitignored, so a missing one is a setup gap rather than a
//! failure to fix in this file.

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use riff_backend::app::state::{
        BrowseMode, BrowserSelection, LibrarySection, PlaybackState, ViewMode,
    };
    use riff_backend::app::store::{LibraryMutationStore, PlaylistStore, SettingsStore};
    use riff_backend::composition::AppRuntime;
    use riff_backend::domain::{Track, TrackId, TrackMetadata};
    use riff_gui::ui::RiffApp;
    use riff_infra::store::SqliteStore;
    use riff_library::app::cover_service::Covers;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    /// Wide enough for the sidebar, three browser columns, and the inspector
    /// to all sit at their preferred widths.
    const WINDOW: egui::Vec2 = egui::vec2(1600.0, 1000.0);

    /// Frame budget between a state change and the capture. A render settles
    /// when the cover service goes quiet — nothing in flight, nothing being
    /// re-requested. The pause matters as much as the count: the app
    /// re-requests every cover still missing from its texture cache on every
    /// frame, so a tight loop hands the cover worker a fresh backlog faster
    /// than it can resolve one, and the large boxes at the back of the queue
    /// never land.
    const MIN_SETTLE_FRAMES: usize = 12;
    const MAX_SETTLE_FRAMES: usize = 60;
    const QUIET_FRAMES: usize = 5;
    const FRAME_PAUSE: Duration = Duration::from_millis(250);

    /// One row of `demo-library/seed.tsv`.
    struct DemoTrack {
        path: PathBuf,
        title: String,
        artist: String,
        album: String,
        album_artist: String,
        genre: String,
        year: u32,
        track_number: u32,
        duration: Duration,
        play_count: u32,
        favorite: bool,
        added_days_ago: u64,
        last_played_days_ago: Option<u64>,
        gain: f32,
        peak: f32,
        album_gain: f32,
    }

    impl DemoTrack {
        fn id(&self) -> TrackId {
            TrackId(self.path.to_string_lossy().into_owned())
        }
    }

    /// The cover port, counting the traffic that passes through it. The count
    /// is what [`capture`] waits on: a frame that neither re-requested nor
    /// delivered anything means every visible cover has landed.
    struct CountedCovers {
        inner: Mutex<Box<dyn Covers>>,
        traffic: Arc<AtomicUsize>,
    }

    impl Covers for CountedCovers {
        fn request(&self, track_id: TrackId, path: PathBuf, size: RequestedSize) {
            self.traffic.fetch_add(1, Ordering::Relaxed);
            self.lock().request(track_id, path, size);
        }

        fn request_folder(&self, folder: &Path, size: RequestedSize) {
            self.traffic.fetch_add(1, Ordering::Relaxed);
            self.lock().request_folder(folder, size);
        }

        fn poll(&self) -> CoverResults {
            let results = self.lock().poll();
            self.traffic.fetch_add(results.len(), Ordering::Relaxed);
            results
        }
    }

    /// The port's vocabulary, named once so the wrapper stays legible.
    type RequestedSize = riff_backend::app::traits::RequestedSize;
    type CoverResults = Vec<(
        TrackId,
        riff_backend::app::traits::RequestedSize,
        Option<riff_backend::app::traits::DecodedCover>,
    )>;

    impl CountedCovers {
        fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn Covers>> {
            self.inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the test crate lives one directory below the workspace root")
            .to_path_buf()
    }

    /// The synthetic library, or `RIFF_DEMO_LIBRARY` when the caller keeps it
    /// somewhere else.
    fn demo_root() -> PathBuf {
        std::env::var_os("RIFF_DEMO_LIBRARY")
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_root().join(".scratch").join("demo-library"))
    }

    fn output_dir() -> PathBuf {
        std::env::var_os("RIFF_SCREENSHOT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_root().join("docs").join("images"))
    }

    fn days_ago(days: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(days * 86_400)
    }

    fn locked<T>(cell: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        cell.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Read the seed table. A row with the wrong field count is a truncated
    /// file rather than a track to skip.
    fn read_seed(root: &Path) -> Vec<DemoTrack> {
        let path = root.join("seed.tsv");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "no demo library at {}: {e}\n\
                 generate one first, or point RIFF_DEMO_LIBRARY at an existing one",
                path.display()
            )
        });
        text.lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let f: Vec<&str> = line.split('\t').collect();
                let [
                    p,
                    title,
                    artist,
                    album,
                    album_artist,
                    genre,
                    year,
                    number,
                    seconds,
                    plays,
                    favorite,
                    added,
                    last_played,
                    gain,
                    peak,
                    album_gain,
                ] = f.as_slice()
                else {
                    panic!("seed row has {} fields, expected 16: {line}", f.len());
                };
                DemoTrack {
                    path: PathBuf::from(p),
                    title: (*title).into(),
                    artist: (*artist).into(),
                    album: (*album).into(),
                    album_artist: (*album_artist).into(),
                    genre: (*genre).into(),
                    year: year.parse().expect("a numeric year"),
                    track_number: number.parse().expect("a numeric track number"),
                    duration: Duration::from_secs(seconds.parse().expect("a numeric length")),
                    play_count: plays.parse().expect("a numeric play count"),
                    favorite: *favorite == "1",
                    added_days_ago: added.parse().expect("a numeric added date"),
                    last_played_days_ago: last_played.parse().ok(),
                    gain: gain.parse().expect("a numeric ReplayGain"),
                    peak: peak.parse().expect("a numeric ReplayGain peak"),
                    album_gain: album_gain.parse().expect("a numeric ReplayGain album gain"),
                }
            })
            .collect()
    }

    /// Commit the demo library into a fresh Application Store: the scan batch
    /// carries the tags, then the user facts — favorites and plays — land
    /// through the ports that write them for real.
    fn seed_store(store: &mut SqliteStore, rows: &[DemoTrack], root: &Path) -> Vec<TrackId> {
        store
            .save_library_paths(&[root.to_path_buf()])
            .expect("the library root registers");

        let mut ids = Vec::with_capacity(rows.len());
        for chunk in rows.chunks(10) {
            let tracks: Vec<Track> = chunk
                .iter()
                .map(|row| Track {
                    id: row.id(),
                    file_path: row.path.clone(),
                    metadata: TrackMetadata {
                        title: Some(row.title.clone()),
                        artist: Some(row.artist.clone()),
                        album: Some(row.album.clone()),
                        album_artist: Some(row.album_artist.clone()),
                        track_number: Some(row.track_number),
                        disc_number: Some(1),
                        genre: Some(row.genre.clone()),
                        year: Some(row.year),
                        composer: None,
                        comment: None,
                        replaygain_track_gain: Some(row.gain),
                        replaygain_track_peak: Some(row.peak),
                        replaygain_album_gain: Some(row.album_gain),
                    },
                    duration: Some(row.duration),
                    sample_rate: Some(44_100),
                    channels: Some(2),
                    // Play count and last-played arrive through their own
                    // ports below, which is where the store keeps them.
                    play_count: 0,
                    last_played: None,
                    date_added: Some(days_ago(row.added_days_ago)),
                    favorite: false,
                    search_text: String::new(),
                })
                .collect();
            store
                .apply_scan_batch(&tracks)
                .expect("the demo library commits");
            ids.extend(tracks.iter().map(|track| track.id.clone()));
        }

        for (row, id) in rows.iter().zip(&ids) {
            if row.favorite {
                store
                    .set_track_favorite(id, true)
                    .expect("a favorite commits");
            }
            // One transaction per counted play; the last one stamps the
            // "last played" date the smart lists sort on.
            for counted in 0..row.play_count {
                let when = match (row.last_played_days_ago, counted + 1 == row.play_count) {
                    (Some(days), true) => days_ago(days),
                    _ => days_ago(row.added_days_ago + u64::from(counted)),
                };
                store.record_track_played(id, when).expect("a play commits");
            }
        }
        ids
    }

    /// Two named playlists, so the sidebar's PLAYLISTS section is not empty.
    fn seed_playlists(store: &mut SqliteStore, rows: &[DemoTrack], ids: &[TrackId]) {
        let pick = |genres: &[&str], limit: usize| -> Vec<TrackId> {
            rows.iter()
                .zip(ids)
                .filter(|(row, _)| genres.contains(&row.genre.as_str()))
                .take(limit)
                .map(|(_, id)| id.clone())
                .collect()
        };
        store
            .create_playlist("Night Drive", &pick(&["Synthwave", "Shoegaze"], 11))
            .expect("the first playlist commits");
        store
            .create_playlist("Sunday Reset", &pick(&["Jazz", "Folk"], 9))
            .expect("the second playlist commits");
    }

    /// The real shell over the real runtime, at screenshot resolution.
    ///
    /// The runtime the Composition Root spawned stays whole: the one override
    /// is field assignment — the Cover front end is wrapped in the counting
    /// adapter in place — and the bundle moves whole into the app's test twin.
    fn build_shell(
        mut runtime: AppRuntime,
        traffic: Arc<AtomicUsize>,
    ) -> (
        egui_kittest::Harness<'static, RiffApp>,
        Arc<Mutex<riff_backend::app::state::PlaybackSession>>,
        Arc<Mutex<riff_backend::app::state::LibrarySession>>,
    ) {
        let playback = Arc::clone(&runtime.playback);
        let library = Arc::clone(&runtime.library);
        let covers = runtime.covers;
        runtime.covers = Box::new(CountedCovers {
            inner: Mutex::new(covers),
            traffic,
        });

        let (app, _visibility_tx) = RiffApp::new_for_test(runtime);

        let harness = egui_kittest::Harness::builder()
            .with_size(WINDOW)
            .build_eframe(|cc| {
                // The same font install `main.rs` performs; the shell's named
                // Inter families are required by the render path and a frame
                // panics without them.
                riff_gui::ui::fonts::configure_fonts(&cc.egui_ctx);
                app
            });

        (harness, playback, library)
    }

    /// Settle the frame, then write it out.
    fn capture(
        harness: &mut egui_kittest::Harness<'static, RiffApp>,
        dir: &Path,
        name: &str,
        traffic: &AtomicUsize,
    ) -> Result<(), String> {
        let mut quiet = 0_usize;
        let mut frames = 0_usize;
        for frame in 0..MAX_SETTLE_FRAMES {
            let before = traffic.load(Ordering::Relaxed);
            harness.step();
            std::thread::sleep(FRAME_PAUSE);
            frames = frame + 1;
            quiet = if traffic.load(Ordering::Relaxed) == before {
                quiet + 1
            } else {
                0
            };
            if frame >= MIN_SETTLE_FRAMES && quiet >= QUIET_FRAMES {
                break;
            }
        }
        if frames == MAX_SETTLE_FRAMES {
            return Err(format!(
                "{name}: the cover service never went quiet, so some artwork is missing"
            ));
        }
        println!("{name}: settled after {frames} frames");
        harness
            .render()?
            .save(dir.join(format!("{name}.png")))
            .map_err(|e| e.to_string())
    }

    /// Point the library session at one album, with one of its tracks selected,
    /// so the columns, the breadcrumb and the inspector all have content. The
    /// drill-down path comes from the caller because it is per-facet: the
    /// genre section sits one level deeper than the artist one.
    fn show_album(
        library: &Mutex<riff_backend::app::state::LibrarySession>,
        row: &DemoTrack,
        section: LibrarySection,
        path: Vec<BrowserSelection>,
    ) {
        let mut library = locked(library);
        library.view_mode = ViewMode::Library;
        library.browse_mode = BrowseMode::Library;
        library.library_section = section;
        library.search_query.clear();
        library.browser_path = path;
        library.selected_track = Some(row.id());
    }

    /// The artist/album drill-down under the facet the shot is about.
    fn drill(row: &DemoTrack, through_genre: bool) -> Vec<BrowserSelection> {
        let album = BrowserSelection::Album {
            artist: row.album_artist.clone(),
            title: row.album.clone(),
        };
        if through_genre {
            vec![
                BrowserSelection::Genre(row.genre.clone()),
                BrowserSelection::Artist(row.album_artist.clone()),
                album,
            ]
        } else {
            vec![BrowserSelection::Artist(row.album_artist.clone()), album]
        }
    }

    /// Start "playing" an album: the queue, the position, and the state the
    /// transport renders from. The audio engine is untouched — a frame only
    /// ever reads what the session says.
    fn start_playing(
        playback: &Mutex<riff_backend::app::state::PlaybackSession>,
        album: &[&DemoTrack],
        index: usize,
    ) {
        let mut playback = locked(playback);
        playback.queue = riff_backend::app::state::PlaybackQueue::new(
            album.iter().map(|row| row.id()).collect(),
        );
        playback.queue.current_index = Some(index);
        playback.playback_state = PlaybackState::Playing;
        playback.current_position = riff_backend::domain::PlaybackPosition {
            current: album[index].duration / 3,
            total: Some(album[index].duration),
        };
        playback.current_volume = 0.72;
    }

    #[test]
    #[ignore = "refreshes the README screenshots; run it explicitly"]
    fn render_readme_screenshots() {
        let root = demo_root();
        let rows = read_seed(&root);
        let out = output_dir();
        std::fs::create_dir_all(&out).expect("the screenshot directory is creatable");

        // The store lives in a scratch dir that has to outlive every handle
        // into it, so it is bound rather than dropped at the end of a helper.
        let dir = tempfile::tempdir().expect("a scratch directory");
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("a fresh store opens");
        let ids = seed_store(&mut store, &rows, &root);
        seed_playlists(&mut store, &rows, &ids);
        drop(store);

        let (runtime, mut lifecycle) =
            AppRuntime::spawn(&db_path).expect("the runtime spawns over the seeded store");
        let traffic = Arc::new(AtomicUsize::new(0));
        let _slot = crate::harness_gate::harness_slot();
        let (mut harness, playback, library) = build_shell(runtime, traffic.clone());

        let album = |name: &str| -> Vec<&DemoTrack> {
            rows.iter().filter(|row| row.album == name).collect()
        };

        // 1. The library, drilled into an album: entity columns, the track
        //    listing, and the inspector with its cover and tags.
        let cartography = album("Neon Cartography");
        show_album(
            &library,
            cartography[0],
            LibrarySection::Artists,
            drill(cartography[0], false),
        );
        capture(&mut harness, &out, "riff-library", &traffic).expect("the library view renders");

        // 2. The same collection through the genre facet — the browse path
        //    that runs four columns deep.
        let bloom = album("Static Bloom");
        show_album(
            &library,
            bloom[1],
            LibrarySection::Genres,
            drill(bloom[1], true),
        );
        capture(&mut harness, &out, "riff-genres", &traffic).expect("the genre drill-down renders");

        // 3. Now Playing: the large cover, the full metadata set, and Up Next.
        start_playing(&playback, &bloom, 1);
        locked(&library).view_mode = ViewMode::NowPlaying;
        capture(&mut harness, &out, "riff-now-playing", &traffic)
            .expect("the now playing view renders");

        // 4. A playlist opened from the sidebar over the queue panel: the
        //    curator's own view of the collection.
        locked(&library).view_mode = ViewMode::Library;
        harness.step();
        harness.get_by_label("Night Drive").click();
        // The harness paints the pointer wherever it last acted; send it off
        // the viewport so the shot reads as a window rather than a screenshot
        // of a screenshot.
        harness.hover_at(egui::pos2(-64.0, -64.0));
        locked(&library).queue_open = true;
        capture(&mut harness, &out, "riff-playlist", &traffic).expect("the playlist view renders");

        println!("screenshots written to {}", out.display());
        lifecycle.shutdown();
    }
}
