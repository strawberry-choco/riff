// Golden-image snapshot tests (Issue 05).
//
// Renders real egui frames headlessly through `egui_kittest` (wgpu software
// path, no window required) and compares them pixel-for-pixel against
// committed baselines under `tests/snapshots/`. The set is authored against
// the **dark** palette per ADR 0004, plus the light-palette mirrors and the
// High Contrast token-set variants; render through
// [`tests::snapshot`] / [`tests::snapshot_animating`], never a bare harness.
//
// See docs/engineering/golden-image-testing.md for the authoring,
// re-baselining, and diff-review workflow.

#[cfg(test)]
mod tests {
    use riff_gui::ui::fonts::{self, INTER_FACES};
    use riff_gui::ui::theme::{self, Palette};

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

    /// Deterministic font definitions for golden rendering: the vendored
    /// Inter faces only. Unlike [`riff_gui::ui::fonts::font_definitions`] this
    /// never scans system CJK fonts, whose presence varies per machine —
    /// a golden must rasterize identically everywhere the suite runs.
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

    /// Build a harness whose ui closure stays inert until the palette's style
    /// and the Inter faces are installed on it.
    ///
    /// `HarnessBuilder::build_ui` draws — and then `run_ok`s — frames from
    /// inside its own constructor, i.e. before the caller gets `harness.ctx`
    /// back and can install anything. Gating the closure on `ready` keeps
    /// those construction frames empty, so the first frame that actually
    /// paints already carries the installed style. Without the gate a draw fn
    /// that names an Inter family outright rather than going through the
    /// installed [`egui::TextStyle`]s (`draw_type_scale` does) panics with
    /// `FontFamily::Name("riff-inter-medium") is not bound to any fonts`.
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
        theme::install(&harness.ctx, &palette);
        harness.ctx.set_fonts(inter_only_font_definitions());
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

    /// Like [`snapshot`], for a composition where a single `run()` is wrong or
    /// not enough:
    ///
    /// - **An always-repainting widget.** The playing row's equalizer asks for
    ///   a repaint every 50 ms, which blows `run()`'s step budget
    ///   (`sidebar_playing_dark` / `folder_tree_stage_dark` are the goldens
    ///   that pin it).
    /// - **Focus requested late.** Focus asked for through a widget's own
    ///   `Response` (rather than through `Memory` before the draw) lands in
    ///   the *next* frame — `browser_column_focused_dark` needs frame two for
    ///   its focus ring.
    ///
    /// The harness never advances `input.time`, so a fixed frame count is
    /// still deterministic: the equalizer bars sit at phase 0 in every frame.
    fn snapshot_animating(
        name: &str,
        size: egui::Vec2,
        palette: Palette,
        draw: impl FnMut(&mut egui::Ui, &Palette) + 'static,
    ) {
        let _slot = harness_slot();
        let mut harness = with_golden_style(size, palette, draw);
        harness.run_steps(2);
        harness.snapshot(name);
    }

    /// The first golden component: a primary "Play" button on a surface card.
    /// Every color comes from the [`Palette`] the harness installed, so the
    /// image pins the Issue 01 foundation: window background, surface fill,
    /// brand-500 fill, ink text, and both radius steps, with the button
    /// label in Inter Medium.
    fn draw_play_card(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::theme::{RADIUS_MD, RADIUS_SM};

        // Paint the window background across the ENTIRE canvas. The root UI
        // under the kittest harness is inset from the true screen rect, so a
        // panel fill would leave an unpainted clear-color ring around the
        // golden image. Painting on the root layer itself keeps the card
        // above it (same-layer shapes render in submission order) while the
        // layer painter's clip rect spans the full canvas.
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);

