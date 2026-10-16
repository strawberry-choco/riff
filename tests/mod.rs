//! riff Music Player - Test Suite
//!
//! This is the single integration-test crate root (declared in `Cargo.toml`
//! via `[[test]] path = "tests/mod.rs"` with `autotests = false`). It wires the
//! individual test modules together and re-exports the library so the test
//! bodies can refer to types by their short names (via `use super::*`).
//!
//! # Test Organization
//!
//! - `domain_tests.rs`: Tests for domain objects like Track, `TrackId`, `PlaybackState`, etc.
//! - `app_tests.rs`: Tests for application logic like `PlaybackSession` /
//!   `LibrarySession`, the Session Projections, and scan-side Track construction.
//! - `ui_tests.rs`: Tests for UI-related functionality like settings storage, etc.
//! - `golden_tests.rs`: Golden-image snapshot tests rendering real egui frames headlessly.
//! - `integration_tests.rs`: End-to-end integration tests that test multiple components together.
//!
//! # Running Tests
//!
//! ```bash
//! cargo test                # Run all tests
//! cargo test domain_tests   # Run specific test module
//! cargo test -- --nocapture  # Run tests with output
//! ```

pub mod app_tests;
pub mod domain_tests;
pub mod golden_tests;
pub mod integration_tests;
pub mod readme_screenshots;
pub mod ui_tests;

// --- Shared rendering-determinism contract ------------------------------------
//
// The ONE home for the determinism knobs every headless frame this crate paints
// rides — the golden baselines in `golden_tests` and the README screenshot
// assets in `readme_screenshots`. Both suites used to hold their own copies of
// the first three helpers; the README generator's were unguarded (it asserted
// nothing). Sharing one module means the two can never drift, and the README
// generator now rides the same settled-time assertion the goldens do.
pub(crate) mod determinism {
    use riff_gui_theme::fonts::{self, INTER_FACES};

    /// Deterministic font definitions for rendering: the vendored Inter faces
    /// only. Unlike [`riff_gui_theme::fonts::font_definitions`] this never scans
    /// system CJK fonts, whose presence varies per machine — a rendered frame
    /// must rasterize identically everywhere the suite runs.
    pub(crate) fn inter_only_font_definitions() -> egui::FontDefinitions {
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

    /// Pin the three time-driven [`egui::Style`] fields on **both** theme
    /// slots: tween duration, programmatic scroll animation, cursor blink.
    ///
    /// egui 0.35 has no `Context::set_animation_time` and no
    /// `Context::disable_scroll_animation` — these are plain fields on
    /// `Style`, so the style is the only lever and the whole install has to be
    /// re-asserted, not patched field-by-field before someone else's install
    /// runs.
    ///
    /// **Both** slots, even though `theme::install` writes one: it calls
    /// `set_style_of`, which replaces that theme's whole `Arc<Style>` with a
    /// `Style::default()`-derived build, silently restoring egui's 0.2 s tween,
    /// its 0.1–0.3 s scroll tween and a blinking cursor for that slot; the
    /// other slot keeps whatever egui shipped. Asserting this *after* the
    /// install is the whole point — "baselines capture settled state" has to
    /// be a decision the suite makes, not an accident of call order.
    pub(crate) fn pin_settled_time(ctx: &egui::Context) {
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.scroll_animation = egui::style::ScrollAnimation::none();
            style.visuals.text_cursor.blink = false;
        });
    }

    /// Assert [`pin_settled_time`]'s invariant on both theme slots.
    ///
    /// The failure this guards is silent: a `style_from` that grew a motion
    /// field, or a `set_style_of` moved ahead of the re-assert, would shift
    /// every affected baseline by a fraction of a tween and fail nothing until
    /// someone diffed eighty PNGs. So the invariant is asserted, not just
    /// commented — and the assertion is about the *style*, which is the
    /// decision input, not about a pixel.
    pub(crate) fn assert_settled_on_both_slots(ctx: &egui::Context) {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let style = ctx.style_of(theme);
            assert_eq!(
                style.animation_time, 0.0,
                "{theme:?} slot: tweened values must snap, so a golden never \
                 captures a mid-flight value"
            );
            assert_eq!(
                style.scroll_animation.duration,
                egui::Rangef::new(0.0, 0.0),
                "{theme:?} slot: a scroll-to offset must land in one frame, not lerp"
            );
            assert!(
                style.scroll_animation.points_per_second.is_infinite()
                    && style.scroll_animation.points_per_second.is_sign_positive(),
                "{theme:?} slot: `ScrollAnimation::none()` spells 'no animation' as \
                 an infinite speed — anything finite reintroduces a distance-scaled \
                 duration"
            );
            assert!(
                !style.visuals.text_cursor.blink,
                "{theme:?} slot: a blinking caret is a function of the clock, not of \
                 the frame, and would make every focused-text golden a coin flip"
            );
        }
    }

    /// Count the distinct 8-bit RGBA colors in a rendered frame. A composed
    /// shell that actually painted chrome, text, and separators has many; a
    /// blank, clipped-to-nothing, or unpainted render collapses to a handful.
    pub(crate) fn distinct_colors(image: &image::RgbaImage) -> usize {
        let mut seen = std::collections::HashSet::new();
        for pixel in image.pixels() {
            seen.insert(pixel.0);
        }
        seen.len()
    }
}

// --- The composed-shell fixture -----------------------------------------------
//
// The ONE home for building the real `RiffApp` over the mock port kit. The
// composed goldens, the README screenshot generator and the whole-frame gesture
// shells each used to spell the wiring out for themselves — the same
// many-argument test constructor and its order, the same fonts-before-building
// closure, the same settled-time pin — and differed only in the ports they hand
// in and the display facts. Their copies could drift: `readme_screenshots`'s own
// doc even said it was "the same mechanism as `golden_tests.rs`'s". One composer
// now owns that fixed remainder, so a new port argument or a reordered
// constructor is edited once and cannot drift between suites — the same fix
// `determinism` already applied to the render knobs.
pub(crate) mod composed_shell {
    use crate::mocks::{FakeTagEdits, MockScans, RecordingPasses, SettingsCall};
    use riff_backend::app::events::BackendEvents;
    use riff_backend::app::state::{LibrarySession, PlaybackSession};
    use riff_backend::app::store::{
        LibraryMutationStore, LibraryQueryStore, PlaylistStore, SettingsStore, StoreGeneration,
    };
    use riff_backend::app::transport::Transport;
    use riff_backend::app::views::SessionViews;
    use riff_backend::domain::TrackId;
    use riff_gui::ui::RiffApp;
    use riff_library::app::cover_service::Covers;
    use riff_library::app::traits::RequestedSize;
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    /// kittest's own default step: a quarter of a second per frame. The
    /// composed goldens and the README shot ride it; the gesture suites whose
    /// TIMING is the thing under test pass their own.
    pub(crate) const DEFAULT_STEP_DT: f32 = 0.25;

    /// Which font set the composed harness installs before building.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum ShellFonts {
        /// The vendored Inter faces only — the render-determinism contract the
        /// goldens and the README assets ride.
        InterOnly,
        /// The production set `main.rs` installs; the shell's named families
        /// are required by the render path, so a frame panics without them.
        Production,
    }

    /// The facts a composed shell varies by.
    ///
    /// Everything a caller must hand in lives here; the fixed remainder of the
    /// wiring — the tag-edit and passes fakes, the event inbox, the
    /// fonts-before-building closure and the settled-time guard — does not.
    pub(crate) struct ComposedShell {
        /// The live playback session the app writes UI-owned fields back to.
        pub playback: Arc<Mutex<PlaybackSession>>,
        /// The live library session the app holds for the whole frame.
        pub library: Arc<Mutex<LibrarySession>>,
        /// The playback command front end.
        pub transport: Box<dyn Transport>,
        /// The scripted scan front end the app polls every frame.
        pub scans: MockScans,
        /// The Application Store's Settings the app hydrates from at launch.
        pub settings_store: Box<dyn SettingsStore>,
        /// The record every committed settings mutation lands in.
        pub settings_calls: Arc<Mutex<Vec<SettingsCall>>>,
        /// The playlist mutation port the app writes through.
        pub playlist_store: Box<dyn PlaylistStore>,
        /// The library mutation port (tag refresh, favorites, play history).
        pub library_mutations: Box<dyn LibraryMutationStore>,
        /// The read side: the query port, the playlist read port and both
        /// session generations.
        pub views: SessionViews,
        /// The cover adapter. Each suite keeps its OWN adapter — some record
        /// track requests, some folder requests, some answer with pixels — so
        /// folding the wiring changes what no suite records or returns.
        pub covers: Box<dyn Covers>,
        /// The record the folder-cover adapter writes to, in order.
        pub folder_covers: Arc<Mutex<Vec<(TrackId, RequestedSize)>>>,
        /// The window size the composed frame renders at.
        pub size: egui::Vec2,
        /// The clock advance per step.
        pub step_dt: f32,
        /// The fixed instant every time-driven readout is stamped with, when
        /// the caller wants the clock pinned; `None` means the real clock.
        pub now: Option<SystemTime>,
        /// Which font set to install before building.
        pub fonts: ShellFonts,
    }

    /// The live handles a composed consumer reads back.
    pub(crate) struct ComposedHarness {
        /// The harness owns the app; `state_mut()` reaches it between frames.
        pub harness: egui_kittest::Harness<'static, RiffApp>,
        /// The live playback session the app writes UI-owned fields back to.
        pub playback: Arc<Mutex<PlaybackSession>>,
        /// The live library session the app holds for the whole frame.
        pub library: Arc<Mutex<LibrarySession>>,
        /// The event inbox the app drains at the start of every frame.
        pub backend_events: Arc<Mutex<BackendEvents>>,
        /// The scripted scan front end the app polls every frame.
        pub scans: MockScans,
        /// Every settings mutation the app committed at a frame end.
        pub settings_calls: Arc<Mutex<Vec<SettingsCall>>>,
        /// Every folder cover request the app sent, in order.
        pub folder_covers: Arc<Mutex<Vec<(TrackId, RequestedSize)>>>,
    }

    /// Build the real `RiffApp` over `shell`'s ports — the ONE composed-shell
    /// construction every composed consumer crosses.
    ///
    /// This owns the fixed remainder: the app's test-constructor argument list
    /// and its order, the tag-edit and passes fakes, the event inbox, the
    /// fonts-before-building closure, the clock pin, and — for the
    /// deterministic-render (`InterOnly`) consumers — the settled-time guard
    /// re-asserted (and asserted) after construction.
    pub(crate) fn compose(shell: ComposedShell) -> ComposedHarness {
        let ComposedShell {
            playback,
            library,
            transport,
            scans,
            settings_store,
            settings_calls,
            playlist_store,
            library_mutations,
            views,
            covers,
            folder_covers,
            size,
            step_dt,
            now,
            fonts,
        } = shell;

        let backend_events = Arc::new(Mutex::new(BackendEvents::default()));

        let harness = egui_kittest::Harness::builder()
            .with_size(size)
            .with_pixels_per_point(1.0)
            .with_step_dt(step_dt)
            .build_eframe({
                let playback = Arc::clone(&playback);
                let library = Arc::clone(&library);
                let backend_events = Arc::clone(&backend_events);
                let scans = scans.clone();
                move |cc| {
                    match fonts {
                        ShellFonts::InterOnly => cc
                            .egui_ctx
                            .set_fonts(crate::determinism::inter_only_font_definitions()),
                        ShellFonts::Production => {
                            riff_gui_theme::fonts::configure_fonts(&cc.egui_ctx);
                        }
                    }
                    let (mut app, _visibility_tx) = RiffApp::new_for_test(
                        playback,
                        library,
                        transport,
                        Box::new(scans),
                        settings_store,
                        playlist_store,
                        library_mutations,
                        views,
                        Box::new(FakeTagEdits::new()),
                        Box::new(RecordingPasses::default()),
                        covers,
                        backend_events,
                    );
                    if let Some(now) = now {
                        app.set_now_for_test(now);
                    }
                    app
                }
            });

        // The app installs its palette from inside its own first `update`, so a
        // warm-up frame could have cached a system-fallback metric into a galley
        // that survives into the frame under test: force the vendored set once
        // more after construction. `configure_fonts` is deliberately NOT used —
        // it appends a machine-dependent CJK fallback, which would make the
        // render non-portable.
        //
        // The settled-time guard rides the same deterministic-render contract,
        // and only it: the Production gesture shells drive LIVE motion — the
        // Folders tree's `CollapsingState::openness` reads `Style::animation_time`
        // — so pinning there would snap the in-flight frames their frame-loop
        // tests exist to observe. One composer, one guard, applied where the
        // contract it guards actually applies.
        if fonts == ShellFonts::InterOnly {
            harness
                .ctx
                .set_fonts(crate::determinism::inter_only_font_definitions());
            crate::determinism::pin_settled_time(&harness.ctx);
            crate::determinism::assert_settled_on_both_slots(&harness.ctx);
        }

        ComposedHarness {
            harness,
            playback,
            library,
            backend_events,
            scans,
            settings_calls,
            folder_covers,
        }
    }

    /// Production's playlist wiring: ONE store cloned into both the mutation
    /// port the app writes through and the read side the sidebar and stage list
    /// from, with two freshly-made session generations — the shape every
    /// mock-port entry point wants.
    #[must_use]
    pub(crate) fn playlist_ports<P>(
        query: Box<dyn LibraryQueryStore>,
        playlist: P,
    ) -> (Box<dyn PlaylistStore>, SessionViews)
    where
        P: PlaylistStore + Clone + 'static,
    {
        playlist_ports_at(
            query,
            playlist,
            StoreGeneration::new(),
            StoreGeneration::new(),
        )
    }

    /// [`playlist_ports`] at generations the caller supplies — the real-store
    /// suites pass the store's own, which their mutation tests depend on.
    #[must_use]
    pub(crate) fn playlist_ports_at<P>(
        query: Box<dyn LibraryQueryStore>,
        playlist: P,
        library_generation: StoreGeneration,
        playlist_generation: StoreGeneration,
    ) -> (Box<dyn PlaylistStore>, SessionViews)
    where
        P: PlaylistStore + Clone + 'static,
    {
        (
            Box::new(playlist.clone()),
            SessionViews::new(
                query,
                Box::new(playlist),
                library_generation,
                playlist_generation,
            ),
        )
    }
}

