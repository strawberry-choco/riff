//! One-off README screenshot generator (not part of the regression suite).
//!
//! Renders three full-app screenshots headlessly through `egui_kittest` — the
//! same mechanism [`golden_tests`] uses — over a composed `RiffApp` seeded with
//! invented demo content, and writes PNGs into `docs/assets/readme/` for the
//! README's image references. The artists, albums, and playlist shown are all
//! invented for these screenshots (they are not real bands, and none of them
//! reuse the golden suite's demo rows).
//!
//! # Running
//!
//! The tests are `#[ignore]`d AND gated behind a `README_SHOTS=1` env var, so
//! neither an ordinary `cargo test` nor a `--ignored` run without the var can
//! quietly rewrite the docs assets:
//!
//! ```bash
//! README_SHOTS=1 cargo test -p riff-tests --test integration \
//!     readme_screenshots -- --ignored --test-threads=1
//! ```
//!
//! Output (workspace-relative): `docs/assets/readme/{library,playlist-queue,
//! now-playing}.png` at 1280x800, dark theme, pixels-per-point 1.
//!
//! # Determinism
//!
//! These are documentation assets, not golden baselines, but they follow the
//! golden suite's determinism contract all the same (see
//! `docs/engineering/golden-image-testing.md`): only the vendored Inter faces
//! are installed (never a system CJK fallback), animation time / scroll
//! animation / cursor blink are pinned on both theme slots after construction
//! ([`pin_settled_time`]), every screenshot renders a fixed number of frames,
//! and the fake cursor is removed before the final render. Cover art is
//! generated procedurally from the album name's hash, so the same album always
//! yields the same art on every machine.

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use riff_backend::app::cover_service::Covers;
    use riff_backend::app::state::{
        BrowserSelection, LibrarySection, LibrarySession, PlaybackSession, ViewMode,
    };
    use riff_backend::app::store::{
        LibraryQueryStore, PlaylistStore as PlaylistStoreTrait, StoreGeneration,
    };
    use riff_backend::app::views::SessionViews;
    use riff_backend::domain::{
        Album, Artist, GenreCount, PlaybackPosition, PlaybackQueue, PlaybackState, Playlist,
        PlaylistId, RepeatMode, SmartPlaylistKind, Track, TrackId,
    };
    use riff_gui::ui::RiffApp;
    use riff_gui::ui::fonts::{self, INTER_FACES};
    use riff_library::app::traits::{DecodedCover, RequestedSize};
    use riff_persistence::errors::StoreError;
    use riff_persistence::store::{Page, PlaylistEntry};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // Note: `MockCovers` is deliberately NOT used here — its empty `poll`
    // would paint no cover art at all. `ProceduralCovers` below takes its
    // place in the composed shell.
    use crate::mocks::{
        MockLibraryMutationStore, MockLibraryQueryStore, MockScans, MockSettingsStore,
        MockTagEdits, MockTransport,
    };

    /// Where the PNGs land: `docs/assets/readme/` in the workspace root. The
    /// test binary runs with the package dir (`tests/`) as its cwd, so anchor
    /// on `CARGO_MANIFEST_DIR` rather than the cwd.
    fn output_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/assets/readme")
    }

    /// Belt-and-braces gate: every screenshot test early-returns unless
    /// `README_SHOTS=1`, on top of `#[ignore]`.
    fn shots_enabled() -> bool {
        std::env::var_os("README_SHOTS").is_some_and(|v| v == "1")
    }

    // --- Harness helpers (copied verbatim from golden_tests.rs) ------------------

    /// Deterministic font definitions for headless rendering: the vendored
    /// Inter faces only. Never scans system CJK fonts, whose presence varies
    /// per machine — output must rasterize identically everywhere.
    fn inter_only_font_definitions() -> egui::FontDefinitions {
        let mut fonts = egui::FontDefinitions::default();
        for (key, bytes) in INTER_FACES {
            fonts
                .font_data
                .insert((*key).to_owned(), egui::FontData::from_static(bytes).into());
        }
        if let Some(chain) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            chain.insert(0, fonts::INTER_PRIMARY_KEY.to_owned());
        }
        fonts
            .families
            .insert(fonts::family_medium(), vec!["inter-medium".to_owned()]);
        fonts
            .families
            .insert(fonts::family_semibold(), vec!["inter-semibold".to_owned()]);
        fonts
            .families
            .insert(fonts::family_bold(), vec!["inter-bold".to_owned()]);
        fonts
    }

    /// Pin the three time-driven `egui::Style` fields on **both** theme slots:
    /// tween duration, programmatic scroll animation, cursor blink. Copied
    /// verbatim from `golden_tests.rs`; see the rationale there.
    fn pin_settled_time(ctx: &egui::Context) {
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.scroll_animation = egui::style::ScrollAnimation::none();
            style.visuals.text_cursor.blink = false;
        });
    }

    /// How many distinct RGBA colors a frame painted — the blankness check the
    /// golden composed-shell assertions use (a real frame paints far more than
    /// 40 distinct colors).
    fn distinct_colors(image: &image::RgbaImage) -> usize {
        let mut seen = std::collections::HashSet::new();
        for pixel in image.pixels() {
            seen.insert(pixel.0);
        }
        seen.len()
    }

    // --- Demo collection ---------------------------------------------------------

    /// `(track number, title, seconds)` for one demo album's songs.
    type DemoSong = (u32, &'static str, u64);

    /// One invented demo album: artist, title, year, genre, and its tracks.
    type DemoAlbum = (
        &'static str,
        &'static str,
        u32,
        &'static str,
        &'static [DemoSong],
    );

    const DEMO_ALBUMS: &[DemoAlbum] = &[
        (
            "Aurora Vale",
            "Slow Current",
            2023,
            "Ambient",
            &[
                (1, "First Light", 252),
                (2, "Low Tide", 303),
                (3, "Glass Coast", 238),
                (4, "Salt Air", 285),
            ],
        ),
        (
            "Cassette Moon",
            "Analog Hearts",
            2021,
            "Electronic",
            &[
                (1, "Neon Rain", 227),
                (2, "Signal Fade", 271),
                (3, "Paper Moon", 213),
            ],
        ),
        (
            "The Paper Lanterns",
            "Harbor Lights",
            2022,
            "Indie",
            &[
                (1, "Amber Glow", 227),
                (2, "Quiet Hours", 258),
                (3, "Nightfall", 312),
            ],
        ),
        (
            "Ember & Oak",
            "Night Bloom",
            2020,
            "Folk",
            &[(1, "Ember Line", 235), (2, "Hollow Pine", 264)],
        ),
        (
            "Velvet Harbor",
            "Static Fields",
            2024,
            "Electronic",
            &[
                (1, "Driftwood", 242),
                (2, "Zenith Decay", 221),
                (3, "Wide Open", 306),
            ],
        ),
    ];

    /// How many times each demo track has "been played": deterministic filler
    /// so the Most Played smart count and the row play-count clusters have
    /// plausible values. `0` = never played.
    fn play_count_for(album_title: &str, track_no: u32) -> u32 {
        match album_title {
            "Slow Current" => [12, 9, 7, 4][track_no as usize - 1],
            "Analog Hearts" => [8, 6, 5][track_no as usize - 1],
            "Harbor Lights" => [14, 11, 8][track_no as usize - 1],
            "Night Bloom" => [3, 2][track_no as usize - 1],
            "Static Fields" => [6, 1, 0][track_no as usize - 1],
            _ => 0,
        }
    }

    /// The demo files' root: a fixed directory under the system temp dir.
    /// The playlist (and track rows) run a read-time filesystem check — an
    /// entry is only "valid" when its file exists — so the generator
    /// materializes empty stand-in audio files there at test start
    /// ([`materialize_demo_files`]). Nothing ever reads the bytes: covers
    /// come from the procedural fake and playback from a mock transport.
    /// Idempotent, so parallel test functions share it safely.
    fn demo_base() -> PathBuf {
        std::env::temp_dir().join("riff-readme-demo-music")
    }

    /// Create an empty stand-in file for every demo track, plus its parent
    /// directories. Empty files carry no tags and are never decoded.
    fn materialize_demo_files() {
        for track in demo_tracks() {
            std::fs::create_dir_all(track.file_path.parent().expect("track paths have parents"))
                .expect("creating the demo album directories must work");
            std::fs::File::create(&track.file_path)
                .expect("creating the demo stand-in files must work");
        }
    }

    /// Build one demo Track. Identity is the full path
    /// (`<base>/<artist>/<album>/NN <title>.flac`), the same rule the real
    /// store uses. Two favorites are marked so rows show their heart affordance.
    fn demo_track(artist: &str, album_title: &str, track_no: u32, title: &str, secs: u64) -> Track {
        let file_name = format!("{track_no:02} {title}.flac");
        let path = demo_base().join(artist).join(album_title).join(file_name);
        let favorite = matches!(title, "Low Tide" | "Amber Glow");
        Track {
            id: TrackId(path.to_string_lossy().to_string()),
            file_path: path.clone(),
            metadata: riff_backend::domain::TrackMetadata {
                title: Some(title.to_string()),
                artist: Some(artist.to_string()),
                album: Some(album_title.to_string()),
                album_artist: Some(artist.to_string()),
                track_number: Some(track_no),
                disc_number: Some(1),
                genre: DEMO_ALBUMS
                    .iter()
                    .find(|(_, a, ..)| *a == album_title)
                    .map(|(.., genre, _)| genre.to_string()),
                year: DEMO_ALBUMS
                    .iter()
                    .find(|(_, a, ..)| *a == album_title)
                    .map(|(_, _, year, ..)| *year),
                ..Default::default()
            },
            duration: Some(Duration::from_secs(secs)),
            sample_rate: Some(44_100),
            channels: Some(2),
            play_count: play_count_for(album_title, track_no),
            last_played: None,
            date_added: None,
            favorite,
            search_text: format!("{title} {artist} {album_title} {artist}").to_lowercase(),
        }
    }

    /// Every demo track, in canonical path-ascending order.
    fn demo_tracks() -> Vec<Track> {
        let mut tracks: Vec<Track> = DEMO_ALBUMS
            .iter()
            .flat_map(|(artist, album, .., songs)| {
                songs
                    .iter()
                    .map(|(no, title, secs)| demo_track(artist, album, *no, title, *secs))
            })
            .collect();
        tracks.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        tracks
    }

    /// The playback queue for the "playing" screenshots: Harbor Lights front to
    /// back, then Night Bloom and a Slow Current favorite — six entries with
    /// track 1 playing.
    fn demo_queue_ids() -> Vec<TrackId> {
        [
            ("The Paper Lanterns", "Harbor Lights", 1),
            ("The Paper Lanterns", "Harbor Lights", 2),
            ("The Paper Lanterns", "Harbor Lights", 3),
            ("Ember & Oak", "Night Bloom", 1),
            ("Ember & Oak", "Night Bloom", 2),
            ("Aurora Vale", "Slow Current", 2),
        ]
        .iter()
        .map(|(artist, album, no)| {
            let title = DEMO_ALBUMS
                .iter()
                .find(|(_, a, ..)| *a == *album)
                .and_then(|(.., songs)| songs.iter().find(|(n, ..)| n == no))
                .map(|(_, title, _)| *title)
                .expect("queue references a demo track");
            id_of(artist, album, *no, title)
        })
        .collect()
    }

    /// The demo track id for `(artist, album, track_no, title)` — the same
    /// path shape [`demo_track`] builds.
    fn id_of(artist: &str, album: &str, track_no: u32, title: &str) -> TrackId {
        TrackId(
            demo_base()
                .join(artist)
                .join(album)
                .join(format!("{track_no:02} {title}.flac"))
                .to_string_lossy()
                .to_string(),
        )
    }

    /// The "Night Drive" playlist: six entries spanning four of the albums.
    const NIGHT_DRIVE: &[(&str, &str, u32, &str)] = &[
        ("Aurora Vale", "Slow Current", 2, "Low Tide"),
        ("Cassette Moon", "Analog Hearts", 1, "Neon Rain"),
        ("The Paper Lanterns", "Harbor Lights", 3, "Nightfall"),
        ("Ember & Oak", "Night Bloom", 1, "Ember Line"),
        ("Aurora Vale", "Slow Current", 4, "Salt Air"),
        ("Velvet Harbor", "Static Fields", 1, "Driftwood"),
    ];

    /// The playlist id is constructed directly (not via the time-stamped
    /// `PlaylistId::new`) so the output is deterministic.
    fn night_drive_id() -> PlaylistId {
        PlaylistId("night-drive".to_string())
    }

    fn night_drive_playlist() -> Playlist {
        Playlist {
            id: night_drive_id(),
            name: "Night Drive".to_string(),
            tracks: NIGHT_DRIVE
                .iter()
                .map(|(artist, album, no, title)| id_of(artist, album, *no, title))
                .collect(),
            created: None,
        }
    }

    // --- Fake ports ----------------------------------------------------------------

    /// A `LibraryQueryStore` that answers per artist and per album — the one
    /// thing the shared [`MockLibraryQueryStore`] cannot do (its canned
    /// `albums`/`album_tracks` answers are artist- and album-independent, so
    /// every artist column row would list the same album). Everything else
    /// delegates to the seeded inner mock untouched.
    struct DemoQueryStore {
        inner: MockLibraryQueryStore,
        albums_by_artist: HashMap<String, Vec<Album>>,
        tracks_by_album: HashMap<(String, String), Vec<Track>>,
    }

    impl LibraryQueryStore for DemoQueryStore {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            self.inner.get_track(id)
        }

        fn metadata_version(&self) -> Result<u32, StoreError> {
            self.inner.metadata_version()
        }

        fn tracks_page(&self, offset: usize, limit: usize) -> Result<Page<Track>, StoreError> {
            self.inner.tracks_page(offset, limit)
        }

        fn library_counts(&self) -> Result<riff_persistence::store::LibraryCounts, StoreError> {
            self.inner.library_counts()
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            self.inner.all_track_ids()
        }

        fn search_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Page<Track>, StoreError> {
            self.inner.search_page(query, offset, limit)
        }

        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            self.inner.all_artists()
        }

        fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError> {
            Ok(self
                .albums_by_artist
                .get(artist)
                .cloned()
                .unwrap_or_default())
        }

        fn album_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(self
                .tracks_by_album
                .get(&(album_artist.to_string(), album_title.to_string()))
                .cloned()
                .unwrap_or_default())
        }

        fn folder_has_audio(&self, folder: &Path) -> Result<bool, StoreError> {
            self.inner.folder_has_audio(folder)
        }

        fn folder_has_search_match(&self, folder: &Path, query: &str) -> Result<bool, StoreError> {
            self.inner.folder_has_search_match(folder, query)
        }

        fn track_ids_in_folder_tree(&self, folder: &Path) -> Result<Vec<TrackId>, StoreError> {
            self.inner.track_ids_in_folder_tree(folder)
        }

        fn tracks_in_folder(&self, folder: &Path) -> Result<Vec<Track>, StoreError> {
            self.inner.tracks_in_folder(folder)
        }

        fn folder_track_count(&self, folder: &Path) -> Result<usize, StoreError> {
            self.inner.folder_track_count(folder)
        }

        fn last_full_scan(
            &self,
        ) -> Result<Option<riff_backend::app::store::FullScanSummary>, StoreError> {
            self.inner.last_full_scan()
        }

        fn subdirs_with_audio(&self, folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
            self.inner.subdirs_with_audio(folder)
        }

        fn smart_playlist(
            &self,
            kind: SmartPlaylistKind,
            limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            self.inner.smart_playlist(kind, limit)
        }

        fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError> {
            self.inner.smart_list_counts()
        }

        fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError> {
            self.inner.genre_counts()
        }

        fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError> {
            self.inner.artists_in_genre(genre)
        }

        fn artist_albums_in_genre(
            &self,
            artist: &str,
            genre: &str,
        ) -> Result<Vec<Album>, StoreError> {
            self.inner.artist_albums_in_genre(artist, genre)
        }

        fn album_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.inner
                .album_tracks_in_genre(album_artist, album_title, genre)
        }

        fn hit_albums_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Page<Album>, StoreError> {
            self.inner.hit_albums_page(query, offset, limit)
        }

        fn hit_artists_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Page<Artist>, StoreError> {
            self.inner.hit_artists_page(query, offset, limit)
        }

        fn album_hit_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.inner
                .album_hit_tracks(album_artist, album_title, query)
        }

        fn album_is_name_hit(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<bool, StoreError> {
            self.inner
                .album_is_name_hit(album_artist, album_title, query)
        }

        fn hit_albums_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Album>, StoreError> {
            self.inner.hit_albums_in_genre(genre, query, offset, limit)
        }

        fn hit_artists_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Artist>, StoreError> {
            self.inner.hit_artists_in_genre(genre, query, offset, limit)
        }

        fn album_hit_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.inner
                .album_hit_tracks_in_genre(album_artist, album_title, genre, query)
        }

        fn hit_genre_counts(&self, query: &str) -> Result<Vec<GenreCount>, StoreError> {
            self.inner.hit_genre_counts(query)
        }

        fn artists_page(
            &self,
            direction: riff_backend::app::store::SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<Page<Artist>, StoreError> {
            self.inner.artists_page(direction, offset, limit)
        }

        fn albums_page(
            &self,
            direction: riff_backend::app::store::SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<Page<Album>, StoreError> {
            self.inner.albums_page(direction, offset, limit)
        }

        fn genres_page(
            &self,
            direction: riff_backend::app::store::SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<Page<GenreCount>, StoreError> {
            self.inner.genres_page(direction, offset, limit)
        }

        fn artists_in_genre_page(
            &self,
            genre: &str,
            direction: riff_backend::app::store::SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<Page<Artist>, StoreError> {
            self.inner
                .artists_in_genre_page(genre, direction, offset, limit)
        }

        fn artist_albums_in_genre_page(
            &self,
            artist: &str,
            genre: &str,
            direction: riff_backend::app::store::SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<Page<Album>, StoreError> {
            self.inner
                .artist_albums_in_genre_page(artist, genre, direction, offset, limit)
        }
    }

    /// Seed the demo collection into a [`DemoQueryStore`]: the library map,
    /// the flat page, the artist/album/genre read models, the sidebar counts,
    /// and the per-artist / per-album drill answers the wrapper serves.
    fn seeded_query_store() -> DemoQueryStore {
        let tracks = demo_tracks();
        let mut albums_by_artist: HashMap<String, Vec<Album>> = HashMap::new();
        let mut tracks_by_album: HashMap<(String, String), Vec<Track>> = HashMap::new();
        let mut albums: Vec<Album> = Vec::new();
        let mut artists: Vec<Artist> = Vec::new();

        for (artist, album_title, year, genre, songs) in DEMO_ALBUMS {
            let album_tracks: Vec<Track> = songs
                .iter()
                .map(|(no, title, secs)| demo_track(artist, album_title, *no, title, *secs))
                .collect();
            let album = Album {
                title: (*album_title).to_string(),
                artist: (*artist).to_string(),
                tracks: album_tracks.iter().map(|t| t.id.clone()).collect(),
                year: Some(*year),
                genre: Some((*genre).to_string()),
            };
            albums_by_artist
                .entry((*artist).to_string())
                .or_default()
                .push(album.clone());
            tracks_by_album.insert(
                ((*artist).to_string(), (*album_title).to_string()),
                album_tracks,
            );
            albums.push(album);
            artists.push(Artist {
                name: (*artist).to_string(),
                albums: vec![format!("{artist} - {album_title}")],
            });
        }
        artists.sort_by(|a, b| a.name.cmp(&b.name));

        let mut inner = MockLibraryQueryStore::default();
        for track in &tracks {
            inner.library.insert(track.id.clone(), track.clone());
        }
        inner.flat = tracks;
        inner.artists = artists;
        inner.paged_albums = albums;
        inner.library_counts = riff_persistence::store::LibraryCounts {
            tracks: 15,
            artists: 5,
            albums: 5,
            genres: 4,
        };
        inner.genre_counts = vec![
            GenreCount {
                genre: "Ambient".to_string(),
                tracks: 4,
            },
            GenreCount {
                genre: "Electronic".to_string(),
                tracks: 6,
            },
            GenreCount {
                genre: "Folk".to_string(),
                tracks: 2,
            },
            GenreCount {
                genre: "Indie".to_string(),
                tracks: 3,
            },
        ];
        inner.smart_list_counts = vec![
            (SmartPlaylistKind::Favorites, 2),
            (SmartPlaylistKind::RecentlyAdded, 15),
            (SmartPlaylistKind::MostPlayed, 8),
            (SmartPlaylistKind::RecentlyPlayed, 12),
            (SmartPlaylistKind::NeverPlayed, 3),
            (SmartPlaylistKind::LostGems, 1),
        ];

        DemoQueryStore {
            inner,
            albums_by_artist,
            tracks_by_album,
        }
    }

    /// Read-only `PlaylistStore` fake serving the one "Night Drive" playlist
    /// with its fixed entries. Every entry resolves to a `Some(track)` the
    /// library mock also serves, so rows render with titles; mutations are
    /// benign no-ops (the screenshots never mutate).
    #[derive(Clone, Default)]
    struct NightDrivePlaylistStore;

    fn night_drive_entries() -> Vec<PlaylistEntry> {
        NIGHT_DRIVE
            .iter()
            .map(|(artist, album, no, title)| {
                let id = id_of(artist, album, *no, title);
                let track = demo_tracks().into_iter().find(|t| t.id == id);
                PlaylistEntry {
                    id,
                    track,
                    valid: true,
                }
            })
            .collect()
    }

    impl PlaylistStoreTrait for NightDrivePlaylistStore {
        fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError> {
            Ok(vec![night_drive_playlist()])
        }

        fn load_playlist_entries(&self, id: &PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError> {
            if *id == night_drive_id() {
                Ok(night_drive_entries())
            } else {
                Ok(Vec::new())
            }
        }

        fn create_playlist(
            &mut self,
            _name: &str,
            _initial_tracks: &[TrackId],
        ) -> Result<PlaylistId, StoreError> {
            Ok(night_drive_id())
        }

        fn rename_playlist(
            &mut self,
            _id: &PlaylistId,
            _new_name: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn delete_playlist(&mut self, _id: &PlaylistId) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn add_playlist_entry(
            &mut self,
            _id: &PlaylistId,
            _track: &TrackId,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn remove_playlist_entries(
            &mut self,
            _id: &PlaylistId,
            _track: &TrackId,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn reorder_playlist_entries(
            &mut self,
            _id: &PlaylistId,
            _ordered: &[TrackId],
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
    }

    // --- Procedural cover art -------------------------------------------------------

    /// FNV-1a 64: a tiny deterministic hash so the same album name always
    /// yields the same art, with no external dependency.
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// HSV (h in degrees, s/v in 0..1) to RGB8.
    fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
        let c = v * s;
        let h = h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        [
            ((r + m) * 255.0).round() as u8,
            ((g + m) * 255.0).round() as u8,
            ((b + m) * 255.0).round() as u8,
        ]
    }

    /// Two distinct hues (degrees) deterministically derived from an album
    /// name. The second hue is pushed apart when the hash lands the two too
    /// close, so every album reads as its own color scheme.
    fn album_hues(name: &str) -> (f32, f32) {
        let hash = fnv1a(name.as_bytes());
        let h1 = (hash % 360) as f32;
        let h2 = ((hash >> 17) % 360) as f32;
        let h2 = if (h2 - h1).abs() < 60.0 {
            (h1 + 140.0).rem_euclid(360.0)
        } else {
            h2
        };
        (h1, h2)
    }

    /// A dark-theme-friendly cover: a smooth vertical gradient between the
    /// album's two hues with one soft circular glow, sized to the request.
    /// Deterministic in `(album name, size)`.
    fn gradient_cover(hues: (f32, f32), size: RequestedSize) -> DecodedCover {
        let (h1, h2) = hues;
        let top = hsv_to_rgb(h1, 0.42, 0.34);
        let bottom = hsv_to_rgb(h2, 0.50, 0.13);
        let glow = hsv_to_rgb((h1 + 20.0).rem_euclid(360.0), 0.35, 0.52);
        let (w, h) = (size.width as usize, size.height as usize);
        // Glow center in the upper-left third; radius relative to the box.
        let (cx, cy) = (w as f32 * 0.30, h as f32 * 0.28);
        let radius = (w.min(h) as f32) * 0.75;

        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            // Vertical gradient position, eased slightly toward the top color.
            let t = (y as f32 / (h.max(2) - 1) as f32).powf(0.85);
            let row = [
                (top[0] as f32 * (1.0 - t) + bottom[0] as f32 * t) as u8,
                (top[1] as f32 * (1.0 - t) + bottom[1] as f32 * t) as u8,
                (top[2] as f32 * (1.0 - t) + bottom[2] as f32 * t) as u8,
            ];
            for x in 0..w {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let d = (dx * dx + dy * dy).sqrt() / radius;
                // Soft glow: quadratic falloff, clamped, at ~22% strength.
                let strength = (1.0 - d).max(0.0).powi(2) * 0.22;
                let blended = [
                    (row[0] as f32 + (glow[0] as f32 - row[0] as f32) * strength).round() as u8,
                    (row[1] as f32 + (glow[1] as f32 - row[1] as f32) * strength).round() as u8,
                    (row[2] as f32 + (glow[2] as f32 - row[2] as f32) * strength).round() as u8,
                ];
                rgba.extend_from_slice(&[blended[0], blended[1], blended[2], 255]);
            }
        }
        DecodedCover {
            rgba,
            width: size.width,
            height: size.height,
        }
    }

    /// A [`Covers`] fake that answers every request with deterministic
    /// procedural album art. Track covers take their hues from the enclosing
    /// album directory (the file path's parent), folder covers from the
    /// directory name itself — so the same album always paints the same art,
    /// distinct from every other album's.
    #[derive(Default)]
    struct ProceduralCovers {
        pending: Mutex<Vec<PendingCover>>,
    }

    /// One queued cover answer: the identity it was asked for, the box, and
    /// the two hues the procedural art derives from.
    type PendingCover = (TrackId, RequestedSize, (f32, f32));

    fn hues_for_dir(dir: Option<&Path>) -> (f32, f32) {
        album_hues(
            dir.and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown Album"),
        )
    }

    impl Covers for ProceduralCovers {
        fn request(&self, track_id: TrackId, path: PathBuf, size: RequestedSize) {
            let hues = hues_for_dir(path.parent());
            self.pending.lock().unwrap().push((track_id, size, hues));
        }

        fn request_folder(&self, folder: &Path, size: RequestedSize) {
            let hues = hues_for_dir(Some(folder));
            self.pending
                .lock()
                .unwrap()
                .push((TrackId::from_path(folder), size, hues));
        }

        fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
            self.pending
                .lock()
                .unwrap()
                .drain(..)
                .map(|(track_id, size, hues)| (track_id, size, Some(gradient_cover(hues, size))))
                .collect()
        }

        fn clear_cache(&self) {}

        fn poll_cache_clear(&self) -> Option<riff_backend::app::cover_service::ClearCacheOutcome> {
            None
        }
    }

    // --- Composed shell ----------------------------------------------------------------

    /// The demo `PlaybackSession`: "Amber Glow" playing at 1:23 of 3:47 with
    /// the six-entry queue up, at 80% volume.
    fn demo_playback_session() -> PlaybackSession {
        let queue_ids = demo_queue_ids();
        // `PlaybackQueue::new` sets `current_index` to the first entry; it has
        // a private invariant field, so it is built through the constructor
        // rather than a struct literal.
        let mut queue = PlaybackQueue::new(queue_ids);
        queue.repeat = RepeatMode::None;
        PlaybackSession {
            queue,
            playback_state: PlaybackState::Playing,
            current_position: PlaybackPosition {
                current: Duration::from_secs(83),
                total: Some(Duration::from_secs(227)),
            },
            current_volume: 0.8,
            muted: false,
            replaygain_enabled: false,
        }
    }

    /// Build the composed-shell harness over the demo stores (same mechanism
    /// as `golden_tests.rs`'s `composed_shell`): a real `RiffApp` headlessly
    /// through `build_eframe`, vendored Inter fonts installed during and after
    /// construction, settled time pinned on both theme slots. Returns the
    /// harness plus the two session handles so tests can move the UI's state
    /// between frames.
    fn build_shell(
        queries: DemoQueryStore,
        playback: Arc<Mutex<PlaybackSession>>,
        library: Arc<Mutex<LibrarySession>>,
    ) -> egui_kittest::Harness<'static, RiffApp> {
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_pixels_per_point(1.0)
            .build_eframe(|cc| {
                cc.egui_ctx.set_fonts(inter_only_font_definitions());
                let (app, _visibility_tx) = RiffApp::new_for_test(
                    Arc::clone(&playback),
                    Arc::clone(&library),
                    Box::new(MockTransport::new()),
                    Box::new(MockScans::default()),
                    Box::new(MockSettingsStore::default()),
                    Box::new(NightDrivePlaylistStore),
                    Box::new(MockLibraryMutationStore::new()),
                    SessionViews::new(
                        Box::new(queries),
                        Box::new(NightDrivePlaylistStore),
                        StoreGeneration::new(),
                        StoreGeneration::new(),
                    ),
                    Box::new(MockTagEdits),
                    Box::new(ProceduralCovers::default()),
                    Arc::new(Mutex::new(
                        riff_backend::app::events::BackendEvents::default(),
                    )),
                );
                app
            });

        // Same re-assert as `composed_shell`: the app installs its dark palette
        // inside its first `update`, which happens in `build_eframe`'s warm-up
        // frames, so pinning must land after construction (and survives the
        // screenshot frames because `apply_theme` short-circuits on an
        // unchanged selection).
        pin_settled_time(&harness.ctx);
        harness
    }

    /// Settle the shell and save the last frame: a fixed step count (the
    /// golden suite's determinism contract — the clock lands on the same
    /// value every run), the fake cursor removed so no pointer triangle bakes
    /// into the shot, then render and write the PNG.
    fn save_screenshot(
        shell: &mut egui_kittest::Harness<'static, RiffApp>,
        file_name: &str,
    ) -> PathBuf {
        shell.run_steps(3);
        shell.remove_cursor();
        shell.run_steps(2);
        let frame = shell
            .render()
            .expect("the composed shell must render headlessly");
        assert_eq!(
            (frame.width(), frame.height()),
            (1280, 800),
            "the rendered frame must fill the requested shell size"
        );
        assert!(
            distinct_colors(&frame) > 40,
            "a real composed frame paints many distinct colors — a near-blank \
             render means the demo state did not take"
        );

        let dir = output_dir();
        std::fs::create_dir_all(&dir).expect("creating docs/assets/readme must work");
        let path = dir.join(file_name);
        frame
            .save(&path)
            .unwrap_or_else(|e| panic!("saving {path:?} must work: {e}"));
        path
    }

    // --- The three screenshots -----------------------------------------------------------

    /// README shot 1 — the Library view: Artists section drilled into
    /// Aurora Vale / Slow Current, so the columns show artists, albums, and
    /// tracks with procedural cover art and the album inspector populated.
    #[test]
    #[ignore = "one-off README screenshot; run with README_SHOTS=1 --ignored"]
    fn readme_screenshot_library() {
        if !shots_enabled() {
            return;
        }
        materialize_demo_files();
        let playback = Arc::new(Mutex::new(demo_playback_session()));
        let library = Arc::new(Mutex::new(LibrarySession::default()));
        let mut shell = build_shell(seeded_query_store(), Arc::clone(&playback), library.clone());

        {
            let mut lib = library.lock().unwrap();
            lib.library_section = LibrarySection::Artists;
            lib.browser_path = vec![
                BrowserSelection::Artist("Aurora Vale".to_string()),
                BrowserSelection::Album {
                    artist: "Aurora Vale".to_string(),
                    title: "Slow Current".to_string(),
                },
            ];
        }

        let path = save_screenshot(&mut shell, "library.png");
        println!("saved {}", path.display());
    }

    /// README shot 2 — the "Night Drive" playlist open (sidebar selected,
    /// entries listed) with the queue panel open over the shell. The playlist
    /// is opened by clicking its sidebar row through kittest: the open state
    /// lives in a private `RiffApp` field with no setter, so the click is the
    /// only honest route in.
    #[test]
    #[ignore = "one-off README screenshot; run with README_SHOTS=1 --ignored"]
    fn readme_screenshot_playlist_queue() {
        if !shots_enabled() {
            return;
        }
        materialize_demo_files();
        let playback = Arc::new(Mutex::new(demo_playback_session()));
        let library = Arc::new(Mutex::new(LibrarySession::default()));
        let mut shell = build_shell(seeded_query_store(), Arc::clone(&playback), library.clone());

        // Warm-up frame so the sidebar's playlist rows are in the a11y tree,
        // then click the "Night Drive" row to open it.
        shell.run_steps(2);
        shell.get_by_label("Night Drive").click();
        shell.run_steps(2);

        // Open the player bar's queue panel over the shell (session state, not
        // a private app field).
        shell.state_mut().library.lock().unwrap().queue_open = true;

        let path = save_screenshot(&mut shell, "playlist-queue.png");
        println!("saved {}", path.display());
    }

    /// README shot 3 — the Now Playing stage: large cover art, full metadata,
    /// progress bar, and the Up Next list. Reaches it by setting the session's
    /// `view_mode` directly.
    #[test]
    #[ignore = "one-off README screenshot; run with README_SHOTS=1 --ignored"]
    fn readme_screenshot_now_playing() {
        if !shots_enabled() {
            return;
        }
        materialize_demo_files();
        let playback = Arc::new(Mutex::new(demo_playback_session()));
        let library = Arc::new(Mutex::new(LibrarySession::default()));
        let mut shell = build_shell(seeded_query_store(), Arc::clone(&playback), library.clone());

        shell.run_steps(2);
        library.lock().unwrap().view_mode = ViewMode::NowPlaying;

        let path = save_screenshot(&mut shell, "now-playing.png");
        println!("saved {}", path.display());
    }
}