        // Center the card vertically within the layout area: half the
        // leftover space above, the card (2 x 12 px margin + 36 px button =
        // 60 px), the rest below.
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space((ui.available_height() - 60.0) / 2.0);
            egui::Frame::new()
                .fill(palette.surface)
                .corner_radius(RADIUS_MD)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    let play = egui::Button::new(egui::RichText::new("Play").color(palette.ink))
                        .fill(palette.brand_primary)
                        .corner_radius(RADIUS_SM)
                        .min_size(egui::vec2(120.0, 36.0));
                    ui.add(play);
                });
        });
    }

    // --- Golden baselines --------------------------------------------------------

    #[test]
    fn dark_play_card_matches_golden_baseline() {
        snapshot(
            "play_card_dark",
            egui::vec2(240.0, 88.0),
            Palette::dark(),
            draw_play_card,
        );
    }

    // --- Row hover wash (Issue 01) ------------------------------------------------

    /// Hovering a tree row paints the design's amber wash. This is a
    /// behavior test, not a golden: render a row with the dark palette,
    /// hover it through the harness, and look for the wash color among the
    /// output pixels — the color the user actually sees, wherever the row
    /// lands in the harness's inset canvas. Tolerance ±2 per channel absorbs
    /// driver-level dithering on flat fills.
    #[test]
    fn hovered_tree_row_paints_the_amber_wash() {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::sidebar::{TreeRow, tree_row};

        fn count_pixels(image: &image::RgbaImage, color: egui::Color32) -> usize {
            image
                .pixels()
                .filter(|p| {
                    p.0[0].abs_diff(color.r()) <= 2
                        && p.0[1].abs_diff(color.g()) <= 2
                        && p.0[2].abs_diff(color.b()) <= 2
                })
                .count()
        }

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
        theme::install(&harness.ctx, &Palette::dark());

        // Idle: no amber wash anywhere in the frame.
        harness.run();
        let idle = harness.render().unwrap();
        assert_eq!(
            count_pixels(&idle, theme::ROW_HOVER),
            0,
            "no amber wash while the row is not hovered"
        );

        // Hovered: the row fill switches to the wash.
        harness.hover_at(egui::pos2(140.0, 24.0));
        harness.run();
        let hovered = harness.render().unwrap();
        assert!(
            count_pixels(&hovered, theme::ROW_HOVER) > 0,
            "hovered tree row paints the amber wash"
        );
    }

    // --- The placeholder tile renders headlessly --------------------------------

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
        use riff_gui::ui::cover_placeholder::lookup_cover_texture;
        use riff_gui::ui::theme::{SURFACE_BG, TEXTURE_TINT};

        const IDENTITY: &str = "f:\\music\\artless golden.mp3";

        fn count_pixels(image: &image::RgbaImage, color: egui::Color32) -> usize {
            image
                .pixels()
                .filter(|p| {
                    p.0[0].abs_diff(color.r()) <= 2
                        && p.0[1].abs_diff(color.g()) <= 2
                        && p.0[2].abs_diff(color.b()) <= 2
                })
                .count()
        }

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
                let tile = lookup_cover_texture(
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
        theme::install(&harness.ctx, &Palette::dark());
        harness.run();

        let frame = harness.render().unwrap();
        let palette = Palette::dark();
        assert!(
            count_pixels(&frame, palette.surface_2) > 0,
            "the placeholder well's surface fill must appear in the headless \
             render — user-loaded textures must not be dropped"
        );
        assert!(
            count_pixels(&frame, palette.ink_3) > 0,
            "the placeholder's music glyph (ink_3) must appear in the headless \
             render — user-loaded textures must not be dropped"
        );
    }

    // --- Shell chrome baseline (Issue 06) ----------------------------------------

    /// The unified shell chrome at exact token dimensions: 56px titlebar
    /// (wordmark, drag region, window + view controls), 280px sidebar,
    /// 88px playerbar strip, and the central stage. The harness renders at
    /// exactly [`riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE`], so the golden pins both
    /// the panel sizes and the chrome-fitting minimum window.
    #[test]
    fn shell_chrome_dark_matches_golden_baseline() {
        snapshot(
            "shell_chrome_dark",
            riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE,
            Palette::dark(),
            draw_shell_chrome,
        );
    }

    fn draw_shell_chrome(ui: &mut egui::Ui, palette: &Palette) {
        draw_shell_chrome_with(
            ui,
            palette,
            riff_gui::ui::chrome::TitleBarContent {
                scan_status: None,
                theme_dark: palette.dark,
                advanced_mode: false,
                active_nav: Some(riff_gui::ui::chrome::NavDestination::Library),
            },
        );
    }

    fn draw_shell_chrome_with(
        ui: &mut egui::Ui,
        palette: &Palette,
        content: riff_gui::ui::chrome::TitleBarContent<'_>,
    ) {
        use riff_gui::ui::chrome::show_titlebar;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::{self, SURFACE_BG};

        // Full-canvas background (determinism rule): the stage reads as the
        // window background while the chrome panels sit on surface tokens.
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let mut search_query = String::new();

        // Top chrome strip: merged frameless titlebar at TITLEBAR_H with the
        // global search field in shared chrome. The `surface` fill is the
        // design's top-bar panel token, named here exactly as `app.rs` names it
        // on the live panel: without it the strip shows the backdrop below,
        // which is why the design handoff measured the chrome darker than any
        // token in the system.
        egui::Panel::top("titlebar")
            .exact_size(theme::TITLEBAR_H)
            .frame(egui::Frame::NONE.fill(palette.surface))
            .show(ui, |ui| {
                show_titlebar(
                    ui,
                    &mut cache,
                    palette,
                    &content,
                    &mut search_query,
                    &mut Vec::new(),
                );
            });

        // Left chrome column: sidebar at SIDEBAR_W with representative
        // browser content (search + Library/Folders nav).
        let mut search = String::new();
        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(12.0);
                ui.heading("Library");
                ui.add_space(8.0);
                ui.text_edit_singleline(&mut search);
                ui.add_space(8.0);
                let _ = ui.selectable_label(true, "Library");
                let _ = ui.selectable_label(false, "Folders");
            });

        // Bottom chrome strip: playerbar at PLAYERBAR_H (transport restyle
        // lands with issue 08; the shell pins the strip itself).
        egui::Panel::bottom("playerbar")
            .exact_size(theme::PLAYERBAR_H)
            .show(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.weak("player bar");
                });
            });

        // Main stage: the active View's surface over the window background.
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.background))
            .show(ui, |_| {});
    }

    // --- Sidebar baseline (design-handoff issue 07) ------------------------------

    /// The restructured sidebar at its exact 280px token width: the flat sectioned nav (LIBRARY / SMART LISTS
    /// / PLAYLISTS) with right-aligned live counts, playlist rows, and the
    /// Add-folder / last-scan footer. Rendered idle (no hover, nothing
    /// playing) so the snapshot is deterministic; the equalizer animation
    /// itself is covered headlessly in `ui_tests`.
    #[test]
    fn sidebar_dark_matches_golden_baseline() {
        snapshot(
            "sidebar_dark",
            egui::vec2(theme::SIDEBAR_W, 640.0),
            Palette::dark(),
            draw_sidebar,
        );
    }

    fn draw_sidebar(ui: &mut egui::Ui, palette: &Palette) {
        draw_sidebar_with(ui, palette, false);
    }

    fn draw_sidebar_with(ui: &mut egui::Ui, palette: &Palette, playing: bool) {
        use riff_gui::ui::icons::{Icon, IconCache};
        use riff_gui::ui::sidebar::{self, TreeRow};
        use riff_gui::ui::theme::{SIDEBAR_W, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();

        egui::Panel::left("sidebar")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| {
                sidebar::section_header(ui, palette, "Library");

                for (label, icon, count, selected) in [
                    ("All Tracks", Some(Icon::ListMusic), 128, true),
                    ("Artists", Some(Icon::Library), 23, false),
                    ("Albums", Some(Icon::Disc), 41, false),
                    ("Genres", Some(Icon::Music), 9, false),
                    ("Folders", Some(Icon::Folder), 2, false),
                ] {
                    sidebar::tree_row(
                        ui,
                        &mut cache,
                        palette,
                        TreeRow {
                            indent_level: 0,
                            icon,
                            cover: None,
                            label,
                            count: Some(count),
                            meta: None,
                            favorite: None,
                            selected,
                            now_playing: false,
                            playing: false,
                            art_slot: false,
                        },
                    );
                }
                ui.add_space(8.0);

                sidebar::section_header(ui, palette, "Smart Lists");
                for (i, (name, count)) in [
                    ("Recently Added", 50),
                    ("Recently Played", 37),
                    ("Most Played", 50),
                    ("Favorites", 12),
                ]
                .into_iter()
                .enumerate()
                {
                    sidebar::tree_row(
                        ui,
                        &mut cache,
                        palette,
                        TreeRow {
                            indent_level: 0,
                            icon: Some(Icon::Sparkles),
                            cover: None,
                            label: name,
                            count: Some(count),
                            meta: None,
                            favorite: None,
                            selected: i == 1,
                            now_playing: false,
                            playing: false,
                            art_slot: false,
                        },
                    );
                }
                ui.add_space(8.0);

                sidebar::section_header(ui, palette, "Playlists");
                sidebar::playlist_row(
                    ui,
                    &mut cache,
                    palette,
                    "Focus Mix",
                    "Focus Mix (12)",
                    false,
                );
                sidebar::playlist_row(ui, &mut cache, palette, "Workout", "Workout (3)", true);

                // A nested track row pair showing the indent scale in action.
                sidebar::tree_row(
                    ui,
                    &mut cache,
                    palette,
                    TreeRow {
                        indent_level: 1,
                        icon: None,
                        cover: None,
                        label: "01. Moonlight Sonata",
                        count: None,
                        meta: None,
                        favorite: None,
                        selected: false,
                        now_playing: true,
                        playing,
                        art_slot: false,
                    },
                );
                sidebar::tree_row(
                    ui,
                    &mut cache,
                    palette,
                    TreeRow {
                        indent_level: 2,
                        icon: None,
                        cover: None,
                        label: "02. Für Elise",
                        count: None,
                        meta: None,
                        favorite: None,
                        selected: false,
                        now_playing: false,
                        playing: false,
                        art_slot: false,
                    },
                );

                // The Add-folder / last-scan footer.
                sidebar::sidebar_footer(ui, &mut cache, palette, Some("Last scan 5m ago"));
            });
    }

    // --- Player bar baseline (Issue 08) --------------------------------------------

    /// The restyled playerbar at its exact 88px token height: 56×56 cover
    /// with the surface-2→surface-3 Mesh gradient placeholder, circular ghost
    /// transport around the 40px primary-filled play, the 4px seek row with
    /// brand fill and monospace time readouts, and the right cluster — queue
    /// position label, shuffle/repeat toggles (shuffle engaged), mute, and
    /// the styled volume slider with its round thumb.
    #[test]
    fn playerbar_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            draw_playerbar,
        );
    }

    fn draw_playerbar(ui: &mut egui::Ui, palette: &Palette) {
        draw_playerbar_with(ui, palette, playerbar_content());
    }

    /// The transport the `playerbar_*` goldens start from: playing,
    /// shuffle engaged, unmuted, no repeat, queue closed.
    fn playerbar_content() -> riff_gui::ui::playerbar::PlayerBarContent<'static> {
        riff_gui::ui::playerbar::PlayerBarContent {
            cover: None,
            playback: riff_backend::domain::PlaybackState::Playing,
            position: std::time::Duration::from_mins(2),
            total: Some(std::time::Duration::from_secs(245)),
            volume: 0.65,
            muted: false,
            shuffle: true,
            repeat: riff_backend::domain::RepeatMode::None,
            queue_position: "3/12",
            queue_open: false,
            expanded: false,
            advanced: false,
        }
    }

    fn draw_playerbar_with(
        ui: &mut egui::Ui,
        palette: &Palette,
        content: riff_gui::ui::playerbar::PlayerBarContent<'_>,
    ) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        playerbar::show_player_bar(
            ui,
            &mut cache,
            palette,
            &content,
            &mut riff_gui::ui::playerbar::SeekReadouts::default(),
            &mut Vec::new(),
        );
    }

    // --- Queue panel baseline (handoff issue 13) -----------------------------------

    /// The queue panel open over a strip of canvas: the floating sheet
    /// anchored above the player bar's right edge, its "Up Next" header, and
    /// the scrollable queue rows. The player bar itself is pinned by
    /// `playerbar_dark`.
    #[test]
    fn queue_panel_dark_matches_golden_baseline() {
        snapshot(
            "queue_panel_dark",
            egui::vec2(420.0, 360.0),
            Palette::dark(),
            draw_queue_panel,
        );
    }

    fn draw_queue_panel(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::now_playing::UpNextEntry;
        use riff_gui::ui::playerbar;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let entries = [
            UpNextEntry {
                id: riff_backend::domain::TrackId("a.flac".to_string()),
                label: "Roy Ayers - Everybody Loves the Sunshine".to_string(),
            },
            UpNextEntry {
                id: riff_backend::domain::TrackId("b.flac".to_string()),
                label: "Boards of Canada - Roygbiv".to_string(),
            },
            UpNextEntry {
                id: riff_backend::domain::TrackId("c.flac".to_string()),
                label: "Broadcast - Papercuts".to_string(),
            },
            UpNextEntry {
                id: riff_backend::domain::TrackId("d.flac".to_string()),
                label: "Ohio Players - Love Rollercoaster".to_string(),
            },
        ];
        let mut cache = IconCache::new();
        playerbar::show_queue_panel(ui, &mut cache, palette, &entries, &mut Vec::new());
    }

    // --- Library stage baselines (Issue 09) ---------------------------------------

    /// The Library stage canvas at the minimum window: 520×456 = 800−280 wide
    /// × 600−56−88 high, so both goldens pin the hero and the track list at
    /// the smallest real estate they must fit without clipping.
    fn library_stage_size() -> egui::Vec2 {
        egui::vec2(
            riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE.x - theme::SIDEBAR_W,
            riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE.y
                - theme::TITLEBAR_H
                - theme::PLAYERBAR_H,
        )
    }

    /// The empty-library hero on the minimum-window main stage: the 160px
    /// disc circle with its layered amber glow, the semibold title, and the
    /// muted subtitle, centered per the mockup's index.html stage.
    #[test]
    fn library_hero_dark_matches_golden_baseline() {
        snapshot(
            "library_hero_dark",
            library_stage_size(),
            Palette::dark(),
            draw_library_hero,
        );
    }

    fn draw_library_hero(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::library::empty_state_hero;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        empty_state_hero(ui, &mut cache, palette);
    }

    /// The populated-library track list: the styled 40px rows the explorer
    /// lists tracks with — "Artist - Title" labels on the row seam the flat
    /// list renders through, one selected and one now-playing (idle, so
    /// nothing animates between runs).
    #[test]
    fn library_track_list_dark_matches_golden_baseline() {
        snapshot(
            "library_track_list_dark",
            library_stage_size(),
            Palette::dark(),
            draw_library_track_list,
        );
    }

    fn draw_library_track_list(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::sidebar::{self, TreeRow};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();

        // Same row shape `RiffApp::render_track_row` produces for the flat
        // list: indent 0, no leading glyph, "Artist - Title" label, and the
        // right-aligned `Plays · Time` cluster.
        // ... favorite control included: this is the row the flat list
        // renders, hearts and all.
        let rows = [
            ("Daft Punk - One More Time", 54, 337, false, false, true),
            (
                "Radiohead - Weird Fishes",
                31,
                318,
                false,
                true, // now-playing, idle
                false,
            ),
            (
                "Miles Davis - So What",
                128,
                562,
                true, // selected
                false,
                true,
            ),
            ("Portishead - Roads", 76, 303, false, false, false),
            ("Burial - Archangel", 22, 240, false, false, false),
            ("Nils Frahm - Says", 9, 412, false, false, false),
        ];
        for (label, plays, time, selected, now_playing, favorite) in rows {
            sidebar::tree_row(
                ui,
                &mut cache,
                palette,
                TreeRow {
                    indent_level: 0,
                    icon: None,
                    cover: None,
                    label,
                    count: None,
                    meta: Some(sidebar::RowMeta {
                        plays: Some(plays),
                        time: Some(std::time::Duration::from_secs(time)),
                    }),
                    favorite: Some(favorite),
                    selected,
                    now_playing,
                    playing: false,
                    art_slot: false,
                },
            );
        }
    }

    // --- Now Playing baseline (Issue 10) -------------------------------------------

    /// The restyled Now Playing stage at the DEFAULT launch main stage
    /// (1200−280 wide × 800−56−88 high): the 240px cover with its
    /// extra-large radius and layered brand glow, the 3xl semibold title,
    /// the meta line, the in-view seek row, and the Up Next queue rows. The
    /// fixed mockup column only fits whole at the launch size — smaller
    /// windows keep the cover fixed and scroll the Up Next list instead —
    /// so this golden pins the design at its real proportions. Rendered idle
    /// (no hover) so the snapshot is deterministic; the placeholder gradient
    /// stands in for cover art.
    #[test]
    fn now_playing_dark_matches_golden_baseline() {
        snapshot(
            "now_playing_dark",
            egui::vec2(
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .x
                    - theme::SIDEBAR_W,
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .y
                    - theme::TITLEBAR_H
                    - theme::PLAYERBAR_H,
            ),
            Palette::dark(),
            draw_now_playing,
        );
    }

    fn draw_now_playing(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::now_playing::{self, NowPlayingContent, UpNextEntry};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let content = NowPlayingContent {
            cover: None,
            title: Some("Nightcall".into()),
            meta_line: Some("Kavinsky - OutRun".into()),
            details: Some("2013 \u{b7} Synthwave \u{b7} Track 1".into()),
            position: std::time::Duration::from_secs(83),
            total: Some(std::time::Duration::from_mins(4)),
            up_next: vec![
                UpNextEntry {
                    id: riff_backend::domain::TrackId("a.flac".to_owned()),
                    label: "The Midnight - Sunset".to_owned(),
                },
                UpNextEntry {
                    id: riff_backend::domain::TrackId("b.flac".to_owned()),
                    label: "Timecop1983 - On the Run".to_owned(),
                },
            ]
            .into(),
        };
        let mut cache = IconCache::new();
        now_playing::show_now_playing(
            ui,
            &mut cache,
            palette,
            &content,
            &mut riff_gui::ui::playerbar::SeekReadouts::default(),
            &mut Vec::new(),
        );
    }

    // --- Settings modal baseline (Issue 11) ------------------------------------------

    /// The sectioned Settings modal at its launch-stage size: the centered
    /// card with the header's close control, the left nav listing the eight
    /// sections (Library active), and the Library pane's Music Libraries card
    /// with a per-path Readiness dot beside Scan / Watch / trash and the
    /// Add Library + Scan All row, plus the destructive ghost Clear Library
    /// action. Rendered idle so the snapshot is deterministic.
    #[test]
    fn settings_dark_matches_golden_baseline() {
        snapshot(
            "settings_dark",
            egui::vec2(
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .x
                    - theme::SIDEBAR_W,
                840.0,
            ),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    fn draw_settings_modal(
        ui: &mut egui::Ui,
        palette: &Palette,
        section: riff_gui::ui::settings::SettingsSection,
    ) {
        draw_settings_modal_with_libraries(
            ui,
            palette,
            section,
            vec![library_row(
                "C:\\Users\\local\\Music",
                riff_backend::app::state::LibraryStatus::Scanned(1284),
                riff_backend::app::state::WatchState::Enabled,
                1284,
            )],
        );
    }

    fn draw_settings_modal_with_libraries(
        ui: &mut egui::Ui,
        palette: &Palette,
        section: riff_gui::ui::settings::SettingsSection,
        libraries: Vec<riff_gui::ui::settings::LibraryRow>,
    ) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::settings;

        // Full-canvas background (determinism rule).
        //
        // `palette.background`, not the fixed `SURFACE_BG` the component
        // goldens use. The settings page used to paint its own card fill
        // (`palette.surface`) over this backdrop, so a hardcoded dark
        // background was harmless; now that the card's fill is gone (ticket
        // 02) the page has no backdrop of its own and whatever is behind it
        // shows through. Production fills the whole app frame with
        // `palette.background` (`RiffApp::update`) and sets
        // `v.window_fill` from it, so the golden must too — otherwise the
        // light palette renders near-black ink on the harness's dark
        // `SURFACE_BG` and the heading and nav labels disappear.
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);

        let mut cache = IconCache::new();
        let content = settings_content_with(libraries);
        settings::show_settings_modal(ui, &mut cache, palette, &content, section);
    }

    fn settings_content_with(
        libraries: Vec<riff_gui::ui::settings::LibraryRow>,
    ) -> riff_gui::ui::settings::SettingsContent {
        riff_gui::ui::settings::SettingsContent {
            libraries,
            advanced_mode: true,
            high_contrast: false,
            replaygain_enabled: false,
            watch_any: true,
            skip_hidden_files: true,
            scan_formats: riff_backend::app::store::AUDIO_EXTENSIONS
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
            read_embedded_artwork: true,
            close_quits_app: false,
            last_scan: Some(riff_backend::app::store::FullScanSummary {
                // Rendered immediately, so the relative stamp reads "just
                // now" deterministically.
                at: std::time::SystemTime::now(),
                files: 1284,
                errors: 3,
            }),
        }
    }

    /// One library root for the Settings goldens: the path, its scan
    /// status, its watch state, and how many tracks are indexed under it
    /// (the trio [`riff_gui::ui::settings::readiness`] derives from).
    fn library_row(
        path: &str,
        status: riff_backend::app::state::LibraryStatus,
        watch: riff_backend::app::state::WatchState,
        indexed_tracks: usize,
    ) -> riff_gui::ui::settings::LibraryRow {
        riff_gui::ui::settings::LibraryRow {
            path: std::path::PathBuf::from(path),
            status,
            watch,
            indexed_tracks,
        }
    }

    // --- Titlebar search (shared chrome) --------------------------------------
    //
    // The global "Search or jump to…" field lives in the titlebar — shared
    // chrome present on every View (the content top bar was deleted). These
    // replace the former content-top-bar goldens: the same field, relocated
    // to the chrome, centered between the wordmark/scan-status cluster and
    // the nav/caption cluster. Rendered idle (empty query so the hint text
    // shows) so the snapshot is deterministic.

    #[test]
    fn titlebar_search_dark_matches_golden_baseline() {
        snapshot(
            "titlebar_search_dark",
            egui::vec2(800.0, theme::TITLEBAR_H),
            Palette::dark(),
            draw_titlebar_search,
        );
    }

    fn draw_titlebar_search(ui: &mut egui::Ui, palette: &Palette) {
        draw_titlebar_search_with(ui, palette, false);
    }

    fn draw_titlebar_search_with(ui: &mut egui::Ui, palette: &Palette, search_focused: bool) {
        use riff_gui::ui::chrome::{NavDestination, TitleBarContent, show_titlebar};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        if search_focused {
            // The titlebar search field's stable id: focus it through
            // memory, so the ring lands this frame.
            ui.memory_mut(|m| m.request_focus(egui::Id::new("riff_global_search")));
        }

        let mut cache = IconCache::new();
        let mut query = String::new();
        let content = TitleBarContent {
            scan_status: None,
            theme_dark: palette.dark,
            advanced_mode: false,
            active_nav: Some(NavDestination::Library),
        };

        egui::Panel::top("titlebar_search")
            .exact_size(theme::TITLEBAR_H)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                show_titlebar(
                    ui,
                    &mut cache,
                    palette,
                    &content,
                    &mut query,
                    &mut Vec::new(),
                );
            });
    }

    // --- Explorer widget baselines (design-handoff issue 15) ---------------------
    //
    // The explorer's widget seams: browser column, detail column, and the
    // selection panel — the widgets the elastic stage (elastic-column spec)
    // composes side by side at the widths its sizing policy hands them. The
    // stage's own column compositions (sized by `column_widths`) are pinned
    // by the `elastic_*_dark` goldens below. The browser column is
    // permanently list-only (the grid path was retired end-to-end); its
    // rows and the top-bar search are pinned by their own baselines.

    /// The browser column (the explorer's entity-list widget) at the elastic
    /// stage's preferred column width ([`riff_gui::ui::theme::COLUMN_WIDTH`],
    /// 280): the A–Z sort control and list rows with placeholder thumbnail
    /// slots, secondary detail lines, one selected and one now-playing row.
    /// Rendered idle so the snapshot is deterministic.
    #[test]
    fn browser_column_dark_matches_golden_baseline() {
        snapshot(
            "browser_column_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::dark(),
            draw_browser_column,
        );
    }

    fn draw_browser_column(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();

        let rows = [
            ("Boards of Canada", "12 albums", false, false),
            ("Daft Punk", "9 albums", true, false), // selected
            ("Miles Davis", "31 albums", false, false),
            ("Portishead", "5 albums", false, true), // now-playing, idle
        ];
        let items: Vec<BrowserItem> = rows
            .into_iter()
            .enumerate()
            .map(|(i, (label, detail, selected, now_playing))| BrowserItem {
                key: format!("item-{i}"),
                label: label.to_string(),
                detail: Some(detail.to_string()),
                thumbnail: None,
                selected,
                now_playing,
            })
            .collect();
        let mut provider = |i: usize| items.get(i).cloned();
        let column = BrowserColumn {
            sort_desc: false,
            show_sort: true,
            total: items.len(),
            item: &mut provider,
            virtualize: false,
            empty_title: "",
            empty_hint: "",
        };
        browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    /// The detail column (the explorer's Tracks widget — the elastic stage's
    /// last column, which absorbs the remaining width) at album level: the
    /// breadcrumb trail, the album header with its subtitle, and the
    /// `# / Title / Plays / Time` track table — one favorite, one selected,
    /// one now-playing (idle, so nothing animates). 480 is a representative
    /// absorbing width for the last column; the harness is wide enough that
    /// the Time column clears the right edge — a clipped golden would bake
    /// truncation into the baseline.
    #[test]
    fn detail_column_dark_matches_golden_baseline() {
        snapshot(
            "detail_column_dark",
            egui::vec2(480.0, 420.0),
            Palette::dark(),
            draw_detail_column,
        );
    }

    fn draw_detail_column(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::detail::{self, Crumb, DetailColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();

        let crumbs = [
            Crumb {
                label: "Artists".to_string(),
            },
            Crumb {
                label: "Boards of Canada".to_string(),
            },
            Crumb {
                label: "Geogaddi".to_string(),
            },
        ];
        let header = detail::AlbumHeader {
            title: "Geogaddi".to_string(),
            subtitle: Some("Boards of Canada \u{b7} 2002".to_string()),
        };
        let tracks = geogaddi_tracks();
        let column = DetailColumn {
            breadcrumb: &crumbs,
            header: Some(&header),
            tracks: &tracks,
            ..DetailColumn::empty("", "")
        };
        detail::show_detail_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    /// The dummy Geogaddi track table shared by the detail-column golden and
    /// the elastic drilled compositions: one favorite, one selected, one
    /// now-playing (idle, so nothing animates).
    fn geogaddi_tracks() -> Vec<riff_gui::ui::detail::TrackRow> {
        use riff_gui::ui::detail::TrackRow;
        vec![
            TrackRow {
                key: "t1".to_string(),
                title: "Ready Let's Go".to_string(),
                plays: 12,
                duration: Some(std::time::Duration::from_secs(201)),
                favorite: false,
                selected: false,
                now_playing: false,
            },
            TrackRow {
                key: "t2".to_string(),
                title: "Music Is Math".to_string(),
                plays: 34,
                duration: Some(std::time::Duration::from_secs(322)),
                favorite: true,
                selected: false,
                now_playing: false,
            },
            TrackRow {
                key: "t3".to_string(),
                title: "Beware the Friendly Stranger".to_string(),
                plays: 5,
                duration: Some(std::time::Duration::from_secs(27)),
                favorite: false,
                selected: true,
                now_playing: false,
            },
            TrackRow {
                key: "t4".to_string(),
                title: "Gyroscope".to_string(),
                plays: 21,
                duration: Some(std::time::Duration::from_secs(207)),
                favorite: false,
                selected: false,
                now_playing: true,
            },
        ]
    }

    /// The tag-section rows the golden compositions render: an album
    /// readout's aggregation — shared values, a `(different)` Title and Track
    /// Number, a `(none)` Genre — and a single-track readout's per-track
    /// values with a `(none)` for the track's missing genre (Issue 01).
    /// Artist/Genre readouts pass an empty slice. The `(different)`/`(none)`
    /// text colors come from the palette's warning / muted tokens (never
    /// hardcoded colors).
    fn golden_tag_rows(single: bool) -> Vec<riff_gui::ui::selection::TagRow> {
        use riff_gui::ui::selection::{TagField, TagRow, TagRowState};

        let row = |field: TagField,
                   state: TagRowState,
                   text: &str,
                   originals: Vec<Option<String>>| TagRow {
            field,
            state,
            text: text.to_string(),
            originals,
        };
        if single {
            vec![
                row(
                    TagField::Title,
                    TagRowState::Value,
                    "Beware the Friendly Stranger",
                    vec![Some("Beware the Friendly Stranger".into())],
                ),
                row(
                    TagField::Artist,
                    TagRowState::Value,
                    "Boards of Canada",
                    vec![Some("Boards of Canada".into())],
                ),
                row(
                    TagField::Album,
                    TagRowState::Value,
                    "Geogaddi",
                    vec![Some("Geogaddi".into())],
                ),
                row(
                    TagField::AlbumArtist,
                    TagRowState::Value,
                    "Boards of Canada",
                    vec![Some("Boards of Canada".into())],
                ),
                row(TagField::Genre, TagRowState::None, "(none)", vec![None]),
                row(
                    TagField::Year,
                    TagRowState::Value,
                    "2002",
                    vec![Some("2002".into())],
                ),
                row(
                    TagField::TrackNumber,
                    TagRowState::Value,
                    "1",
                    vec![Some("1".into())],
                ),
            ]
        } else {
            vec![
                row(
                    TagField::Title,
                    TagRowState::Different,
                    "(different)",
                    vec![Some("Nothing Is Real".into()); 8],
                ),
                row(
                    TagField::Artist,
                    TagRowState::Value,
                    "Boards of Canada",
                    vec![Some("Boards of Canada".into()); 8],
                ),
                row(
                    TagField::Album,
                    TagRowState::Value,
                    "Tomorrow's Harvest",
                    vec![Some("Tomorrow's Harvest".into()); 8],
                ),
                row(
                    TagField::AlbumArtist,
                    TagRowState::Value,
                    "Boards of Canada",
                    vec![Some("Boards of Canada".into()); 8],
                ),
                row(TagField::Genre, TagRowState::None, "(none)", vec![None; 8]),
                row(
                    TagField::Year,
                    TagRowState::Value,
                    "2013",
                    vec![Some("2013".into()); 8],
                ),
                row(
                    TagField::TrackNumber,
                    TagRowState::Different,
                    "(different)",
                    vec![Some("1".into()); 8],
                ),
            ]
        }
    }

    /// The selection panel (the inspector's content widget — the collapsible
    /// panel the elastic stage shows as its rightmost column while a
    /// selection exists) at the inspector's width
    /// ([`riff_gui::ui::theme::INSPECTOR_WIDTH`], 300): the SELECTION
    /// header with its kind chip, the 268×200 placeholder art block, the
    /// album title over its artist · year line, the Play album action, and
    /// the details grid. Rendered without art so no texture load is
    /// involved — the generated-colour placeholder path is pinned by the
    /// headless behavior test above.
    #[test]
    fn selection_panel_dark_matches_golden_baseline() {
        snapshot(
            "selection_panel_dark",
            egui::vec2(riff_gui::ui::theme::INSPECTOR_WIDTH, 1000.0),
            Palette::dark(),
            draw_selection_panel,
        );
    }

    fn draw_selection_panel(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::selection::{self, SelectionDetail, SelectionPanel};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();

        let details = [
            SelectionDetail {
                label: "Artist".to_string(),
                value: "Boards of Canada".to_string(),
            },
            SelectionDetail {
                label: "Released".to_string(),
                value: "2013".to_string(),
            },
            SelectionDetail {
                label: "Tracks".to_string(),
                value: "8 \u{b7} 27:16".to_string(),
            },
        ];
        let tags = golden_tag_rows(false);
        let panel = SelectionPanel {
            art: None,
            title: Some("Tomorrow's Harvest"),
            subtitle: Some("Boards of Canada \u{b7} 2013"),
            details: &details,
            tags: &tags,
            editor: None,
            single: false,
            // The golden pins the panel's single Play album action — the
            // rendering the original fixed pane used; the inspector's Play /
            // Add to Queue row (`queue: true`) is pinned by the
            // `elastic_all_tracks_inspector_dark` composition below.
            queue: false,
        };
        selection::show_selection_panel(ui, &mut cache, palette, panel, &mut Vec::new());
    }

    // --- Elastic column stage compositions (elastic-column spec) ------------------
    //
    // The elastic stage's column compositions: the widgets it composes side
    // by side, sized by `column_widths` — the Artists and Genres drill-downs
    // and the flat Tracks listing beside the collapsible inspector. Each
    // draw function mirrors `render_elastic_stage` exactly: zero item
    // spacing, a hairline separator between columns, every column in a
    // `width`-constrained child ui, and the sizing policy fed the width the
    // separators leave.

    /// The stage's horizontal composition. The width allocation, separator
    /// accounting, zero-gap layout, and stable child identities all come from
    /// the production geometry seam ([`riff_gui::ui::stage::show_elastic_stage`])
    /// — the SAME helper `render_elastic_stage` drives — so a golden can never
    /// pin a stage layout production no longer produces. The only thing added
    /// here is the fixed-height rect the kittest root needs: its root ui sizes
    /// itself to content, which would collapse the columns to stub heights.
    /// `column(index)` draws one list column; the inspector reports as index
    /// `columns`.
    fn horizontal_stage(
        ui: &mut egui::Ui,
        columns: usize,
        inspector: bool,
        height: f32,
        mut column: impl FnMut(&mut egui::Ui, usize),
    ) {
        use riff_gui::ui::stage::{StageSlot, show_elastic_stage};
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            show_elastic_stage(ui, columns, inspector, |ui, slot| match slot {
                StageSlot::Column(i) => column(ui, i),
                StageSlot::Inspector => column(ui, columns),
            });
        });
    }

    /// The elastic stage's Artists drill-down composition: the three list
    /// columns the stage sizes side by side — the Artists root (A–Z sort,
    /// artist rows) · the artist's Albums column (list rows, no sort) · the
    /// Tracks column (breadcrumb
    /// `Artists / Boards of Canada / Geogaddi`, album header, track table).
    #[test]
    fn elastic_artists_drilled_dark_matches_golden_baseline() {
        snapshot(
            "elastic_artists_drilled_dark",
            egui::vec2(1180.0, 420.0),
            Palette::dark(),
            draw_elastic_artists_drilled,
        );
    }

    fn draw_elastic_artists_drilled(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::detail::{self, Crumb, DetailColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        horizontal_stage(ui, 3, false, 420.0, |ui, column| {
            // Column 1 — the Artists root: sort control, genre chips, rows
            // keyed by artist name (the same dummy rows the browser-column
            // golden uses).
            if column == 0 {
                let rows = [
                    ("Boards of Canada", "12 albums", false, false),
                    ("Daft Punk", "9 albums", true, false), // selected
                    ("Miles Davis", "31 albums", false, false),
                    ("Portishead", "5 albums", false, true), // now-playing, idle
                ];
                let items: Vec<BrowserItem> = rows
                    .into_iter()
                    .map(|(label, detail, selected, now_playing)| BrowserItem {
                        key: label.to_string(),
                        label: label.to_string(),
                        detail: Some(detail.to_string()),
                        thumbnail: None,
                        selected,
                        now_playing,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: true,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            // Column 2 — the artist's Albums: list rows, no sort, no chips,
            // `(album artist, title)` composite keys, one selected.
            if column == 1 {
                let albums = [
                    (
                        "Music Has the Right to Children",
                        "Boards of Canada \u{b7} 1998",
                        false,
                    ),
                    ("Geogaddi", "Boards of Canada \u{b7} 2002", true), // selected
                    (
                        "The Campfire Headphase",
                        "Boards of Canada \u{b7} 2005",
                        false,
                    ),
                    ("Tomorrow's Harvest", "Boards of Canada \u{b7} 2013", false),
                ];
                let items: Vec<BrowserItem> = albums
                    .into_iter()
                    .map(|(label, detail, selected)| BrowserItem {
                        key: format!("Boards of Canada\u{1f}{label}"),
                        label: label.to_string(),
                        detail: Some(detail.to_string()),
                        thumbnail: None,
                        selected,
                        now_playing: false,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: false,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            // Column 3 — the Tracks column: breadcrumb, header, track table.
            let crumbs = [
                Crumb {
                    label: "Artists".to_string(),
                },
                Crumb {
                    label: "Boards of Canada".to_string(),
                },
                Crumb {
                    label: "Geogaddi".to_string(),
                },
            ];
            let header = detail::AlbumHeader {
                title: "Geogaddi".to_string(),
                subtitle: Some("Boards of Canada \u{b7} 2002".to_string()),
            };
            let tracks = geogaddi_tracks();
            let column = DetailColumn {
                breadcrumb: &crumbs,
                header: Some(&header),
                tracks: &tracks,
                ..DetailColumn::empty("", "")
            };
            detail::show_detail_column(ui, &mut cache, palette, column, &mut Vec::new());
        });
    }

    /// The elastic stage's Genres drill-down composition: the four list
    /// columns the stage sizes side by side — the Genres root · the
    /// artists-in-genre column · the albums-in-genre column · the Tracks
    /// column (breadcrumb `Genres / Electronic / Autechre / Tri Repetae`).
    #[test]
    fn elastic_genres_drilled_dark_matches_golden_baseline() {
        snapshot(
            "elastic_genres_drilled_dark",
            egui::vec2(1460.0, 420.0),
            Palette::dark(),
            draw_elastic_genres_drilled,
        );
    }

    fn draw_elastic_genres_drilled(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::detail::{self, Crumb, DetailColumn, TrackRow};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        horizontal_stage(ui, 4, false, 420.0, |ui, column| {
            // Column 1 — the Genres root: A–Z sort and genre rows keyed by
            // genre name.
            if column == 0 {
                let rows = [
                    ("Electronic", "42 tracks", true), // selected
                    ("Jazz", "31 tracks", false),
                    ("Rock", "19 tracks", false),
                ];
                let items: Vec<BrowserItem> = rows
                    .into_iter()
                    .map(|(label, detail, selected)| BrowserItem {
                        key: label.to_string(),
                        label: label.to_string(),
                        detail: Some(detail.to_string()),
                        thumbnail: None,
                        selected,
                        now_playing: false,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: true,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            // Column 2 — the artists in the genre: list rows, no sort, no
            // chips, keyed by artist name, one selected.
            if column == 1 {
                let rows = [
                    ("Autechre", "2 albums", true), // selected
                    ("Aphex Twin", "5 albums", false),
                    ("Boards of Canada", "3 albums", false),
                ];
                let items: Vec<BrowserItem> = rows
                    .into_iter()
                    .map(|(label, detail, selected)| BrowserItem {
                        key: label.to_string(),
                        label: label.to_string(),
                        detail: Some(detail.to_string()),
                        thumbnail: None,
                        selected,
                        now_playing: false,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: false,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            // Column 3 — the albums in the genre: `(album artist, title)`
            // composite keys, one selected.
            if column == 2 {
                let albums = [
                    ("Tri Repetae", "Autechre \u{b7} 1995", true), // selected
                    ("Amber", "Autechre \u{b7} 1994", false),
                    ("Incunabula", "Autechre \u{b7} 1993", false),
                ];
                let items: Vec<BrowserItem> = albums
                    .into_iter()
                    .map(|(label, detail, selected)| BrowserItem {
                        key: format!("Autechre\u{1f}{label}"),
                        label: label.to_string(),
                        detail: Some(detail.to_string()),
                        thumbnail: None,
                        selected,
                        now_playing: false,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: false,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            // Column 4 — the Tracks column: breadcrumb, header, track table.
            let crumbs = [
                Crumb {
                    label: "Genres".to_string(),
                },
                Crumb {
                    label: "Electronic".to_string(),
                },
                Crumb {
                    label: "Autechre".to_string(),
                },
                Crumb {
                    label: "Tri Repetae".to_string(),
                },
            ];
            let header = detail::AlbumHeader {
                title: "Tri Repetae".to_string(),
                subtitle: Some("Autechre \u{b7} 1995".to_string()),
            };
            let tracks = [
                TrackRow {
                    key: "g1".to_string(),
                    title: "Drane".to_string(),
                    plays: 14,
                    duration: Some(std::time::Duration::from_secs(377)),
                    favorite: false,
                    selected: false,
                    now_playing: false,
                },
                TrackRow {
                    key: "g2".to_string(),
                    title: "Eutow".to_string(),
                    plays: 27,
                    duration: Some(std::time::Duration::from_secs(255)),
                    favorite: true,
                    selected: false,
                    now_playing: false,
                },
                TrackRow {
                    key: "g3".to_string(),
                    title: "C/Pach".to_string(),
                    plays: 8,
                    duration: Some(std::time::Duration::from_secs(237)),
                    favorite: false,
                    selected: true,
                    now_playing: false,
                },
                TrackRow {
                    key: "g4".to_string(),
                    title: "Gnit".to_string(),
                    plays: 19,
                    duration: Some(std::time::Duration::from_secs(353)),
                    favorite: false,
                    selected: false,
                    now_playing: true,
                },
            ];
            let column = DetailColumn {
                breadcrumb: &crumbs,
                header: Some(&header),
                tracks: &tracks,
                ..DetailColumn::empty("", "")
            };
            detail::show_detail_column(ui, &mut cache, palette, column, &mut Vec::new());
        });
    }

    /// The elastic stage's All Tracks composition: the flat Tracks listing
    /// beside the collapsible inspector (the selection panel's Play / Add to
    /// Queue variant) — one selected and one now-playing (idle) track row,
    /// the inspector at [`riff_gui::ui::theme::INSPECTOR_WIDTH`].
    #[test]
    fn elastic_all_tracks_inspector_dark_matches_golden_baseline() {
        snapshot(
            "elastic_all_tracks_inspector_dark",
            egui::vec2(900.0, 1000.0),
            Palette::dark(),
            draw_elastic_all_tracks_inspector,
        );
    }

    fn draw_elastic_all_tracks_inspector(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::selection::{self, SelectionDetail, SelectionPanel};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        // The inspector's tag section adds seven rows, so the stage grows to a
        // thousand pixels of height.
        let stage_height = 1000.0;

        horizontal_stage(ui, 1, true, stage_height, |ui, column| {
            // Column 1 — the flat Tracks listing: track rows keyed by
            // `TrackId`, one selected and one now-playing (idle).
            if column == 0 {
                let rows = [
                    ("Daft Punk - One More Time", false, false),
                    ("Radiohead - Weird Fishes", false, true), // now-playing, idle
                    ("Miles Davis - So What", true, false),    // selected
                    ("Portishead - Roads", false, false),
                    ("Burial - Archangel", false, false),
                    ("Nils Frahm - Says", false, false),
                    ("Aphex Twin - Xtal", false, false),
                    ("Brian Eno - An Ending", false, false),
                    ("Massive Attack - Teardrop", false, false),
                    ("Tycho - Awake", false, false),
                    ("Jon Hopkins - Open Eye Signal", false, false),
                    ("Four Tet - She Moves She", false, false),
                ];
                let keys = [
                    "a.flac", "b.flac", "c.flac", "d.flac", "e.flac", "f.flac", "g.flac", "h.flac",
                    "i.flac", "j.flac", "k.flac", "l.flac",
                ];
                let items: Vec<BrowserItem> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(i, (label, selected, now_playing))| BrowserItem {
                        key: keys[i].to_string(),
                        label: label.to_string(),
                        detail: None,
                        thumbnail: None,
                        selected,
                        now_playing,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: false,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            // Column 2 — the inspector: the selection panel's Play / Add
            // to Queue variant inside the same 16px inset the app's
            // `render_inspector` gives it, no art (no texture load).
            let details = [
                SelectionDetail {
                    label: "Artist".to_string(),
                    value: "Boards of Canada".to_string(),
                },
                SelectionDetail {
                    label: "Released".to_string(),
                    value: "2013".to_string(),
                },
                SelectionDetail {
                    label: "Tracks".to_string(),
                    value: "8 \u{b7} 27:16".to_string(),
                },
            ];
            let tags = golden_tag_rows(false);
            egui::Frame::new()
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Tomorrow's Harvest"),
                        subtitle: Some("Boards of Canada \u{b7} 2013"),
                        details: &details,
                        tags: &tags,
                        editor: None,
                        single: false,
                        queue: true,
                    };
                    selection::show_selection_panel(
                        ui,
                        &mut cache,
                        palette,
                        panel,
                        &mut Vec::new(),
                    );
                });
        });
    }
    // =========================================================================
    // Coverage-audit goldens
    //
    // Same determinism rules as every golden above: fixed size, fixed DPI,
    // full-canvas background, Inter only, and no hover-dependent rendering.
    // Where a golden needs *state* (focus, a muted transport, a missing
    // library root) it is set directly rather than simulated through the
    // pointer, so the render stays reproducible.
    // =========================================================================

    use riff_gui::ui::settings::SettingsSection;

    /// Which inspector readout the `elastic_inspector_*` goldens pin: one per
    /// [`riff_gui::ui::app::InspectorKind`] the audit lists as unexercised.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InspectorVariant {
        Artist,
        Genre,
        Track,
    }

    /// One row of the folder-tree golden: the copy, its indent level, whether
    /// it owns the 32px art slot (folders only), and its selection state.
    struct FolderNode {
        label: &'static str,
        level: usize,
        art_slot: bool,
        selected: bool,
        now_playing: bool,
        playing: bool,
        icon: Option<riff_gui::ui::icons::Icon>,
    }

    // --- P0-1: the light palette ----------------------------------------------
    //
    // `Palette::light()` is *derived by rule* (channel-wise mirror of the dark
    // ramp), so a mirrored surface that reads wrong is invisible to every unit
    // test. These eight mirror the structural baselines — enough to catch a
    // bad mirror without doubling the suite's whole review surface.

    #[test]
    fn shell_chrome_light_matches_golden_baseline() {
        snapshot(
            "shell_chrome_light",
            riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE,
            Palette::light(),
            draw_shell_chrome,
        );
    }

    #[test]
    fn sidebar_light_matches_golden_baseline() {
        snapshot(
            "sidebar_light",
            egui::vec2(theme::SIDEBAR_W, 640.0),
            Palette::light(),
            draw_sidebar,
        );
    }

    #[test]
    fn titlebar_search_light_matches_golden_baseline() {
        snapshot(
            "titlebar_search_light",
            egui::vec2(800.0, theme::TITLEBAR_H),
            Palette::light(),
            draw_titlebar_search,
        );
    }

    #[test]
    fn browser_column_light_matches_golden_baseline() {
        snapshot(
            "browser_column_light",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::light(),
            draw_browser_column,
        );
    }

    #[test]
    fn detail_column_light_matches_golden_baseline() {
        snapshot(
            "detail_column_light",
            egui::vec2(480.0, 420.0),
            Palette::light(),
            draw_detail_column,
        );
    }

    #[test]
    fn playerbar_light_matches_golden_baseline() {
        snapshot(
            "playerbar_light",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::light(),
            draw_playerbar,
        );
    }

    #[test]
    fn settings_light_matches_golden_baseline() {
        snapshot(
            "settings_light",
            settings_stage_size(),
            Palette::light(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    #[test]
    fn elastic_artists_drilled_light_matches_golden_baseline() {
        snapshot(
            "elastic_artists_drilled_light",
            egui::vec2(1180.0, 420.0),
            Palette::light(),
            draw_elastic_artists_drilled,
        );
    }

    // --- P0-2 / P0-3: High Contrast and the focus ring ------------------------

    /// High Contrast is a token-set variant over the dark base: ink pinned to
    /// the extreme, doubled line alphas, and the yellow `HC_FOCUS_RING`.
    #[test]
    fn browser_column_hc_matches_golden_baseline() {
        snapshot(
            "browser_column_hc",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::dark().high_contrast(),
            draw_browser_column,
        );
    }

    /// The most accessibility-critical pixel in the app: the focused titlebar
    /// search well's ring, in the High Contrast variant that thickens it.
    #[test]
    fn titlebar_search_focused_hc_matches_golden_baseline() {
        snapshot(
            "titlebar_search_focused_hc",
            egui::vec2(800.0, theme::TITLEBAR_H),
            Palette::dark().high_contrast(),
            |ui, palette| draw_titlebar_search_with(ui, palette, true),
        );
    }

    /// The titlebar search well with keyboard focus — the ring
    /// `sidebar::search_ring_stroke` paints. Focus is requested directly
    /// through memory (fully deterministic; the determinism rule bans
    /// *hover*-dependent rendering, not focus).
    #[test]
    fn titlebar_search_focused_dark_matches_golden_baseline() {
        snapshot(
            "titlebar_search_focused_dark",
            egui::vec2(800.0, theme::TITLEBAR_H),
            Palette::dark(),
            |ui, palette| draw_titlebar_search_with(ui, palette, true),
        );
    }

    /// A focused entity row: `theme::focus_ring_stroke` on the browser row.
    /// Two frames: the focus is requested from the row's own `Response`, i.e.
    /// after that row has already run its ring check this frame.
    #[test]
    fn browser_column_focused_dark_matches_golden_baseline() {
        snapshot_animating(
            "browser_column_focused_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::dark(),
            draw_browser_column_focused,
        );
    }

    /// The browser's own entity row with keyboard focus. Rendered through
    /// `browser::detail_entity_row` (the same `browser_row` widget the column
    /// uses) so the ring the column paints is what gets pinned.
    fn draw_browser_column_focused(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let rows = [
            ("Boards of Canada", "12 albums", false),
            ("Daft Punk", "9 albums", true), // focused
            ("Miles Davis", "31 albums", false),
            ("Portishead", "5 albums", false),
        ];
        for (label, detail, focus) in rows {
            let item = BrowserItem {
                key: label.to_string(),
                label: label.to_string(),
                detail: Some(detail.to_string()),
                thumbnail: None,
                selected: false,
                now_playing: false,
            };
            let response = browser::detail_entity_row(ui, &mut cache, palette, &item);
            if focus {
                response.request_focus();
            }
        }
    }

    // --- P0-5: the Folders tree -----------------------------------------------

    /// `BrowseMode::Folders` renders through `TreeRow::art_slot` on the
    /// `INDENT_STEP` indent scale — a code path no golden exercised. The rows
    /// are the app's own folder-node shape: open/closed folder glyphs,
    /// three indent levels, one selected node, and one playing track.
    #[test]
    fn folder_tree_stage_dark_matches_golden_baseline() {
        snapshot_animating(
            "folder_tree_stage_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::dark(),
            draw_folder_tree,
        );
    }

    fn draw_folder_tree(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::{Icon, IconCache};
        use riff_gui::ui::sidebar::{self, TreeRow};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let nodes = [
            FolderNode {
                label: "Music",
                level: 0,
                art_slot: true,
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::FolderOpen),
            },
            FolderNode {
                label: "Boards of Canada",
                level: 1,
                art_slot: true,
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::FolderOpen),
            },
            FolderNode {
                label: "Geogaddi",
                level: 2,
                art_slot: false,
                selected: true,
                now_playing: false,
                playing: false,
                icon: Some(Icon::Folder),
            },
            FolderNode {
                label: "Autechre",
                level: 1,
                art_slot: false,
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::Folder),
            },
            FolderNode {
                label: "01. Ready Let's Go",
                level: 2,
                art_slot: false,
                selected: false,
                now_playing: true,
                playing: true,
                icon: None,
            },
            FolderNode {
                label: "02. Music Is Math",
                level: 2,
                art_slot: false,
                selected: false,
                now_playing: true,
                playing: false,
                icon: None,
            },
            FolderNode {
                label: "Tomorrow's Harvest",
                level: 1,
                art_slot: false,
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::Folder),
            },
        ];
        for node in nodes {
            sidebar::tree_row(
                ui,
                &mut cache,
                palette,
                TreeRow {
                    indent_level: node.level,
                    icon: node.icon,
                    cover: None,
                    label: node.label,
                    count: None,
                    meta: None,
                    favorite: None,
                    selected: node.selected,
                    now_playing: node.now_playing,
                    playing: node.playing,
                    art_slot: node.art_slot,
                },
            );
        }
    }

    // --- P1-6: the Settings panes ---------------------------------------------

    /// The Settings stage's size: the launch-stage width minus the sidebar,
    /// tall enough for the tallest pane (Advanced carries the most rows).
    fn settings_stage_size() -> egui::Vec2 {
        egui::vec2(
            riff_gui::ui::chrome::viewport_builder()
                .inner_size
                .expect("launch size is configured")
                .x
                - theme::SIDEBAR_W,
            840.0,
        )
    }

    #[test]
    fn settings_playback_dark_matches_golden_baseline() {
        snapshot(
            "settings_playback_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Playback),
        );
    }

    #[test]
    fn settings_appearance_dark_matches_golden_baseline() {
        snapshot(
            "settings_appearance_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Appearance),
        );
    }

    /// The Advanced pane also carries the `#[cfg(not(linux))]` platform
    /// info-lines branch, so it renders differently off Linux.
    #[test]
    fn settings_advanced_dark_matches_golden_baseline() {
        snapshot(
            "settings_advanced_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Advanced),
        );
    }

    /// `About` is a placeholder pane — this is the golden that notices if it
    /// regresses to an empty card.
    #[test]
    fn settings_about_dark_matches_golden_baseline() {
        snapshot(
            "settings_about_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::About),
        );
    }

    // --- P1-7: the hand-built modals and prompts -------------------------------

    /// The destructive Clear Library confirmation: the warning line in the
    /// palette's warning token over Confirm / Cancel.
    #[test]
    fn clear_library_confirm_dark_matches_golden_baseline() {
        snapshot(
            "clear_library_confirm_dark",
            egui::vec2(560.0, 96.0),
            Palette::dark(),
            draw_clear_library_confirm,
        );
    }

    fn draw_clear_library_confirm(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = riff_gui::ui::icons::IconCache::new();
        let _ = riff_gui::ui::prompts::clear_library_confirm(ui, &mut cache, palette);
    }

    #[test]
    fn playlist_create_prompt_dark_matches_golden_baseline() {
        snapshot(
            "playlist_create_prompt_dark",
            egui::vec2(420.0, 48.0),
            Palette::dark(),
            |ui, _palette| {
                let _ = riff_gui::ui::prompts::playlist_create_prompt(
                    ui,
                    &mut "Late Night Drive".to_string(),
                );
            },
        );
    }

    #[test]
    fn playlist_rename_prompt_dark_matches_golden_baseline() {
        snapshot(
            "playlist_rename_prompt_dark",
            egui::vec2(420.0, 48.0),
            Palette::dark(),
            |ui, _palette| {
                let _ =
                    riff_gui::ui::prompts::playlist_rename_prompt(ui, &mut "Focus Mix".to_string());
            },
        );
    }

    // --- P1-8: empty states ----------------------------------------------------

    /// `browser::empty_state` with real copy — every existing golden invokes it
    /// with empty title/hint, so the composition itself was never pinned.
    #[test]
    fn empty_column_dark_matches_golden_baseline() {
        snapshot(
            "empty_column_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 300.0),
            Palette::dark(),
            draw_empty_column,
        );
    }

    fn draw_empty_column(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let mut provider = |_: usize| None;
        let column = BrowserColumn {
            sort_desc: false,
            show_sort: true,
            total: 0,
            item: &mut provider,
            virtualize: false,
            empty_title: "No artists yet",
            empty_hint: "Add a music folder to fill your library.",
        };
        browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    /// Now Playing with no track: the calm empty state, close affordance and
    /// all.
    #[test]
    fn now_playing_empty_dark_matches_golden_baseline() {
        snapshot(
            "now_playing_empty_dark",
            egui::vec2(
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .x
                    - theme::SIDEBAR_W,
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .y
                    - theme::TITLEBAR_H
                    - theme::PLAYERBAR_H,
            ),
            Palette::dark(),
            draw_now_playing_empty,
        );
    }

    fn draw_now_playing_empty(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::now_playing::{self, NowPlayingContent};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        now_playing::show_now_playing(
            ui,
            &mut cache,
            palette,
            &NowPlayingContent::default(),
            &mut riff_gui::ui::playerbar::SeekReadouts::default(),
            &mut Vec::new(),
        );
    }

    /// The queue panel with no entries (the empty state that only had a
    /// behavior test).
    #[test]
    fn queue_panel_empty_dark_matches_golden_baseline() {
        snapshot(
            "queue_panel_empty_dark",
            egui::vec2(420.0, 160.0),
            Palette::dark(),
            draw_queue_panel_empty,
        );
    }

    fn draw_queue_panel_empty(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        playerbar::show_queue_panel(ui, &mut cache, palette, &[], &mut Vec::new());
    }

    // --- P1-9: unexercised elastic plans and inspector kinds -------------------

    /// The Albums drill-down: the `Root + Tracks` plan — the only section
    /// shape whose two-column form was not pinned.
    #[test]
    fn elastic_albums_drilled_dark_matches_golden_baseline() {
        snapshot(
            "elastic_albums_drilled_dark",
            egui::vec2(900.0, 420.0),
            Palette::dark(),
            draw_elastic_albums_drilled,
        );
    }

    fn draw_elastic_albums_drilled(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::detail::{self, Crumb, DetailColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        horizontal_stage(ui, 2, false, 420.0, |ui, column| {
            if column == 0 {
                let albums = [
                    ("Geogaddi", "Boards of Canada · 2002", true),
                    (
                        "Music Has the Right to Children",
                        "Boards of Canada · 1998",
                        false,
                    ),
                    ("Tomorrow's Harvest", "Boards of Canada · 2013", false),
                    ("Tri Repetae", "Autechre · 1995", false),
                ];
                let items: Vec<BrowserItem> = albums
                    .into_iter()
                    .map(|(label, detail, selected)| BrowserItem {
                        key: label.to_string(),
                        label: label.to_string(),
                        detail: Some(detail.to_string()),
                        thumbnail: None,
                        selected,
                        now_playing: false,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: true,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            let crumbs = [
                Crumb {
                    label: "Albums".to_string(),
                },
                Crumb {
                    label: "Geogaddi".to_string(),
                },
            ];
            let header = detail::AlbumHeader {
                title: "Geogaddi".to_string(),
                subtitle: Some("Boards of Canada · 2002".to_string()),
            };
            let tracks = geogaddi_tracks();
            let column = DetailColumn {
                breadcrumb: &crumbs,
                header: Some(&header),
                tracks: &tracks,
                ..DetailColumn::empty("", "")
            };
            detail::show_detail_column(ui, &mut cache, palette, column, &mut Vec::new());
        });
    }

    /// The three-deep Genres plan: `GenreArtists + GenreArtistAlbums + Tracks`.
    #[test]
    fn elastic_genres_deep_dark_matches_golden_baseline() {
        snapshot(
            "elastic_genres_deep_dark",
            egui::vec2(1180.0, 420.0),
            Palette::dark(),
            draw_elastic_genres_deep,
        );
    }

    fn draw_elastic_genres_deep(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::detail::{self, Crumb, DetailColumn, TrackRow};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        horizontal_stage(ui, 3, false, 420.0, |ui, column| {
            if column < 2 {
                let rows = if column == 0 {
                    vec![
                        ("Autechre", "2 albums", true),
                        ("Aphex Twin", "5 albums", false),
                        ("Boards of Canada", "3 albums", false),
                    ]
                } else {
                    vec![
                        ("Tri Repetae", "Autechre · 1995", true),
                        ("Amber", "Autechre · 1994", false),
                        ("Incunabula", "Autechre · 1993", false),
                    ]
                };
                let items: Vec<BrowserItem> = rows
                    .into_iter()
                    .map(|(label, detail, selected)| BrowserItem {
                        key: label.to_string(),
                        label: label.to_string(),
                        detail: Some(detail.to_string()),
                        thumbnail: None,
                        selected,
                        now_playing: false,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: false,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            let crumbs = [
                Crumb {
                    label: "Genres".to_string(),
                },
                Crumb {
                    label: "Electronic".to_string(),
                },
                Crumb {
                    label: "Autechre".to_string(),
                },
                Crumb {
                    label: "Tri Repetae".to_string(),
                },
            ];
            let header = detail::AlbumHeader {
                title: "Tri Repetae".to_string(),
                subtitle: Some("Autechre · 1995".to_string()),
            };
            let tracks = [
                TrackRow {
                    key: "g1".to_string(),
                    title: "Drane".to_string(),
                    plays: 14,
                    duration: Some(std::time::Duration::from_secs(377)),
                    favorite: false,
                    selected: false,
                    now_playing: false,
                },
                TrackRow {
                    key: "g2".to_string(),
                    title: "Eutow".to_string(),
                    plays: 27,
                    duration: Some(std::time::Duration::from_secs(255)),
                    favorite: true,
                    selected: false,
                    now_playing: false,
                },
                TrackRow {
                    key: "g3".to_string(),
                    title: "C/Pach".to_string(),
                    plays: 8,
                    duration: Some(std::time::Duration::from_secs(237)),
                    favorite: false,
                    selected: true,
                    now_playing: false,
                },
            ];
            let column = DetailColumn {
                breadcrumb: &crumbs,
                header: Some(&header),
                tracks: &tracks,
                ..DetailColumn::empty("", "")
            };
            detail::show_detail_column(ui, &mut cache, palette, column, &mut Vec::new());
        });
    }

    /// The inspector readouts: one per kind the audit lists as unexercised,
    /// each beside a list column at the inspector's width.
    #[test]
    fn elastic_inspector_artist_dark_matches_golden_baseline() {
        snapshot(
            "elastic_inspector_artist_dark",
            egui::vec2(900.0, 640.0),
            Palette::dark(),
            |ui, palette| draw_elastic_inspector(ui, palette, InspectorVariant::Artist),
        );
    }

    #[test]
    fn elastic_inspector_genre_dark_matches_golden_baseline() {
        snapshot(
            "elastic_inspector_genre_dark",
            egui::vec2(900.0, 640.0),
            Palette::dark(),
            |ui, palette| draw_elastic_inspector(ui, palette, InspectorVariant::Genre),
        );
    }

    /// The compact single-track readout.
    #[test]
    fn elastic_inspector_track_dark_matches_golden_baseline() {
        snapshot(
            "elastic_inspector_track_dark",
            egui::vec2(900.0, 1000.0),
            Palette::dark(),
            |ui, palette| draw_elastic_inspector(ui, palette, InspectorVariant::Track),
        );
    }

    fn draw_elastic_inspector(ui: &mut egui::Ui, palette: &Palette, kind: InspectorVariant) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::selection::{self, SelectionDetail, SelectionPanel};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        // The Track readout carries the seven-row tag section, so the stage
        // gets a thousand pixels of height; the short Artist/Genre entity
        // readouts stay at the panel's original six-forty (their goldens
        // must not move).
        let stage_height = if kind == InspectorVariant::Track {
            1000.0
        } else {
            640.0
        };

        horizontal_stage(ui, 1, true, stage_height, |ui, column| {
            if column == 0 {
                let rows = [
                    ("Daft Punk - One More Time", false, false),
                    ("Radiohead - Weird Fishes", false, true),
                    ("Miles Davis - So What", true, false),
                    ("Portishead - Roads", false, false),
                    ("Burial - Archangel", false, false),
                    ("Nils Frahm - Says", false, false),
                    ("Aphex Twin - Xtal", false, false),
                    ("Brian Eno - An Ending", false, false),
                ];
                let items: Vec<BrowserItem> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(i, (label, selected, now_playing))| BrowserItem {
                        key: format!("track-{i}"),
                        label: label.to_string(),
                        detail: None,
                        thumbnail: None,
                        selected,
                        now_playing,
                    })
                    .collect();
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    sort_desc: false,
                    show_sort: false,
                    total: items.len(),
                    item: &mut provider,
                    virtualize: false,
                    empty_title: "",
                    empty_hint: "",
                };
                browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
                return;
            }
            let (title, subtitle, details, single) = match kind {
                InspectorVariant::Artist => (
                    "Boards of Canada",
                    "Artist",
                    vec![
                        SelectionDetail {
                            label: "Albums".to_string(),
                            value: "4".to_string(),
                        },
                        SelectionDetail {
                            label: "Tracks".to_string(),
                            value: "47".to_string(),
                        },
                        SelectionDetail {
                            label: "Genres".to_string(),
                            value: "Electronic, IDM".to_string(),
                        },
                    ],
                    false,
                ),
                InspectorVariant::Genre => (
                    "Electronic",
                    "Genre",
                    vec![
                        SelectionDetail {
                            label: "Tracks".to_string(),
                            value: "42".to_string(),
                        },
                        SelectionDetail {
                            label: "Artists".to_string(),
                            value: "9".to_string(),
                        },
                    ],
                    false,
                ),
                InspectorVariant::Track => (
                    "Beware the Friendly Stranger",
                    "Boards of Canada · Geogaddi",
                    vec![
                        SelectionDetail {
                            label: "Duration".to_string(),
                            value: "0:27".to_string(),
                        },
                        SelectionDetail {
                            label: "Plays".to_string(),
                            value: "5".to_string(),
                        },
                    ],
                    true,
                ),
            };
            // Artist and Genre entity readouts carry no tag section.
            let tags = match kind {
                InspectorVariant::Artist | InspectorVariant::Genre => Vec::new(),
                _ => golden_tag_rows(single),
            };
            egui::Frame::new()
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some(title),
                        subtitle: Some(subtitle),
                        details: &details,
                        tags: &tags,
                        editor: None,
                        single,
                        queue: true,
                    };
                    selection::show_selection_panel(
                        ui,
                        &mut cache,
                        palette,
                        panel,
                        &mut Vec::new(),
                    );
                });
        });
    }

    // --- P1-10: the narrow-window shrink branch --------------------------------

    /// The stage at `theme::geometry::window::MIN_WINDOW_SIZE`: `column_widths` shrinks every
    /// column proportionally once the width falls below the floors. Only the
    /// arithmetic was tested before.
    #[test]
    fn elastic_min_window_dark_matches_golden_baseline() {
        snapshot(
            "elastic_min_window_dark",
            library_stage_size(),
            Palette::dark(),
            draw_elastic_artists_drilled,
        );
    }

    // --- P2: state variants of already-pinned regions --------------------------

    /// The transport in every state the pinned `playerbar_dark` does not
    /// cover.
    #[test]
    fn playerbar_paused_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_paused_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            |ui, palette| {
                let mut content = playerbar_content();
                content.playback = riff_backend::domain::PlaybackState::Paused;
                draw_playerbar_with(ui, palette, content);
            },
        );
    }

    /// Stopped / no track: no duration, no position to show.
    #[test]
    fn playerbar_stopped_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_stopped_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            |ui, palette| {
                let mut content = playerbar_content();
                content.playback = riff_backend::domain::PlaybackState::Stopped;
                content.position = std::time::Duration::ZERO;
                content.total = None;
                draw_playerbar_with(ui, palette, content);
            },
        );
    }

    #[test]
    fn playerbar_muted_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_muted_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            |ui, palette| {
                let mut content = playerbar_content();
                content.muted = true;
                draw_playerbar_with(ui, palette, content);
            },
        );
    }

    #[test]
    fn playerbar_repeat_one_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_repeat_one_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            |ui, palette| {
                let mut content = playerbar_content();
                content.repeat = riff_backend::domain::RepeatMode::One;
                draw_playerbar_with(ui, palette, content);
            },
        );
    }

    #[test]
    fn playerbar_repeat_all_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_repeat_all_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            |ui, palette| {
                let mut content = playerbar_content();
                content.repeat = riff_backend::domain::RepeatMode::All;
                draw_playerbar_with(ui, palette, content);
            },
        );
    }

    /// Advanced mode adds the stop button to the transport cluster.
    #[test]
    fn playerbar_advanced_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_advanced_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            |ui, palette| {
                let mut content = playerbar_content();
                content.advanced = true;
                draw_playerbar_with(ui, palette, content);
            },
        );
    }

    /// The expanded queue position readout.
    #[test]
    fn playerbar_queue_open_dark_matches_golden_baseline() {
        snapshot(
            "playerbar_queue_open_dark",
            egui::vec2(800.0, theme::PLAYERBAR_H),
            Palette::dark(),
            |ui, palette| {
                let mut content = playerbar_content();
                content.queue_open = true;
                content.expanded = true;
                draw_playerbar_with(ui, palette, content);
            },
        );
    }

    /// A real 240px cover with the layered brand glow: the `painter.image`
    /// path through a texture the golden itself loads.
    #[test]
    fn now_playing_cover_dark_matches_golden_baseline() {
        snapshot(
            "now_playing_cover_dark",
            egui::vec2(
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .x
                    - theme::SIDEBAR_W,
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .y
                    - theme::TITLEBAR_H
                    - theme::PLAYERBAR_H,
            ),
            Palette::dark(),
            draw_now_playing_cover,
        );
    }

    fn draw_now_playing_cover(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::now_playing::{self, NowPlayingContent, UpNextEntry};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        // A deterministic 64×64 diagonal gradient: real cover art's pixels
        // without any dependency on a file on disk.
        let side = 64_usize;
        let mut pixels = Vec::with_capacity(side * side * 4);
        for y in 0..side {
            for x in 0..side {
                let t = ((x + y) as f32 / ((side + side) as f32)).clamp(0.0, 1.0);
                pixels.push((32.0 + t * 200.0) as u8);
                pixels.push((24.0 + t * 90.0) as u8);
                pixels.push((48.0 + t * 40.0) as u8);
                pixels.push(u8::MAX);
            }
        }
        let cover = ui.ctx().load_texture(
            "golden_now_playing_cover",
            egui::ColorImage::from_rgba_unmultiplied([side, side], &pixels),
            egui::TextureOptions::LINEAR,
        );

        let content = NowPlayingContent {
            cover: Some(cover),
            title: Some("Nightcall".into()),
            meta_line: Some("Kavinsky - OutRun".into()),
            details: Some("2013 · Synthwave · Track 1".into()),
            position: std::time::Duration::from_secs(83),
            total: Some(std::time::Duration::from_mins(4)),
            up_next: vec![
                UpNextEntry {
                    id: riff_backend::domain::TrackId("a.flac".to_owned()),
                    label: "The Midnight - Sunset".to_owned(),
                },
                UpNextEntry {
                    id: riff_backend::domain::TrackId("b.flac".to_owned()),
                    label: "Timecop1983 - On the Run".to_owned(),
                },
            ]
            .into(),
        };
        let mut cache = IconCache::new();
        now_playing::show_now_playing(
            ui,
            &mut cache,
            palette,
            &content,
            &mut riff_gui::ui::playerbar::SeekReadouts::default(),
            &mut Vec::new(),
        );
    }

    /// The sidebar's `playing: true` equalizer row. The bars read `input.time`,
    /// which the harness leaves at 0, so the animation is deterministic here.
    #[test]
    fn sidebar_playing_dark_matches_golden_baseline() {
        snapshot_animating(
            "sidebar_playing_dark",
            egui::vec2(theme::SIDEBAR_W, 640.0),
            Palette::dark(),
            |ui, palette| draw_sidebar_with(ui, palette, true),
        );
    }

    /// The shell chrome with a scan status line beside the wordmark.
    #[test]
    fn shell_chrome_scanning_dark_matches_golden_baseline() {
        snapshot(
            "shell_chrome_scanning_dark",
            riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE,
            Palette::dark(),
            |ui, palette| {
                draw_shell_chrome_with(
                    ui,
                    palette,
                    riff_gui::ui::chrome::TitleBarContent {
                        scan_status: Some("Scanning 812 tracks…"),
                        theme_dark: true,
                        advanced_mode: false,
                        active_nav: Some(riff_gui::ui::chrome::NavDestination::Library),
                    },
                );
            },
        );
    }

    /// `active_nav: None` — Now Playing replaces the view, so no destination
    /// is highlighted.
    #[test]
    fn shell_chrome_now_playing_dark_matches_golden_baseline() {
        snapshot(
            "shell_chrome_now_playing_dark",
            riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE,
            Palette::dark(),
            |ui, palette| {
                draw_shell_chrome_with(
                    ui,
                    palette,
                    riff_gui::ui::chrome::TitleBarContent {
                        scan_status: None,
                        theme_dark: true,
                        advanced_mode: false,
                        active_nav: None,
                    },
                );
            },
        );
    }

    #[test]
    fn shell_chrome_settings_dark_matches_golden_baseline() {
        snapshot(
            "shell_chrome_settings_dark",
            riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE,
            Palette::dark(),
            |ui, palette| {
                draw_shell_chrome_with(
                    ui,
                    palette,
                    riff_gui::ui::chrome::TitleBarContent {
                        scan_status: None,
                        theme_dark: true,
                        advanced_mode: false,
                        active_nav: Some(riff_gui::ui::chrome::NavDestination::Settings),
                    },
                );
            },
        );
    }

    /// A library root that has gone missing: the strikethrough path and the
    /// error dot.
    #[test]
    fn settings_missing_dark_matches_golden_baseline() {
        snapshot(
            "settings_missing_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| {
                draw_settings_modal_with_libraries(
                    ui,
                    palette,
                    SettingsSection::Library,
                    vec![library_row(
                        "D:\\Music",
                        riff_backend::app::state::LibraryStatus::Unavailable,
                        riff_backend::app::state::WatchState::Disabled,
                        0,
                    )],
                );
            },
        );
    }

    /// Ticket 04, the **two-column branch**: a stage wide enough for the
    /// Library pane's lower four sections to settle into two balanced columns
    /// under a full-width Libraries card. Named for the branch it pins.
    ///
    /// NOTE the width: the *launch* stage (`settings_stage_size()`, 920px)
    /// does **not** split. The pane's content column there measures 607px
    /// after the 176px nav, the hairline, `NAV_GAP` and `PANE_PAD`, and the
    /// `ScrollArea` reserves a further ~16px — 13px short of
    /// `MIN_TWO_COL_W` (620). Two columns need a stage of about 936px, i.e. a
    /// window of roughly 1216px. This golden therefore uses a stage that
    /// genuinely reaches the branch it names.
    /// A stage between the minimum and the two-column breakpoint, where the
    /// pane's sections are still one stack but the rows have enough room for
    /// the three-band reflow to breathe.
    ///
    /// Added alongside the minimum-stage golden because the two pin different
    /// regimes: at 520 the pane content is ~205px and the reflow is barely
    /// viable, while here it is comfortable. A regression that only shows at one
    /// of the two — over-eager path truncation at the tight end, say — is
    /// distinguishable rather than hidden by whichever one is pinned.
    #[test]
    fn settings_library_rows_stacked_mid_dark_matches_golden_baseline() {
        snapshot(
            "settings_library_rows_stacked_mid_dark",
            egui::vec2(700.0, 840.0),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    /// A stage wide enough for the Library pane to actually take its
    /// two-column branch (ticket 04). See the note on that golden for why the
    /// launch stage does not qualify.
    fn settings_two_column_stage_size() -> egui::Vec2 {
        egui::vec2(1280.0, 840.0)
    }

    #[test]
    fn settings_library_two_column_dark_matches_golden_baseline() {
        snapshot(
            "settings_library_two_column_dark",
            settings_two_column_stage_size(),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    /// The same stage in the **light** palette. Ticket 05 exists because the
    /// redesign would otherwise ship on dark evidence alone, and the light
    /// palette is derived by rule (a channel-wise mirror), so a mirrored surface
    /// that reads wrong is invisible to every unit test.
    ///
    /// Same stage size as the dark sibling on purpose: the palette is then the
    /// *only* variable between the three two-column goldens, which makes the
    /// ticket's "scrutinise the mirrored surfaces" a controlled comparison
    /// rather than three unrelated pictures. The hairline between the columns
    /// and the gap around it are exactly what a mirror is most likely to get
    /// wrong, so the comparison is only meaningful if the geometry is pinned.
    #[test]
    fn settings_library_two_column_light_matches_golden_baseline() {
        snapshot(
            "settings_library_two_column_light",
            settings_two_column_stage_size(),
            Palette::light(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    /// The same stage in **High Contrast**. The only high-contrast settings
    /// coverage in the suite; the two existing HC goldens are the browser column
    /// and the focused titlebar search. HC raises ink to pure white/grey and
    /// re-points the focus ring, so it is the palette most able to make the
    /// hairline and the error-coloured scan line fall apart.
    #[test]
    fn settings_library_two_column_hc_matches_golden_baseline() {
        snapshot(
            "settings_library_two_column_hc",
            settings_two_column_stage_size(),
            Palette::dark().high_contrast(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    /// Ticket 04, the **stacked fallback**: the minimum stage, where the pane is
    /// far too narrow for two columns and falls back to one stack. The minimum
    /// stage is what the ticket reasons about — a ~284px content column cannot
    /// carry two.
    #[test]
    fn settings_library_stacked_min_dark_matches_golden_baseline() {
        snapshot(
            "settings_library_stacked_min_dark",
            library_stage_size(),
            Palette::dark(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    // --- Ticket 05: the palette × stage matrix, completed -----------------------
    //
    // Ticket 05 asks for the settings page "in all three palettes at both window
    // sizes: dark, light, and high contrast, at the normal stage and at the
    // minimum stage". The matrix now reads:
    //
    // ```text
    //                    920 normal   1280 two-column   520 min   700 mid
    //   dark              settings_dark  ..._two_column_  ..._min_  ..._mid_
    //   light             settings_light ..._two_column_     below
    //   high contrast        below     ..._two_column_     below
    // ```
    //
    // These three close the gaps, and each is a *controlled* comparison: the
    // stage size and the fixture come from the same helpers as its dark sibling
    // (`settings_stage_size` / `library_stage_size` and `draw_settings_modal`,
    // which already routes through `draw_settings_modal_with_libraries` and
    // `settings_content_with`), so the palette is the only variable. That is
    // what makes a mirrored surface readable: `Palette::light()` is derived by
    // rule rather than hand-picked, so a surface that inverts wrongly is
    // invisible to any test that is not looking at it.
    //
    // Filenames carry palette and stage because the `settings_library_*` prefix
    // already means the two-column family, and these are single-column states.

    /// The minimum stage in the **light** palette — the narrowest, mirrored
    /// surface in the whole matrix, and the one the ticket singles out as
    /// riskiest. The three-band stacked row and the wrapped chip row both have
    /// to stay legible when every surface relationship is inverted.
    #[test]
    fn settings_stacked_min_light_matches_golden_baseline() {
        snapshot(
            "settings_stacked_min_light",
            library_stage_size(),
            Palette::light(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    /// The **normal** stage in High Contrast — the missing "at the normal stage"
    /// half of the matrix for HC, and a new state for this page: the only HC
    /// settings coverage until now was the 1280 two-column render. HC lifts ink
    /// to pure white, pushes secondary text to grey(200), brightens the
    /// hairlines and borders, and re-points the focus ring, so it is the palette
    /// most able to make a hairline or a muted description fall apart.
    #[test]
    fn settings_normal_hc_matches_golden_baseline() {
        snapshot(
            "settings_normal_hc",
            settings_stage_size(),
            Palette::dark().high_contrast(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    /// The minimum stage in High Contrast. The narrowest column and the
    /// highest-contrast ink in the same frame, which is where a border that
    /// reads as too heavy and text that reads as too dim would both show.
    #[test]
    fn settings_stacked_min_hc_matches_golden_baseline() {
        snapshot(
            "settings_stacked_min_hc",
            library_stage_size(),
            Palette::dark().high_contrast(),
            |ui, palette| draw_settings_modal(ui, palette, SettingsSection::Library),
        );
    }

    #[test]
    fn settings_scanning_dark_matches_golden_baseline() {
        snapshot(
            "settings_scanning_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| {
                draw_settings_modal_with_libraries(
                    ui,
                    palette,
                    SettingsSection::Library,
                    vec![library_row(
                        "C:\\Users\\local\\Music",
                        riff_backend::app::state::LibraryStatus::Scanning { files_found: 812 },
                        riff_backend::app::state::WatchState::Enabled,
                        640,
                    )],
                );
            },
        );
    }

    #[test]
    fn settings_not_indexed_dark_matches_golden_baseline() {
        snapshot(
            "settings_not_indexed_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| {
                draw_settings_modal_with_libraries(
                    ui,
                    palette,
                    SettingsSection::Library,
                    vec![library_row(
                        "C:\\Users\\local\\Music",
                        riff_backend::app::state::LibraryStatus::Scanned(0),
                        riff_backend::app::state::WatchState::Enabled,
                        0,
                    )],
                );
            },
        );
    }

    /// `WatchState::Warning` — the watch box the row disables, with its
    /// reason on hover.
    #[test]
    fn settings_watch_warning_dark_matches_golden_baseline() {
        snapshot(
            "settings_watch_warning_dark",
            settings_stage_size(),
            Palette::dark(),
            |ui, palette| {
                draw_settings_modal_with_libraries(
                    ui,
                    palette,
                    SettingsSection::Library,
                    vec![library_row(
                        "C:\\Users\\local\\Music",
                        riff_backend::app::state::LibraryStatus::Scanned(1284),
                        riff_backend::app::state::WatchState::Warning(
                            "Too many directories to watch".to_string(),
                        ),
                        1284,
                    )],
                );
            },
        );
    }

    /// The inspector with real cover art: the 268×200 block through the
    /// texture path instead of the placeholder well.
    #[test]
    fn selection_panel_art_dark_matches_golden_baseline() {
        snapshot(
            "selection_panel_art_dark",
            egui::vec2(riff_gui::ui::theme::INSPECTOR_WIDTH, 1000.0),
            Palette::dark(),
            |ui, palette| draw_selection_panel_with(ui, palette, true, false),
        );
    }

    /// The single-track readout: the primary action reads **Play**.
    #[test]
    fn selection_panel_single_dark_matches_golden_baseline() {
        snapshot(
            "selection_panel_single_dark",
            egui::vec2(riff_gui::ui::theme::INSPECTOR_WIDTH, 1000.0),
            Palette::dark(),
            |ui, palette| draw_selection_panel_with(ui, palette, false, true),
        );
    }

    /// The detail column's no-selection state.
    #[test]
    fn detail_column_no_selection_dark_matches_golden_baseline() {
        snapshot(
            "detail_column_no_selection_dark",
            egui::vec2(480.0, 420.0),
            Palette::dark(),
            draw_detail_column_no_selection,
        );
    }

    /// A listing without an album header (the breadcrumb-only shape).
    #[test]
    fn detail_column_no_header_dark_matches_golden_baseline() {
        snapshot(
            "detail_column_no_header_dark",
            egui::vec2(480.0, 420.0),
            Palette::dark(),
            draw_detail_column_no_header,
        );
    }

    fn draw_selection_panel_with(
        ui: &mut egui::Ui,
        palette: &Palette,
        with_art: bool,
        single: bool,
    ) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::selection::{self, SelectionDetail, SelectionPanel};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let art = with_art.then(|| {
            let side = 32_usize;
            let mut pixels = Vec::with_capacity(side * side * 4);
            for y in 0..side {
                for x in 0..side {
                    let t = ((x + y) as f32 / (2.0 * side as f32)).clamp(0.0, 1.0);
                    pixels.push((40.0 + t * 120.0) as u8);
                    pixels.push((80.0 + t * 60.0) as u8);
                    pixels.push((120.0 - t * 40.0) as u8);
                    pixels.push(u8::MAX);
                }
            }
            ui.ctx().load_texture(
                "golden_selection_art",
                egui::ColorImage::from_rgba_unmultiplied([side, side], &pixels),
                egui::TextureOptions::LINEAR,
            )
        });

        let details = [
            SelectionDetail {
                label: "Artist".to_string(),
                value: "Boards of Canada".to_string(),
            },
            SelectionDetail {
                label: "Released".to_string(),
                value: "2013".to_string(),
            },
            SelectionDetail {
                label: "Tracks".to_string(),
                value: "8 · 27:16".to_string(),
            },
        ];
        let title = if single {
            "Beware the Friendly Stranger"
        } else {
            "Tomorrow's Harvest"
        };
        let subtitle = if single {
            "Boards of Canada · Geogaddi"
        } else {
            "Boards of Canada · 2013"
        };
        let tags = golden_tag_rows(single);
        let panel = SelectionPanel {
            art: art.as_ref(),
            title: Some(title),
            subtitle: Some(subtitle),
            details: &details,
            tags: &tags,
            editor: None,
            single,
            queue: false,
        };
        selection::show_selection_panel(ui, &mut cache, palette, panel, &mut Vec::new());
    }

    fn draw_detail_column_no_selection(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::detail::{self, DetailColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        // The app's own no-selection copy, as `app/browser_pane.rs` passes it.
        let mut cache = IconCache::new();
        let column = DetailColumn::empty("Nothing here yet", "This selection has nothing to show.");
        detail::show_detail_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    fn draw_detail_column_no_header(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::detail::{self, Crumb, DetailColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let crumbs = [Crumb {
            label: "Recently Played".to_string(),
        }];
        let tracks = geogaddi_tracks();
        let column = DetailColumn {
            breadcrumb: &crumbs,
            header: None,
            tracks: &tracks,
            ..DetailColumn::empty("", "")
        };
        detail::show_detail_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    // --- P3: cheap regression nets ---------------------------------------------

    /// Every `Icon` variant in one frame: `test_icon_inventory_is_vendored_and_complete`
    /// only checks the SVG files exist, so a blank or broken glyph passed.
    #[test]
    fn icons_atlas_dark_matches_golden_baseline() {
        snapshot(
            "icons_atlas_dark",
            egui::vec2(420.0, 260.0),
            Palette::dark(),
            draw_icons_atlas,
        );
    }

    fn draw_icons_atlas(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::{Icon, IconCache};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            for icon in Icon::ALL {
                let tex = cache.texture(ui.ctx(), *icon, 20.0, palette.ink);
                let sized = egui::load::SizedTexture::new(tex, egui::vec2(20.0, 20.0));
                ui.add(egui::Image::from_texture(sized));
            }
        });
    }

    /// An Inter weight/size specimen: catches font-registration regressions
    /// (Medium / SemiBold / Bold) that unit tests cannot, since they never
    /// rasterize.
    #[test]
    fn type_scale_dark_matches_golden_baseline() {
        snapshot(
            "type_scale_dark",
            egui::vec2(520.0, 300.0),
            Palette::dark(),
            draw_type_scale,
        );
    }

    fn draw_type_scale(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        for style in [
            egui::TextStyle::Heading,
            egui::TextStyle::Body,
            egui::TextStyle::Monospace,
            egui::TextStyle::Button,
            egui::TextStyle::Small,
        ] {
            ui.label(
                egui::RichText::new(format!("{style:?} — Inter · riff 0123"))
                    .text_style(style)
                    .color(palette.ink),
            );
        }
        ui.separator();
        for (label, family) in [
            ("Regular", egui::FontFamily::Proportional),
            ("Medium", fonts::family_medium()),
            ("SemiBold", fonts::family_semibold()),
            ("Bold", fonts::family_bold()),
        ] {
            ui.label(
                egui::RichText::new(format!("{label} · 24px"))
                    .font(egui::FontId::new(24.0, family))
                    .color(palette.ink),
            );
        }
    }

    /// The toggle switch matrix: on/off × enabled, in one frame.
    #[test]
    fn toggle_switch_matrix_dark_matches_golden_baseline() {
        snapshot(
            "toggle_switch_matrix_dark",
            egui::vec2(320.0, 160.0),
            Palette::dark(),
            draw_toggle_switch_matrix,
        );
    }

    fn draw_toggle_switch_matrix(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::theme::SURFACE_BG;
        use riff_gui::ui::toggle_switch;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        for (label, checked, enabled) in [
            ("On / enabled", true, true),
            ("Off / enabled", false, true),
            ("On / disabled", true, false),
            ("Off / disabled", false, false),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
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
    }

    // --- Query filters the section's columns (issue 04) ---------------------
    //
    // Goldens for the entity-level search contract: a query leaves the open
    // section's columns in place and each column lists its hits — the Albums
    // root under a query (sort control hidden), the query-aware empty copy,
    // and the Folders tree staying pruned rather than yanked into the flat
    // list.

    /// The Albums root under a query: only hit albums in canonical hit
    /// order with no A–Z sort control (the hit ordering is fixed) — the
    /// stage keeps its section columns, the root lists the hits.
    #[test]
    fn elastic_albums_under_query_dark_matches_golden_baseline() {
        snapshot(
            "elastic_albums_under_query_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::dark(),
            draw_albums_root_under_query,
        );
    }

    fn draw_albums_root_under_query(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        // The root under a query: hit albums only, no sort control (canonical
        // hit order, exactly like the app's render_albums_browser under a
        // non-empty query).
        let items: Vec<BrowserItem> = [
            ("Geogaddi", "Boards of Canada \u{b7} 2002", false),
            (
                "Music Has the Right to Children",
                "Boards of Canada \u{b7} 1998",
                true, // selected
            ),
            ("Tri Repetae", "Autechre \u{b7} 1995", false),
        ]
        .into_iter()
        .map(|(label, detail, selected)| BrowserItem {
            key: format!("Boards\u{1f}{label}"),
            label: label.to_string(),
            detail: Some(detail.to_string()),
            thumbnail: None,
            selected,
            now_playing: false,
        })
        .collect();
        let mut provider = |i: usize| items.get(i).cloned();
        let column = BrowserColumn {
            sort_desc: false,
            show_sort: false,
            total: items.len(),
            item: &mut provider,
            virtualize: false,
            empty_title: "No matching albums",
            empty_hint: "Nothing in your library matches 'geo'.",
        };
        browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    /// The query-aware empty copy in a section column: a filtered-to-empty
    /// Albums root explains the query instead of the empty-library copy.
    #[test]
    fn browser_column_query_empty_dark_matches_golden_baseline() {
        snapshot(
            "browser_column_query_empty_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 300.0),
            Palette::dark(),
            draw_browser_column_query_empty,
        );
    }

    fn draw_browser_column_query_empty(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::browser::{self, BrowserColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let mut provider = |_: usize| None;
        let column = BrowserColumn {
            sort_desc: false,
            show_sort: false,
            total: 0,
            item: &mut provider,
            virtualize: false,
            empty_title: "No matching albums",
            empty_hint: "Nothing in your library matches 'zzz'.",
        };
        browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    /// The Folders tree under a query: branches with no match are pruned and
    /// the tree stays in place — the query never yanks it into the flat list.
    #[test]
    fn folder_tree_pruned_dark_matches_golden_baseline() {
        snapshot_animating(
            "folder_tree_pruned_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::dark(),
            draw_folder_tree_pruned,
        );
    }

    fn draw_folder_tree_pruned(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::{Icon, IconCache};
        use riff_gui::ui::sidebar::{self, TreeRow};
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        // The pruned shape of `draw_folder_tree` under a query: branches
        // without a match (e.g. "Autechre", "Tomorrow's Harvest") drop out,
        // the matched branch stays — the tree, not the flat list.
        let nodes = [
            FolderNode {
                label: "Music",
                level: 0,
                art_slot: true,
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::FolderOpen),
            },
            FolderNode {
                label: "Boards of Canada",
                level: 1,
                art_slot: false,
                selected: true,
                now_playing: false,
                playing: false,
                icon: Some(Icon::Folder),
            },
            FolderNode {
                label: "01. Ready Let's Go",
                level: 2,
                art_slot: false,
                selected: false,
                now_playing: true,
                playing: true,
                icon: None,
            },
        ];
        for node in nodes {
            sidebar::tree_row(
                ui,
                &mut cache,
                palette,
                TreeRow {
                    indent_level: node.level,
                    icon: node.icon,
                    cover: None,
                    label: node.label,
                    count: None,
                    meta: None,
                    favorite: None,
                    selected: node.selected,
                    now_playing: node.now_playing,
                    playing: node.playing,
                    art_slot: node.art_slot,
                },
            );
        }
    }

    // --- Real composed shell coverage (UI component layer, ticket 01) ----------
    //
    // The `shell_chrome_*` goldens above paint a *stand-in* stage, sidebar, and
    // playerbar. These goldens render the REAL production composition — the
    // actual `RiffApp` frame with its Titlebar, Sidebar, active Library Section
    // stage, and Playerbar — driven through the same fakes the composed UI suite
    // (`ui_tests`) already uses. They are the visual safety net the later
    // extraction tickets depend on: a chrome, sidebar, or playerbar change that
    // only reaches the live composition now moves a picture.
    //
    // Determinism: the real render path names the vendored Inter families
    // (`riff-inter-semibold`, …), so the harness must install those before the
    // first frame. Unlike `with_golden_style` (which gates a `build_ui` closure),
    // `build_eframe` draws inside its own constructor, so the font install runs
    // in the closure. `configure_fonts` is deliberately NOT used — it appends a
    // machine-dependent system CJK fallback; the Inter-only set keeps the
    // baseline portable, exactly as every other golden here.

    /// Count the distinct 8-bit RGBA colors in a rendered frame. A composed
    /// shell that actually painted chrome, text, and separators has many; a
    /// blank, clipped-to-nothing, or unpainted render collapses to a handful.
    fn distinct_colors(image: &image::RgbaImage) -> usize {
        let mut seen = std::collections::HashSet::new();
        for pixel in image.pixels() {
            seen.insert(pixel.0);
        }
        seen.len()
    }

    /// Build a real `RiffApp` frame at `size` from the composed UI suite's
    /// no-op fakes: no Application Store, no audio device, no tray, no cover
    /// worker. The default sessions leave it on the Library Section in the
    /// dark palette, which is what every golden in this file pins.
    fn composed_shell(size: egui::Vec2) -> egui_kittest::Harness<'static, riff_gui::ui::RiffApp> {
        use crate::mocks::{
            MockCovers, MockLibraryMutationStore, MockLibraryQueryStore, MockPlaylistStore,
            MockScans, MockSettingsStore, MockTagEdits, MockTransport,
        };
        use riff_backend::app::events::BackendEvents;
        use riff_backend::app::state::{LibrarySession, PlaybackSession};
        use riff_backend::app::store::StoreGeneration;
        use riff_backend::app::views::SessionViews;
        use riff_gui::ui::RiffApp;
        use std::sync::{Arc, Mutex};

        let harness = egui_kittest::Harness::builder()
            .with_size(size)
            .with_pixels_per_point(1.0)
            .build_eframe(|cc| {
                cc.egui_ctx.set_fonts(inter_only_font_definitions());
                let (app, _visibility_tx) = RiffApp::new_for_test(
                    Arc::new(Mutex::new(PlaybackSession::default())),
                    Arc::new(Mutex::new(LibrarySession::default())),
                    Box::new(MockTransport::new()),
                    Box::new(MockScans::default()),
                    Box::new(MockSettingsStore::default()),
                    Box::new(MockPlaylistStore::default()),
                    Box::new(MockLibraryMutationStore::new()),
                    SessionViews::new(
                        Box::new(MockLibraryQueryStore::default()),
                        Box::new(MockPlaylistStore::default()),
                        StoreGeneration::new(),
                        StoreGeneration::new(),
                    ),
                    Box::new(MockTagEdits),
                    Box::new(MockCovers),
                    Arc::new(Mutex::new(BackendEvents::default())),
                );
                app
            });

        // The app installs its own (dark) palette on the first `update`. Force
        // the vendored Inter set once more after construction so a warm-up
        // frame cannot have cached a system-fallback metric into a galley that
        // survives into the snapshot frame.
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness
    }

    /// Render `shell` to a stable frame and assert it actually painted a
    /// composed UI surface at `expected` before pinning it as a baseline.
    fn snapshot_composed_shell(
        shell: &mut egui_kittest::Harness<'static, riff_gui::ui::RiffApp>,
        expected: egui::Vec2,
        name: &str,
    ) {
        // The app schedules a periodic repaint tick every frame (the end-of-
        // frame responsiveness heartbeat), so `run()` would spin past its step
        // budget. The harness never advances `input.time`, so a fixed two
        // frames is deterministic — the same contract `snapshot_animating`
        // relies on.
        shell.run_steps(2);
        let frame = shell
            .render()
            .expect("the composed shell must render headlessly");
        assert_eq!(
            (frame.width(), frame.height()),
            (expected.x as u32, expected.y as u32),
            "the rendered frame must fill the requested shell size"
        );
        assert!(
            distinct_colors(&frame) > 40,
            "the real Titlebar/Sidebar/Stage/Playerbar composition must paint \
             many distinct colors, not a blank or single-fill frame"
        );
        shell.snapshot(name);
    }

    #[test]
    fn composed_shell_normal_matches_golden_baseline() {
        let _slot = harness_slot();
        let size = egui::vec2(1280.0, 800.0);
        let mut shell = composed_shell(size);
        snapshot_composed_shell(&mut shell, size, "composed_shell_normal_dark");
    }

    #[test]
    fn composed_shell_minimum_matches_golden_baseline() {
        let _slot = harness_slot();
        let size = riff_gui::ui::theme::geometry::window::MIN_WINDOW_SIZE;
        let mut shell = composed_shell(size);
        snapshot_composed_shell(&mut shell, size, "composed_shell_minimum_dark");
    }
}
