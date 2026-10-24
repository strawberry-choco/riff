// Golden-image snapshot tests (Issue 05).
//
// Renders real egui frames headlessly through `egui_kittest` (wgpu software
// path, no window required) and compares them pixel-for-pixel against
// committed baselines under `tests/snapshots/`. The set is authored against
// the **dark** palette per ADR 0004, plus the light-palette mirrors and the
// High Contrast token-set variants.
//
// **Every composition golden here runs the real `RiffApp` frame.** A golden
// that is about the sidebar, the player bar, one stage column or the Settings
// page renders the whole production composition — Titlebar, Sidebar, Playerbar,
// whichever View the Library Session selects — over mock ports, and crops the
// labels, its counts, its row order, or which widgets a panel is built from:
// a fixture supplies FACTS (tracks, playlists, a root, a session flag) and
// production decides the composition, so a change to the composition moves a
// picture rather than a comment. The four `*_specimen`-style goldens at the
// bottom are the declared exception and say so individually: they pin design
// primitives that no production surface composes, and they call the same
// production entry points the app does.
//
// See docs/engineering/golden-image-testing.md for the authoring,
// re-baselining, and diff-review workflow.

#[cfg(test)]
mod tests {
    use crate::determinism::{
        assert_settled_on_both_slots, distinct_colors, inter_only_font_definitions,
        pin_settled_time,
    };
    use crate::mocks::{MockLibraryQueryStore, MockPlaylistStore, MockSettingsStore};
    use riff_backend::app::state::{LibrarySession, PlaybackSession};
    use riff_gui_theme::fonts;
    use riff_gui_theme::theme::{self, Palette};

    // --- Harness plumbing ------------------------------------------------------

    /// How many golden harnesses may be alive at once.
    ///
    /// Every harness brings up its own wgpu device, and this machine's driver
    /// does not survive seventy of them arriving together: a full
    /// `cargo test --all-targets` died twice with `STATUS_ACCESS_VIOLATION`
    /// (0xc0000005) part-way through the golden block — no failing test, no
    /// diff, green on the next run. Capping the concurrency keeps the suite
    /// reliable while still using several cores; the render work is ~1-2 s per
    /// golden either way.
    const MAX_CONCURRENT_HARNESSES: usize = 4;

    static HARNESSES_IN_FLIGHT: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
    static HARNESS_SLOT_RELEASED: std::sync::Condvar = std::sync::Condvar::new();

    /// A reserved harness slot; released on drop, panics included.
    struct HarnessSlot;

    impl Drop for HarnessSlot {
        fn drop(&mut self) {
            let mut in_flight = lock_harness_slots();
            *in_flight -= 1;
            HARNESS_SLOT_RELEASED.notify_one();
        }
    }

    /// Poisoning is tolerated on purpose: one failing golden must not cascade
    /// into every later one.
    fn lock_harness_slots() -> std::sync::MutexGuard<'static, usize> {
        HARNESSES_IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Block until fewer than [`MAX_CONCURRENT_HARNESSES`] harnesses are in
    /// flight, then reserve one.
    fn harness_slot() -> HarnessSlot {
        let mut in_flight = lock_harness_slots();
        while *in_flight >= MAX_CONCURRENT_HARNESSES {
            in_flight = HARNESS_SLOT_RELEASED
                .wait(in_flight)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *in_flight += 1;
        HarnessSlot
    }

    // --- Composed-golden harness (Issue 05) -------------------------------------
    //
    // Every composition golden in this file renders the REAL `RiffApp` frame:
    // the actual `ui()` composition over mock ports. A golden that wants a
    // *sub*-surface (just the sidebar, just the player bar, one stage column)
    // renders the whole window and CROPS to a rect derived from the same
    // production geometry the frame itself was laid out with, so the crop
    // tracks a token change rather than hardcoding pixels twice. The rendered
    // surface is still the production one — the crop only decides which part of
    // it this golden is about.

    /// The window region a composed golden pins, in window points.
    ///
    /// Every variant resolves from production geometry ([`theme`] tokens and,
    /// for the stage's columns, the very same
    /// [`stage::column_widths`](riff_gui_components::stage::column_widths) the frame
    /// laid itself out with), so "the sidebar" means the 280px panel wherever
    /// `SIDEBAR_W` currently is.
    #[derive(Debug, Clone, Copy)]
    enum Region {
        /// The whole window: every panel at once.
        Window,
        /// The titlebar strip, `TITLEBAR_H` tall across the full width.
        Titlebar,
        /// The `SIDEBAR_W` panel between the two strips.
        Sidebar,
        /// The `PLAYERBAR_H` strip. Only as wide as the panel actually is: the
        /// bottom panel is drawn *after* the sidebar, so it claims the width to
        /// its right and the sidebar's own footer keeps the strip's left edge.
        Playerbar,
        /// The central stage: everything right of the sidebar, between the
        /// titlebar and the playerbar. The Settings and Now Playing subjects.
        Stage,
        /// The stage's list column `index`, at the width
        /// `stage::column_widths` hands it in the frame's plan (the inspector
        /// excluded — [`Region::Inspector`] is its own).
        StageColumn { index: usize },
        /// The stage's inspector column, `INSPECTOR_WIDTH` wide at the
        /// stage's right edge.
        Inspector,
        /// The queue panel sheet, anchored where `show_queue_panel` anchors it
        /// — above the playerbar's right edge, inset by the same 16px — and
        /// sized by the same `QUEUE_PANEL_W` / header / list tokens.
        QueuePanel {
            /// How many Up Next rows the sheet is showing; its height is the
            /// header plus `rows` row heights — or the empty-state floor when
            /// `rows` is 0 — capped at `QUEUE_PANEL_MAX_LIST_H`, plus the
            /// frame's own margin.
            rows: usize,
        },
    }

    impl Region {
        /// The window rect this region covers, given the window size and the
        /// column plan the frame laid out.
        ///
        /// The stage's rect is what is left after the three panels: panels
        /// claim space in draw order, so the central view sits at `SIDEBAR_W`
        /// from the left and between `TITLEBAR_H` and `PLAYERBAR_H`
        /// vertically, whatever else the frame drew.
        fn rect(self, size: egui::Vec2, plan: Plan) -> egui::Rect {
            let stage = egui::Rect::from_min_max(
                egui::pos2(theme::SIDEBAR_W, theme::TITLEBAR_H),
                egui::pos2(size.x, size.y - theme::PLAYERBAR_H),
            );
            let strip = |min_y: f32, max_y: f32| {
                egui::Rect::from_min_max(egui::pos2(0.0, min_y), egui::pos2(size.x, max_y))
            };
            match self {
                Self::Window => egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size),
                Self::Titlebar => strip(0.0, theme::TITLEBAR_H),
                Self::Playerbar => egui::Rect::from_min_max(
                    egui::pos2(theme::SIDEBAR_W, size.y - theme::PLAYERBAR_H),
                    egui::pos2(size.x, size.y),
                ),
                Self::Sidebar => egui::Rect::from_min_max(
                    egui::pos2(0.0, theme::TITLEBAR_H),
                    egui::pos2(theme::SIDEBAR_W, size.y - theme::PLAYERBAR_H),
                ),
                Self::Stage => stage,
                Self::StageColumn { index } => {
                    let (left, width) = stage_column_span(stage, plan, index);
                    egui::Rect::from_min_max(
                        egui::pos2(left, stage.top()),
                        egui::pos2(left + width, stage.bottom()),
                    )
                }
                Self::Inspector => egui::Rect::from_min_max(
                    egui::pos2(stage.right() - theme::INSPECTOR_WIDTH, stage.top()),
                    egui::pos2(stage.right(), stage.bottom()),
                ),
                Self::QueuePanel { rows } => {
                    use riff_gui_theme::theme::geometry::playerbar::{
                        QUEUE_PANEL_EMPTY_LIST_H, QUEUE_PANEL_HEADER_H, QUEUE_PANEL_MAX_LIST_H,
                        QUEUE_PANEL_W,
                    };
                    let list_h = if rows == 0 {
                        QUEUE_PANEL_EMPTY_LIST_H
                    } else {
                        (rows as f32 * riff_gui_theme::theme::geometry::sidebar::ROW_H)
                            .min(QUEUE_PANEL_MAX_LIST_H)
                    };
                    let height = QUEUE_PANEL_HEADER_H + list_h + 2.0 * 8.0;
                    let right = size.x - 16.0;
                    let bottom = size.y - theme::PLAYERBAR_H - 8.0;
                    egui::Rect::from_min_max(
                        egui::pos2(right - QUEUE_PANEL_W, bottom - height),
                        egui::pos2(right, bottom),
                    )
                }
            }
        }
    }

    /// The elastic stage's column plan for a frame: how many list columns the
    /// composition laid out, and whether the inspector took the right-hand
    /// `INSPECTOR_WIDTH`. The crop needs both to find a column's edges, because
    /// the sizing policy gives the last list column whatever is left over.
    #[derive(Debug, Clone, Copy)]
    struct Plan {
        columns: usize,
        inspector: bool,
    }

    impl Plan {
        /// A stage with no inspector.
        const fn columns(columns: usize) -> Self {
            Self {
                columns,
                inspector: false,
            }
        }

        /// A stage with the inspector open, as a selection makes it.
        const fn with_inspector(columns: usize) -> Self {
            Self {
                columns,
                inspector: true,
            }
        }
    }

    /// The gap `stage::show_elastic_stage` puts between two columns: egui's
    /// own `SeparatorStyle::spacing`, which the layout reads off the style and
    /// which the token-built palette leaves at egui's default. Named once here
    /// so the crop and the frame are visibly the same number, not two
    /// independent recollections of one.
    const STAGE_SEPARATOR_W: f32 = 6.0;