// --- Library re-exports ---------------------------------------------------
//
// Bring the library modules into the test crate root so qualified paths such as
// `crate::domain::TrackMetadata`, `crate::app::state::PlaybackSession` and
// `crate::app::scan_service::ScanService` resolve from inside the test modules.
pub use riff_backend::app;
pub use riff_backend::domain;
pub use riff_gui::ui;
pub use riff_infra;

// Prelude of bare names used inside the test bodies through `use super::*`.
// Kept explicit (rather than glob re-exports) to avoid name collisions.
pub use riff_backend::app::MutexExt;
pub use riff_backend::app::gapless::{
    GaplessConditions, duration_from_frames, elapsed_from_samples, frames_from_duration,
    is_gapless_eligible, pre_buffer_cap, samples_from_duration,
};
pub use riff_backend::app::state::{
    LibrarySession, LibraryStatus, PlaybackQueue, PlaybackSession, ReplayGainMode, WatchState,
    replaygain_factor,
};
pub use riff_backend::app::store::SortDirection;
pub use riff_backend::app::transport::clamp_seek;
pub use riff_backend::domain::{
    Album, Artist, PlaybackCommand, PlaybackPosition, PlaybackState, PlaybackUpdate, Playlist,
    PlaylistId, RepeatMode, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};
pub use riff_gui::ui::app::{format_duration, lru_insert};
pub use riff_gui::ui::selection::TagDraft;
pub use riff_gui::ui::settings::{expand_tilde, suggest_directories};
pub use riff_infra::audio::{CpalAudioOutput, SymphoniaDecoder};
pub use riff_infra::filesystem::{AudioFileScanner, FilesystemWatcher};
pub use riff_infra::media::metadata_reader::parse_replaygain_gain;
pub use riff_infra::media::{ImageCoverLoader, LoftyMetadataReader, LoftyMetadataWriter};
pub use riff_persistence::store::{
    CustomSmartPlaylist, RuleTree, ScanOptions, SmartPlaylistTree, UnreadableRule,
};

// Standard-library names referenced unqualified in some suites.
pub use std::path::PathBuf;
pub use std::sync::atomic::AtomicBool;
pub use std::sync::{Arc, Mutex};

/// An `Arc<AtomicBool>` stop flag no test ever sets.
///
/// The service workers take their stop flag as a constructor argument, and
/// every test that builds a worker by hand ends it the way the code did
/// before the runtime gained a lifecycle: by dropping the front-end handle.
/// Those tests therefore need a flag that stays false for the whole test.
/// Tests that do exercise shutdown take their flags from the
/// `RuntimeLifecycle` the Composition Root returns instead of from here.
#[must_use]
pub fn inert_stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// The two halves every [`riff_library::app::traits::CoverLoader`] fake shares.
///
/// They delegate to `riff-infra`'s real decode and encode rather than inventing
/// their own, because the whole point of the split is that the cached route and
/// the source route run one implementation: a fake that stubbed `decode_thumbnail`
/// would let the suite pass while the two routes drifted apart in production. The
/// root suite depends on `riff-infra`, so it can hold the real thing.
#[allow(clippy::missing_errors_doc)]
pub fn fake_decode_thumbnail(
    bytes: &[u8],
    size: riff_library::app::traits::RequestedSize,
) -> Result<riff_library::app::traits::DecodedCover, riff_library::app::errors::LibraryError> {
    riff_infra::media::cover_loader::decode_thumbnail(bytes, size)
}

/// The encode half, for a fake's write path. See [`fake_decode_thumbnail`].
#[allow(clippy::missing_errors_doc)]
pub fn fake_encode_thumbnail(
    cover: &riff_library::app::traits::DecodedCover,
) -> Result<riff_persistence::thumbnail::EncodedThumbnail, riff_library::app::errors::LibraryError>
{
    riff_infra::media::cover_loader::encode_thumbnail(cover)
}

// Test utilities that can be used across test modules
pub mod test_utils {
    use crate::domain::{TrackId, TrackMetadata};
    use std::path::PathBuf;

    /// Approximate equality for `f32` values. Tests compare audio parameters
    /// (volume, sample values) with this instead of exact `==` so assertions
    /// stay robust to float representation (`clippy::float_cmp`).
    #[must_use]
    pub fn float_close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-6
    }

    /// Create a test track with the given ID and file path
    pub fn create_test_track(id: &str, file_path: &str) -> crate::domain::Track {
        crate::domain::Track::fresh_scan(
            TrackId(id.to_string()),
            PathBuf::from(file_path),
            TrackMetadata::default(),
            None,
            None,
            None,
            None,
        )
    }

    /// Create a test track with custom metadata
    pub fn create_test_track_with_metadata(
        id: &str,
        file_path: &str,
        artist: &str,
        title: &str,
        album: &str,
    ) -> crate::domain::Track {
        crate::domain::Track::fresh_scan(
            TrackId(id.to_string()),
            PathBuf::from(file_path),
            TrackMetadata {
                artist: Some(artist.to_string()),
                title: Some(title.to_string()),
                album: Some(album.to_string()),
                ..Default::default()
            },
            None,
            None,
            None,
            None,
        )
    }

    /// Is pixel `p` within `tol` of every channel of `color`?
    ///
    /// The ONE per-channel tolerance rule every pixel counter in this crate
    /// shares. A rendered frame antialiases rounded corners and dithers flat
    /// fills, so a channel lands a hair off the design token; `tol` absorbs
    /// that hair. Each caller passes its own tolerance, which keeps every
    /// assertion's slack visible at the call instead of held by prose.
    #[must_use]
    pub fn near(p: &image::Rgba<u8>, color: egui::Color32, tol: u8) -> bool {
        p.0[0].abs_diff(color.r()) <= tol
            && p.0[1].abs_diff(color.g()) <= tol
            && p.0[2].abs_diff(color.b()) <= tol
    }

    /// Count the pixels of `image` within `tol` of every channel of `color`.
    #[must_use]
    pub fn count_near(image: &image::RgbaImage, color: egui::Color32, tol: u8) -> usize {
        image.pixels().filter(|p| near(p, color, tol)).count()
    }

    /// [`count_near`] scoped to `rect`.
    ///
    /// The rect's edges are floored/ceiled onto whole pixels and clamped to
    /// the image bounds; the counted range is the half-open
    /// `[x0, x1) * [y0, y1)`.
    #[must_use]
    pub fn count_near_in(
        image: &image::RgbaImage,
        color: egui::Color32,
        rect: egui::Rect,
        tol: u8,
    ) -> usize {
        let (width, height) = image.dimensions();
        let x0 = rect.min.x.floor().max(0.0) as u32;
        let y0 = rect.min.y.floor().max(0.0) as u32;
        let x1 = (rect.max.x.ceil().max(0.0) as u32).min(width);
        let y1 = (rect.max.y.ceil().max(0.0) as u32).min(height);
        image
            .enumerate_pixels()
            .filter(|(x, y, p)| *x >= x0 && *x < x1 && *y >= y0 && *y < y1 && near(p, color, tol))
            .count()
    }
}

/// Shared fakes for exercising the port boundaries without real audio
/// hardware or files on disk. Each fake implements the ONE port trait the
/// production code consumes — the playback capability's audio ports and the
/// library capability's media ports — so a test's substitute and the real
/// adapter answer to the same interface.
///
/// The audio fakes record through an `Arc<Mutex<..>>` they hand out clones
/// of: the engine takes ownership of one clone and the test keeps another, so
/// the counters stay readable while and after it runs.
pub mod mocks {
    use riff_backend::app::cover_service::{ClearCacheOutcome, Covers};
    use riff_backend::app::state::PlaybackSession;
    use riff_backend::app::store::{
        CustomSmartPlaylist, LibraryMutationStore, LibraryQueryStore, PlaylistStore, Settings,
        SettingsStore, SmartPlaylistTree, SortDirection,
    };
    use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
    use riff_backend::app::traits::{
        CoverLoader, DecodedCover, MetadataReader, MetadataWriter, RequestedSize, TagEdit,
    };
    use riff_backend::app::transport::clamp_seek;
    use riff_backend::domain::{
        Album, Artist, CoverSource, GenreCount, Playlist, PlaylistId, RepeatMode,
        SmartPlaylistKind, Track, TrackId, TrackMetadata,
    };
    use riff_library::app::errors::LibraryError;
    use riff_persistence::errors::StoreError;
    use riff_playback::app::errors::PlaybackError;
    use riff_playback::infra::ports::{AudioDecoder, AudioFormatInfo, AudioOutput};
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Scripted [`AudioDecoder`]: `init` returns the configured format,
    /// `next_frames` drains a queue of sample batches and then reports EOF,
    /// and every `seek` is recorded and resets the stream to the start of the
    /// script. Cloning hands out another handle to the same recording state.
    #[derive(Clone)]
    pub struct MockAudioDecoder {
        /// The path this decoder instance last opened, as the engine sees it.
        source: PathBuf,
        state: Arc<Mutex<DecoderState>>,
    }

    struct DecoderState {
        format: AudioFormatInfo,
        duration: Option<Duration>,
        /// Full sample script; `queue` is refilled from this on `init` and
        /// reset on `seek`.
        scripted: Vec<Vec<f32>>,
        queue: Vec<Vec<f32>>,
        seeks: Vec<Duration>,
        opened: Vec<PathBuf>,
    }

    impl MockAudioDecoder {
        pub fn new(format: AudioFormatInfo) -> Self {
            Self {
                source: PathBuf::new(),
                state: Arc::new(Mutex::new(DecoderState {
                    format,
                    duration: None,
                    scripted: Vec::new(),
                    queue: Vec::new(),
                    seeks: Vec::new(),
                    opened: Vec::new(),
                })),
            }
        }

        /// Script the sample batches that `next_frames` will yield, in order.
        #[must_use]
        pub fn with_batches(self, batches: Vec<Vec<f32>>) -> Self {
            self.state.lock().unwrap().scripted = batches;
            self
        }

        /// Set what the port's `duration` answers with.
        #[must_use]
        pub fn with_duration(self, duration: Option<Duration>) -> Self {
            self.state.lock().unwrap().duration = duration;
            self
        }

        /// Every path this decoder has opened, in order.
        #[must_use]
        pub fn opened(&self) -> Vec<PathBuf> {
            self.state.lock().unwrap().opened.clone()
        }

        /// Every position this decoder has been seeked to, in order.
        #[must_use]
        pub fn seeks(&self) -> Vec<Duration> {
            self.state.lock().unwrap().seeks.clone()
        }
    }

    impl AudioDecoder for MockAudioDecoder {
        fn source_path(&self) -> &Path {
            &self.source
        }

        fn init(&mut self, path: &Path) -> Result<AudioFormatInfo, PlaybackError> {
            self.source = path.to_path_buf();
            let mut state = self.state.lock().unwrap();
            state.opened.push(path.to_path_buf());
            state.queue = state.scripted.clone();
            Ok(state.format.clone())
        }

        fn next_frames(&mut self, buf: &mut [f32]) -> Option<usize> {
            let mut state = self.state.lock().unwrap();
            // `None` here is the scripted EOF.
            let batch = state.queue.first_mut()?;
            // Fill as much of `buf` as the current scripted batch holds; a
            // batch larger than `buf` keeps its remainder queued for the next
            // call, mirroring how the real decoder spills oversized packets
            // into `pending_samples` (nothing is ever dropped).
            let n = buf.len().min(batch.len());
            buf[..n].copy_from_slice(&batch[..n]);
            batch.drain(..n);
            if batch.is_empty() {
                state.queue.remove(0);
            }
            Some(n)
        }

        fn seek(&mut self, position: Duration) -> Duration {
            let mut state = self.state.lock().unwrap();
            state.seeks.push(position);
            state.queue = state.scripted.clone();
            position
        }

        fn duration(&self) -> Option<Duration> {
            self.state.lock().unwrap().duration
        }
    }

    /// Recording [`AudioOutput`]: tracks every invocation the engine makes on
    /// the port. Cloning hands out another handle to the same recording state.
    #[derive(Clone)]
    pub struct MockAudioOutput {
        state: Arc<Mutex<OutputState>>,
    }

