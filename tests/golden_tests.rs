// Golden-image snapshot tests (Issue 05).
//
// Renders real egui frames headlessly through `egui_kittest` (wgpu software
// path, no window required) and compares them pixel-for-pixel against
// committed baselines under `tests/snapshots/`. The set is authored against
// the **dark** palette per ADR 0004, plus the light-palette mirrors and the
// High Contrast token-set variants the coverage audit
// (docs/engineering/golden-image-gaps.md) asked for; render through
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
                        disclosure: None,
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
    /// exactly [`riff_gui::ui::chrome::MIN_WINDOW_SIZE`], so the golden pins both
    /// the panel sizes and the chrome-fitting minimum window.
    #[test]
    fn shell_chrome_dark_matches_golden_baseline() {
        snapshot(
            "shell_chrome_dark",
            riff_gui::ui::chrome::MIN_WINDOW_SIZE,
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

        // Top chrome strip: merged frameless titlebar at TITLEBAR_H.
        egui::Panel::top("titlebar")
            .exact_size(theme::TITLEBAR_H)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                show_titlebar(ui, &mut cache, palette, &content, &mut Vec::new());
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
                            disclosure: None,
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
                            disclosure: None,
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
                        disclosure: None,
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
                        disclosure: None,
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
            riff_gui::ui::chrome::MIN_WINDOW_SIZE.x - theme::SIDEBAR_W,
            riff_gui::ui::chrome::MIN_WINDOW_SIZE.y - theme::TITLEBAR_H - theme::PLAYERBAR_H,
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
                    disclosure: None,
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
                "C:\\Users\\stink\\Music",
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
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

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

    // --- Content top bar (design-handoff issue 06) ------------------------------
    //
    // The second content strip above the library stage: orange wordmark,
    // "Search or jump to…" field, and the list/grid view toggles. Rendered
    // idle (empty query so the hint text shows, list layout active) so the
    // snapshot is deterministic.

    #[test]
    fn top_bar_dark_matches_golden_baseline() {
        snapshot(
            "top_bar_dark",
            egui::vec2(800.0, theme::TOPBAR_H),
            Palette::dark(),
            draw_top_bar,
        );
    }

    fn draw_top_bar(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;
        use riff_gui::ui::topbar;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let mut query = String::new();
        let mut actions = Vec::new();

        egui::Panel::top("top_bar")
            .exact_size(theme::TOPBAR_H)
            .show(ui, |ui| {
                topbar::show_top_bar(
                    ui,
                    &mut cache,
                    palette,
                    &mut query,
                    topbar::TopBarContent {
                        layout: riff_backend::app::state::BrowserLayout::List,
                    },
                    &mut actions,
                );
            });
    }

    // --- Explorer widget baselines (design-handoff issue 15) ---------------------
    //
    // The explorer's widget seams: browser column, detail column, and the
    // selection panel — the widgets the elastic stage (elastic-column spec)
    // composes side by side at the widths its sizing policy hands them. The
    // stage's own column compositions (sized by `column_widths`) are pinned
    // by the `elastic_*_dark` goldens below. The list/grid toggle state and
    // the top-bar search are pinned by `top_bar_dark`; the grid state gets
    // its own baseline below.

    /// The browser column (the explorer's entity-list widget) at the elastic
    /// stage's preferred column width ([`riff_gui::ui::theme::COLUMN_WIDTH`],
    /// 280): the A–Z sort control, the artist variant's genre chip row (one
    /// filter engaged), and list rows with placeholder thumbnail slots,
    /// secondary detail lines, one selected and one now-playing row. Rendered
    /// idle so the snapshot is deterministic.
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
        use riff_backend::domain::GenreCount;
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
        let genres = [
            GenreCount {
                genre: "Electronic".to_string(),
                tracks: 42,
            },
            GenreCount {
                genre: "Jazz".to_string(),
                tracks: 31,
            },
            GenreCount {
                genre: "Rock".to_string(),
                tracks: 19,
            },
        ];
        let mut provider = |i: usize| items.get(i).cloned();
        let column = BrowserColumn {
            layout: riff_backend::app::state::BrowserLayout::List,
            sort_desc: false,
            show_sort: true,
            genres: &genres,
            genre_filter: Some("Electronic"),
            total: items.len(),
            item: &mut provider,
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
            egui::vec2(riff_gui::ui::theme::INSPECTOR_WIDTH, 640.0),
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
        let panel = SelectionPanel {
            art: None,
            title: Some("Tomorrow's Harvest"),
            subtitle: Some("Boards of Canada \u{b7} 2013"),
            details: &details,
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

    /// The stage's column sizing arithmetic, mirrored from
    /// `render_elastic_stage`: `column_widths` over the width the hairline
    /// separators between the columns leave (each consumes the style's
    /// separator spacing in the horizontal layout).
    fn stage_column_widths(ui: &egui::Ui, columns: usize, inspector: bool) -> Vec<f32> {
        let gaps = columns.saturating_sub(1) + usize::from(inspector);
        let separator_w = ui
            .style()
            .separator_style(
                &egui::widget_style::Classes::default(),
                egui::widget_style::WidgetState::default(),
            )
            .spacing;
        riff_gui::ui::app::column_widths(
            (ui.available_width() - separator_w * gaps as f32).max(0.0),
            columns,
            inspector,
        )
    }

    /// The stage's horizontal composition, mirrored from
    /// `render_elastic_stage`: a row of zero item spacing with a hairline
    /// separator between columns, each column drawn inside a
    /// `width`-constrained child ui. `column(index)` draws one list column;
    /// when `inspector_width` is `Some`, a final separator and the inspector
    /// column follow the list columns. The stage runs inside a `height`-tall
    /// rect: the kittest root ui sizes itself to its content, which would
    /// collapse the columns to stub heights — pin the composition at a real
    /// window size instead, like the app's CentralPanel.
    fn horizontal_stage(
        ui: &mut egui::Ui,
        widths: &[f32],
        inspector_width: Option<f32>,
        height: f32,
        mut column: impl FnMut(&mut egui::Ui, usize),
    ) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            // Mirror `render_elastic_stage`'s column scoping exactly: every
            // column child ui gets a distinct id salt, because sibling
            // `allocate_ui_with_layout` children share one stable id and their
            // persistent-id widgets (each column's ScrollArea) would collide.
            let mut scope = |ui: &mut egui::Ui, width: f32, i: usize| {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(width, ui.available_height()),
                    egui::Sense::hover(),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt(("column", i)),
                    |ui| column(ui, i),
                );
            };
            // `horizontal_top`, like the stage: plain `horizontal` sizes the
            // row to `interact_size.y` and only grows with content, which
            // would collapse the columns to stub heights.
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, width) in widths.iter().copied().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    scope(ui, width, i);
                }
                if let Some(width) = inspector_width {
                    ui.separator();
                    scope(ui, width, widths.len());
                }
            });
        });
    }

    /// The elastic stage's Artists drill-down composition: the three list
    /// columns the stage sizes side by side — the Artists root (A–Z sort,
    /// genre chips, artist rows) · the artist's Albums column (list rows,
    /// no sort, no chips) · the Tracks column (breadcrumb
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
        use riff_backend::domain::GenreCount;
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::detail::{self, Crumb, DetailColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let widths = stage_column_widths(ui, 3, false);

        horizontal_stage(ui, &widths, None, 420.0, |ui, column| {
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
                let genres = [
                    GenreCount {
                        genre: "Electronic".to_string(),
                        tracks: 42,
                    },
                    GenreCount {
                        genre: "Jazz".to_string(),
                        tracks: 31,
                    },
                    GenreCount {
                        genre: "Rock".to_string(),
                        tracks: 19,
                    },
                ];
                let mut provider = |i: usize| items.get(i).cloned();
                let column = BrowserColumn {
                    layout: riff_backend::app::state::BrowserLayout::List,
                    sort_desc: false,
                    show_sort: true,
                    genres: &genres,
                    genre_filter: Some("Electronic"),
                    total: items.len(),
                    item: &mut provider,
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
                    layout: riff_backend::app::state::BrowserLayout::List,
                    sort_desc: false,
                    show_sort: false,
                    genres: &[],
                    genre_filter: None,
                    total: items.len(),
                    item: &mut provider,
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
        let widths = stage_column_widths(ui, 4, false);

        horizontal_stage(ui, &widths, None, 420.0, |ui, column| {
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
                    layout: riff_backend::app::state::BrowserLayout::List,
                    sort_desc: false,
                    show_sort: true,
                    genres: &[],
                    genre_filter: None,
                    total: items.len(),
                    item: &mut provider,
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
                    layout: riff_backend::app::state::BrowserLayout::List,
                    sort_desc: false,
                    show_sort: false,
                    genres: &[],
                    genre_filter: None,
                    total: items.len(),
                    item: &mut provider,
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
                    layout: riff_backend::app::state::BrowserLayout::List,
                    sort_desc: false,
                    show_sort: false,
                    genres: &[],
                    genre_filter: None,
                    total: items.len(),
                    item: &mut provider,
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
            egui::vec2(900.0, 640.0),
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
        let widths = stage_column_widths(ui, 1, true);

        horizontal_stage(
            ui,
            &widths,
            Some(riff_gui::ui::theme::INSPECTOR_WIDTH),
            640.0,
            |ui, column| {
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
                        "a.flac", "b.flac", "c.flac", "d.flac", "e.flac", "f.flac", "g.flac",
                        "h.flac", "i.flac", "j.flac", "k.flac", "l.flac",
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
                        layout: riff_backend::app::state::BrowserLayout::List,
                        sort_desc: false,
                        show_sort: false,
                        genres: &[],
                        genre_filter: None,
                        total: items.len(),
                        item: &mut provider,
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
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        let panel = SelectionPanel {
                            art: None,
                            title: Some("Tomorrow's Harvest"),
                            subtitle: Some("Boards of Canada \u{b7} 2013"),
                            details: &details,
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
            },
        );
    }

    // --- Grid toggle state (design-handoff issue 15) -----------------------------
    //
    // The top bar with the grid layout engaged: the grid toggle carries the
    // brand tint and the wordmark/search field stay put. Complements
    // `top_bar_dark`, which pins the list state.

    #[test]
    fn top_bar_grid_dark_matches_golden_baseline() {
        snapshot(
            "top_bar_grid_dark",
            egui::vec2(800.0, theme::TOPBAR_H),
            Palette::dark(),
            draw_top_bar_grid,
        );
    }

    fn draw_top_bar_grid(ui: &mut egui::Ui, palette: &Palette) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;
        use riff_gui::ui::topbar;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let mut query = String::new();
        let mut actions = Vec::new();

        egui::Panel::top("top_bar_grid")
            .exact_size(theme::TOPBAR_H)
            .show(ui, |ui| {
                topbar::show_top_bar(
                    ui,
                    &mut cache,
                    palette,
                    &mut query,
                    topbar::TopBarContent {
                        layout: riff_backend::app::state::BrowserLayout::Grid,
                    },
                    &mut actions,
                );
            });
    }
    // =========================================================================
    // Gap-audit goldens (docs/engineering/golden-image-gaps.md)
    //
    // Everything below closes a gap the audit lists: the light palette and
    // High Contrast (P0-1/2), the focus ring (P0-3), the grid and folder-tree
    // code paths (P0-4/5), the unpinned surfaces (P1), state variants of
    // already-pinned regions (P2), and three cheap regression nets (P3).
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

    /// One row of the folder-tree golden: the copy, its indent level, its
    /// disclosure state (folders only), and its selection state.
    struct FolderNode {
        label: &'static str,
        level: usize,
        disclosure: Option<bool>,
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
            riff_gui::ui::chrome::MIN_WINDOW_SIZE,
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
    fn top_bar_light_matches_golden_baseline() {
        snapshot(
            "top_bar_light",
            egui::vec2(800.0, theme::TOPBAR_H),
            Palette::light(),
            draw_top_bar,
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

    /// The most accessibility-critical pixel in the app: the focused search
    /// well's ring, in the High Contrast variant that thickens it.
    #[test]
    fn top_bar_search_focused_hc_matches_golden_baseline() {
        snapshot(
            "top_bar_search_focused_hc",
            egui::vec2(800.0, theme::TOPBAR_H),
            Palette::dark().high_contrast(),
            |ui, palette| draw_top_bar_with(ui, palette, true),
        );
    }

    /// The global search well with keyboard focus — the ring
    /// `sidebar::search_ring_stroke` paints. Focus is requested directly
    /// through memory (fully deterministic; the determinism rule bans
    /// *hover*-dependent rendering, not focus).
    #[test]
    fn top_bar_search_focused_dark_matches_golden_baseline() {
        snapshot(
            "top_bar_search_focused_dark",
            egui::vec2(800.0, theme::TOPBAR_H),
            Palette::dark(),
            |ui, palette| draw_top_bar_with(ui, palette, true),
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

    fn draw_top_bar_with(ui: &mut egui::Ui, palette: &Palette, search_focused: bool) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;
        use riff_gui::ui::topbar;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        if search_focused {
            // The global search field's stable id (`topbar::show_top_bar`):
            // focus it through memory, so the ring lands this frame.
            ui.memory_mut(|m| m.request_focus(egui::Id::new("riff_global_search")));
        }

        let mut cache = IconCache::new();
        let mut query = String::new();
        let mut actions = Vec::new();

        egui::Panel::top("top_bar_focus_variant")
            .exact_size(theme::TOPBAR_H)
            .show(ui, |ui| {
                topbar::show_top_bar(
                    ui,
                    &mut cache,
                    palette,
                    &mut query,
                    topbar::TopBarContent {
                        layout: riff_backend::app::state::BrowserLayout::List,
                    },
                    &mut actions,
                );
            });
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

    // --- P0-4: the browser grid layout ----------------------------------------

    /// `show_browser_grid` / `grid_tile` at the column's preferred width: two
    /// [`riff_gui::ui::browser::TILE_SIZE`] tiles per row. The whole grid code
    /// path was a blind spot — every other composition passes
    /// `BrowserLayout::List`.
    #[test]
    fn browser_grid_dark_matches_golden_baseline() {
        snapshot(
            "browser_grid_dark",
            egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0),
            Palette::dark(),
            draw_browser_grid,
        );
    }

    fn draw_browser_grid(ui: &mut egui::Ui, palette: &Palette) {
        use riff_backend::app::state::BrowserLayout;
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        let rows = [
            ("Geogaddi", true, false),
            ("Music Has the Right", false, false),
            ("Tomorrow's Harvest", false, true),
            ("Amber", false, false),
            ("Tri Repetae", false, false),
            ("Incunabula", false, false),
        ];
        let items: Vec<BrowserItem> = rows
            .into_iter()
            .map(|(label, selected, now_playing)| BrowserItem {
                key: label.to_string(),
                label: label.to_string(),
                detail: None,
                thumbnail: None,
                selected,
                now_playing,
            })
            .collect();
        let mut provider = |i: usize| items.get(i).cloned();
        let column = BrowserColumn {
            layout: BrowserLayout::Grid,
            sort_desc: false,
            show_sort: true,
            genres: &[],
            genre_filter: None,
            total: items.len(),
            item: &mut provider,
            empty_title: "",
            empty_hint: "",
        };
        browser::show_browser_column(ui, &mut cache, palette, column, &mut Vec::new());
    }

    // --- P0-5: the Folders tree -----------------------------------------------

    /// `BrowseMode::Folders` renders through `TreeRow::disclosure` on the
    /// `INDENT_STEP` indent scale — a code path no golden exercised. The rows
    /// are the app's own folder-node shape: open/closed disclosure glyphs,
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
                disclosure: Some(true),
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::FolderOpen),
            },
            FolderNode {
                label: "Boards of Canada",
                level: 1,
                disclosure: Some(true),
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::FolderOpen),
            },
            FolderNode {
                label: "Geogaddi",
                level: 2,
                disclosure: Some(false),
                selected: true,
                now_playing: false,
                playing: false,
                icon: Some(Icon::Folder),
            },
            FolderNode {
                label: "Autechre",
                level: 1,
                disclosure: Some(false),
                selected: false,
                now_playing: false,
                playing: false,
                icon: Some(Icon::Folder),
            },
            FolderNode {
                label: "01. Ready Let's Go",
                level: 2,
                disclosure: None,
                selected: false,
                now_playing: true,
                playing: true,
                icon: None,
            },
            FolderNode {
                label: "02. Music Is Math",
                level: 2,
                disclosure: None,
                selected: false,
                now_playing: true,
                playing: false,
                icon: None,
            },
            FolderNode {
                label: "Tomorrow's Harvest",
                level: 1,
                disclosure: None,
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
                    disclosure: node.disclosure,
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

    /// The Edit Tags modal: the track path, one labeled field per editable
    /// tag, and Save / Cancel. Writing only ever happens on an explicit Save,
    /// so rendering it is side-effect free.
    #[test]
    fn tag_edit_modal_dark_matches_golden_baseline() {
        snapshot(
            "tag_edit_modal_dark",
            egui::vec2(560.0, 460.0),
            Palette::dark(),
            draw_tag_edit_modal,
        );
    }

    fn draw_tag_edit_modal(ui: &mut egui::Ui, palette: &Palette) {
        use riff_backend::domain::TrackId;
        use riff_gui::ui::app::TagEditState;
        use riff_gui::ui::theme::SURFACE_BG;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut state = TagEditState {
            track_id: TrackId("f:\\music\\geogaddi\\01.flac".to_string()),
            path: std::path::PathBuf::from(
                "F:\\music\\Boards of Canada\\Geogaddi\\01. Ready Let's Go.flac",
            ),
            title: "Ready Let's Go".to_string(),
            artist: "Boards of Canada".to_string(),
            album: "Geogaddi".to_string(),
            album_artist: "Boards of Canada".to_string(),
            genre: "Electronic".to_string(),
            year: "2002".to_string(),
            track_number: "1".to_string(),
            error: None,
            saving: false,
        };
        let _ = riff_gui::ui::prompts::tag_edit_modal(ui.ctx(), palette, &mut state);
    }

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

        let _ = riff_gui::ui::prompts::clear_library_confirm(ui, palette);
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
            layout: riff_backend::app::state::BrowserLayout::List,
            sort_desc: false,
            show_sort: true,
            genres: &[],
            genre_filter: None,
            total: 0,
            item: &mut provider,
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
        let widths = stage_column_widths(ui, 2, false);

        horizontal_stage(ui, &widths, None, 420.0, |ui, column| {
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
                    layout: riff_backend::app::state::BrowserLayout::List,
                    sort_desc: false,
                    show_sort: true,
                    genres: &[],
                    genre_filter: None,
                    total: items.len(),
                    item: &mut provider,
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
        let widths = stage_column_widths(ui, 3, false);

        horizontal_stage(ui, &widths, None, 420.0, |ui, column| {
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
                    layout: riff_backend::app::state::BrowserLayout::List,
                    sort_desc: false,
                    show_sort: false,
                    genres: &[],
                    genre_filter: None,
                    total: items.len(),
                    item: &mut provider,
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
            egui::vec2(900.0, 640.0),
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
        let widths = stage_column_widths(ui, 1, true);

        horizontal_stage(
            ui,
            &widths,
            Some(riff_gui::ui::theme::INSPECTOR_WIDTH),
            640.0,
            |ui, column| {
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
                        layout: riff_backend::app::state::BrowserLayout::List,
                        sort_desc: false,
                        show_sort: false,
                        genres: &[],
                        genre_filter: None,
                        total: items.len(),
                        item: &mut provider,
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
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        let panel = SelectionPanel {
                            art: None,
                            title: Some(title),
                            subtitle: Some(subtitle),
                            details: &details,
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
            },
        );
    }

    // --- P1-10: the narrow-window shrink branch --------------------------------

    /// The stage at `chrome::MIN_WINDOW_SIZE`: `column_widths` shrinks every
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
            riff_gui::ui::chrome::MIN_WINDOW_SIZE,
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
            riff_gui::ui::chrome::MIN_WINDOW_SIZE,
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
            riff_gui::ui::chrome::MIN_WINDOW_SIZE,
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
                        "C:\\Users\\stink\\Music",
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
                        "C:\\Users\\stink\\Music",
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
                        "C:\\Users\\stink\\Music",
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
            egui::vec2(riff_gui::ui::theme::INSPECTOR_WIDTH, 640.0),
            Palette::dark(),
            |ui, palette| draw_selection_panel_with(ui, palette, true, false),
        );
    }

    /// The single-track readout: the primary action reads **Play**.
    #[test]
    fn selection_panel_single_dark_matches_golden_baseline() {
        snapshot(
            "selection_panel_single_dark",
            egui::vec2(riff_gui::ui::theme::INSPECTOR_WIDTH, 640.0),
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
        let panel = SelectionPanel {
            art: art.as_ref(),
            title: Some(title),
            subtitle: Some(subtitle),
            details: &details,
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
}