    /// `(left, width)` of the stage's list column `index` inside `stage`, laid
    /// out by the production sizing policy with the production separator gap.
    fn stage_column_span(stage: egui::Rect, plan: Plan, index: usize) -> (f32, f32) {
        let Plan { columns, inspector } = plan;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a column count is a small non-negative number"
        )]
        let gaps = (columns.saturating_sub(1) + usize::from(inspector)) as f32;
        let widths = riff_gui_components::stage::column_widths(
            stage.width() - STAGE_SEPARATOR_W * gaps,
            columns,
            inspector,
        );
        let mut left = stage.left();
        for (i, width) in widths.iter().enumerate() {
            if i > 0 {
                left += STAGE_SEPARATOR_W;
            }
            if i == index {
                return (left, *width);
            }
            left += width;
        }
        panic!("stage column {index} is outside a {columns}-column plan");
    }

    /// A window whose main stage is exactly `stage` wide and tall — the shape
    /// a golden states when its subject is a region *inside* the stage, rather
    /// than the stage itself.
    fn window_with_stage(stage: egui::Vec2) -> egui::Vec2 {
        egui::vec2(
            theme::SIDEBAR_W + stage.x,
            theme::TITLEBAR_H + theme::PLAYERBAR_H + stage.y,
        )
    }

    /// The launch main stage, the size the Now Playing and Settings page
    /// subjects are composed at.
    fn launch_window() -> egui::Vec2 {
        riff_gui::ui::chrome::viewport_builder()
            .inner_size
            .expect("launch size is configured")
    }

    /// How the composed shell is seeded: the two sessions' state, the mock
    /// ports' canned rows, and the window size.
    ///
    /// A fixture names only what its subject needs. Everything left at its
    /// default is production's own default — an empty library, dark palette,
    /// All Tracks, no selection — so an unmentioned default is a fact the
    /// composition itself supplies.
    #[derive(Default)]
    struct Seed {
        /// The window size the composed frame renders at.
        size: egui::Vec2,
        /// The Library Session the app renders from. Its `library_paths`,
        /// `ui_flags` and `scan_prefs` are overwritten by hydration from
        /// `settings`, so set those on the settings mock instead.
        library: LibrarySession,
        /// The Playback Session the app renders from. Volume, shuffle and
        /// repeat are overwritten by hydration from `settings` too.
        playback: PlaybackSession,
        /// The Application Store's Settings the app hydrates from at launch.
        settings: MockSettingsStore,
        /// The Library query port: every canned row the stage's listings read.
        query: MockLibraryQueryStore,
        /// The Playlists store, wired into BOTH the mutation port and the read
        /// projection so a seeded playlist is visible in the sidebar and
        /// openable in the stage.
        playlists: MockPlaylistStore,
        /// Scan outcomes the app's first frame drains into the status line.
        scans: Vec<riff_library::app::scan_service::ScanOutcome>,
        /// Answer cover requests with deterministic pixels (`StubCovers`)
        /// rather than nothing (`RecordingCovers`).
        art: bool,
    }

    impl Seed {
        /// A seed over `library`, with the app's launch defaults restored from
        /// that library's Settings row.
        fn over(library: &Library) -> Self {
            Self {
                settings: library.settings(),
                query: library.query(),
                ..Self::default()
            }
        }
    }

    /// Build the real `RiffApp` at `size` from `seed`, with the vendored Inter
    /// faces installed and the determinism pass pinned.
    ///
    /// A thin adapter over the shared [`crate::composed_shell::compose`]: the
    /// seed names only the facts this golden differs by, and the composer owns
    /// the rest of the wiring.
    fn composed(seed: Seed) -> egui_kittest::Harness<'static, riff_gui::ui::RiffApp> {
        use crate::composed_shell::{self, ComposedShell, ShellFonts};
        use crate::mocks::{MockLibraryMutationStore, MockScans, MockTransport, StubCovers};
        use std::sync::{Arc, Mutex};

        let Seed {
            size,
            library,
            playback,
            settings,
            query,
            playlists,
            scans: outcomes,
            art,
        } = seed;

        let scans = MockScans::default();
        for outcome in outcomes {
            scans.queue(outcome);
        }
        // One Playlists store, cloned into both ports (`playlist_ports` states
        // that wiring once), so a seeded playlist is the SAME playlist in both —
        // the mutation port the app writes through and the read projection the
        // sidebar and stage list from.
        let (playlist_store, views) = composed_shell::playlist_ports(Box::new(query), playlists);
        let covers: Box<dyn riff_library::app::cover_service::Covers> = if art {
            Box::new(StubCovers::default())
        } else {
            Box::new(crate::mocks::RecordingCovers::new())
        };

        composed_shell::compose(ComposedShell {
            playback: Arc::new(Mutex::new(playback)),
            library: Arc::new(Mutex::new(library)),
            transport: Box::new(MockTransport::new()),
            scans,
            settings_store: Box::new(settings),
            settings_calls: Arc::new(Mutex::new(Vec::new())),
            playlist_store,
            library_mutations: Box::new(MockLibraryMutationStore::new()),
            views,
            covers,
            folder_covers: Arc::new(Mutex::new(Vec::new())),
            size,
            step_dt: composed_shell::DEFAULT_STEP_DT,
            // PIN the clock (the F1 seam): every composed golden reads its
            // "Last played" / "Last scan" / "Last full scan" stamps against
            // this fixed instant, so a baseline bakes no time-of-run string.
            now: Some(golden_now()),
            fonts: ShellFonts::InterOnly,
        })
        .harness
    }

    /// How many frames a composed golden runs before the one it pins.
    ///
    /// The app schedules a repaint tick every frame (the end-of-frame
    /// responsiveness heartbeat), so `run()` would spin past its step budget;
    /// a fixed count is the whole contract. The clock is never frozen — kittest
    /// only sets `predicted_dt` to its fixed 0.25 s step and egui derives
    /// `time = prev + predicted_dt` — so the count pins a clock value that is
    /// the same on every machine, which is the same contract
    /// `snapshot_animating` relies on.
    ///
    /// Four, not two: a cover requested while a row paints is answered by the
    /// next frame's `CoverCache::settle` and only becomes a texture the frame
    /// after that, so a golden whose subject includes art needs the extra pair.
    const COMPOSED_FRAMES: usize = 4;

    /// Click the one widget whose accessibility label is `label`, then drop the
    /// synthetic pointer.
    ///
    /// Production names every control it draws for the a11y tree, so a label is
    /// a real handle on the real widget — which is how a fixture reaches a state
    /// the frontend keeps to itself (the Settings nav's current section, the
    /// clear-library confirmation, a playlist rename prompt) without a test seam
    /// in `crates/riff-gui/src`.
    ///
    /// The [`Self::remove_cursor`] is not optional: kittest paints a cursor
    /// triangle at the hover position on every render, so a fixture that
    /// clicked and stopped would bake a triangle into its baseline.
    fn click(harness: &mut egui_kittest::Harness<'static, riff_gui::ui::RiffApp>, label: &str) {
        use egui_kittest::kittest::Queryable as _;
        harness.get_by_label(label).click();
        harness.remove_cursor();
    }

    /// Press `Ctrl+K` — the production gesture for the global search field.
    fn focus_search(harness: &mut egui_kittest::Harness<'static, riff_gui::ui::RiffApp>) {
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::K);
        harness.remove_cursor();
    }

    /// Flip the titlebar's theme button — the only production way the palette
    /// family changes, since the app has no other dark/light input.
    fn toggle_theme(harness: &mut egui_kittest::Harness<'static, riff_gui::ui::RiffApp>) {
        click(harness, "Theme");
    }

    /// Render the real composition from `seed`, crop it to `region`, and
    /// compare that crop against the committed baseline.
    ///
    /// `drive` runs real kittest gestures against the live app between
    /// construction and the snapshot frame. It is deliberately narrow: a
    /// fixture may reach a frontend-local state through a gesture, and may not
    /// seed one directly.
    fn snapshot_composed(
        name: &str,
        size: egui::Vec2,
        region: Region,
        plan: Plan,
        drive: impl FnOnce(&mut egui_kittest::Harness<'static, riff_gui::ui::RiffApp>),
        mut seed: Seed,
    ) {
        let _slot = harness_slot();
        seed.size = size;
        let mut shell = composed(seed);
        drive(&mut shell);
        shell.run_steps(COMPOSED_FRAMES);
        let frame = shell
            .render()
            .expect("the composed app must render headlessly");
        let rect = region.rect(size, plan);
        let cropped = crop(&frame, rect);
        assert!(
            distinct_colors(&cropped) > 8,
            "{name}: the crop {rect:?} of a {}x{} frame painted {n} distinct colors, \
             so it is blank or single-filled — it is probably not on the surface \
             this golden is about",
            frame.width(),
            frame.height(),
            n = distinct_colors(&cropped),
        );
        egui_kittest::image_snapshot_options(&cropped, name, shell.options());
    }

    /// The pixels of `frame` inside `rect`, snapped to whole pixels.
    ///
    /// egui rounds panel rects to whole points, so a token-derived rect lands
    /// on integer boundaries in practice; the rounding is here so a fractional
    /// token cannot silently crop a pixel short.
    fn crop(frame: &image::RgbaImage, rect: egui::Rect) -> image::RgbaImage {
        let left = rect.left().round().max(0.0) as u32;
        let top = rect.top().round().max(0.0) as u32;
        let right = (rect.right().round() as u32).min(frame.width());
        let bottom = (rect.bottom().round() as u32).min(frame.height());
        assert!(
            right > left && bottom > top,
            "the crop rect {rect:?} is empty inside a {}x{} frame",
            frame.width(),
            frame.height()
        );
        image::imageops::crop_imm(frame, left, top, right - left, bottom - top).to_image()
    }

    // --- Canned library data ----------------------------------------------------
    //
    // The rows the mock ports serve. What a golden supplies here is DATA —
    // tracks, albums, artists, playlists, counts — and never the composition
    // that reads it: no Section label, no row order, no count arithmetic, no
    // "which panel holds this". That is the line this file now draws: facts in,
    // composition out.

    /// An absolute instant for the canned library's play history. A constant,
    /// not `now`: "added six months ago" must mean the same thing on the
    /// machine that renders the baseline and the one that reads it.
    fn canned_instant(secs: u64) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)
    }

    /// The ONE pinned "now" every composed golden reads its time-of-run stamps
    /// against, installed through [`RiffApp::set_now_for_test`] in [`composed`]
    /// — the seam that ends the recurring golden drift. A constant, exactly one
    /// day after the canned play history (`canned_instant(1_700_000_000)`), so
    /// the Track and Album inspectors' "Last played" reads `1d ago` and the
    /// sidebar footer / Settings scan card stamp — whose fixtures are set to
    /// this same instant — reads `just now`. Both strings are now identical on
    /// every machine and every run.
    fn golden_now() -> std::time::SystemTime {
        canned_instant(1_700_086_400)
    }

    /// One track of the canned library. The path IS the id (production's own
    /// rule), and the tags are what the stage's rows format.
    #[derive(Clone)]
    struct LibTrack {
        path: String,
        artist: String,
        album: String,
        title: String,
        genre: String,
        year: u32,
        number: u32,
        plays: u32,
        secs: u32,
        favorite: bool,
    }

    impl LibTrack {
        /// A track with the tags the listings display, and the play history a
        /// listing's `Plays` cluster and the smart lists read.
        fn new(path: &str, artist: &str, album: &str, title: &str) -> Self {
            Self {
                path: path.to_string(),
                artist: artist.to_string(),
                album: album.to_string(),
                title: title.to_string(),
                genre: "Electronic".to_string(),
                year: 2002,
                number: 1,
                plays: 12,
                secs: 240,
                favorite: false,
            }
        }

        fn genre(mut self, genre: &str) -> Self {
            self.genre = genre.to_string();
            self
        }

        fn year(mut self, year: u32) -> Self {
            self.year = year;
            self
        }

        fn number(mut self, number: u32) -> Self {
            self.number = number;
            self
        }

        fn plays(mut self, plays: u32) -> Self {
            self.plays = plays;
            self
        }

        fn secs(mut self, secs: u32) -> Self {
            self.secs = secs;
            self
        }

        fn favorite(mut self) -> Self {
            self.favorite = true;
            self
        }

        fn id(&self) -> riff_backend::domain::TrackId {
            riff_backend::domain::TrackId(self.path.clone())
        }

        fn track(&self) -> riff_backend::domain::Track {
            riff_backend::domain::Track {
                id: self.id(),
                file_path: std::path::PathBuf::from(&self.path),
                metadata: riff_backend::domain::TrackMetadata {
                    title: Some(self.title.clone()),
                    artist: Some(self.artist.clone()),
                    album: Some(self.album.clone()),
                    album_artist: Some(self.artist.clone()),
                    track_number: Some(self.number),
                    disc_number: Some(1),
                    genre: Some(self.genre.clone()),
                    year: Some(self.year),
                    ..riff_backend::domain::TrackMetadata::default()
                },
                duration: Some(std::time::Duration::from_secs(u64::from(self.secs))),
                sample_rate: Some(44_100),
                channels: Some(2),
                play_count: self.plays,
                last_played: Some(canned_instant(1_700_000_000)),
                date_added: Some(canned_instant(1_600_000_000)),
                favorite: self.favorite,
                search_text: format!(
                    "{} {} {} {}",
                    self.title, self.artist, self.album, self.artist
                )
                .to_lowercase(),
            }
        }
    }

    /// The canned library a fixture seeds: one `MockLibraryQueryStore` filled
    /// from the tracks, and the Settings row that registers the library root
    /// the sidebar footer and the Settings Library pane read.
    struct Library {
        tracks: Vec<LibTrack>,
        /// The library root as the user registered it. Hydration reports
        /// `Unavailable` for a root that does not exist on the rendering
        /// machine, which is the honest verdict and what the Settings golden
        /// that wants `Unavailable` relies on.
        root: std::path::PathBuf,
    }

    impl Library {
        /// The album artist and title every album-level fixture drills into.
        const ALBUM_ARTIST: &'static str = "Boards of Canada";
        const ALBUM_TITLE: &'static str = "Geogaddi";

        /// A library of `tracks` under `root`.
        fn new(root: &str, tracks: Vec<LibTrack>) -> Self {
            Self {
                tracks,
                root: std::path::PathBuf::from(root),
            }
        }

        /// The canned library every populated fixture shares: three artists'
        /// worth of rows under one root, enough for the sidebar's counts, the
        /// entity columns' drill-downs, the flat list and the Folders tree to
        /// all have content.
        fn geogaddi() -> Self {
            Self::new(
                "/music",
                vec![
                    LibTrack::new(
                        "/music/Boards of Canada/Geogaddi/01 - Ready Let's Go.flac",
                        "Boards of Canada",
                        "Geogaddi",
                        "Ready Let's Go",
                    )
                    .number(1)
                    .secs(201),
                    LibTrack::new(
                        "/music/Boards of Canada/Geogaddi/02 - Music Is Math.flac",
                        "Boards of Canada",
                        "Geogaddi",
                        "Music Is Math",
                    )
                    .number(2)
                    .plays(34)
                    .secs(322)
                    .favorite(),
                    LibTrack::new(
                        "/music/Boards of Canada/Geogaddi/03 - Beware the Friendly Stranger.flac",
                        "Boards of Canada",
                        "Geogaddi",
                        "Beware the Friendly Stranger",
                    )
                    .number(3)
                    .plays(5)
                    .secs(27),
                    LibTrack::new(
                        "/music/Boards of Canada/Geogaddi/04 - Gyroscope.flac",
                        "Boards of Canada",
                        "Geogaddi",
                        "Gyroscope",
                    )
                    .number(4)
                    .plays(21)
                    .secs(207),
                    LibTrack::new(
                        "/music/Autechre/Tri Repetae/01 - Flutter.flac",
                        "Autechre",
                        "Tri Repetae",
                        "Flutter",
                    )
                    .genre("IDM")
                    .year(1995)
                    .plays(41)
                    .secs(319),
                    LibTrack::new(
                        "/music/Daft Punk/Discovery/01 - One More Time.flac",
                        "Daft Punk",
                        "Discovery",
                        "One More Time",
                    )
                    .genre("House")
                    .year(2001)
                    .plays(54)
                    .secs(337)
                    .favorite(),
                ],
            )
        }

        /// A library with one artist and one album, for the fixtures whose
        /// subject is a single column's contents rather than the read model.
        fn single_album() -> Self {
            Self::new(
                "/music",
                vec![
                    LibTrack::new(
                        "/music/Boards of Canada/Geogaddi/01 - Ready Let's Go.flac",
                        "Boards of Canada",
                        "Geogaddi",
                        "Ready Let's Go",
                    )
                    .number(1)
                    .secs(201),
                    LibTrack::new(
                        "/music/Boards of Canada/Geogaddi/02 - Music Is Math.flac",
                        "Boards of Canada",
                        "Geogaddi",
                        "Music Is Math",
                    )
                    .number(2)
                    .plays(34)
                    .secs(322)
                    .favorite(),
                ],
            )
        }

        /// A library whose `Geogaddi` album resolves no tracks, which is how
        /// production reaches the Tracks column's "Nothing here yet" copy.
        fn album_without_tracks() -> Self {
            let mut library = Self::single_album();
            library.tracks.clear();
            library
        }

        /// The `Geogaddi` album as a drill-down path entry.
        fn album_selection() -> riff_backend::app::state::BrowserSelection {
            riff_backend::app::state::BrowserSelection::Album {
                artist: Self::ALBUM_ARTIST.to_string(),
                title: Self::ALBUM_TITLE.to_string(),
            }
        }

        /// The `Gyroscope` track, the one the playback fixtures queue.
        fn current(&self) -> LibTrack {
            self.tracks
                .first()
                .cloned()
                .expect("the canned library carries the current track")
        }

        /// Every stored row, in the canonical order the flat list reads.
        fn stored(&self) -> Vec<riff_backend::domain::Track> {
            self.tracks.iter().map(LibTrack::track).collect()
        }

        /// The query port the stage's listings read: the flat list, the
        /// per-track lookup, the album's tracks, and the entity read models the
        /// sidebar's counts and the columns' rows come from — all derived from
        /// the same tracks, so the counts cannot disagree with the rows.
        fn query(&self) -> MockLibraryQueryStore {
            let tracks = self.stored();
            let mut query = MockLibraryQueryStore {
                flat: tracks.clone(),
                search: tracks.clone(),
                library: tracks.iter().map(|t| (t.id.clone(), t.clone())).collect(),
                album_tracks: tracks
                    .iter()
                    .filter(|t| t.metadata.album.as_deref() == Some(Self::ALBUM_TITLE))
                    .cloned()
                    .collect(),
                // Read back as an elapsed time, so the pinned [`composed`]
                // clock is what makes the stamp render as "just now": the
                // fixture stamp and the app's `now` are the SAME fixed instant,
                // so the elapsed is exactly zero on every machine and run.
                last_full_scan: Some(golden_now()),
                ..MockLibraryQueryStore::default()
            };

            // Distinct album artists and albums, in first-appearance order, and
            // the genre tally — derived, so a fixture cannot state a count that
            // disagrees with the rows it seeded.
            let mut artists: Vec<riff_backend::domain::Artist> = Vec::new();
            let mut albums: Vec<riff_backend::domain::Album> = Vec::new();
            let mut genres: Vec<(String, usize)> = Vec::new();
            for track in &tracks {
                let artist = track
                    .metadata
                    .album_artist
                    .clone()
                    .unwrap_or_else(|| "Unknown Artist".to_string());
                if !artists.iter().any(|a| a.name == artist) {
                    artists.push(riff_backend::domain::Artist {
                        name: artist.clone(),
                        albums: Vec::new(),
                    });
                }
                let title = track
                    .metadata
                    .album
                    .clone()
                    .unwrap_or_else(|| "Unknown Album".to_string());
                if !albums
                    .iter()
                    .any(|a| a.artist == artist && a.title == title)
                {
                    albums.push(riff_backend::domain::Album {
                        title: title.clone(),
                        artist: artist.clone(),
                        tracks: Vec::new(),
                        year: track.metadata.year,
                        genre: track.metadata.genre.clone(),
                    });
                }
                if let Some(genre) = track.metadata.genre.clone() {
                    match genres.iter_mut().find(|(g, _)| *g == genre) {
                        Some((_, count)) => *count += 1,
                        None => genres.push((genre, 1)),
                    }
                }
            }
            query.genre_counts = genres
                .iter()
                .map(|(genre, tracks)| riff_backend::domain::GenreCount {
                    genre: genre.clone(),
                    tracks: *tracks,
                })
                .collect();
            query.paged_genres = query.genre_counts.clone();
            query.library_counts = riff_backend::app::store::LibraryCounts {
                tracks: tracks.len(),
                artists: artists.len(),
                albums: albums.len(),
                genres: query.genre_counts.len(),
            };
            query.smart_list_counts = vec![
                (riff_backend::domain::SmartPlaylistKind::RecentlyAdded, 6),
                (riff_backend::domain::SmartPlaylistKind::RecentlyPlayed, 4),
                (riff_backend::domain::SmartPlaylistKind::MostPlayed, 6),
                (riff_backend::domain::SmartPlaylistKind::Favorites, 2),
            ];
            query.artists = artists;
            query.albums = albums.clone();
            query.paged_albums = albums;
            // One shared list per query kind, which is all a single-album drill
            // needs; the deeper Genres drills read the genre-scoped lists.
            query.genre_artists = query.artists.clone();
            query.genre_albums = query.albums.clone();
            query.genre_album_tracks = query.album_tracks.clone();
            query.smart = tracks.clone();
            query
        }

        /// The Settings the app hydrates from: the library root registered, and
        /// default scalars a fixture can turn on.
        fn settings(&self) -> MockSettingsStore {
            MockSettingsStore {
                state: riff_backend::app::store::Settings {
                    scalars: riff_backend::app::state::ScalarSettings::default(),
                    library_paths: vec![self.root.clone()],
                    watch_states: std::collections::HashMap::new(),
                },
                ..MockSettingsStore::default()
            }
        }
    }

    // --- Shared scenario builders ----------------------------------------------
    //
    // A handful of session shapes several goldens share, named after what they
    // are rather than where they are used. Each is a fact set, not a picture.

    /// A `PlaybackSession` playing `library`'s first track from a queue of the
    /// whole library, at `position` of `total`, volume `0.65`, unmuted, no
    /// repeat.
    ///
    /// The queue is the library rather than a list of made-up ids, because the
    /// Up Next read model resolves every queued id through the store's
    /// `get_track`: an id the library does not carry is an empty row, and a
    /// golden that showed one would be pinning a fake rather than the
    /// composition. The transport fixtures vary one field of this.
    fn playing(library: &Library, position: u32, total: u32) -> PlaybackSession {
        PlaybackSession {
            playback_state: riff_backend::domain::PlaybackState::Playing,
            current_position: riff_backend::domain::PlaybackPosition {
                current: std::time::Duration::from_secs(u64::from(position)),
                total: Some(std::time::Duration::from_secs(u64::from(total))),
            },
            current_volume: 0.65,
            queue: riff_backend::app::state::PlaybackQueue::new(
                library.tracks.iter().map(LibTrack::id).collect(),
            ),
            ..PlaybackSession::default()
        }
    }

    /// `seed` with `track` selected in the library, so the inspector follows a
    /// track the way a single-click in any listing makes it.
    fn with_selected_track(mut seed: Seed, track: &LibTrack) -> Seed {
        seed.library.selected_track = Some(track.id());
        seed
    }

    /// `seed` with the Albums section open and the `Geogaddi` album drilled
    /// into, the plan every album-level fixture composes.
    fn in_album(mut seed: Seed) -> Seed {
        seed.library.library_section = riff_backend::app::state::LibrarySection::Albums;
        seed.library.browser_path = vec![Library::album_selection()];
        seed
    }

    // --- Golden baselines --------------------------------------------------------

    // --- Whole window ------------------------------------------------------------

    /// The production composition at the default launch state, dark.
    #[test]
    fn composed_shell_normal_matches_golden_baseline() {
        snapshot_composed(
            "composed_shell_normal_dark",
            egui::vec2(1280.0, 800.0),
            Region::Window,
            Plan::columns(1),
            |_| {},
            Seed::default(),
        );
    }

    /// The same composition at the chrome-fitting minimum window
    /// ([`MIN_WINDOW_SIZE`]), so the golden pins the panel sizes AND the
    /// smallest window they still fit in.
    #[test]
    fn composed_shell_minimum_matches_golden_baseline() {
        snapshot_composed(
            "composed_shell_minimum_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Window,
            Plan::columns(1),
            |_| {},
            Seed::default(),
        );
    }

    /// The same composition in the **light** palette, reached by pressing the
    /// titlebar's theme button — the app's only dark/light input.
    #[test]
    fn shell_chrome_light_matches_golden_baseline() {
        snapshot_composed(
            "shell_chrome_light",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Window,
            Plan::columns(1),
            toggle_theme,
            Seed::default(),
        );
    }

    /// The whole composition with a scan status line beside the wordmark. The
    /// status is a real `ScanOutcome` drained by the real scan poll, not a
    /// string written into the session.
    #[test]
    fn shell_chrome_scanning_dark_matches_golden_baseline() {
        snapshot_composed(
            "shell_chrome_scanning_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Window,
            Plan::columns(1),
            |_| {},
            Seed {
                scans: vec![riff_library::app::scan_service::ScanOutcome::Progress {
                    path: std::path::PathBuf::from("/music"),
                    files_found: 1284,
                }],
                ..Seed::default()
            },
        );
    }

    /// The whole composition on Now Playing: `active_nav: None`, because Now
    /// Playing replaces the view and leaves no destination highlighted.
    #[test]
    fn shell_chrome_now_playing_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "shell_chrome_now_playing_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Window,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    view_mode: riff_backend::app::state::ViewMode::NowPlaying,
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                ..Seed::over(&library)
            },
        );
    }

    /// The whole composition on the Settings stage.
    #[test]
    fn shell_chrome_settings_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "shell_chrome_settings_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Window,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    view_mode: riff_backend::app::state::ViewMode::Settings,
                    ..LibrarySession::default()
                },
                ..Seed::over(&library)
            },
        );
    }

    // --- Sidebar -----------------------------------------------------------------

    /// The real sidebar at its exact `SIDEBAR_W` token width, cropped out of
    /// the live composition: the flat sectioned nav (LIBRARY / SMART LISTS /
    /// PLAYLISTS) with right-aligned live counts, the listener's own Custom
    /// Smart Playlist rows below the six Built-ins (no count beside them),
    /// playlist rows, and the Add-folder / last-scan footer. Every label,
    /// every count and the row order are production's; the fixture supplies
    /// only the library, the playlists and the custom lists.
    #[test]
    fn sidebar_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "sidebar_dark",
            window_with_stage(egui::vec2(
                riff_gui_theme::theme::geometry::window::MIN_STAGE_SIZE.x,
                640.0,
            )),
            Region::Sidebar,
            Plan::columns(1),
            |_| {},
            Seed {
                playlists: playlists(&[
                    (
                        "focus",
                        "Focus Mix",
                        &["/music/a.flac", "/music/b.flac", "/music/c.flac"],
                    ),
                    ("workout", "Workout", &["/music/d.flac"]),
                ]),
                query: MockLibraryQueryStore {
                    custom_smart_playlists: custom_lists(&[
                        ("spl-late-night-1", "Late night jazz"),
                        ("spl-deep-focus-1", "Deep focus"),
                    ]),
                    ..library.query()
                },
                ..Seed::over(&library)
            },
        );
    }

    /// The same sidebar in the **light** palette, with a Custom row opened by
    /// a real click — the selected band is production's paint, so the fixture
    /// reaches it the way a listener does and the baseline pins which row the
    /// open-list slot marks.
    #[test]
    fn sidebar_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "sidebar_light",
            window_with_stage(egui::vec2(
                riff_gui_theme::theme::geometry::window::MIN_STAGE_SIZE.x,
                640.0,
            )),
            Region::Sidebar,
            Plan::columns(1),
            |harness| {
                toggle_theme(harness);
                click(harness, "Deep focus");
            },
            Seed {
                playlists: playlists(&[
                    (
                        "focus",
                        "Focus Mix",
                        &["/music/a.flac", "/music/b.flac", "/music/c.flac"],
                    ),
                    ("workout", "Workout", &["/music/d.flac"]),
                ]),
                query: MockLibraryQueryStore {
                    custom_smart_playlists: custom_lists(&[
                        ("spl-late-night-1", "Late night jazz"),
                        ("spl-deep-focus-1", "Deep focus"),
                    ]),
                    ..library.query()
                },
                ..Seed::over(&library)
            },
        );
    }

    /// The sidebar's inline "New Playlist" name prompt, opened by pressing the
    /// Playlists header's `+`. The prompt is a frontend-local draft, so this is
    /// the only honest way to reach it: the real gesture on the real widget.
    #[test]
    fn playlist_create_prompt_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "playlist_create_prompt_dark",
            window_with_stage(egui::vec2(
                riff_gui_theme::theme::geometry::window::MIN_STAGE_SIZE.x,
                640.0,
            )),
            Region::Sidebar,
            Plan::columns(1),
            |shell| click(shell, "New Playlist"),
            Seed::over(&library),
        );
    }

    /// The same prompt with a draft already typed — the production key handler
    /// writes the session's query into it, so the field shows real text.
    #[test]
    fn playlist_rename_prompt_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "playlist_rename_prompt_dark",
            window_with_stage(egui::vec2(
                riff_gui_theme::theme::geometry::window::MIN_STAGE_SIZE.x,
                640.0,
            )),
            Region::Sidebar,
            Plan::columns(1),
            |shell| click(shell, "Rename playlist"),
            Seed {
                playlists: playlists(&[("focus", "Focus Mix", &["/music/a.flac"])]),
                ..Seed::over(&library)
            },
        );
    }

    // --- Player bar --------------------------------------------------------------

    /// The real player bar at its exact `PLAYERBAR_H` token height: the cover
    /// well, the circular ghost transport around the primary-filled play, the
    /// seek row with brand fill and time readouts, and the right cluster —
    /// queue position, shuffle/repeat, mute and the styled volume slider.
    #[test]
    fn playerbar_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "playerbar_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback: playing(&library, 120, 245),
                settings: shuffle_on(library.settings()),
                ..Seed::over(&library)
            },
        );
    }

    /// The same bar in the **light** palette.
    #[test]
    fn playerbar_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "playerbar_light",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            toggle_theme,
            Seed {
                playback: playing(&library, 120, 245),
                settings: shuffle_on(library.settings()),
                ..Seed::over(&library)
            },
        );
    }

    /// The transport in every state the pinned `playerbar_dark` does not cover.
    #[test]
    fn playerbar_paused_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let mut playback = playing(&library, 120, 245);
        playback.playback_state = riff_backend::domain::PlaybackState::Paused;

        snapshot_composed(
            "playerbar_paused_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback,
                ..Seed::over(&library)
            },
        );
    }

    /// Stopped, no track: no duration, nothing to show a position against.
    #[test]
    fn playerbar_stopped_dark_matches_golden_baseline() {
        snapshot_composed(
            "playerbar_stopped_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed::default(),
        );
    }

    /// Muted: the slider keeps its value while the speaker glyph changes, which
    /// is the mute/volume split the two fields exist for.
    #[test]
    fn playerbar_muted_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let mut playback = playing(&library, 120, 245);
        playback.muted = true;
        snapshot_composed(
            "playerbar_muted_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback,
                ..Seed::over(&library)
            },
        );
    }

    /// Repeat One: the transport's single-track loop.
    #[test]
    fn playerbar_repeat_one_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let mut playback = playing(&library, 120, 245);
        playback.queue.repeat = riff_backend::domain::RepeatMode::One;
        snapshot_composed(
            "playerbar_repeat_one_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback,
                ..Seed::over(&library)
            },
        );
    }

    /// Repeat All: the queue loop.
    #[test]
    fn playerbar_repeat_all_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let mut playback = playing(&library, 120, 245);
        playback.queue.repeat = riff_backend::domain::RepeatMode::All;
        snapshot_composed(
            "playerbar_repeat_all_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback,
                ..Seed::over(&library)
            },
        );
    }

    /// Advanced mode adds the stop button to the transport cluster.
    #[test]
    fn playerbar_advanced_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let mut settings = library.settings();
        settings.state.scalars.advanced_mode = true;
        snapshot_composed(
            "playerbar_advanced_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback: playing(&library, 120, 245),
                query: library.query(),
                settings,
                ..Seed::default()
            },
        );
    }

    /// The expanded queue position readout — the bar's queue button held open.
    /// The sheet it reveals is a floating `Area`, so it is outside the strip;
    /// `queue_panel_dark` pins that.
    #[test]
    fn playerbar_queue_open_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "playerbar_queue_open_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    queue_open: true,
                    ..LibrarySession::default()
                },
                playback: playing(&library, 120, 245),
                ..Seed::over(&library)
            },
        );
    }

    /// The now-playing zone at a roomy 1100px, playing a short title: cover +
    /// full title + meta line with room to spare.
    #[test]
    fn playerbar_now_playing_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "playerbar_now_playing_dark",
            window_with_stage(egui::vec2(1100.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback: playing(&library, 120, 245),
                ..Seed::over(&library)
            },
        );
    }

    /// The same bar with a title that cannot fit its column: one elided line,
    /// meta still shown.
    #[test]
    fn playerbar_now_playing_long_title_dark_matches_golden_baseline() {
        let mut library = Library::geogaddi();
        library.tracks[0].title =
            "Music Is Math (A Reprise Of Everything We Never Played Live)".to_string();
        library.tracks[0].album = "Music Has the Right to Children".to_string();
        snapshot_composed(
            "playerbar_now_playing_long_title_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed {
                playback: playing(&library, 120, 245),
                ..Seed::over(&library)
            },
        );
    }

    /// Idle bar: no current track — the placeholder cover and the two muted
    /// idle lines at the zone's exact playing-state geometry.
    #[test]
    fn playerbar_idle_dark_matches_golden_baseline() {
        snapshot_composed(
            "playerbar_idle_dark",
            window_with_stage(egui::vec2(800.0, 600.0)),
            Region::Playerbar,
            Plan::columns(1),
            |_| {},
            Seed::default(),
        );
    }

    /// The Settings store with shuffle restored, which is the persisted way
    /// the player's shuffle toggle reads `true` at launch.
    fn shuffle_on(mut settings: MockSettingsStore) -> MockSettingsStore {
        settings.state.scalars.shuffle = true;
        settings
    }

    // --- Queue panel -------------------------------------------------------------

    /// The queue sheet as production anchors it: a floating `Area` above the
    /// player bar's right edge, its "Up Next" header and the scrollable rows.
    /// Cropped from the live composition, so the anchor, the sheet's width and
    /// its row heights are the production ones.
    #[test]
    fn queue_panel_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "queue_panel_dark",
            window_with_stage(egui::vec2(520.0, 600.0)),
            Region::QueuePanel { rows: 5 },
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    queue_open: true,
                    ..LibrarySession::default()
                },
                playback: playing(&library, 120, 245),
                ..Seed::over(&library)
            },
        );
    }

    /// The same sheet with no entries: the empty state, cropped to the same
    /// anchor so only the contents differ.
    #[test]
    fn queue_panel_empty_dark_matches_golden_baseline() {
        snapshot_composed(
            "queue_panel_empty_dark",
            window_with_stage(egui::vec2(520.0, 600.0)),
            Region::QueuePanel { rows: 0 },
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    queue_open: true,
                    ..LibrarySession::default()
                },
                ..Seed::default()
            },
        );
    }

    /// The empty sheet gives its empty state a real body: the hover widget the
    /// composition plants so assistive tech (and this harness) can read it
    /// spans actual height instead of the degenerate sliver that pushes the
    /// title/description block past the sheet's bottom edge.
    #[test]
    fn queue_panel_empty_state_has_a_real_body() {
        use egui_kittest::kittest::Queryable as _;

        let mut shell = composed(Seed {
            library: LibrarySession {
                queue_open: true,
                ..LibrarySession::default()
            },
            ..Seed::default()
        });
        shell.run_steps(COMPOSED_FRAMES);
        let body = shell
            .query_all_by_label("Queue is empty")
            .next()
            .expect("the empty queue sheet's empty state must be readable");
        assert!(
            body.rect().height() > 0.0,
            "the empty state's read surface is degenerate ({:?}), \
             so its description paints outside the sheet",
            body.rect(),
        );
    }

    // --- Titlebar ----------------------------------------------------------------

    /// The global "Search or jump to…" field where it actually lives: the
    /// titlebar, shared chrome present on every View. Rendered with an empty
    /// query so the hint text shows.
    #[test]
    fn titlebar_search_dark_matches_golden_baseline() {
        snapshot_composed(
            "titlebar_search_dark",
            window_with_stage(egui::vec2(520.0, 600.0)),
            Region::Titlebar,
            Plan::columns(1),
            |_| {},
            Seed::default(),
        );
    }

    /// The same strip in the **light** palette.
    #[test]
    fn titlebar_search_light_matches_golden_baseline() {
        snapshot_composed(
            "titlebar_search_light",
            window_with_stage(egui::vec2(520.0, 600.0)),
            Region::Titlebar,
            Plan::columns(1),
            toggle_theme,
            Seed::default(),
        );
    }

    /// The titlebar search well with keyboard focus — the ring
    /// `sidebar::search_ring_stroke` paints. Focus is requested through the
    /// app's own `Ctrl+K` gesture (fully deterministic; the determinism rule
    /// bans *hover*-dependent rendering, not focus).
    #[test]
    fn titlebar_search_focused_dark_matches_golden_baseline() {
        snapshot_composed(
            "titlebar_search_focused_dark",
            window_with_stage(egui::vec2(520.0, 600.0)),
            Region::Titlebar,
            Plan::columns(1),
            focus_search,
            Seed::default(),
        );
    }

    /// The most accessibility-critical pixel in the app: the focused titlebar
    /// search well's ring, in the High Contrast variant that thickens it.
    #[test]
    fn titlebar_search_focused_hc_matches_golden_baseline() {
        snapshot_composed(
            "titlebar_search_focused_hc",
            window_with_stage(egui::vec2(520.0, 600.0)),
            Region::Titlebar,
            Plan::columns(1),
            focus_search,
            high_contrast(Seed::default()),
        );
    }

    /// The Settings store with High Contrast restored — the persisted way the
    /// app resolves that palette, through `Preferences::hydrate`.
    fn high_contrast(mut seed: Seed) -> Seed {
        seed.settings.state.scalars.high_contrast = true;
        seed
    }

    // --- Library stage -----------------------------------------------------------

    /// The populated-library flat list at the minimum window: the 40px rows the
    /// explorer lists tracks with — "Artist - Title" on the shared row seam,
    /// the `Plays · Time` cluster and the heart, one row selected and one
    /// now-playing. The now-playing row is also where the equalizer the
    /// composition actually renders appears.
    #[test]
    fn library_track_list_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let selected = library.tracks[2].clone();
        snapshot_composed(
            "library_track_list_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    selected_track: Some(selected.id()),
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                art: true,
                ..Seed::over(&library)
            },
        );
    }

    /// The empty-library copy the flat list shows before anything is indexed:
    /// `browser::empty_state` with production's own title and hint, filling the
    /// stage. Every existing golden invoked it with empty strings, so the copy
    /// itself was never pinned.
    #[test]
    fn empty_column_dark_matches_golden_baseline() {
        snapshot_composed(
            "empty_column_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed::default(),
        );
    }

    /// A Custom Smart Playlist opened as the full-width Track listing: the
    /// name heading with the fresh total, the shared sort control on its own
    /// header row, and the shared 40px Track rows — "Artist - Title" at the
    /// row seam, the plays/time cluster and the heart per row.
    ///
    /// The crop is the WHOLE window on purpose: the fixture's list row in the
    /// sidebar carries **no count**, and a whole-frame golden pins that
    /// omission in the same picture as the listing it opens. The opening
    /// gesture is itself part of the assertion: `click` addresses the row by
    /// its exact label, and the count would fold into the label as
    /// "Late night jazz (4)" — so a row that grew a count would stop answering
    /// this golden's click.
    #[test]
    fn custom_smart_list_listing_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let listed: Vec<_> = library
            .stored()
            .into_iter()
            .filter(|track| track.metadata.album_artist.as_deref() == Some(Library::ALBUM_ARTIST))
            .collect();
        snapshot_composed(
            "custom_smart_list_listing_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Window,
            Plan::columns(1),
            |shell| click(shell, "Late night jazz"),
            Seed {
                query: MockLibraryQueryStore {
                    custom_smart_playlists: custom_lists(&[(
                        "spl-late-night-1",
                        "Late night jazz",
                    )]),
                    smart_playlist_trees: std::collections::HashMap::from([(
                        riff_backend::domain::PlaylistId("spl-late-night-1".to_string()),
                        custom_list_tree(),
                    )]),
                    custom_page: listed,
                    ..library.query()
                },
                art: true,
                ..Seed::over(&library)
            },
        );
    }

    // --- Now Playing -------------------------------------------------------------

    /// The Now Playing stage at the launch main stage: the cover with its
    /// extra-large radius and layered brand glow, the 3xl semibold title, the
    /// meta line, the in-view seek row, and the Up Next rows. The fixed
    /// mockup column only fits whole at the launch size — smaller windows keep
    /// the cover fixed and scroll the Up Next list instead.
    #[test]
    fn now_playing_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "now_playing_dark",
            launch_window(),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    view_mode: riff_backend::app::state::ViewMode::NowPlaying,
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                ..Seed::over(&library)
            },
        );
    }

    /// Now Playing with no track: the calm empty state, close affordance and
    /// all.
    #[test]
    fn now_playing_empty_dark_matches_golden_baseline() {
        snapshot_composed(
            "now_playing_empty_dark",
            launch_window(),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    view_mode: riff_backend::app::state::ViewMode::NowPlaying,
                    ..LibrarySession::default()
                },
                ..Seed::default()
            },
        );
    }

    /// The same stage with a real cover: a `DecodedCover` served through the
    /// real `Covers` port, settled by the real `CoverCache`, uploaded by the
    /// real `artwork::store_cover_texture` and painted through
    /// `painter.image`. Four frames is what lets that round trip land.
    #[test]
    fn now_playing_cover_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "now_playing_cover_dark",
            launch_window(),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    view_mode: riff_backend::app::state::ViewMode::NowPlaying,
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                art: true,
                ..Seed::over(&library)
            },
        );
    }

    // --- Entity columns ----------------------------------------------------------

    /// The Albums root column at the elastic stage's preferred column width
    /// ([`COLUMN_WIDTH`], 280): the A–Z sort control and list rows with
    /// placeholder thumbnail slots, secondary detail lines, one selected. The
    /// stage is cropped to that one column, and it is only 280 wide because the
    /// album drill puts a second column beside it — production's own sizing
    /// policy, not a width the fixture chose.
    #[test]
    fn browser_column_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "browser_column_dark",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 0 },
            Plan::columns(2),
            |_| {},
            in_album(Seed::over(&library)),
        );
    }

    /// The same column in the **light** palette.
    #[test]
    fn browser_column_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "browser_column_light",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 0 },
            Plan::columns(2),
            toggle_theme,
            in_album(Seed::over(&library)),
        );
    }

    /// High Contrast is a token-set variant over the dark base: ink pinned to
    /// the extreme, doubled line alphas, and the yellow `HC_FOCUS_RING`.
    #[test]
    fn browser_column_hc_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "browser_column_hc",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 0 },
            Plan::columns(2),
            |_| {},
            high_contrast(in_album(Seed::over(&library))),
        );
    }

    /// A focused entity row: `theme::focus_ring_stroke` on the browser row.
    /// Focus arrives the way a user gives it — a click on the row — and the
    /// click also drills, which is why the stage is a two-column plan.
    #[test]
    fn browser_column_focused_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "browser_column_focused_dark",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 0 },
            Plan::columns(2),
            |shell| {
                // The row's accessibility name is the label and its detail
                // line, joined the way `browser::accessible_label` joins them.
                // The detail is production's `artist · year` for an album row,
                // so the fixture spells it out rather than clicking blind.
                click(shell, "Geogaddi (Boards of Canada · 2002)");
            },
            Seed {
                library: LibrarySession {
                    library_section: riff_backend::app::state::LibrarySection::Albums,
                    ..LibrarySession::default()
                },
                ..Seed::over(&library)
            },
        );
    }

    /// The query-aware empty copy in a section column: a filtered-to-empty
    /// Albums root explains the query instead of the empty-library copy.
    #[test]
    fn browser_column_query_empty_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "browser_column_query_empty_dark",
            window_with_stage(egui::vec2(766.0, 300.0)),
            Region::StageColumn { index: 0 },
            Plan::columns(2),
            |_| {},
            Seed {
                query: no_hits(library.query()),
                ..searching(in_album(Seed::over(&library)), "zzz")
            },
        );
    }

    /// The Albums root under a query: only hit albums in canonical hit order
    /// with no A–Z sort control (the hit ordering is fixed) — the stage keeps
    /// its section columns, the root lists the hits.
    #[test]
    fn elastic_albums_under_query_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_albums_under_query_dark",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 0 },
            Plan::columns(2),
            |_| {},
            Seed {
                query: hits_only(library.query(), &["Geogaddi", "Tri Repetae"]),
                ..searching(in_album(Seed::over(&library)), "geo")
            },
        );
    }

    /// The detail column (the explorer's Tracks widget — the elastic stage's
    /// last column, which absorbs the remaining width) at album level: a bare
    /// track list, one shared 40px row per track with the `Plays · Time`
    /// cluster on the right — one favorite, one now-playing. The stage is
    /// sized so the last column lands on 480, a representative absorbing width
    /// with the Time cluster clear of the right edge.
    #[test]
    fn detail_column_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "detail_column_dark",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 1 },
            Plan::columns(2),
            |_| {},
            Seed {
                playback: playing(&library, 83, 240),
                ..in_album(Seed::over(&library))
            },
        );
    }

    /// The same column in the **light** palette.
    #[test]
    fn detail_column_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "detail_column_light",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 1 },
            Plan::columns(2),
            toggle_theme,
            Seed {
                playback: playing(&library, 83, 240),
                ..in_album(Seed::over(&library))
            },
        );
    }

    /// The Tracks column's empty copy, at the state that actually produces it:
    /// an album selected whose track list resolves nothing. A Tracks column
    /// with *no selection* is not a state the composition reaches —
    /// `column_plan` only adds it for a selected album — so the copy is pinned
    /// where it is really painted.
    #[test]
    fn detail_column_empty_dark_matches_golden_baseline() {
        let library = Library::album_without_tracks();
        snapshot_composed(
            "detail_column_empty_dark",
            window_with_stage(egui::vec2(766.0, 420.0)),
            Region::StageColumn { index: 1 },
            Plan::columns(2),
            |_| {},
            in_album(Seed::over(&library)),
        );
    }

    // --- Inspector ---------------------------------------------------------------

    /// The inspector at its `INSPECTOR_WIDTH`, cropped out of the live stage: an
    /// album readout's art block, title, artist · year line, the tag rows and
    /// the details grid.
    ///
    /// The panel is a READOUT and this baseline pins that: it holds no action
    /// row, and the kind chip is a plain label rather than the pressed-looking
    /// pill it used to wear.
    #[test]
    fn selection_panel_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "selection_panel_dark",
            window_with_stage(egui::vec2(900.0, 1000.0)),
            Region::Inspector,
            Plan::with_inspector(1),
            |_| {},
            in_album(Seed::over(&library)),
        );
    }

    /// The inspector with real cover art: the art block through the texture
    /// path instead of the placeholder well — a cover requested by the real
    /// `Covers` port and uploaded by the real artwork seam.
    #[test]
    fn selection_panel_art_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "selection_panel_art_dark",
            window_with_stage(egui::vec2(900.0, 1000.0)),
            Region::Inspector,
            Plan::with_inspector(1),
            |_| {},
            Seed {
                art: true,
                ..in_album(Seed::over(&library))
            },
        );
    }

    /// The compact single-TRACK readout: the per-track tag rows, the
    /// "Artist · Album" subtitle, and a header chip that says **Track**. A
    /// distinct composition from the album readout, not just a different
    /// button.
    #[test]
    fn selection_panel_single_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "selection_panel_single_dark",
            window_with_stage(egui::vec2(900.0, 1000.0)),
            Region::Inspector,
            Plan::with_inspector(1),
            |_| {},
            with_selected_track(Seed::over(&library), &library.current()),
        );
    }

    // --- Elastic stage -----------------------------------------------------------

    /// The Artists drill-down: the three list columns the stage sizes side by
    /// side — the Artists root (A–Z sort, artist rows) · the artist's Albums
    /// column · the Tracks column.
    #[test]
    fn elastic_artists_drilled_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_artists_drilled_dark",
            window_with_stage(egui::vec2(1180.0, 420.0)),
            Region::Stage,
            Plan::columns(3),
            |_| {},
            Seed {
                library: LibrarySession {
                    library_section: riff_backend::app::state::LibrarySection::Artists,
                    browser_path: vec![
                        riff_backend::app::state::BrowserSelection::Artist(
                            Library::ALBUM_ARTIST.to_string(),
                        ),
                        Library::album_selection(),
                    ],
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                ..Seed::over(&library)
            },
        );
    }

    /// The same three columns in the **light** palette.
    #[test]
    fn elastic_artists_drilled_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_artists_drilled_light",
            window_with_stage(egui::vec2(1180.0, 420.0)),
            Region::Stage,
            Plan::columns(3),
            toggle_theme,
            Seed {
                library: LibrarySession {
                    library_section: riff_backend::app::state::LibrarySection::Artists,
                    browser_path: vec![
                        riff_backend::app::state::BrowserSelection::Artist(
                            Library::ALBUM_ARTIST.to_string(),
                        ),
                        Library::album_selection(),
                    ],
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                ..Seed::over(&library)
            },
        );
    }

    /// The Albums drill-down: the `Root + Tracks` plan.
    #[test]
    fn elastic_albums_drilled_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_albums_drilled_dark",
            window_with_stage(egui::vec2(900.0, 420.0)),
            Region::Stage,
            Plan::columns(2),
            |_| {},
            Seed {
                playback: playing(&library, 83, 240),
                ..in_album(Seed::over(&library))
            },
        );
    }

    /// The Genres drill-down: the Genres root · the artists-in-genre column.
    #[test]
    fn elastic_genres_drilled_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_genres_drilled_dark",
            window_with_stage(egui::vec2(1460.0, 420.0)),
            Region::Stage,
            Plan::columns(2),
            |_| {},
            in_genre(&library),
        );
    }

    /// The three-deep Genres plan: `GenreArtists + GenreArtistAlbums + Tracks`.
    #[test]
    fn elastic_genres_deep_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_genres_deep_dark",
            window_with_stage(egui::vec2(1180.0, 420.0)),
            Region::Stage,
            Plan::columns(4),
            |_| {},
            Seed {
                library: LibrarySession {
                    library_section: riff_backend::app::state::LibrarySection::Genres,
                    browser_path: vec![
                        riff_backend::app::state::BrowserSelection::Genre("Electronic".to_string()),
                        riff_backend::app::state::BrowserSelection::Artist(
                            Library::ALBUM_ARTIST.to_string(),
                        ),
                        Library::album_selection(),
                    ],
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                ..Seed::over(&library)
            },
        );
    }

    /// The All Tracks composition: the flat listing beside the collapsible
    /// inspector. The stage's width allocation and the listing beside it are
    /// what this image adds over `selection_panel_dark`.
    #[test]
    fn elastic_all_tracks_inspector_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_all_tracks_inspector_dark",
            window_with_stage(egui::vec2(900.0, 1000.0)),
            Region::Stage,
            Plan::with_inspector(1),
            |_| {},
            Seed {
                playback: playing(&library, 83, 240),
                ..in_album(Seed::over(&library))
            },
        );
    }

    /// The inspector's Artist readout, beside the entity column it describes.
    #[test]
    fn elastic_inspector_artist_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_inspector_artist_dark",
            window_with_stage(egui::vec2(900.0, 640.0)),
            Region::Stage,
            Plan::with_inspector(2),
            |_| {},
            Seed {
                library: LibrarySession {
                    library_section: riff_backend::app::state::LibrarySection::Artists,
                    browser_path: vec![riff_backend::app::state::BrowserSelection::Artist(
                        Library::ALBUM_ARTIST.to_string(),
                    )],
                    ..LibrarySession::default()
                },
                ..Seed::over(&library)
            },
        );
    }

    /// The inspector's Genre readout, beside the Genres root it describes.
    #[test]
    fn elastic_inspector_genre_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_inspector_genre_dark",
            window_with_stage(egui::vec2(900.0, 640.0)),
            Region::Stage,
            Plan::with_inspector(1),
            |_| {},
            in_genre(&library),
        );
    }

    /// The inspector's compact single-track readout, beside the flat list it
    /// came from.
    #[test]
    fn elastic_inspector_track_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_inspector_track_dark",
            window_with_stage(egui::vec2(900.0, 1000.0)),
            Region::Stage,
            Plan::with_inspector(1),
            |_| {},
            with_selected_track(Seed::over(&library), &library.current()),
        );
    }

    /// The stage at [`MIN_WINDOW_SIZE`]: `column_widths` shrinks every column
    /// proportionally once the width falls below the floors. Only the
    /// arithmetic was tested before.
    #[test]
    fn elastic_min_window_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "elastic_min_window_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Stage,
            Plan::columns(3),
            |_| {},
            Seed {
                library: LibrarySession {
                    library_section: riff_backend::app::state::LibrarySection::Artists,
                    browser_path: vec![
                        riff_backend::app::state::BrowserSelection::Artist(
                            Library::ALBUM_ARTIST.to_string(),
                        ),
                        Library::album_selection(),
                    ],
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                ..Seed::over(&library)
            },
        );
    }

    // --- Folders tree ------------------------------------------------------------

    /// `BrowseMode::Folders` renders through `TreeRow::art_slot` on the
    /// `INDENT_STEP` indent scale: open/closed folder glyphs, three indent
    /// levels, and one playing track under the branch that contains it. The
    /// Folders plan is a single column, so it fills the stage.
    #[test]
    fn folder_tree_stage_dark_matches_golden_baseline() {
        let library = folders_library();
        snapshot_composed(
            "folder_tree_stage_dark",
            window_with_stage(egui::vec2(520.0, 420.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    browse_mode: riff_backend::app::state::BrowseMode::Folders,
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                query: folder_tree(&library),
                settings: library.settings(),
                ..Seed::default()
            },
        );
    }

    /// The Folders tree under a query: branches with no match are pruned and
    /// the tree stays in place — the query never yanks it into the flat list.
    #[test]
    fn folder_tree_pruned_dark_matches_golden_baseline() {
        let library = folders_library();
        snapshot_composed(
            "folder_tree_pruned_dark",
            window_with_stage(egui::vec2(520.0, 420.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed {
                library: LibrarySession {
                    browse_mode: riff_backend::app::state::BrowseMode::Folders,
                    search_query: "Autechre".to_string(),
                    ..LibrarySession::default()
                },
                playback: playing(&library, 83, 240),
                query: folder_tree_pruned(&library),
                settings: library.settings(),
                ..Seed::default()
            },
        );
    }

    /// The library the Folders tree goldens read: a root whose subtree the
    /// mock port answers with real directories and real track rows.
    fn folders_library() -> Library {
        Library::new(
            "/music",
            vec![
                LibTrack::new(
                    "/music/Boards of Canada/Geogaddi/01 - Ready Let's Go.flac",
                    "Boards of Canada",
                    "Geogaddi",
                    "Ready Let's Go",
                )
                .number(1)
                .secs(201),
                LibTrack::new(
                    "/music/Autechre/Tri Repetae/01 - Flutter.flac",
                    "Autechre",
                    "Tri Repetae",
                    "Flutter",
                )
                .genre("IDM")
                .year(1995)
                .plays(41)
                .secs(319),
            ],
        )
    }

    /// The folder-tree answers for `library`: the root has audio, it holds
    /// one directory per artist, and each of those holds its own album's
    /// tracks. Both levels are named explicitly, because the app recurses into
    /// whatever `subdirs_with_audio` answers — a flat child list would describe
    /// an infinite tree.
    fn folder_tree(library: &Library) -> MockLibraryQueryStore {
        let tracks = library.stored();
        let mut query = MockLibraryQueryStore {
            library: tracks.iter().map(|t| (t.id.clone(), t.clone())).collect(),
            folder_has_audio: true,
            folder_search_match: true,
            folder_tree_ids: tracks.iter().map(|t| t.id.clone()).collect(),
            ..MockLibraryQueryStore::default()
        };
        let root = library.root.clone();
        let mut children = Vec::new();
        for track in &library.tracks {
            let Some(dir) = std::path::Path::new(&track.path).parent() else {
                continue;
            };
            if dir == root {
                continue;
            }
            if !children.contains(&dir.to_path_buf()) {
                children.push(dir.to_path_buf());
                query.tracks_by_dir.insert(
                    dir.to_path_buf(),
                    library
                        .tracks
                        .iter()
                        .filter(|t| std::path::Path::new(&t.path).parent() == Some(dir))
                        .map(LibTrack::track)
                        .collect(),
                );
            }
        }
        query
            .folder_children_by_dir
            .insert(root.clone(), children.clone());
        for child in &children {
            query
                .folder_children_by_dir
                .insert(child.clone(), Vec::new());
        }
        query
    }

    /// The same tree with only the `Autechre` branch left: the pruning
    /// production performs before it draws a node, reached by the query.
    fn folder_tree_pruned(library: &Library) -> MockLibraryQueryStore {
        let mut query = folder_tree(library);
        let root = library.root.clone();
        query
            .folder_children_by_dir
            .insert(root, vec![std::path::PathBuf::from("/music/Autechre")]);
        query.folder_search_match = true;
        query
    }

    // --- Settings ----------------------------------------------------------------

    /// The sectioned Settings page at the launch stage: the header's close
    /// control, the left nav listing every section (Library active), and the
    /// Library pane's Music Libraries card with a per-path Readiness dot beside
    /// Scan / Watch / trash, the Add Library + Scan All row, and the destructive
    /// Clear Library action.
    #[test]
    fn settings_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The same page in the **light** palette.
    #[test]
    fn settings_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_light",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            toggle_theme,
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The Smart Playlists pane (ticket 15), reached by clicking its nav item —
    /// the nav's current section is a frontend-local field, so the gesture is
    /// the only honest way in. Pins the empty state the sidebar `+` and the
    /// section both open on: the way to write the first list and the editor
    /// column waiting beside it.
    #[test]
    fn settings_smart_playlists_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_smart_playlists_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| click(shell, "Smart Playlists"),
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The saved Custom Smart Playlist every Smart Playlists golden below
    /// opens: the list row, its nested tree, and the two Tracks its Rule
    /// matches, seeded exactly as the query port serves them.
    fn morning_drive_seed(library: &Library) -> Seed {
        use riff_backend::app::store::{CustomSmartPlaylist, SmartPlaylistTree};
        use riff_backend::domain::{
            Combine, Comparison, MatchableField, PlaylistId, RootBox, Rule, RuleBox, RuleChild,
            RuleValue,
        };

        let id = PlaylistId::new("spl-morning-1");
        let mut seed = on_settings(scanned(Seed::over(library), library, 1284));
        seed.query.custom_smart_playlists = vec![CustomSmartPlaylist {
            id: id.clone(),
            name: "Morning drive".to_string(),
            created_at: std::time::SystemTime::UNIX_EPOCH,
        }];
        let root = RootBox::new(RuleBox::new(Combine::All).with_children(vec![
            RuleChild::Rule(Rule::new(
                MatchableField::Title,
                Comparison::Is,
                Some(RuleValue::Text("midnight".to_string())),
            )),
            RuleChild::Box(RuleBox::new(Combine::Any).with_children(vec![
                RuleChild::Rule(Rule::new(
                    MatchableField::Year,
                    Comparison::GreaterThan,
                    Some(RuleValue::Number(1975)),
                )),
                RuleChild::Box(
                    RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(Rule::new(
                        MatchableField::PlayCount,
                        Comparison::GreaterThan,
                        Some(RuleValue::Number(10)),
                    ))]),
                ),
            ])),
        ]));
        seed.query
            .smart_playlist_trees
            .insert(id, SmartPlaylistTree::new(root));
        // The live count's read (ticket 17): the double serves its canned
        // listing, so two seeded rows put "2 songs" in the name well — a count,
        // not the double's empty zero.
        seed.query.custom_page = library.stored().into_iter().take(2).collect();
        seed
    }

    /// Open one saved list in the Smart Playlists editor through its own row —
    /// the honest way in, since the draft is frontend-local. The list's name is
    /// worn by that row AND by ticket 12's sidebar row: the same shared
    /// `tree_row` primitive, the same label and role, so the a11y tree cannot
    /// tell them apart and the click has to take the match right of the
    /// sidebar, which is what every one of these goldens crops.
    fn open_saved_smart_playlist(
        shell: &mut egui_kittest::Harness<'static, riff_gui::ui::RiffApp>,
        name: &str,
    ) {
        use egui_kittest::kittest::Queryable as _;

        click(shell, "Smart Playlists");
        // The nav click applies at the end of its frame; the list's row exists
        // in the tree only after the pane has painted.
        shell.run_steps(2);
        shell
            .query_all_by_label(name)
            .find(|node| node.rect().center().x > theme::SIDEBAR_W)
            .expect("the pane's own row for the saved list")
            .click();
        shell.remove_cursor();
    }

    /// The Smart Playlists pane holding a **nested** tree (ticket 16): a saved
    /// list whose Rules sit inside Boxes inside Boxes.
    ///
    /// What the picture pins: the Root Box's sentence — "Match songs where",
    /// its one-word combine chip, "of the following are true" — each Rule as one
    /// row of chips with no disclosure glyph, and every Box's contents inside a
    /// card: a step of fill above the page with a left edge, so a nested Box is
    /// *seen* rather than counted off an indent. Also the count in the name
    /// well, the interleaved Rules-and-Boxes order, each Box's own remove
    /// control, and each Box's add controls under its children.
    #[test]
    fn settings_smart_playlists_nested_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_smart_playlists_nested_dark",
            window_with_stage(egui::vec2(1280.0, 960.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| open_saved_smart_playlist(shell, "Morning drive"),
            morning_drive_seed(&library),
        );
    }

    /// The same pane at a stage too narrow to split — below the
    /// `MIN_TWO_COL_W` the Library pane already honors — so the lists column
    /// draws first and the editor sits under it: the sentence, the chips and a
    /// card inside a card, at the width they have least room at.
    ///
    /// Its Rules are `is missing` ones on purpose. A comparison that takes no
    /// value draws no well, and that is the only row a stage this narrow can
    /// hold: the value wells are fixed-width, so a full row measures ~415px
    /// against the ~512 the stacked pane leaves, and no width inside the stacked
    /// band fits one. A picture with a typed value in it would therefore pin
    /// nothing but the clipping — a standing defect of the row's own, and not
    /// what this golden is the guard for.
    #[test]
    fn settings_smart_playlists_stacked_dark_matches_golden_baseline() {
        use riff_backend::app::store::SmartPlaylistTree;
        use riff_backend::domain::{
            Combine, Comparison, MatchableField, RootBox, Rule, RuleBox, RuleChild,
        };

        let library = Library::geogaddi();
        let mut seed = morning_drive_seed(&library);
        let id = seed.query.custom_smart_playlists[0].id.clone();
        seed.query.smart_playlist_trees.insert(
            id,
            SmartPlaylistTree::new(RootBox::new(RuleBox::new(Combine::All).with_children(
                vec![
                    RuleChild::Rule(Rule::new(
                        MatchableField::Title,
                        Comparison::IsMissing,
                        None,
                    )),
                    RuleChild::Box(RuleBox::new(Combine::Any).with_children(vec![
                        RuleChild::Rule(Rule::new(
                            MatchableField::Year,
                            Comparison::IsMissing,
                            None,
                        )),
                    ])),
                ],
            ))),
        );
        snapshot_composed(
            "settings_smart_playlists_stacked_dark",
            window_with_stage(egui::vec2(560.0, 900.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| open_saved_smart_playlist(shell, "Morning drive"),
            seed,
        );
    }

    /// The name well wearing a count the engine refused: the store's own reason
    /// in the error ink, and no number standing in for it.
    #[test]
    fn settings_smart_playlists_count_failed_dark_matches_golden_baseline() {
        use crate::mocks::FailingQuery;

        let library = Library::geogaddi();
        let mut seed = morning_drive_seed(&library);
        seed.query.failing = vec![FailingQuery::CustomSmartPlaylistCount];
        snapshot_composed(
            "settings_smart_playlists_count_failed_dark",
            window_with_stage(egui::vec2(1280.0, 960.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| open_saved_smart_playlist(shell, "Morning drive"),
            seed,
        );
    }

    /// The Playback pane, reached by clicking its nav item — the nav's current
    /// section is a frontend-local field, so the gesture is the only honest way
    /// in.
    #[test]
    fn settings_playback_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_playback_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| click(shell, "Playback"),
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The Appearance pane.
    #[test]
    fn settings_appearance_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_appearance_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| click(shell, "Appearance"),
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The Advanced pane. It also carries the `#[cfg(not(linux))]` platform
    /// info-lines branch, so it renders differently off Linux.
    #[test]
    fn settings_advanced_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_advanced_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| click(shell, "Advanced"),
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// `About` is a placeholder pane — this is the golden that notices if it
    /// regresses to an empty card.
    #[test]
    fn settings_about_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_about_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| click(shell, "About"),
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The destructive Clear Library confirmation, opened by pressing the
    /// footer button that raises it. The confirmation is a frontend-local flag,
    /// so the real gesture is the only way to reach it.
    #[test]
    fn clear_library_confirm_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "clear_library_confirm_dark",
            // The confirmation is drawn UNDER the settings page, which fills
            // the whole stage — so the stage has to be tall enough to leave the
            // prompt its own band, or the crop cuts its buttons off.
            window_with_stage(egui::vec2(920.0, 960.0)),
            Region::Stage,
            Plan::columns(1),
            |shell| click(shell, "Clear Library"),
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// A library root that has gone missing: the strikethrough path and the
    /// error dot. `/music` does not exist on the machine that renders the
    /// baseline, which is the same honest verdict hydration reaches in
    /// production for a root that is not on disk.
    #[test]
    fn settings_missing_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_missing_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            on_settings(Seed::over(&library)),
        );
    }

    /// A root mid-scan: the Readiness dot, the live file count, and the scan
    /// line the pane shows while the worker is walking. The status is reported
    /// through the real readiness slot the real scan poll writes.
    #[test]
    fn settings_scanning_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_scanning_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            Seed {
                scans: vec![riff_library::app::scan_service::ScanOutcome::Progress {
                    path: library.root.clone(),
                    files_found: 1284,
                }],
                ..on_settings(Seed::over(&library))
            },
        );
    }

    /// A root that exists but has nothing indexed under it: the `Idle` dot and
    /// the pane's not-indexed copy. The root is a real directory on every
    /// machine, which is the only way production reaches `Idle`.
    #[test]
    fn settings_not_indexed_dark_matches_golden_baseline() {
        let library = Library::new(&std::env::temp_dir().to_string_lossy(), Vec::new());
        snapshot_composed(
            "settings_not_indexed_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            on_settings(Seed::over(&library)),
        );
    }

    /// `WatchState::Warning` — the watch box the row disables, with its reason
    /// on hover. A root recorded as watched restores to a fresh warning in a
    /// test process, because no watcher is running here; that is the same
    /// honest verdict production reports when its watcher could not start.
    #[test]
    fn settings_watch_warning_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        let root = library.root.clone();
        library
            .settings()
            .state
            .watch_states
            .insert(root, riff_backend::app::state::WatchState::Enabled);
        snapshot_composed(
            "settings_watch_warning_dark",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// Ticket 04, the **two-column branch**: a stage wide enough for the Library
    /// pane's lower sections to settle into two balanced columns under a
    /// full-width Libraries card. Named for the branch it pins.
    ///
    /// NOTE the width: the *launch* stage (920px) does **not** split. The
    /// pane's content column there measures 607px after the 176px nav, the
    /// hairline, `NAV_GAP` and `PANE_PAD`, and the `ScrollArea` reserves a
    /// further ~16px — 13px short of `MIN_TWO_COL_W` (620). Two columns need a
    /// stage of about 936px. This golden therefore uses a stage that genuinely
    /// reaches the branch it names.
    #[test]
    fn settings_library_two_column_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_library_two_column_dark",
            window_with_stage(egui::vec2(1280.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The same stage in the **light** palette — the same geometry on purpose,
    /// so the palette is the *only* variable between the two-column goldens and
    /// the ticket's "scrutinise the mirrored surfaces" is a controlled
    /// comparison. A mirrored surface that reads wrong is invisible to every
    /// unit test, and the hairline between the columns is exactly what a mirror
    /// is most likely to get wrong.
    #[test]
    fn settings_library_two_column_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_library_two_column_light",
            window_with_stage(egui::vec2(1280.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            toggle_theme,
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The same stage in **High Contrast**. HC raises ink to pure white/grey and
    /// re-points the focus ring, so it is the palette most able to make the
    /// hairline and the error-coloured scan line fall apart.
    #[test]
    fn settings_library_two_column_hc_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_library_two_column_hc",
            window_with_stage(egui::vec2(1280.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            high_contrast(on_settings(scanned(Seed::over(&library), &library, 1284))),
        );
    }

    /// A stage between the minimum and the two-column breakpoint, where the
    /// pane's sections are still one stack but the rows have enough room for
    /// the three-band reflow to breathe.
    ///
    /// Added alongside the minimum-stage golden because the two pin different
    /// regimes: at 520 the pane content is ~205px and the reflow is barely
    /// viable, while here it is comfortable.
    #[test]
    fn settings_library_rows_stacked_mid_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_library_rows_stacked_mid_dark",
            window_with_stage(egui::vec2(700.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// Ticket 04, the **stacked fallback**: the minimum stage, where the pane is
    /// far too narrow for two columns and falls back to one stack. The minimum
    /// stage is what the ticket reasons about — a ~284px content column cannot
    /// carry two.
    #[test]
    fn settings_library_stacked_min_dark_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_library_stacked_min_dark",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Stage,
            Plan::columns(1),
            |_| {},
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The minimum stage in the **light** palette — the narrowest, mirrored
    /// surface in the whole matrix, and the one the redesign singles out as
    /// riskiest. The three-band stacked row and the wrapped chip row both have
    /// to stay legible when every surface relationship is inverted.
    #[test]
    fn settings_stacked_min_light_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_stacked_min_light",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Stage,
            Plan::columns(1),
            toggle_theme,
            on_settings(scanned(Seed::over(&library), &library, 1284)),
        );
    }

    /// The minimum stage in High Contrast: the narrowest column and the
    /// highest-contrast ink in the same frame, which is where a border that
    /// reads as too heavy and text that reads as too dim would both show.
    #[test]
    fn settings_stacked_min_hc_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_stacked_min_hc",
            riff_gui_theme::theme::geometry::window::MIN_WINDOW_SIZE,
            Region::Stage,
            Plan::columns(1),
            |_| {},
            high_contrast(on_settings(scanned(Seed::over(&library), &library, 1284))),
        );
    }

    /// The **normal** stage in High Contrast — the missing "at the normal stage"
    /// half of the matrix for HC.
    #[test]
    fn settings_normal_hc_matches_golden_baseline() {
        let library = Library::geogaddi();
        snapshot_composed(
            "settings_normal_hc",
            window_with_stage(egui::vec2(920.0, 840.0)),
            Region::Stage,
            Plan::columns(1),
            |_| {},
            high_contrast(on_settings(scanned(Seed::over(&library), &library, 1284))),
        );
    }

    /// `seed` switched to the Settings stage — the only view whose content the
    /// Frame does not answer, so the ViewMode is the whole switch.
    fn on_settings(mut seed: Seed) -> Seed {
        seed.library.view_mode = riff_backend::app::state::ViewMode::Settings;
        seed
    }

    // --- Component specimens -----------------------------------------------------
    //
    // The declared exception to "every golden runs the production
    // composition": these four pin design PRIMITIVES that no production
    // surface composes. There is no screen that shows every icon at once, every
    // type style at once, or a two-by-two matrix of the toggle's states, so a
    // composed frame cannot exist for them and the honest thing is to say so
    // rather than to invent a screen. Each calls the SAME production entry
    // point the app calls — `button::text_button` / `paint_primary_face`,
    // `IconCache::texture` over `Icon::ALL`, the `fonts::family_*` families,
    // `toggle_switch::toggle_switch` — so a change to the primitive still moves
    // the picture.

    /// The primary action, through the one authority for that paint
    /// ([`button::text_button`] with [`Variant::Primary`], which routes its face
    /// through [`button::paint_primary_face`]). The window background, the
    /// surface card's radius and the brand gradient all come from the installed
    /// palette, so this pins the Issue 01 foundation through production's own
    /// button rather than a hand-built `egui::Button`.
    #[test]
    fn dark_play_card_matches_golden_baseline() {
        use riff_gui_components::button::{TextButton, Variant, text_button, text_button_size};
        use riff_gui_theme::icons::IconCache;
        use riff_gui_theme::theme::RADIUS_MD;

        snapshot(
            "play_card_dark",
            egui::vec2(240.0, 88.0),
            Palette::dark(),
            |ui, palette| {
                // Full-canvas background (determinism rule): the root UI under
                // the kittest harness is inset from the true screen rect, so a
                // panel fill would leave an unpainted ring around the golden.
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);

                let mut cache = IconCache::new();
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.add_space((ui.available_height() - 60.0) / 2.0);
                    egui::Frame::new()
                        .fill(palette.surface)
                        .corner_radius(RADIUS_MD)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            let size = text_button_size(ui, palette, "Play", false);
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(size.x.max(120.0), size.y),
                                egui::Sense::hover(),
                            );
                            text_button(
                                ui,
                                &mut cache,
                                palette,
                                false,
                                &TextButton {
                                    id: ui.id().with("golden_play"),
                                    rect,
                                    label: "Play",
                                    a11y: "Play",
                                    tooltip: None,
                                    icon: None,
                                    small: false,
                                    variant: Variant::Primary,
                                    enabled: true,
                                },
                            );
                        });
                });
            },
        );
    }

    /// Every `Icon` variant in one frame. `test_icon_inventory_is_vendored_and_complete`
    /// only checks the SVG files exist, so a blank or broken glyph passed.
    /// Production has no screen that enumerates the set, so this stays a
    /// specimen — but it rasterizes through the app's own `IconCache`.
    #[test]
    fn icons_atlas_dark_matches_golden_baseline() {
        use riff_gui_theme::icons::{Icon, IconCache};
        use riff_gui_theme::theme::SURFACE_BG;

        snapshot(
            "icons_atlas_dark",
            egui::vec2(420.0, 260.0),
            Palette::dark(),
            |ui, palette| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);
                let mut cache = IconCache::new();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                    for icon in Icon::ALL {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                        let uv =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        let texture = cache.texture(ui.ctx(), *icon, 24.0, palette.ink_2);
                        ui.painter_at(rect)
                            .image(texture, rect, uv, theme::TEXTURE_TINT);
                    }
                });
            },
        );
    }

    /// An Inter weight/size specimen: catches font-registration regressions
    /// (Medium / SemiBold / Bold) that unit tests cannot, since they never
    /// rasterize. The families are the app's own `fonts::family_*` ones.
    #[test]
    fn type_scale_dark_matches_golden_baseline() {
        use riff_gui_theme::theme::SURFACE_BG;

        snapshot(
            "type_scale_dark",
            egui::vec2(520.0, 300.0),
            Palette::dark(),
            |ui, palette| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);
                for (label, size, family) in [
                    (
                        "Display 3XL",
                        theme::TEXT_3XL,
                        egui::FontFamily::Proportional,
                    ),
                    ("Title XL", theme::TEXT_XL, egui::FontFamily::Proportional),
                    ("Body SM", theme::TEXT_SM, egui::FontFamily::Proportional),
                    ("Caption XS", theme::TEXT_XS, egui::FontFamily::Proportional),
                    ("Medium", theme::TEXT_SM, fonts::family_medium()),
                    ("SemiBold", theme::TEXT_SM, fonts::family_semibold()),
                    ("Bold", theme::TEXT_SM, fonts::family_bold()),
                    ("Monospace", theme::TEXT_XS, egui::FontFamily::Monospace),
                ]
                .map(|(label, size, family): (&str, f32, egui::FontFamily)| (label, size, family))
                {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(label)
                                .size(size)
                                .family(family)
                                .color(palette.ink),
                        );
                    });
                }
            },
        );
    }

    /// The toggle switch matrix: on/off × enabled, in one frame. Production
    /// shows toggles one pane at a time and never an enabled/disabled pair side
    /// by side, so this stays a specimen — through the app's own widget.
    #[test]
    fn toggle_switch_matrix_dark_matches_golden_baseline() {
        use riff_gui_components::toggle_switch;
        use riff_gui_theme::theme::SURFACE_BG;

        snapshot(
            "toggle_switch_matrix_dark",
            egui::vec2(320.0, 160.0),
            Palette::dark(),
            |ui, palette| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);
                for (label, checked, enabled) in [
                    ("On", true, true),
                    ("Off", false, true),
                    ("On (disabled)", true, false),
                    ("Off (disabled)", false, false),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label).color(palette.ink));
                        ui.add_enabled_ui(enabled, |ui| {
                            let _ = toggle_switch::toggle_switch(
                                ui,
                                palette,
                                egui::Id::new(label),
                                label,
                                checked,
                            );
                        });
                    });
                }
            },
        );
    }

    /// The inline option-list selector captured AFTER a real click changes
    /// the selection — ticket 06's reachability promise, in pixels.
    ///
    /// A declared specimen: no production surface composes this primitive
    /// yet (tickets 14/15/20 will place it in the rule editor), so the host
    /// is a test-only pane — a settings-style card at a settings-pane
    /// content width — rendering the selector through its production entry
    /// point. The frame proves the hit areas are real because the capture
    /// happens after `get_by_label("Electronic").click()` landed, and the
    /// test asserts the caller's selection buffer changed before it
    /// snapshots: the picture is evidence of a click, not of a paint.
    #[test]
    fn option_list_after_click_dark_matches_golden_baseline() {
        use egui_kittest::kittest::Queryable as _;
        use riff_gui_components::option_list::{OptionChoice, OptionList, option_list};
        use riff_gui_theme::theme::RADIUS_MD;

        let _slot = harness_slot();
        let palette = Palette::dark();
        let ready = std::rc::Rc::new(std::cell::Cell::new(false));
        let drawing = std::rc::Rc::clone(&ready);
        let options: Vec<OptionChoice<'static, String>> = [
            ("Rock", "rock"),
            ("Electronic", "electronic"),
            ("Ambient", "ambient"),
        ]
        .into_iter()
        .map(|(label, key)| OptionChoice {
            key: key.to_string(),
            label,
        })
        .collect();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 200.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, current: &mut String| {
                    // The `ready` gate and the install order are
                    // `with_golden_style`'s contract, re-expressed here
                    // because this specimen needs a state buffer the click
                    // writes into.
                    if !drawing.get() {
                        return;
                    }
                    // Full-canvas background (determinism rule), then the
                    // test-only pane the selector sits in.
                    let background = ui.ctx().layer_painter(egui::LayerId::background());
                    background.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                    egui::Frame::new()
                        .fill(palette.surface)
                        .corner_radius(RADIUS_MD)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if let Some(key) = option_list(
                                ui,
                                &palette,
                                &OptionList {
                                    id: egui::Id::new("golden_option_list"),
                                    options: &options,
                                    current: Some(current),
                                },
                            ) {
                                *current = key;
                            }
                        });
                },
                "rock".to_string(),
            );
        theme::install(&harness.ctx, &palette, false);
        harness.ctx.set_fonts(inter_only_font_definitions());
        pin_settled_time(&harness.ctx);
        ready.set(true);
        harness.run();
        assert_eq!(
            harness.state().as_str(),
            "rock",
            "the fixture opens with Rock as the caller's selection"
        );

        harness.get_by_label("Electronic").click();
        harness.remove_cursor();
        // The click applies at the end of its frame; the mark renders in
        // the frame after — the settling the shell's own notes pin.
        harness.run_steps(2);
        assert_eq!(
            harness.state().as_str(),
            "electronic",
            "a real click on an inline option changed the selection the \
             captured frame renders — not merely painted a list"
        );

        let frame = harness
            .render()
            .expect("the option-list fixture must render headlessly");
        assert!(
            distinct_colors(&frame) > 8,
            "the fixture frame is blank or single-filled — the selector is \
             not on the surface this golden is about"
        );
        harness.snapshot("option_list_after_click_dark");
    }

    // --- Specimen harness --------------------------------------------------------
    //
    // The three remaining specimens and the behavior tests below need a
    // `build_ui` harness: there is no production frame to compose them into, so
    // `with_golden_style` is the honest harness for a primitive. It gates the
    // draw closure on `ready` because `HarnessBuilder::build_ui` draws — and
    // then `run_ok`s — frames from inside its own constructor, before the caller
    // can install the palette or the fonts; without the gate the first frames
    // would paint with egui's defaults and the baseline would bake them in.

    /// Build a harness whose ui closure stays inert until the palette's style
    /// and the Inter faces are installed on it, with time-driven style fields
    /// pinned after both ([`pin_settled_time`]).
    fn with_golden_style<'a>(
        size: egui::Vec2,
        palette: Palette,
        mut draw: impl FnMut(&mut egui::Ui, &Palette) + 'a,
    ) -> egui_kittest::Harness<'a> {
        let ready = std::rc::Rc::new(std::cell::Cell::new(false));
        let drawing = std::rc::Rc::clone(&ready);
        let harness = egui_kittest::Harness::builder()
            .with_size(size)
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                if drawing.get() {
                    draw(ui, &palette);
                }
            });
        theme::install(&harness.ctx, &palette, false);
        harness.ctx.set_fonts(inter_only_font_definitions());
        // AFTER the palette install, never before: `theme::install` replaces
        // the whole `Arc<Style>` for the palette's theme from a
        // `Style::default()`-derived build, so a determinism pass issued
        // earlier is silently undone.
        pin_settled_time(&harness.ctx);
        ready.set(true);
        harness
    }

    /// Render `draw` through a fixed-size, fixed-DPI harness styled with
    /// `palette`, then compare the result against the committed baseline.
    ///
    /// `palette` is a parameter (not always dark) because the gap-audit
    /// goldens pin the light and High Contrast token sets too.
    fn snapshot(
        name: &str,
        size: egui::Vec2,
        palette: Palette,
        draw: impl FnMut(&mut egui::Ui, &Palette) + 'static,
    ) {
        let _slot = harness_slot();
        let mut harness = with_golden_style(size, palette, draw);
        harness.run();
        harness.snapshot(name);
    }

    // --- Harness determinism (ui-motion issue 01) -------------------------------

    #[test]
    fn golden_helper_pins_settled_time_on_both_theme_slots() {
        let _slot = harness_slot();
        let harness = with_golden_style(
            egui::vec2(64.0, 64.0),
            Palette::dark(),
            |_ui: &mut egui::Ui, _palette: &Palette| {},
        );
        assert_settled_on_both_slots(&harness.ctx);
    }

    /// The composed-`RiffApp` harness installs its palette from *inside* its own
    /// first `update`, so it inherits neither the `ready` gate nor the
    /// re-assert that [`with_golden_style`] issues. It gets its own
    /// [`pin_settled_time`] for exactly that reason, and this test is what
    /// keeps the two from drifting apart — a future harness that forgot to
    /// re-assert would otherwise be a silent mid-blink or mid-tween capture,
    /// which is the failure mode this whole mechanism exists to prevent.
    #[test]
    fn composed_shell_harness_pins_settled_time_on_both_theme_slots() {
        let _slot = harness_slot();
        let shell = composed(Seed {
            size: egui::vec2(1280.0, 800.0),
            ..Seed::default()
        });
        assert_settled_on_both_slots(&shell.ctx);
    }

    // --- Behavior tests ----------------------------------------------------------

    /// Hovering a tree row paints the design's amber wash. This is a
    /// behavior test, not a golden: render a row with the dark palette,
    /// hover it through the harness, and look for the wash color among the
    /// output pixels — the color the user actually sees, wherever the row
    /// lands in the harness's inset canvas. Tolerance ±2 per channel absorbs
    /// driver-level dithering on flat fills.
    #[test]
    fn hovered_tree_row_paints_the_amber_wash() {
        use riff_gui::ui::sidebar::{TreeRow, tree_row};
        use riff_gui_theme::icons::IconCache;

        let _slot = harness_slot();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(280.0, 48.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, theme::SURFACE_BG);
                let palette = Palette::dark();
                let mut cache = IconCache::new();
                tree_row(
                    ui,
                    &mut cache,
                    &palette,
                    false,
                    TreeRow {
                        indent_level: 0,
                        icon: None,
                        cover: None,
                        label: "All Tracks",
                        count: None,
                        meta: None,
                        favorite: None,
                        selected: false,
                        now_playing: false,
                        playing: false,
                        art_slot: false,
                    },
                );
            });
        theme::install(&harness.ctx, &Palette::dark(), false);

        // Idle: no amber wash anywhere in the frame.
        harness.run();
        let idle = harness.render().unwrap();
        assert_eq!(
            crate::test_utils::count_near(&idle, theme::ROW_HOVER, 2),
            0,
            "no amber wash while the row is not hovered"
        );

        // Hovered: the row fill switches to the wash.
        harness.hover_at(egui::pos2(140.0, 24.0));
        harness.run();
        let hovered = harness.render().unwrap();
        assert!(
            crate::test_utils::count_near(&hovered, theme::ROW_HOVER, 2) > 0,
            "hovered tree row paints the amber wash"
        );
    }

    /// The placeholder tile is a *user-loaded* texture (built through
    /// `ctx.load_texture` inside the frame), the exact category the pinned
    /// egui 0.35 must keep rendering in headless snapshots (the 0.36
    /// regression in the workspace notes). Behavior test, not a golden:
    /// resolve the tile for one identity through the shared-cache seam, paint
    /// it full-frame through the same `painter.image` path the playerbar and
    /// now-playing cover use, and look for the tile's compose colours — the
    /// `surface_2` well and the `ink_3` music glyph — among the output
    /// pixels. Tolerance ±2 per channel absorbs driver-level dithering on
    /// flat fills.
    #[test]
    fn placeholder_tile_renders_in_a_headless_snapshot() {
        use riff_gui::ui::artwork::lookup_cover_texture;
        use riff_gui_theme::theme::{SURFACE_BG, TEXTURE_TINT};

        const IDENTITY: &str = "f:\\music\\artless golden.mp3";

        let _slot = harness_slot();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(400.0, 400.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

                // A full miss on the shared cover cache resolves the
                // placeholder tile, exactly as the app's views do.
                let mut textures = std::collections::HashMap::new();
                let mut lru_keys = Vec::new();
                let mut cache = riff_gui::ui::cover_cache::CoverCache::new();
                let tile = lookup_cover_texture(
                    &mut cache,
                    &mut textures,
                    &mut lru_keys,
                    ui.ctx(),
                    &Palette::dark(),
                    IDENTITY,
                    riff_library::app::traits::RequestedSize {
                        width: 512,
                        height: 512,
                    },
                );
                let canvas = ui.available_rect_before_wrap();
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(tile.id(), canvas, uv, TEXTURE_TINT);
            });
        theme::install(&harness.ctx, &Palette::dark(), false);
        harness.run();

        let frame = harness.render().unwrap();
        let palette = Palette::dark();
        assert!(
            crate::test_utils::count_near(&frame, palette.surface_2, 2) > 0,
            "the placeholder well's surface fill must appear in the headless \
             render — user-loaded textures must not be dropped"
        );
        assert!(
            crate::test_utils::count_near(&frame, palette.ink_3, 2) > 0,
            "the placeholder's music glyph (ink_3) must appear in the headless \
             render — user-loaded textures must not be dropped"
        );
    }

    // --- Shared scenario helpers -------------------------------------------------

    /// The playlist rows the sidebar and playlist stage share, keyed by an
    /// explicit id so a golden's row is addressable without a timestamp.
    fn playlists(rows: &[(&str, &str, &[&str])]) -> MockPlaylistStore {
        MockPlaylistStore::with_playlists(
            rows.iter()
                .map(|(id, name, tracks)| {
                    (
                        riff_backend::domain::PlaylistId((*id).to_string()),
                        (*name).to_string(),
                        tracks
                            .iter()
                            .map(|t| riff_backend::domain::TrackId((*t).to_string()))
                            .collect(),
                    )
                })
                .collect(),
        )
    }

    /// The Custom Smart Playlist rows the sidebar goldens seed: id, name and
    /// a shared creation instant. The sidebar's row pass paints them in the
    /// order the read answers, and the store enumerates by creation order, so
    /// the fixture's own order is the painted order.
    fn custom_lists(rows: &[(&str, &str)]) -> Vec<riff_backend::app::store::CustomSmartPlaylist> {
        rows.iter()
            .map(|(id, name)| riff_backend::app::store::CustomSmartPlaylist {
                id: riff_backend::domain::PlaylistId((*id).to_string()),
                name: (*name).to_string(),
                created_at: canned_instant(1_700_000_000),
            })
            .collect()
    }

    /// One canned Rule tree — `Artist is "Boards of Canada"` in the Root Box —
    /// the minimum a save accepts and the shape the custom listing's stage
    /// route asks the store for before it opens. The mock's paged read never
    /// consults it (which Tracks a tree matches is the store seam's proof,
    /// never a double's), but the fixture keeps the tree and the rows it pages
    /// consistent anyway.
    fn custom_list_tree() -> riff_backend::app::store::SmartPlaylistTree {
        use riff_backend::domain::{
            Combine, Comparison, MatchableField, RootBox, Rule, RuleBox, RuleChild, RuleValue,
        };
        riff_backend::app::store::SmartPlaylistTree::new(RootBox::new(
            RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(Rule::new(
                MatchableField::Artist,
                Comparison::Is,
                Some(RuleValue::Text(Library::ALBUM_ARTIST.to_string())),
            ))]),
        ))
    }

    /// `seed` with `library`'s root reported as fully scanned at `tracks`
    /// indexed files.
    ///
    /// Readiness is the scan worker's to report, so the fixture queues the
    /// outcome the real worker publishes and the real poll drains — not a
    /// readiness written into the session behind production's back.
    fn scanned(mut seed: Seed, library: &Library, tracks: usize) -> Seed {
        seed.scans = vec![riff_library::app::scan_service::ScanOutcome::Complete {
            path: library.root.clone(),
            total_files: tracks,
        }];
        seed.query
            .folder_track_counts
            .insert(library.root.clone(), tracks);
        seed
    }

    /// `seed` with the Genres section open and the `Electronic` genre drilled
    /// into — the plan the genre-column fixtures compose.
    fn in_genre(library: &Library) -> Seed {
        Seed {
            library: LibrarySession {
                library_section: riff_backend::app::state::LibrarySection::Genres,
                browser_path: vec![riff_backend::app::state::BrowserSelection::Genre(
                    "Electronic".to_string(),
                )],
                ..LibrarySession::default()
            },
            ..Seed::over(library)
        }
    }

    /// `seed` with the global search query set — the one filter the whole
    /// stage reads, so each section's own columns narrow to their hits rather
    /// than the query taking the stage over.
    fn searching(mut seed: Seed, query: &str) -> Seed {
        seed.library.search_query = query.to_string();
        seed
    }

    /// `query` with every hit-listing emptied, which is what a query matching
    /// nothing looks like to the columns.
    fn no_hits(mut query: MockLibraryQueryStore) -> MockLibraryQueryStore {
        query.paged_albums.clear();
        query.genre_counts.clear();
        query.paged_genres.clear();
        query
    }

    /// `query` narrowed to the albums whose title is in `titles` — the hit
    /// ordering the store's `albums_page` returns under a query.
    fn hits_only(mut query: MockLibraryQueryStore, titles: &[&str]) -> MockLibraryQueryStore {
        query
            .paged_albums
            .retain(|album| titles.contains(&album.title.as_str()));
        query
    }

    /// A Rule row that a listener has *clicked* into its chosen state, captured
    /// in the dark palette (ticket 14's surface).
    ///
    /// A declared specimen, the same class ticket 06 used for the selector: no
    /// production surface composes the row until ticket 15's Settings page, so
    /// the host is a test-only pane at a settings-pane content width, driving
    /// the row through its production entry point. The picture is evidence of
    /// clicks, not of a paint — every part of the row got here through a real
    /// `click()` on a named control, and the test re-reads the draft it left
    /// behind before it snapshots. The Not control is on, because an inversion
    /// that only exists in a state buffer is exactly the thing a picture should
    /// be held to show.
    #[test]
    fn rule_row_after_clicks_dark_matches_golden_baseline() {
        use egui_kittest::kittest::Queryable as _;
        use riff_backend::domain::{Comparison, MatchableField};
        use riff_gui::ui::rule_row::{RuleDraft, RulePickerValues, RuleValueDraft, rule_row};
        use riff_gui_theme::icons::IconCache;
        use riff_gui_theme::theme::RADIUS_MD;

        let _slot = harness_slot();
        let palette = Palette::dark();
        let ready = std::rc::Rc::new(std::cell::Cell::new(false));
        let drawing = std::rc::Rc::clone(&ready);
        let drafts = std::rc::Rc::new(std::cell::RefCell::new(vec![RuleDraft::blank()]));
        let observed = std::rc::Rc::clone(&drafts);
        let cache = std::rc::Rc::new(std::cell::RefCell::new(IconCache::new()));
        let values = vec!["ambient".to_string(), "electronic".to_string()];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(620.0, 220.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                if !drawing.get() {
                    return;
                }
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                egui::Frame::new()
                    .fill(palette.surface)
                    .corner_radius(RADIUS_MD)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut drafts = drafts.borrow_mut();
                        let picker = RulePickerValues::Texts(&values);
                        let outcome = rule_row(
                            ui,
                            &mut cache.borrow_mut(),
                            &palette,
                            egui::Id::new("golden_rule_row"),
                            &mut drafts[0],
                            Some(&picker),
                        );
                        for intent in outcome.intents {
                            drafts[0].apply(&intent);
                        }
                    });
            });
        theme::install(&harness.ctx, &palette, false);
        harness.ctx.set_fonts(inter_only_font_definitions());
        pin_settled_time(&harness.ctx);
        ready.set(true);
        harness.run_steps(2);

        let settle = |harness: &mut egui_kittest::Harness<'_>, label: &str| {
            harness.get_by_label(label).click();
            harness.run_steps(2);
        };
        settle(&mut harness, "Choose a field");
        settle(&mut harness, "Genre");
        settle(&mut harness, "Choose a comparison");
        settle(&mut harness, "is");
        settle(&mut harness, "Choose…");
        settle(&mut harness, "electronic");
        // The painted word and the switch's own accessible name are two nodes
        // sharing "Not"; the switch is the one drawn after its word.
        let nots: Vec<_> = harness.query_all_by_label("Not").collect();
        assert_eq!(
            nots.len(),
            2,
            "the row paints the word and the control carries the name"
        );
        nots.last().expect("the switch").click();
        harness.run_steps(2);
        harness.remove_cursor();
        harness.run_steps(2);

        let draft = &observed.borrow()[0];
        assert_eq!(draft.field, Some(MatchableField::Genre));
        assert_eq!(draft.comparison, Some(Comparison::Is));
        assert!(draft.negated);
        assert_eq!(
            draft.value,
            RuleValueDraft::Text("electronic".to_string()),
            "the clicks, not the test, put the chosen Library value in the well"
        );
        assert_eq!(
            draft.open, None,
            "the snapshot is of a settled row, not an open list"
        );

        let frame = harness
            .render()
            .expect("the rule-row fixture must render headlessly");
        assert!(
            distinct_colors(&frame) > 8,
            "the fixture frame is blank or single-filled — the row is not on the              surface this golden is about"
        );
        harness.snapshot("rule_row_after_clicks_dark");
    }
}