    struct OutputState {
        /// The `(sample rate, channels)` of each stream the engine started.
        started: Vec<(u32, u16)>,
        /// The `(sample rate, channels)` of each `start` call that failed and
        /// therefore never opened a stream.
        failed_starts: Vec<(u32, u16)>,
        stop_count: usize,
        written: Vec<Vec<f32>>,
        volumes: Vec<f32>,
        /// When set, every `start` fails instead of opening a stream — the
        /// engine's failure path, driven on demand.
        fail_starts: bool,
    }

    impl MockAudioOutput {
        pub fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(OutputState {
                    started: Vec::new(),
                    failed_starts: Vec::new(),
                    stop_count: 0,
                    written: Vec::new(),
                    volumes: Vec::new(),
                    fail_starts: false,
                })),
            }
        }

        /// Make every later `start` fail (or succeed again) — the one switch
        /// that drives the engine's failed-stream-start path.
        pub fn set_fail_starts(&self, fail: bool) {
            self.state.lock().unwrap().fail_starts = fail;
        }

        /// The format each started stream was built with, in order.
        #[must_use]
        pub fn started(&self) -> Vec<(u32, u16)> {
            self.state.lock().unwrap().started.clone()
        }

        /// The format each failed `start` asked for, in order — attempts that
        /// never opened a stream, so they are absent from
        /// [`MockAudioOutput::started`].
        #[must_use]
        pub fn failed_starts(&self) -> Vec<(u32, u16)> {
            self.state.lock().unwrap().failed_starts.clone()
        }

        /// How many streams were started in total.
        #[must_use]
        pub fn start_count(&self) -> usize {
            self.state.lock().unwrap().started.len()
        }

        #[must_use]
        pub fn stop_count(&self) -> usize {
            self.state.lock().unwrap().stop_count
        }

        /// Every sample batch written, in order.
        #[must_use]
        pub fn written(&self) -> Vec<Vec<f32>> {
            self.state.lock().unwrap().written.clone()
        }

        /// Every volume set, in order.
        #[must_use]
        pub fn volumes(&self) -> Vec<f32> {
            self.state.lock().unwrap().volumes.clone()
        }
    }

    impl Default for MockAudioOutput {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AudioOutput for MockAudioOutput {
        fn start(&mut self, format: AudioFormatInfo) -> Result<(), PlaybackError> {
            let mut state = self.state.lock().unwrap();
            if state.fail_starts {
                state
                    .failed_starts
                    .push((format.sample_rate, format.channels));
                return Err(PlaybackError::AudioOutput(
                    "mock refuses to open a stream".to_string(),
                ));
            }
            state.started.push((format.sample_rate, format.channels));
            Ok(())
        }

        fn write(&mut self, samples: &[f32]) {
            let mut state = self.state.lock().unwrap();
            state.written.push(samples.to_vec());
        }

        fn stop(&mut self) {
            self.state.lock().unwrap().stop_count += 1;
        }

        fn set_volume(&mut self, volume: f32) {
            self.state.lock().unwrap().volumes.push(volume);
        }
    }

    /// Canned [`MetadataReader`] over the library capability's reader port:
    /// returns configured values, or an injected `LibraryError::MetadataRead`
    /// when `fail` is set.
    pub struct MockMetadataReader {
        pub fail: bool,
        pub metadata: TrackMetadata,
        pub duration: Option<Duration>,
        pub cover_source: CoverSource,
        pub audio_format: riff_library::app::traits::AudioFormatInfo,
    }

    impl Default for MockMetadataReader {
        fn default() -> Self {
            Self {
                fail: false,
                metadata: TrackMetadata::default(),
                duration: Some(Duration::from_secs(90)),
                cover_source: CoverSource::None,
                audio_format: riff_library::app::traits::AudioFormatInfo {
                    sample_rate: 44_100,
                    channels: 2,
                },
            }
        }
    }

    impl MetadataReader for MockMetadataReader {
        fn read_cover_source(&self, _path: &Path) -> Result<CoverSource, LibraryError> {
            if self.fail {
                return Err(LibraryError::MetadataRead("mock failure".to_string()));
            }
            Ok(self.cover_source.clone())
        }

        fn read_all(
            &self,
            _path: &Path,
        ) -> Result<
            (
                TrackMetadata,
                Duration,
                CoverSource,
                riff_library::app::traits::AudioFormatInfo,
            ),
            LibraryError,
        > {
            if self.fail {
                return Err(LibraryError::MetadataRead("mock failure".to_string()));
            }
            Ok((
                self.metadata.clone(),
                self.duration.unwrap_or_default(),
                self.cover_source.clone(),
                self.audio_format.clone(),
            ))
        }
    }

    /// Recording [`FilesystemWatch`](riff_backend::app::traits::FilesystemWatch)
    /// — the second adapter at the watch seam that ADR 0006 anticipates. It
    /// answers which roots the watcher was asked to follow and which it was
    /// asked to drop, so a test can prove a retired Library Path stops being
    /// watched without touching a platform event stream.
    #[derive(Clone, Default)]
    pub struct MockFilesystemWatch {
        calls: Arc<Mutex<Vec<(WatchAction, PathBuf)>>>,
    }

    /// What the watch port was asked to do with a path.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum WatchAction {
        Watch,
        Unwatch,
    }

    impl MockFilesystemWatch {
        /// The roots asked to be followed, in order.
        #[must_use]
        pub fn watched(&self) -> Vec<PathBuf> {
            self.paths_for(WatchAction::Watch)
        }

        /// The roots asked to be dropped, in order.
        #[must_use]
        pub fn unwatched(&self) -> Vec<PathBuf> {
            self.paths_for(WatchAction::Unwatch)
        }

        fn paths_for(&self, action: WatchAction) -> Vec<PathBuf> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(recorded, _)| *recorded == action)
                .map(|(_, path)| path.clone())
                .collect()
        }

        fn record(&self, action: WatchAction, path: &Path) {
            self.calls
                .lock()
                .unwrap()
                .push((action, path.to_path_buf()));
        }
    }

    impl riff_backend::app::traits::FilesystemWatch for MockFilesystemWatch {
        fn watch(&mut self, path: &Path) -> Result<(), LibraryError> {
            self.record(WatchAction::Watch, path);
            Ok(())
        }

        fn unwatch(&mut self, path: &Path) -> Result<(), LibraryError> {
            self.record(WatchAction::Unwatch, path);
            Ok(())
        }
    }

    /// Canned [`CoverLoader`]: returns a configured decoded cover, `None`, or
    /// an injected `LibraryError::CoverLoad`. The port belongs to the library
    /// slice, so it answers in that slice's error type.
    ///
    /// It records too: every `load_cover` bumps [`Self::calls`] first, then
    /// pushes the [`CoverSource`] onto [`Self::seen`], then returns
    /// [`Self::result`] — so a test can pin which source reached the loader
    /// and how often it was asked, through handles it keeps itself.
    pub struct MockCoverLoader {
        pub result: Result<Option<DecodedCover>, String>,
        /// Every source `load_cover` was handed, in call order.
        pub seen: Arc<Mutex<Vec<CoverSource>>>,
        /// How many times `load_cover` was entered.
        pub calls: Arc<AtomicUsize>,
    }

    impl CoverLoader for MockCoverLoader {
        fn load_cover(
            &self,
            source: &CoverSource,
            _size: RequestedSize,
        ) -> Result<Option<DecodedCover>, LibraryError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(source.clone());
            self.result.clone().map_err(LibraryError::CoverLoad)
        }

        fn decode_thumbnail(
            &self,
            bytes: &[u8],
            size: RequestedSize,
        ) -> Result<DecodedCover, LibraryError> {
            crate::fake_decode_thumbnail(bytes, size)
        }

        fn encode_thumbnail(
            &self,
            cover: &DecodedCover,
        ) -> Result<riff_persistence::thumbnail::EncodedThumbnail, LibraryError> {
            crate::fake_encode_thumbnail(cover)
        }
    }

    /// Recording [`MetadataWriter`]: successful writes are kept (path + edit)
    /// for assertions; when `fail` is set every write returns an
    /// `LibraryError::MetadataWrite`, simulating an unwritable file (permission
    /// denied, disk full, etc.).
    pub struct MockMetadataWriter {
        pub fail: bool,
        /// When set, the first `n` writes succeed and every later write
        /// fails — lets one test script per-call fates deterministically
        /// (the worker processes requests sequentially).
        pub fail_after_writes: Option<usize>,
        pub writes: Mutex<Vec<(PathBuf, TagEdit)>>,
        /// The `ReplayGain` writes recorded alongside the Metadata ones, in
        /// arrival order — the two write paths stay distinct, so their
        /// records do too.
        pub replaygain_writes: Mutex<Vec<(PathBuf, riff_library::app::traits::ReplayGainTags)>>,
    }

    impl Default for MockMetadataWriter {
        fn default() -> Self {
            Self::recording()
        }
    }

    impl MockMetadataWriter {
        /// A writer that records every write and never fails.
        #[must_use]
        pub fn recording() -> Self {
            Self {
                fail: false,
                fail_after_writes: None,
                writes: Mutex::new(Vec::new()),
                replaygain_writes: Mutex::new(Vec::new()),
            }
        }

        /// A writer that fails every write with a `MetadataWrite` error.
        #[must_use]
        pub fn failing() -> Self {
            Self {
                fail: true,
                fail_after_writes: None,
                writes: Mutex::new(Vec::new()),
                replaygain_writes: Mutex::new(Vec::new()),
            }
        }

        /// A writer whose first `n` writes succeed (and are recorded) and
        /// every subsequent write fails.
        #[must_use]
        pub fn failing_after(n: usize) -> Self {
            Self {
                fail: false,
                fail_after_writes: Some(n),
                writes: Mutex::new(Vec::new()),
                replaygain_writes: Mutex::new(Vec::new()),
            }
        }

        /// Snapshot of every successfully written (path, edit) pair.
        #[must_use]
        pub fn recorded(&self) -> Vec<(PathBuf, TagEdit)> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl MetadataWriter for MockMetadataWriter {
        fn write_tags(&self, path: &Path, edit: &TagEdit) -> Result<(), LibraryError> {
            let spent = self.writes.lock().unwrap().len();
            if self.fail || self.fail_after_writes.is_some_and(|n| spent >= n) {
                return Err(LibraryError::MetadataWrite(format!(
                    "permission denied: {}",
                    path.display()
                )));
            }
            self.writes
                .lock()
                .unwrap()
                .push((path.to_path_buf(), edit.clone()));
            Ok(())
        }
    }

    impl riff_library::app::traits::ReplayGainWriter for MockMetadataWriter {
        fn write_replaygain(
            &self,
            path: &Path,
            tags: &riff_library::app::traits::ReplayGainTags,
        ) -> Result<(), riff_library::app::errors::LibraryError> {
            let spent = self.writes.lock().unwrap().len();
            if self.fail || self.fail_after_writes.is_some_and(|n| spent >= n) {
                return Err(riff_library::app::errors::LibraryError::MetadataWrite(
                    format!("permission denied: {}", path.display()),
                ));
            }
            self.replaygain_writes
                .lock()
                .unwrap()
                .push((path.to_path_buf(), *tags));
            Ok(())
        }
    }

    /// Which [`SettingsStore`] mutation a mock recorded.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum SettingsCall {
        Scalars,
        LibraryPaths,
        WatchStates,
    }

    /// Recording [`SettingsStore`]: starts from defaults, applies every save
    /// to in-memory state (so hydration round-trips), records the call
    /// sequence, and can be switched to fail every mutation.
    ///
    /// The `calls` log is owned, which is what the suites that call
    /// `Preferences` themselves want. A test that hands the mock to something
    /// which owns it — the app shell — cannot read `calls` back afterwards, so
    /// it builds the mock with [`Self::with_shared_calls`] and reads the log
    /// through the handle it keeps.
    pub struct MockSettingsStore {
        pub state: Settings,
        pub calls: Vec<SettingsCall>,
        pub fail: bool,
        /// A second copy of every recorded call, for the owned-elsewhere case.
        /// `None` in the owned-and-inspected style.
        pub shared_calls: Option<Arc<Mutex<Vec<SettingsCall>>>>,
    }

    impl Default for MockSettingsStore {
        fn default() -> Self {
            Self {
                state: Settings {
                    scalars: riff_backend::app::state::ScalarSettings::default(),
                    library_paths: Vec::new(),
                    watch_states: std::collections::HashMap::new(),
                },
                calls: Vec::new(),
                fail: false,
                shared_calls: None,
            }
        }
    }

    impl MockSettingsStore {
        /// A mock that also records into `shared`, which the caller keeps and
        /// reads after the owner holding the mock has dropped.
        pub fn with_shared_calls(shared: Arc<Mutex<Vec<SettingsCall>>>) -> Self {
            Self {
                shared_calls: Some(shared),
                ..Self::default()
            }
        }

        /// Record one mutation, into the owned log and the shared one alike.
        fn record(&mut self, call: SettingsCall) {
            if let Some(shared) = &self.shared_calls {
                shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(call.clone());
            }
            self.calls.push(call);
        }
    }

    impl SettingsStore for MockSettingsStore {
        fn load_settings(&self) -> Result<Settings, StoreError> {
            Ok(self.state.clone())
        }

        fn save_scalars(
            &mut self,
            scalars: &riff_backend::app::state::ScalarSettings,
        ) -> Result<(), StoreError> {
            if self.fail {
                return Err(StoreError::InvalidOperation("mock settings failure".into()));
            }
            self.state.scalars = scalars.clone();
            self.record(SettingsCall::Scalars);
            Ok(())
        }

        fn save_library_paths(&mut self, paths: &[std::path::PathBuf]) -> Result<(), StoreError> {
            if self.fail {
                return Err(StoreError::InvalidOperation("mock settings failure".into()));
            }
            self.state.library_paths = paths.to_vec();
            self.record(SettingsCall::LibraryPaths);
            Ok(())
        }

        fn save_watch_states(
            &mut self,
            states: &std::collections::HashMap<
                std::path::PathBuf,
                riff_backend::app::state::WatchState,
            >,
        ) -> Result<(), StoreError> {
            if self.fail {
                return Err(StoreError::InvalidOperation("mock settings failure".into()));
            }
            self.state.watch_states.clone_from(states);
            self.record(SettingsCall::WatchStates);
            Ok(())
        }
    }

    /// Recording [`LibraryMutationStore`] fake: `apply_tag_refresh` snapshots
    /// every Track it was handed (so tests can pin the refreshed metadata AND
    /// the untouched play history) and can be switched to fail, simulating a
    /// failed store commit. `record_track_played` snapshots every
    /// `(id, played_at)` pair (so playback-continuation tests can pin the
    /// committed plays); the remaining mutations are no-ops returning benign
    /// defaults.
    ///
    /// `Clone` hands out another handle on the SAME recordings: the mock is
    /// the shared recording handle, so a component that takes ownership of
    /// the port and the test that reads the recordings each hold one clone
    /// and neither wraps the other. The failure flag is the one thing a clone
    /// does not carry — it is fixed at construction (`new` records and never
    /// fails, `failing_refresh` always fails), which is what makes a
    /// shareable mock possible.
    #[derive(Clone)]
    pub struct MockLibraryMutationStore {
        /// When set, `apply_tag_refresh` fails with an `InvalidOperation`
        /// error and records nothing.
        pub fail_tag_refresh: bool,
        refreshed: Arc<Mutex<Vec<Track>>>,
        played: Arc<Mutex<Vec<(TrackId, std::time::SystemTime)>>>,
        favorites: Arc<Mutex<Vec<(TrackId, bool)>>>,
        /// Every root passed to `remove_library_path`, in call order.
        removed: Arc<Mutex<Vec<PathBuf>>>,
        /// How many `clear_library` calls landed.
        clears: Arc<Mutex<usize>>,
    }

    impl Default for MockLibraryMutationStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockLibraryMutationStore {
        /// A mutation store that records every tag refresh and never fails.
        #[must_use]
        pub fn new() -> Self {
            Self {
                fail_tag_refresh: false,
                refreshed: Arc::new(Mutex::new(Vec::new())),
                played: Arc::new(Mutex::new(Vec::new())),
                favorites: Arc::new(Mutex::new(Vec::new())),
                removed: Arc::new(Mutex::new(Vec::new())),
                clears: Arc::new(Mutex::new(0)),
            }
        }

        /// A mutation store whose every tag refresh fails with an
        /// `InvalidOperation` error and records nothing, simulating a failed
        /// store commit.
        #[must_use]
        pub fn failing_refresh() -> Self {
            Self {
                fail_tag_refresh: true,
                refreshed: Arc::new(Mutex::new(Vec::new())),
                played: Arc::new(Mutex::new(Vec::new())),
                favorites: Arc::new(Mutex::new(Vec::new())),
                removed: Arc::new(Mutex::new(Vec::new())),
                clears: Arc::new(Mutex::new(0)),
            }
        }

        /// Snapshot of every Track passed to `apply_tag_refresh`, in call
        /// order. Failed commits record nothing.
        #[must_use]
        pub fn refreshed(&self) -> Vec<Track> {
            self.refreshed.lock().unwrap().clone()
        }

        /// Snapshot of every root passed to `remove_library_path`, in call
        /// order.
        #[must_use]
        pub fn removals(&self) -> Vec<PathBuf> {
            self.removed.lock().unwrap().clone()
        }

        /// How many `clear_library` calls landed.
        #[must_use]
        pub fn clear_count(&self) -> usize {
            *self.clears.lock().unwrap()
        }

        /// Snapshot of every `(id, played_at)` passed to
        /// `record_track_played`, in call order.
        #[must_use]
        pub fn played(&self) -> Vec<(TrackId, std::time::SystemTime)> {
            self.played.lock().unwrap().clone()
        }

        /// Snapshot of every `(id, favorite)` passed to
        /// `set_track_favorite`, in call order.
        #[must_use]
        pub fn favorites(&self) -> Vec<(TrackId, bool)> {
            self.favorites.lock().unwrap().clone()
        }
    }

    impl LibraryMutationStore for MockLibraryMutationStore {
        fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn stamp_metadata_version(&mut self, _version: u32) -> Result<(), StoreError> {
            Ok(())
        }

        fn record_track_played(
            &mut self,
            id: &TrackId,
            played_at: std::time::SystemTime,
        ) -> Result<bool, StoreError> {
            self.played.lock().unwrap().push((id.clone(), played_at));
            Ok(true)
        }

        fn set_track_favorite(&mut self, id: &TrackId, favorite: bool) -> Result<bool, StoreError> {
            self.favorites.lock().unwrap().push((id.clone(), favorite));
            Ok(true)
        }

        fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError> {
            if self.fail_tag_refresh {
                return Err(StoreError::InvalidOperation(
                    "mock tag refresh failure".to_string(),
                ));
            }
            self.refreshed.lock().unwrap().push(track.clone());
            Ok(())
        }

        fn remove_library_path(&mut self, root: &Path) -> Result<usize, StoreError> {
            self.removed.lock().unwrap().push(root.to_path_buf());
            Ok(0)
        }

        fn clear_library(&mut self) -> Result<usize, StoreError> {
            *self.clears.lock().unwrap() += 1;
            Ok(0)
        }

        fn record_full_scan_completed(
            &mut self,
            _summary: riff_backend::app::store::FullScanSummary,
        ) -> Result<(), StoreError> {
            Ok(())
        }

        // The four Custom Smart Playlist writes follow this mock's standing rule:
        // a mutation answers "nothing changed" and records nothing. A suite that
        // has to observe a save, a rename, a delete or a Duplicate uses a real
        // `SqliteStore` — the semantics are transactional and the fake has no
        // transaction to give them.
        fn save_smart_playlist(
            &mut self,
            _id: Option<&PlaylistId>,
            _name: &str,
            _tree: &SmartPlaylistTree,
        ) -> Result<PlaylistId, StoreError> {
            Ok(PlaylistId(String::new()))
        }

        fn rename_smart_playlist(
            &mut self,
            _id: &PlaylistId,
            _new_name: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn delete_smart_playlist(&mut self, _id: &PlaylistId) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn duplicate_smart_playlist(
            &mut self,
            _id: &PlaylistId,
        ) -> Result<Option<PlaylistId>, StoreError> {
            Ok(None)
        }
    }

    /// [`PlaylistStore`] fake standing in for the Playlists section of the
    /// Application Store. Reads serve whatever `playlists` / `entries` were
    /// seeded with (empty by default, which is the state most seam tests want)
    /// and every mutation reports "nothing changed" — unless it is refusing
    /// (see `refuse_next_write`). The playlist projection's own behavior is
    /// pinned against real `SQLite` scratch stores in the app tests, not
    /// against this stub.
    #[derive(Clone, Default)]
    pub struct MockPlaylistStore {
        /// When set, every read fails with an `InvalidOperation` error.
        pub fail_loads: bool,
        /// Rows `load_playlists` serves, in the order given.
        pub playlists: Vec<Playlist>,
        /// Rows `load_playlist_entries` serves, keyed by playlist id.
        pub entries:
            std::collections::HashMap<PlaylistId, Vec<riff_backend::app::store::PlaylistEntry>>,
        /// The next create or rename fails with
        /// `StoreError::InvalidOperation(reason)` and commits nothing; every
        /// write after it succeeds. The Application Store raises that ONE
        /// variant for every fault it can meet — `op_error` wraps a constraint
        /// violation, a transaction failure and an IO failure into it alike —
        /// so a test sets the reason it wants reported and crosses the same
        /// single production arm.
        pub refuse_next_write: Option<String>,
        /// Every name a create or rename COMMITTED with, in call order. Cloned
        /// handles share it, so a test can read what its gesture wrote.
        pub committed: Arc<Mutex<Vec<String>>>,
    }

    impl MockPlaylistStore {
        /// A store seeded with `playlists`, each carrying `tracks` as its
        /// entry list. The sidebar's playlist rows and the playlist stage both
        /// read through this one store, so seeding it is what makes a playlist
        /// visible in the real composition.
        #[must_use]
        pub fn with_playlists(playlists: Vec<(PlaylistId, String, Vec<TrackId>)>) -> Self {
            let mut entries = std::collections::HashMap::new();
            let mut rows = Vec::new();
            for (id, name, tracks) in playlists {
                rows.push(Playlist {
                    id: id.clone(),
                    name,
                    tracks: tracks.clone(),
                    created: None,
                });
                entries.insert(
                    id,
                    tracks
                        .into_iter()
                        .map(|id| riff_backend::app::store::PlaylistEntry {
                            id,
                            track: None,
                            valid: true,
                        })
                        .collect(),
                );
            }
            Self {
                playlists: rows,
                entries,
                ..Self::default()
            }
        }
    }

    impl PlaylistStore for MockPlaylistStore {
        fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError> {
            if self.fail_loads {
                return Err(StoreError::InvalidOperation("playlists boom".to_string()));
            }
            Ok(self.playlists.clone())
        }

        fn load_playlist_entries(
            &self,
            id: &PlaylistId,
        ) -> Result<Vec<riff_backend::app::store::PlaylistEntry>, StoreError> {
            if self.fail_loads {
                return Err(StoreError::InvalidOperation("entries boom".to_string()));
            }
            Ok(self.entries.get(id).cloned().unwrap_or_default())
        }

        fn create_playlist(
            &mut self,
            name: &str,
            _initial_tracks: &[TrackId],
        ) -> Result<PlaylistId, StoreError> {
            if let Some(reason) = self.refuse_next_write.take() {
                return Err(StoreError::InvalidOperation(reason));
            }
            self.committed.lock().unwrap().push(name.to_string());
            Ok(PlaylistId::new("mock"))
        }

        fn rename_playlist(
            &mut self,
            _id: &PlaylistId,
            new_name: &str,
        ) -> Result<bool, StoreError> {
            if let Some(reason) = self.refuse_next_write.take() {
                return Err(StoreError::InvalidOperation(reason));
            }
            self.committed.lock().unwrap().push(new_name.to_string());
            Ok(true)
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

    /// One recorded [`Transport`](riff_backend::app::transport::Transport) intent,
    /// in issue order. Seek/volume intents carry the ADAPTED values (clamped
    /// target, effective volume), mirroring what
    /// [`ChannelTransport`](riff_backend::app::transport::ChannelTransport) would
    /// put on the wire.
    #[derive(Debug, Clone, PartialEq)]
    pub enum TransportIntent {
        Play(TrackId),
        PlayMany(TrackId, Vec<TrackId>),
        PlayNext(TrackId),
        AddToQueue(TrackId),
        /// A whole collection enqueued as one batch. Distinct from N
        /// `AddToQueue` sends on purpose: the port's promise is that the queue
        /// mutates once, and a fake that fanned it out could not tell a real
        /// batch from a per-Track loop.
        AddMany(Vec<TrackId>),
        Pause,
        Resume,
        Next,
        Previous,
        Stop,
        /// The clamped seek target.
        Seek(Duration),
        /// The volume command sent to the engine — the effective volume
        /// (zero while muted), per the port's `set_volume` contract.
        ApplyVolume(f32),
        PlayPause,
        ToggleShuffle(bool),
        ToggleRepeat(RepeatMode),
    }

    /// Recording [`Transport`](riff_backend::app::transport::Transport) fake: keeps
    /// every issued intent behind an internal mutex so UI-layer tests can
    /// assert on playback intents instead of raw channel bytes. The
    /// state-coupled methods reuse the production clamp/volume math, exactly
    /// like the real adapter. `Clone` shares the recording cell, so a wired
    /// clone and the test's own handle see the same intents.
    #[derive(Clone)]
    pub struct MockTransport {
        intents: Arc<Mutex<Vec<TransportIntent>>>,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self {
                intents: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// Snapshot of every recorded intent, in issue order.
        #[must_use]
        pub fn recorded(&self) -> Vec<TransportIntent> {
            self.intents.lock().unwrap().clone()
        }

        fn record(&self, intent: TransportIntent) {
            self.intents.lock().unwrap().push(intent);
        }
    }

    impl Default for MockTransport {
        fn default() -> Self {
            Self::new()
        }
    }

    impl riff_backend::app::transport::Transport for MockTransport {
        fn play(&self, track: TrackId) {
            self.record(TransportIntent::Play(track));
        }

        fn play_many(&self, first: TrackId, rest: Vec<TrackId>) {
            self.record(TransportIntent::PlayMany(first, rest));
        }

        fn play_next(&self, track: TrackId) {
            self.record(TransportIntent::PlayNext(track));
        }

        fn add_to_queue(&self, track: TrackId) {
            self.record(TransportIntent::AddToQueue(track));
        }

        fn add_many(&self, tracks: Vec<TrackId>) {
            self.record(TransportIntent::AddMany(tracks));
        }

        fn pause(&self) {
            self.record(TransportIntent::Pause);
        }

        fn resume(&self) {
            self.record(TransportIntent::Resume);
        }

        fn next(&self) {
            self.record(TransportIntent::Next);
        }

        fn previous(&self) {
            self.record(TransportIntent::Previous);
        }

        fn stop(&self) {
            self.record(TransportIntent::Stop);
        }

        fn seek(&self, session: &PlaybackSession, secs: f32) {
            self.record(TransportIntent::Seek(clamp_seek(
                secs,
                session.current_position.total,
            )));
        }

        fn set_volume(&self, session: &mut PlaybackSession, vol: f32) {
            session.current_volume = vol.clamp(0.0, 1.0);
            self.record(TransportIntent::ApplyVolume(session.effective_volume()));
        }

        fn toggle_mute(&self, session: &mut PlaybackSession) {
            session.muted = !session.muted;
            self.record(TransportIntent::ApplyVolume(session.effective_volume()));
        }

        fn toggle_shuffle(&self, session: &mut PlaybackSession) {
            session.queue.set_shuffle(!session.queue.shuffle);
            self.record(TransportIntent::ToggleShuffle(session.queue.shuffle));
        }

        fn toggle_repeat(&self, session: &mut PlaybackSession) {
            session.queue.toggle_repeat();
            self.record(TransportIntent::ToggleRepeat(session.queue.repeat));
        }

        fn play_pause(&self, _session: &PlaybackSession) {
            self.record(TransportIntent::PlayPause);
        }
    }

    /// Which [`LibraryQueryStore`] query a [`MockLibraryQueryStore`]
    /// recorded. Arguments are kept so assertions can pin both call counts
    /// and the exact query shapes the Session Views seam issues. A paged
    /// listing is recorded **twice** when a surface asks for both halves: a
    /// count read and a row read reach the same `*_page` port method, told
    /// apart by the `limit` each carried (0 for a count, a whole window for
    /// rows).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LibraryQueryCall {
        GetTrack(TrackId),
        TracksPage(riff_persistence::store::TrackListOrder, usize, usize),
        AllTrackIds,
        SearchPage(
            String,
            riff_persistence::store::TrackListOrder,
            usize,
            usize,
        ),
        AllArtists,
        ArtistAlbums(String),
        AlbumTracks(String, String),
        FolderHasAudio(PathBuf),
        FolderHasSearchMatch(PathBuf, String),
        TrackIdsInFolderTree(PathBuf),
        TracksInFolder(PathBuf),
        SubdirsWithAudio(PathBuf),
        SmartPlaylist(SmartPlaylistKind, usize),
        LibraryCounts,
        SmartListCounts,
        FolderTrackCount(PathBuf),
        LastFullScan,
        GenreCounts,
        ArtistsInGenre(String),
        ArtistAlbumsInGenre(String, String),
        AlbumTracksInGenre(String, String, String),
        HitAlbumsPage(String, usize, usize),
        HitArtistsPage(String, usize, usize),
        AlbumHitTracks(String, String),
        AlbumIsNameHit(String, String),
        ArtistIsNameHit(String),
        HitAlbumsInGenre(String, usize, usize),
        HitArtistsInGenre(String, usize, usize),
        AlbumHitTracksInGenre(String, String, String),
        HitGenreCounts,
        ArtistsPage(SortDirection, usize, usize),
        AlbumsPage(SortDirection, usize, usize),
        GenresPage(SortDirection, usize, usize),
        ArtistsInGenrePage(String, SortDirection, usize, usize),
        ArtistAlbumsInGenrePage(String, String, SortDirection, usize, usize),
        PickerTexts(String, usize),
        PickerNumbers(String, usize),
        CustomSmartPlaylistPage(
            riff_persistence::store::RuleTree,
            riff_persistence::store::TrackListOrder,
            Option<String>,
            usize,
            usize,
        ),
        SmartPlaylistTree(PlaylistId),
        CustomSmartPlaylists,
    }

    /// Which [`LibraryQueryStore`] query fails while listed in
    /// [`MockLibraryQueryStore::failing`].
    ///
    /// A paged listing is read as **two** store reads, not one: the seam
    /// asks for a listing's total on its own (`*_count`, which reaches the
    /// port asking for zero rows) and for one row at a time on its own
    /// (`*_window`, which asks for a whole window). Each pair of variants
    /// gates one of them, told apart by the limit the read carried, so a
    /// test can fail a listing's count while its rows still serve, or the
    /// reverse.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailingQuery {
        GetTrack,
        PickerTexts,
        PickerNumbers,
        SmartPlaylistTree,
        CustomSmartPlaylists,
        TracksWindow,
        TracksCount,
        SearchWindow,
        SearchCount,
        AllArtists,
        ArtistAlbums,
        SmartPlaylist,
        TrackIdsInFolderTree,
        GenreCounts,
        LibraryCounts,
        HitAlbums,
        HitAlbumsCount,
        HitArtists,
        HitArtistsCount,
        AlbumHitTracks,
        AlbumIsNameHit,
        ArtistIsNameHit,
        HitAlbumsInGenre,
        HitAlbumsInGenreCount,
        HitArtistsInGenre,
        HitArtistsInGenreCount,
        AlbumHitTracksInGenre,
        HitGenreCounts,
        ArtistsWindow,
        ArtistsCount,
        AlbumsWindow,
        AlbumsCount,
        GenresWindow,
        GenresCount,
        ArtistsInGenreWindow,
        ArtistsInGenreCount,
        ArtistAlbumsInGenreWindow,
        ArtistAlbumsInGenreCount,
        CustomSmartPlaylistWindow,
        CustomSmartPlaylistCount,
    }

    /// Canned [`LibraryQueryStore`] fake standing in for the Application
    /// Store's Library collection in `SessionViews` seam tests: every
    /// query serves its configured field, records one
    /// [`LibraryQueryCall`], and fails on demand while listed in
    /// `failing`.
    ///
    /// Configuration happens before wiring; recordings accumulate behind an
    /// internal mutex because the port takes `&self`. Tests that must hand
    /// ownership to the seam keep a shared handle (see
    /// [`SharedQueryStore`]) or lock through the `Arc`.
    pub struct MockLibraryQueryStore {
        // --- canned answers -------------------------------------------------
        /// `get_track` answers keyed by id.
        pub library: std::collections::HashMap<TrackId, Track>,
        /// Rows served by `tracks_page`, in canonical order.
        pub flat: Vec<Track>,
        /// Rows served by `search_page`, in canonical order.
        pub search: Vec<Track>,
        /// Queries that `search_page` treats as matching.
        /// An empty list matches every query (permissive default); once
        /// populated, only listed queries return the canned rows/count.
        pub matching_searches: Vec<String>,
        /// Artists served by `all_artists` and by `artists_page`.
        pub artists: Vec<Artist>,
        /// Albums served by `artist_albums`.
        pub albums: Vec<Album>,
        /// Tracks served by `album_tracks`.
        pub album_tracks: Vec<Track>,
        /// `artist_albums` answers keyed by artist. Consulted first; a miss
        /// falls through to the flat `albums` answer.
        pub albums_by_artist: std::collections::HashMap<String, Vec<Album>>,
        /// `album_tracks` answers keyed by `(album artist, album title)`.
        /// Consulted first; a miss falls through to the flat `album_tracks`
        /// answer.
        pub tracks_by_album: std::collections::HashMap<(String, String), Vec<Track>>,
        /// Tracks served by `smart_playlist`.
        pub smart: Vec<Track>,
        /// Answer served by `library_counts`.
        pub library_counts: riff_backend::app::store::LibraryCounts,
        /// Answer served by `smart_list_counts`.
        pub smart_list_counts: Vec<(SmartPlaylistKind, usize)>,
        /// Answers served by `folder_track_count`, keyed by folder.
        pub folder_track_counts: std::collections::HashMap<PathBuf, usize>,
        /// Answer served by `last_full_scan`.
        pub last_full_scan: Option<std::time::SystemTime>,
        /// Rows served by `genre_counts`.
        pub genre_counts: Vec<GenreCount>,
        /// Artists served by `artists_in_genre` and `artists_in_genre_page`
        /// for any genre.
        pub genre_artists: Vec<Artist>,
        /// Albums served by `artist_albums_in_genre` and
        /// `artist_albums_in_genre_page` for any artist/genre.
        pub genre_albums: Vec<Album>,
        /// Tracks served by `album_tracks_in_genre` for any album/genre.
        pub genre_album_tracks: Vec<Track>,
        /// Albums served by `hit_albums_page`, in canonical order.
        pub hit_albums: Vec<Album>,
        /// Artists served by `hit_artists_page`, name-ascending.
        pub hit_artists: Vec<Artist>,
        /// Tracks served by `album_hit_tracks` for any album.
        pub album_hit_tracks: Vec<Track>,
        /// Albums whose `album_is_name_hit` answers `true` (keys "artist - title").
        pub album_name_hits: Vec<String>,
        /// Artists whose `artist_is_name_hit` answers `true`.
        pub artist_name_hits: Vec<String>,
        /// Albums served by `hit_albums_in_genre`, in canonical order.
        pub hit_albums_in_genre: Vec<Album>,
        /// Artists served by `hit_artists_in_genre`, name-ascending.
        pub hit_artists_in_genre: Vec<Artist>,
        /// Tracks served by `album_hit_tracks_in_genre` for any album/genre.
        pub album_hit_tracks_in_genre: Vec<Track>,
        /// Rows served by `hit_genre_counts`.
        pub hit_genre_counts: Vec<GenreCount>,
        /// Albums served by `albums_page` (flat browsing order); the page
        /// slices this with the direction applied.
        pub paged_albums: Vec<Album>,
        /// Rows served by `genres_page`; the page slices this with the
        /// direction applied.
        pub paged_genres: Vec<GenreCount>,
        /// Answer served by `folder_has_audio`.
        pub folder_has_audio: bool,
        /// Answer served by `folder_has_search_match`.
        pub folder_search_match: bool,
        /// Ids served by `track_ids_in_folder_tree`.
        pub folder_tree_ids: Vec<TrackId>,
        /// Tracks served by `tracks_in_folder`.
        pub folder_direct_tracks: Vec<Track>,
        /// Children served by `subdirs_with_audio` for any folder. A flat
        /// list answers the same children to every node, which is fine for a
        /// one-level tree and INFINITE for a deeper one — the app recurses
        /// into whatever `subdirs_with_audio` returns, so seed
        /// `folder_children_by_dir` for a tree with depth.
        pub folder_children: Vec<PathBuf>,
        /// Children served by `subdirs_with_audio`, keyed by the folder asked
        /// about. Consulted first, with `folder_children` as the fallback, so a
        /// fixture that needs a finite multi-level tree says which directories
        /// sit under which.
        pub folder_children_by_dir: std::collections::HashMap<PathBuf, Vec<PathBuf>>,
        /// Direct track rows served by `tracks_in_folder`, keyed by folder, for
        /// the same reason: one shared list repeats the same rows at every
        /// level of a tree.
        pub tracks_by_dir: std::collections::HashMap<PathBuf, Vec<Track>>,

        // --- value-picker reads ------------------------------------------------
        /// Answer served by `picker_texts` for any field id.
        pub picker_texts: Vec<String>,
        /// Answer served by `picker_numbers` for any field id.
        pub picker_numbers: Vec<i64>,
        /// Trees served by `smart_playlist_tree`, keyed by list id. An id that
        /// is not listed answers `None` — no row — which is the state most
        /// suites want.
        pub smart_playlist_trees: std::collections::HashMap<PlaylistId, SmartPlaylistTree>,
        /// Answer served by `custom_smart_playlists`, in the order the mock
        /// was loaded — the mock hands back creation order exactly as a test
        /// staged it.
        pub custom_smart_playlists: Vec<CustomSmartPlaylist>,
        /// Rows served by `custom_smart_playlist_page`, sliced per window in
        /// list order (the total is the list's length). An empty list answers
        /// the empty Listing Page, which is all a suite that never opens a
        /// custom listing needs — the slice serves page mechanics only:
        /// which Tracks a tree actually matches is the store seam's proof,
        /// never this double's.
        pub custom_page: Vec<Track>,

        // --- failure injection -------------------------------------------------
        /// Queries that fail while listed here.
        pub failing: Vec<FailingQuery>,

        // --- recordings ---------------------------------------------------------
        /// Recorded queries, in call order. Internal: read through the
        /// accessors (`calls`, `window_calls`, `get_track_calls`,
        /// `count_of`) instead of touching this directly; it is `pub` only
        /// so test constructors can use `..Default::default()`.
        pub calls: Mutex<Vec<LibraryQueryCall>>,
    }

    impl Default for MockLibraryQueryStore {
        fn default() -> Self {
            Self {
                library: std::collections::HashMap::new(),
                flat: Vec::new(),
                search: Vec::new(),
                matching_searches: Vec::new(),
                artists: Vec::new(),
                albums: Vec::new(),
                album_tracks: Vec::new(),
                albums_by_artist: std::collections::HashMap::new(),
                tracks_by_album: std::collections::HashMap::new(),
                smart: Vec::new(),
                library_counts: riff_backend::app::store::LibraryCounts::default(),
                smart_list_counts: Vec::new(),
                folder_track_counts: std::collections::HashMap::new(),
                last_full_scan: None,
                genre_counts: Vec::new(),
                genre_artists: Vec::new(),
                genre_albums: Vec::new(),
                genre_album_tracks: Vec::new(),
                hit_albums: Vec::new(),
                hit_artists: Vec::new(),
                album_hit_tracks: Vec::new(),
                album_name_hits: Vec::new(),
                artist_name_hits: Vec::new(),
                hit_albums_in_genre: Vec::new(),
                hit_artists_in_genre: Vec::new(),
                album_hit_tracks_in_genre: Vec::new(),
                hit_genre_counts: Vec::new(),
                paged_albums: Vec::new(),
                paged_genres: Vec::new(),
                folder_has_audio: true,
                folder_search_match: true,
                folder_tree_ids: Vec::new(),
                folder_direct_tracks: Vec::new(),
                folder_children: Vec::new(),
                folder_children_by_dir: std::collections::HashMap::new(),
                tracks_by_dir: std::collections::HashMap::new(),
                picker_texts: Vec::new(),
                picker_numbers: Vec::new(),
                smart_playlist_trees: std::collections::HashMap::new(),
                custom_smart_playlists: Vec::new(),
                custom_page: Vec::new(),
                failing: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl MockLibraryQueryStore {
        /// Snapshot of every recorded query, in call order.
        #[must_use]
        pub fn calls(&self) -> Vec<LibraryQueryCall> {
            self.calls.lock().unwrap().clone()
        }

        /// Every bounded-window **row** read as its `(offset, limit)` pair —
        /// flat, search, and paged-browse listings alike — in call order.
        /// One entry per store read, and a listing's count read is *not* one:
        /// it asks for zero rows, so it is reported by [`Self::count_calls`]
        /// instead.
        #[must_use]
        pub fn window_calls(&self) -> Vec<(usize, usize)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::TracksPage(_, offset, limit)
                    | LibraryQueryCall::SearchPage(_, _, offset, limit)
                    | LibraryQueryCall::ArtistsPage(_, offset, limit)
                    | LibraryQueryCall::AlbumsPage(_, offset, limit)
                    | LibraryQueryCall::GenresPage(_, offset, limit)
                    | LibraryQueryCall::ArtistsInGenrePage(_, _, offset, limit)
                    | LibraryQueryCall::ArtistAlbumsInGenrePage(_, _, _, offset, limit)
                    | LibraryQueryCall::CustomSmartPlaylistPage(_, _, _, offset, limit) => {
                        (*limit > 0).then_some((*offset, *limit))
                    }
                    _ => None,
                })
                .collect()
        }

        /// Every listing **count** read, in call order. A count read reaches
        /// the same page-read port call as a row read but asks for zero
        /// rows, which is what tells the two apart here and what
        /// `FailingQuery::*Count` gates.
        #[must_use]
        pub fn count_calls(&self) -> Vec<(usize, usize)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::TracksPage(_, offset, limit)
                    | LibraryQueryCall::SearchPage(_, _, offset, limit)
                    | LibraryQueryCall::HitAlbumsPage(_, offset, limit)
                    | LibraryQueryCall::HitArtistsPage(_, offset, limit)
                    | LibraryQueryCall::ArtistsPage(_, offset, limit)
                    | LibraryQueryCall::AlbumsPage(_, offset, limit)
                    | LibraryQueryCall::GenresPage(_, offset, limit)
                    | LibraryQueryCall::ArtistsInGenrePage(_, _, offset, limit)
                    | LibraryQueryCall::ArtistAlbumsInGenrePage(_, _, _, offset, limit)
                    | LibraryQueryCall::CustomSmartPlaylistPage(_, _, _, offset, limit) => {
                        (*limit == 0).then_some((*offset, *limit))
                    }
                    _ => None,
                })
                .collect()
        }

        /// Every `get_track` id, in call order.
        #[must_use]
        pub fn get_track_calls(&self) -> Vec<TrackId> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::GetTrack(id) => Some(id.clone()),
                    _ => None,
                })
                .collect()
        }

        /// Every Custom Smart Playlist Listing Page read as
        /// `(order, needle, offset, limit)` — the compiled tree the read was
        /// asked for is on the recorded call itself for shape assertions.
        #[must_use]
        pub fn custom_page_calls(
            &self,
        ) -> Vec<(
            riff_persistence::store::TrackListOrder,
            Option<String>,
            usize,
            usize,
        )> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::CustomSmartPlaylistPage(_, order, needle, offset, limit) => {
                        Some((*order, needle.clone(), *offset, *limit))
                    }
                    _ => None,
                })
                .collect()
        }

        /// Every Custom Smart Playlist tree read, as the list id it was
        /// asked about, in call order.
        #[must_use]
        pub fn smart_playlist_tree_calls(&self) -> Vec<PlaylistId> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|call| match call {
                    LibraryQueryCall::SmartPlaylistTree(id) => Some(id.clone()),
                    _ => None,
                })
                .collect()
        }

        /// How often each single-call query kind fired (counts only).
        #[must_use]
        pub fn count_of(&self, call: &LibraryQueryCall) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|recorded| recorded == &call)
                .count()
        }

        fn record(&self, call: LibraryQueryCall) {
            self.calls.lock().unwrap().push(call);
        }

        /// Whether a paged-listing read of `limit` rows should fail.
        ///
        /// The seam reads a listing as two store reads, told apart by the
        /// limit each carried: a count read asks for zero rows, a row read
        /// for a whole window. So `window` gates the row read, `count` gates
        /// the count read, and a test can fail one while the other serves.
        fn page_fails(&self, window: FailingQuery, count: FailingQuery, limit: usize) -> bool {
            if limit == 0 {
                self.failing.contains(&count)
            } else {
                self.failing.contains(&window)
            }
        }

        /// Whether `query` counts as a match against the canned search rows.
        fn search_matches(&self, query: &str) -> bool {
            self.matching_searches.is_empty() || self.matching_searches.iter().any(|q| q == query)
        }

        /// Slice `rows` into one bounded window with the direction applied —
        /// the shared backing of the paged browse listing reads
        /// (artists, albums, genres, and the genre drill-downs all serve
        /// their canned list).
        fn window_rows<T: Clone>(
            &self,
            rows: &[T],
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Vec<T> {
            let mut rows = rows.to_vec();
            if direction == SortDirection::Descending {
                rows.reverse();
            }
            rows.into_iter().skip(offset).take(limit).collect()
        }
    }

    impl LibraryQueryStore for MockLibraryQueryStore {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            self.record(LibraryQueryCall::GetTrack(id.clone()));
            if self.failing.contains(&FailingQuery::GetTrack) {
                return Err(StoreError::InvalidOperation("store boom".to_string()));
            }
            Ok(self.library.get(id).cloned())
        }

        fn metadata_version(&self) -> Result<u32, StoreError> {
            Ok(riff_persistence::track::METADATA_VERSION)
        }

        fn tracks_page(
            &self,
            order: riff_persistence::store::TrackListOrder,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.record(LibraryQueryCall::TracksPage(order, offset, limit));
            if self.page_fails(FailingQuery::TracksWindow, FailingQuery::TracksCount, limit) {
                return Err(StoreError::InvalidOperation("loader boom".to_string()));
            }
            let total = self.flat.len();
            let rows = self.flat.iter().skip(offset).take(limit).cloned().collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn library_counts(&self) -> Result<riff_backend::app::store::LibraryCounts, StoreError> {
            self.record(LibraryQueryCall::LibraryCounts);
            if self.failing.contains(&FailingQuery::LibraryCounts) {
                return Err(StoreError::InvalidOperation("counts boom".to_string()));
            }
            Ok(self.library_counts)
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            self.record(LibraryQueryCall::AllTrackIds);
            Ok(self.flat.iter().map(|t| t.id.clone()).collect())
        }

        fn search_page(
            &self,
            query: &str,
            order: riff_persistence::store::TrackListOrder,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.record(LibraryQueryCall::SearchPage(
                query.to_string(),
                order,
                offset,
                limit,
            ));
            if self.page_fails(FailingQuery::SearchWindow, FailingQuery::SearchCount, limit) {
                return Err(StoreError::InvalidOperation("search boom".to_string()));
            }
            if !self.search_matches(query) {
                return Ok(riff_persistence::store::Page::new(0, Vec::new()));
            }
            let total = self.search.len();
            let rows = self
                .search
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn custom_smart_playlist_page(
            &self,
            tree: &riff_persistence::store::RuleTree,
            order: riff_persistence::store::TrackListOrder,
            needle: Option<&str>,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.record(LibraryQueryCall::CustomSmartPlaylistPage(
                tree.clone(),
                order,
                needle.map(str::to_string),
                offset,
                limit,
            ));
            if self.page_fails(
                FailingQuery::CustomSmartPlaylistWindow,
                FailingQuery::CustomSmartPlaylistCount,
                limit,
            ) {
                return Err(StoreError::InvalidOperation(
                    "custom smart playlist boom".to_string(),
                ));
            }
            // The canned rows slice into windows exactly like the flat list's
            // do, which is the page mechanics a port-contract test pins.
            // Which Tracks a tree actually matches is the store seam's
            // proof, never this double's.
            let total = self.custom_page.len();
            let rows = self
                .custom_page
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            self.record(LibraryQueryCall::AllArtists);
            if self.failing.contains(&FailingQuery::AllArtists) {
                return Err(StoreError::InvalidOperation("artists boom".to_string()));
            }
            Ok(self.artists.clone())
        }

        fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError> {
            self.record(LibraryQueryCall::ArtistAlbums(artist.to_string()));
            if self.failing.contains(&FailingQuery::ArtistAlbums) {
                return Err(StoreError::InvalidOperation("albums boom".to_string()));
            }
            if let Some(albums) = self.albums_by_artist.get(artist) {
                return Ok(albums.clone());
            }
            Ok(self.albums.clone())
        }

        fn album_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumTracks(
                album_artist.to_string(),
                album_title.to_string(),
            ));
            if let Some(tracks) = self
                .tracks_by_album
                .get(&(album_artist.to_string(), album_title.to_string()))
            {
                return Ok(tracks.clone());
            }
            Ok(self.album_tracks.clone())
        }

        fn folder_has_audio(&self, folder: &Path) -> Result<bool, StoreError> {
            self.record(LibraryQueryCall::FolderHasAudio(folder.to_path_buf()));
            Ok(self.folder_has_audio)
        }

        fn folder_has_search_match(&self, folder: &Path, query: &str) -> Result<bool, StoreError> {
            self.record(LibraryQueryCall::FolderHasSearchMatch(
                folder.to_path_buf(),
                query.to_string(),
            ));
            Ok(self.folder_search_match)
        }

        fn track_ids_in_folder_tree(&self, folder: &Path) -> Result<Vec<TrackId>, StoreError> {
            self.record(LibraryQueryCall::TrackIdsInFolderTree(folder.to_path_buf()));
            if self.failing.contains(&FailingQuery::TrackIdsInFolderTree) {
                return Err(StoreError::InvalidOperation("folder tree boom".to_string()));
            }
            Ok(self.folder_tree_ids.clone())
        }

        fn tracks_in_folder(&self, folder: &Path) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::TracksInFolder(folder.to_path_buf()));
            Ok(self
                .tracks_by_dir
                .get(folder)
                .cloned()
                .unwrap_or_else(|| self.folder_direct_tracks.clone()))
        }

        fn folder_track_count(&self, folder: &Path) -> Result<usize, StoreError> {
            self.record(LibraryQueryCall::FolderTrackCount(folder.to_path_buf()));
            Ok(self.folder_track_counts.get(folder).copied().unwrap_or(0))
        }

        fn last_full_scan(
            &self,
        ) -> Result<Option<riff_backend::app::store::FullScanSummary>, StoreError> {
            self.record(LibraryQueryCall::LastFullScan);
            Ok(self
                .last_full_scan
                .map(|at| riff_backend::app::store::FullScanSummary {
                    at,
                    files: 0,
                    errors: 0,
                }))
        }

        fn subdirs_with_audio(&self, folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
            self.record(LibraryQueryCall::SubdirsWithAudio(folder.to_path_buf()));
            Ok(self
                .folder_children_by_dir
                .get(folder)
                .cloned()
                .unwrap_or_else(|| self.folder_children.clone()))
        }

        fn smart_playlist(
            &self,
            kind: SmartPlaylistKind,
            limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::SmartPlaylist(kind, limit));
            if self.failing.contains(&FailingQuery::SmartPlaylist) {
                return Err(StoreError::InvalidOperation(
                    "smart playlist boom".to_string(),
                ));
            }
            Ok(self.smart.clone())
        }

        fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError> {
            self.record(LibraryQueryCall::SmartListCounts);
            Ok(self.smart_list_counts.clone())
        }

        fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError> {
            self.record(LibraryQueryCall::GenreCounts);
            if self.failing.contains(&FailingQuery::GenreCounts) {
                return Err(StoreError::InvalidOperation(
                    "genre counts boom".to_string(),
                ));
            }
            Ok(self.genre_counts.clone())
        }

        fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError> {
            self.record(LibraryQueryCall::ArtistsInGenre(genre.to_string()));
            Ok(self.genre_artists.clone())
        }

        fn artist_albums_in_genre(
            &self,
            artist: &str,
            genre: &str,
        ) -> Result<Vec<Album>, StoreError> {
            self.record(LibraryQueryCall::ArtistAlbumsInGenre(
                artist.to_string(),
                genre.to_string(),
            ));
            Ok(self.genre_albums.clone())
        }

        fn album_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumTracksInGenre(
                album_artist.to_string(),
                album_title.to_string(),
                genre.to_string(),
            ));
            Ok(self.genre_album_tracks.clone())
        }

        fn hit_albums_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.record(LibraryQueryCall::HitAlbumsPage(
                query.to_string(),
                offset,
                limit,
            ));
            if self.page_fails(FailingQuery::HitAlbums, FailingQuery::HitAlbumsCount, limit) {
                return Err(StoreError::InvalidOperation("hit albums boom".to_string()));
            }
            if !self.search_matches(query) {
                return Ok(riff_persistence::store::Page::new(0, Vec::new()));
            }
            let total = self.hit_albums.len();
            let rows = self
                .hit_albums
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn hit_artists_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.record(LibraryQueryCall::HitArtistsPage(
                query.to_string(),
                offset,
                limit,
            ));
            if self.page_fails(
                FailingQuery::HitArtists,
                FailingQuery::HitArtistsCount,
                limit,
            ) {
                return Err(StoreError::InvalidOperation("hit artists boom".to_string()));
            }
            if !self.search_matches(query) {
                return Ok(riff_persistence::store::Page::new(0, Vec::new()));
            }
            let total = self.hit_artists.len();
            let rows = self
                .hit_artists
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn album_hit_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumHitTracks(
                album_artist.to_string(),
                album_title.to_string(),
            ));
            if self.failing.contains(&FailingQuery::AlbumHitTracks) {
                return Err(StoreError::InvalidOperation(
                    "album hit tracks boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self.album_hit_tracks.clone())
        }

        fn album_is_name_hit(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<bool, StoreError> {
            self.record(LibraryQueryCall::AlbumIsNameHit(
                album_artist.to_string(),
                album_title.to_string(),
            ));
            if self.failing.contains(&FailingQuery::AlbumIsNameHit) {
                return Err(StoreError::InvalidOperation(
                    "album name hit boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(false);
            }
            Ok(self
                .album_name_hits
                .contains(&format!("{album_artist} - {album_title}")))
        }

        fn artist_is_name_hit(&self, artist: &str, query: &str) -> Result<bool, StoreError> {
            self.record(LibraryQueryCall::ArtistIsNameHit(artist.to_string()));
            if self.failing.contains(&FailingQuery::ArtistIsNameHit) {
                return Err(StoreError::InvalidOperation(
                    "artist name hit boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(false);
            }
            Ok(self.artist_name_hits.contains(&artist.to_string()))
        }

        fn hit_albums_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Album>, StoreError> {
            self.record(LibraryQueryCall::HitAlbumsInGenre(
                genre.to_string(),
                offset,
                limit,
            ));
            if self.failing.contains(&FailingQuery::HitAlbumsInGenre) {
                return Err(StoreError::InvalidOperation(
                    "hit albums in genre boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self
                .hit_albums_in_genre
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn hit_artists_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Artist>, StoreError> {
            self.record(LibraryQueryCall::HitArtistsInGenre(
                genre.to_string(),
                offset,
                limit,
            ));
            if self.failing.contains(&FailingQuery::HitArtistsInGenre) {
                return Err(StoreError::InvalidOperation(
                    "hit artists in genre boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self
                .hit_artists_in_genre
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn album_hit_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.record(LibraryQueryCall::AlbumHitTracksInGenre(
                album_artist.to_string(),
                album_title.to_string(),
                genre.to_string(),
            ));
            if self.failing.contains(&FailingQuery::AlbumHitTracksInGenre) {
                return Err(StoreError::InvalidOperation(
                    "album hit tracks in genre boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self.album_hit_tracks_in_genre.clone())
        }

        fn hit_genre_counts(&self, query: &str) -> Result<Vec<GenreCount>, StoreError> {
            self.record(LibraryQueryCall::HitGenreCounts);
            if self.failing.contains(&FailingQuery::HitGenreCounts) {
                return Err(StoreError::InvalidOperation(
                    "hit genre counts boom".to_string(),
                ));
            }
            if !self.search_matches(query) {
                return Ok(Vec::new());
            }
            Ok(self.hit_genre_counts.clone())
        }

        fn artists_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.record(LibraryQueryCall::ArtistsPage(direction, offset, limit));
            if self.page_fails(
                FailingQuery::ArtistsWindow,
                FailingQuery::ArtistsCount,
                limit,
            ) {
                return Err(StoreError::InvalidOperation(
                    "artists listing boom".to_string(),
                ));
            }
            let total = self.artists.len();
            let rows = self.window_rows(&self.artists, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn albums_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.record(LibraryQueryCall::AlbumsPage(direction, offset, limit));
            if self.page_fails(FailingQuery::AlbumsWindow, FailingQuery::AlbumsCount, limit) {
                return Err(StoreError::InvalidOperation(
                    "albums listing boom".to_string(),
                ));
            }
            let total = self.paged_albums.len();
            let rows = self.window_rows(&self.paged_albums, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn genres_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<GenreCount>, StoreError> {
            self.record(LibraryQueryCall::GenresPage(direction, offset, limit));
            if self.page_fails(FailingQuery::GenresWindow, FailingQuery::GenresCount, limit) {
                return Err(StoreError::InvalidOperation(
                    "genres listing boom".to_string(),
                ));
            }
            let total = self.paged_genres.len();
            let rows = self.window_rows(&self.paged_genres, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn artists_in_genre_page(
            &self,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.record(LibraryQueryCall::ArtistsInGenrePage(
                genre.to_string(),
                direction,
                offset,
                limit,
            ));
            if self.page_fails(
                FailingQuery::ArtistsInGenreWindow,
                FailingQuery::ArtistsInGenreCount,
                limit,
            ) {
                return Err(StoreError::InvalidOperation(
                    "genre artists listing boom".to_string(),
                ));
            }
            let total = self.genre_artists.len();
            let rows = self.window_rows(&self.genre_artists, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        fn artist_albums_in_genre_page(
            &self,
            artist: &str,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.record(LibraryQueryCall::ArtistAlbumsInGenrePage(
                artist.to_string(),
                genre.to_string(),
                direction,
                offset,
                limit,
            ));
            if self.page_fails(
                FailingQuery::ArtistAlbumsInGenreWindow,
                FailingQuery::ArtistAlbumsInGenreCount,
                limit,
            ) {
                return Err(StoreError::InvalidOperation(
                    "genre album listing boom".to_string(),
                ));
            }
            let total = self.genre_albums.len();
            let rows = self.window_rows(&self.genre_albums, direction, offset, limit);
            Ok(riff_persistence::store::Page::new(total, rows))
        }

        /// Records the field id and the bound it was asked for, so a suite can
        /// pin the read's cadence (one per picker open, never per frame). The
        /// canned answer is shape-independent: `MockLibraryQueryStore` serves
        /// the same list whatever the id, because which ids a real store
        /// catalogues is the adapter's own test's question.
        fn picker_texts(&self, field_id: &str, limit: usize) -> Result<Vec<String>, StoreError> {
            self.record(LibraryQueryCall::PickerTexts(field_id.to_string(), limit));
            if self.failing.contains(&FailingQuery::PickerTexts) {
                return Err(StoreError::InvalidOperation(
                    "picker texts boom".to_string(),
                ));
            }
            Ok(self.picker_texts.iter().take(limit).cloned().collect())
        }

        fn picker_numbers(&self, field_id: &str, limit: usize) -> Result<Vec<i64>, StoreError> {
            self.record(LibraryQueryCall::PickerNumbers(field_id.to_string(), limit));
            if self.failing.contains(&FailingQuery::PickerNumbers) {
                return Err(StoreError::InvalidOperation(
                    "picker numbers boom".to_string(),
                ));
            }
            Ok(self.picker_numbers.iter().take(limit).copied().collect())
        }

        /// Records the list id it was asked about and serves the seeded tree, or
        /// `None` for a list the suite did not seed. Which ids a real store has
        /// is the adapter's own test's question, not this double's.
        fn smart_playlist_tree(
            &self,
            id: &PlaylistId,
        ) -> Result<Option<SmartPlaylistTree>, StoreError> {
            self.record(LibraryQueryCall::SmartPlaylistTree(id.clone()));
            if self.failing.contains(&FailingQuery::SmartPlaylistTree) {
                return Err(StoreError::InvalidOperation(
                    "rule tree read boom".to_string(),
                ));
            }
            Ok(self.smart_playlist_trees.get(id).cloned())
        }

        fn custom_smart_playlists(&self) -> Result<Vec<CustomSmartPlaylist>, StoreError> {
            self.record(LibraryQueryCall::CustomSmartPlaylists);
            if self.failing.contains(&FailingQuery::CustomSmartPlaylists) {
                return Err(StoreError::InvalidOperation(
                    "custom list enumeration read boom".to_string(),
                ));
            }
            Ok(self.custom_smart_playlists.clone())
        }
    }

    /// [`LibraryQueryStore`] view over one shared [`MockLibraryQueryStore`]
    /// behind a mutex: the seam under test takes ownership of the port, so the
    /// test keeps the other handle for assertions and for post-wire
    /// configuration changes (failure flags, mutated canned data).
    ///
    /// Every suite wiring a query-backed seam needs this pair: the
    /// `SessionViews` app seam and the whole-`RiffApp` shell both own their
    /// query port, so a test cannot reach the mock it handed over. Cloning
    /// this is the other half — the boxed clone answers through the same mock,
    /// and the test keeps the `Arc` it was built from.
    pub struct SharedQueryStore(pub Arc<Mutex<MockLibraryQueryStore>>);

    impl LibraryQueryStore for SharedQueryStore {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            self.0.lock().unwrap().get_track(id)
        }

        fn metadata_version(&self) -> Result<u32, StoreError> {
            Ok(riff_persistence::track::METADATA_VERSION)
        }

        fn tracks_page(
            &self,
            order: riff_persistence::store::TrackListOrder,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.0.lock().unwrap().tracks_page(order, offset, limit)
        }

        fn library_counts(&self) -> Result<riff_backend::app::store::LibraryCounts, StoreError> {
            self.0.lock().unwrap().library_counts()
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            self.0.lock().unwrap().all_track_ids()
        }

        fn search_page(
            &self,
            query: &str,
            order: riff_persistence::store::TrackListOrder,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .search_page(query, order, offset, limit)
        }

        fn custom_smart_playlist_page(
            &self,
            tree: &riff_persistence::store::RuleTree,
            order: riff_persistence::store::TrackListOrder,
            needle: Option<&str>,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .custom_smart_playlist_page(tree, order, needle, offset, limit)
        }

        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            self.0.lock().unwrap().all_artists()
        }

        fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError> {
            self.0.lock().unwrap().artist_albums(artist)
        }

        fn album_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_tracks(album_artist, album_title)
        }

        fn folder_has_audio(&self, folder: &std::path::Path) -> Result<bool, StoreError> {
            self.0.lock().unwrap().folder_has_audio(folder)
        }

        fn folder_has_search_match(
            &self,
            folder: &std::path::Path,
            query: &str,
        ) -> Result<bool, StoreError> {
            self.0
                .lock()
                .unwrap()
                .folder_has_search_match(folder, query)
        }

        fn track_ids_in_folder_tree(
            &self,
            folder: &std::path::Path,
        ) -> Result<Vec<TrackId>, StoreError> {
            self.0.lock().unwrap().track_ids_in_folder_tree(folder)
        }

        fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().tracks_in_folder(folder)
        }

        fn folder_track_count(&self, folder: &std::path::Path) -> Result<usize, StoreError> {
            self.0.lock().unwrap().folder_track_count(folder)
        }

        fn last_full_scan(
            &self,
        ) -> Result<Option<riff_backend::app::store::FullScanSummary>, StoreError> {
            self.0.lock().unwrap().last_full_scan()
        }

        fn subdirs_with_audio(&self, folder: &std::path::Path) -> Result<Vec<PathBuf>, StoreError> {
            self.0.lock().unwrap().subdirs_with_audio(folder)
        }

        fn smart_playlist(
            &self,
            kind: SmartPlaylistKind,
            limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().smart_playlist(kind, limit)
        }

        fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError> {
            self.0.lock().unwrap().smart_list_counts()
        }

        fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError> {
            self.0.lock().unwrap().genre_counts()
        }

        fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError> {
            self.0.lock().unwrap().artists_in_genre(genre)
        }

        fn artist_albums_in_genre(
            &self,
            artist: &str,
            genre: &str,
        ) -> Result<Vec<Album>, StoreError> {
            self.0.lock().unwrap().artist_albums_in_genre(artist, genre)
        }

        fn album_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_tracks_in_genre(album_artist, album_title, genre)
        }

        fn hit_albums_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.0.lock().unwrap().hit_albums_page(query, offset, limit)
        }

        fn hit_artists_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .hit_artists_page(query, offset, limit)
        }

        fn album_hit_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_hit_tracks(album_artist, album_title, query)
        }

        fn album_is_name_hit(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<bool, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_is_name_hit(album_artist, album_title, query)
        }

        fn artist_is_name_hit(&self, artist: &str, query: &str) -> Result<bool, StoreError> {
            self.0.lock().unwrap().artist_is_name_hit(artist, query)
        }

        fn hit_albums_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Album>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .hit_albums_in_genre(genre, query, offset, limit)
        }

        fn hit_artists_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .hit_artists_in_genre(genre, query, offset, limit)
        }

        fn album_hit_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().album_hit_tracks_in_genre(
                album_artist,
                album_title,
                genre,
                query,
            )
        }

        fn hit_genre_counts(&self, query: &str) -> Result<Vec<GenreCount>, StoreError> {
            self.0.lock().unwrap().hit_genre_counts(query)
        }

        fn artists_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .artists_page(direction, offset, limit)
        }

        fn albums_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.0.lock().unwrap().albums_page(direction, offset, limit)
        }

        fn genres_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<GenreCount>, StoreError> {
            self.0.lock().unwrap().genres_page(direction, offset, limit)
        }

        fn artists_in_genre_page(
            &self,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .artists_in_genre_page(genre, direction, offset, limit)
        }

        fn artist_albums_in_genre_page(
            &self,
            artist: &str,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .artist_albums_in_genre_page(artist, genre, direction, offset, limit)
        }

        fn picker_texts(&self, field_id: &str, limit: usize) -> Result<Vec<String>, StoreError> {
            self.0.lock().unwrap().picker_texts(field_id, limit)
        }

        fn picker_numbers(&self, field_id: &str, limit: usize) -> Result<Vec<i64>, StoreError> {
            self.0.lock().unwrap().picker_numbers(field_id, limit)
        }

        fn smart_playlist_tree(
            &self,
            id: &PlaylistId,
        ) -> Result<Option<SmartPlaylistTree>, StoreError> {
            self.0.lock().unwrap().smart_playlist_tree(id)
        }

        fn custom_smart_playlists(&self) -> Result<Vec<CustomSmartPlaylist>, StoreError> {
            self.0.lock().unwrap().custom_smart_playlists()
        }
    }

    /// Scripted [`Scans`] front end for UI tests that drive `RiffApp`: the
    /// test queues the outcomes the app will poll, and the mock records the
    /// paths the app asked for. `request`/`cancel` stay fire-and-forget;
    /// `poll` hands back the queue and empties it.
    ///
    /// `Clone` shares the queue, so a test keeps a handle after boxing another
    /// clone into the app. `is_scanning` is always false: the shell never
    /// gates rendering on it, so modelling an in-flight scan would be
    /// modelling something no test here can observe.
    #[derive(Clone, Default)]
    pub struct MockScans {
        /// Outcomes the next `poll` returns, oldest first.
        pub queued: Arc<Mutex<Vec<riff_library::app::scan_service::ScanOutcome>>>,
        /// Every path `request` was called with, in call order.
        pub requested: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl MockScans {
        /// Queue one outcome for the app's next `poll`.
        pub fn queue(&self, outcome: riff_library::app::scan_service::ScanOutcome) {
            lock_cell(&self.queued).push(outcome);
        }

        /// The paths `request` was called with so far.
        pub fn requested_paths(&self) -> Vec<PathBuf> {
            lock_cell(&self.requested).clone()
        }
    }

    /// Lock a mock's shared cell, tolerating poisoning: one failing test must
    /// not cascade into every later test through a poisoned mock.
    pub(crate) fn lock_cell<T>(cell: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        cell.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Recording [`TagEdits`] fake: captures every submitted request and
    /// yields scripted outcomes from `poll` in order. `Clone` is a second
    /// inspection handle over the same shared state, so the controller can
    /// own one while the test inspects the other.
    #[derive(Clone)]
    pub struct FakeTagEdits {
        submitted: Arc<Mutex<Vec<TagEditRequest>>>,
        outcomes: Arc<Mutex<std::collections::VecDeque<TagEditOutcome>>>,
    }

    impl FakeTagEdits {
        pub fn new() -> Self {
            Self {
                submitted: Arc::new(Mutex::new(Vec::new())),
                outcomes: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            }
        }

        /// A fake that yields `outcomes` from `poll` in order.
        pub fn with_outcomes(outcomes: Vec<TagEditOutcome>) -> Self {
            let fake = Self::new();
            *fake.outcomes.lock().unwrap() = outcomes.into();
            fake
        }

        pub fn requests(&self) -> Vec<TagEditRequest> {
            self.submitted.lock().unwrap().clone()
        }
    }

    impl Default for FakeTagEdits {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TagEdits for FakeTagEdits {
        fn submit(&self, request: TagEditRequest) {
            self.submitted.lock().unwrap().push(request);
        }

        fn poll(&self) -> Option<TagEditOutcome> {
            self.outcomes.lock().unwrap().pop_front()
        }
    }

    /// A [`Passes`](riff_backend::app::pass_service::Passes) that records every
    /// submission, so a menu test can assert exactly which pass command its
    /// item chose. Polls answer idle.
    #[derive(Default)]
    pub struct RecordingPasses {
        pub submitted: Mutex<Vec<riff_backend::app::replaygain_pass::PassCommand>>,
    }

    impl riff_backend::app::pass_service::Passes for crate::mocks::RecordingPasses {
        fn submit(&self, command: riff_backend::app::replaygain_pass::PassCommand) {
            self.submitted.lock().unwrap().push(command);
        }
        fn cancel(&self) {}
        fn is_running(&self) -> bool {
            false
        }
        fn poll(&self) -> Option<riff_backend::app::replaygain_pass::PassReport> {
            None
        }
        fn poll_progress(&self) -> (usize, usize) {
            (0, 0)
        }
    }

    /// Recording [`Covers`] fake: captures request intent, serves nothing.
    /// Records cache-clear requests too, and can be handed a scripted clear
    /// outcome, so the in-progress → settled transition is driven without a thread.
    pub struct RecordingCovers {
        requested: Mutex<Vec<(TrackId, PathBuf, RequestedSize)>>,
        clears: AtomicUsize,
        clear_outcomes: Mutex<VecDeque<ClearCacheOutcome>>,
    }

    impl RecordingCovers {
        pub fn new() -> Self {
            Self {
                requested: Mutex::new(Vec::new()),
                clears: AtomicUsize::new(0),
                clear_outcomes: Mutex::new(VecDeque::new()),
            }
        }

        pub fn requested(&self) -> Vec<(TrackId, PathBuf, RequestedSize)> {
            self.requested.lock().unwrap().clone()
        }

        pub fn clear_calls(&self) -> usize {
            self.clears.load(Ordering::SeqCst)
        }

        /// Queue one settled outcome for the next `poll_cache_clear`.
        pub fn serve_clear(&self, outcome: ClearCacheOutcome) {
            self.clear_outcomes.lock().unwrap().push_back(outcome);
        }
    }

    impl Default for RecordingCovers {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Covers for RecordingCovers {
        fn request(&self, track_id: TrackId, path: PathBuf, size: RequestedSize) {
            self.requested.lock().unwrap().push((track_id, path, size));
        }

        /// A folder request lands in the same recording: its identity is its
        /// directory path, so the entries are told apart by what a caller puts
        /// in the slot, not by which method arrived.
        fn request_folder(&self, folder: &std::path::Path, size: RequestedSize) {
            let folder = folder.to_path_buf();
            let id = TrackId::from_path(&folder);
            self.requested.lock().unwrap().push((id, folder, size));
        }

        fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
            Vec::new()
        }

        fn clear_cache(&self) {
            self.clears.fetch_add(1, Ordering::SeqCst);
        }

        fn poll_cache_clear(&self) -> Option<ClearCacheOutcome> {
            self.clear_outcomes.lock().unwrap().pop_front()
        }
    }

    /// [`Covers`] fake that answers every request with deterministic RGBA
    /// pixels, so a composed golden can show a real cover through the real
    /// arrival path (`CoverCache::settle` → `artwork::store_cover_texture` →
    /// the next frame's `lookup_cover_texture`).
    ///
    /// Determinism contract: the pixels are a pure function of the requested
    /// box and the identity's own bytes, so the same request renders the same
    /// art on every machine and every run. Two flat bands plus a diagonal ramp
    /// rather than a smooth gradient — enough distinct pixels that a dropped
    /// or mis-sized texture cannot pass for a correct one, and cheap enough to
    /// generate for a whole library's worth of rows.
    #[derive(Default)]
    pub struct StubCovers {
        /// One queued answer per request, oldest first.
        pub pending: Mutex<Vec<(TrackId, RequestedSize)>>,
    }

    impl StubCovers {
        /// The pixel answer for one identity at one box. Public so a test can
        /// assert against the same function the fake serves.
        #[must_use]
        pub fn cover_for(identity: &str, size: RequestedSize) -> DecodedCover {
            let (w, h) = (size.width as usize, size.height as usize);
            let seed = identity
                .bytes()
                .fold(0x9e37_79b9_u32, |acc, b| acc.rotate_left(5) ^ u32::from(b));
            let top = [seed & 0xff, (seed >> 8) & 0xff, (seed >> 16) & 0xff];
            let bottom = [255 - top[0] / 2, 255 - top[1] / 2, 255 - top[2] / 2];
            let mut rgba = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                let t = if h > 1 {
                    y as f32 / (h - 1) as f32
                } else {
                    0.0
                };
                let band = [
                    (top[0] as f32 * (1.0 - t) + bottom[0] as f32 * t) as u8,
                    (top[1] as f32 * (1.0 - t) + bottom[1] as f32 * t) as u8,
                    (top[2] as f32 * (1.0 - t) + bottom[2] as f32 * t) as u8,
                ];
                for x in 0..w {
                    // A single bright diagonal keeps the image from being two
                    // flat bands, which a solid-fill bug could reproduce.
                    let on_diagonal = (x + y) % 7 < 2;
                    let px = if on_diagonal {
                        [255 - band[0], 255 - band[1], 255 - band[2]]
                    } else {
                        band
                    };
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            DecodedCover {
                rgba,
                width: size.width,
                height: size.height,
            }
        }
    }
}

// --- Minimal trait impls for the no-op UI test mocks -----------------------

impl riff_library::app::scan_service::Scans for crate::mocks::MockScans {
    fn request(&self, path: std::path::PathBuf) {
        crate::mocks::lock_cell(&self.requested).push(path);
    }

    fn cancel(&self) {}

    fn poll(&self) -> Vec<riff_library::app::scan_service::ScanOutcome> {
        std::mem::take(&mut *crate::mocks::lock_cell(&self.queued))
    }

    fn is_scanning(&self, _path: &std::path::Path) -> bool {
        false
    }
}

impl riff_library::app::cover_service::Covers for crate::mocks::StubCovers {
    fn request(
        &self,
        track_id: riff_backend::domain::TrackId,
        _path: std::path::PathBuf,
        size: riff_library::app::traits::RequestedSize,
    ) {
        crate::mocks::lock_cell(&self.pending).push((track_id, size));
    }

    fn request_folder(
        &self,
        folder: &std::path::Path,
        size: riff_library::app::traits::RequestedSize,
    ) {
        crate::mocks::lock_cell(&self.pending)
            .push((riff_backend::domain::TrackId::from_path(folder), size));
    }

    fn poll(
        &self,
    ) -> Vec<(
        riff_backend::domain::TrackId,
        riff_library::app::traits::RequestedSize,
        Option<riff_backend::app::traits::DecodedCover>,
    )> {
        std::mem::take(&mut *crate::mocks::lock_cell(&self.pending))
            .into_iter()
            .map(|(track_id, size)| {
                let cover = crate::mocks::StubCovers::cover_for(&track_id.0, size);
                (track_id, size, Some(cover))
            })
            .collect()
    }

    fn clear_cache(&self) {}

    fn poll_cache_clear(&self) -> Option<riff_backend::app::cover_service::ClearCacheOutcome> {
        None
    }
}

// Integration test helper functions
pub mod integration_helpers {
    use crate::app::state::{LibrarySession, PlaybackSession};
    use std::sync::{Arc, Mutex};

    /// Create paired test sessions: a `PlaybackSession` and a `LibrarySession`,
    /// each in their own `Arc<Mutex<>>`. Callers unpack with
    /// `let (playback, library) = create_test_sessions();`.
    #[allow(clippy::type_complexity)]
    pub fn create_test_sessions() -> (Arc<Mutex<PlaybackSession>>, Arc<Mutex<LibrarySession>>) {
        (
            Arc::new(Mutex::new(PlaybackSession::default())),
            Arc::new(Mutex::new(LibrarySession::default())),
        )
    }
}
