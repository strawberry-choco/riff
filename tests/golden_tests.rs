// Golden-image snapshot tests (Issue 05).
//
// Renders real egui frames headlessly through `egui_kittest` (wgpu software
// path, no window required) and compares them pixel-for-pixel against
// committed baselines under `tests/snapshots/`. Baselines are authored
// against the **dark** palette per ADR 0004.
//
// See docs/engineering/golden-image-testing.md for the authoring,
// re-baselining, and diff-review workflow.

#[cfg(test)]
mod tests {
    use riff_gui::ui::fonts::{self, INTER_FACES};
    use riff_gui::ui::theme::{self, Palette};

    // --- Harness plumbing ------------------------------------------------------

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

    /// Render the trivial component through a fixed-size, fixed-DPI harness
    /// styled with the dark palette, then compare against the committed
    /// baseline. Installs fonts/style after the constructor's first frame and
    /// re-runs so the snapshotted output reflects them.
    fn snapshot_dark_play_card(name: &str) {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 88.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_play_card);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot(name);
    }

    /// The first golden component: a primary "Play" button on a surface
    /// card. Every color comes from the token constants in
    /// [`riff_gui::ui::theme`], so the image pins the Issue 01 foundation:
    /// window background, surface fill, brand-500 fill, ink text, and both
    /// radius steps, with the button label in Inter Medium.
    fn draw_play_card(ui: &mut egui::Ui) {
        use riff_gui::ui::theme::{BRAND_500, INK, RADIUS_MD, RADIUS_SM, SURFACE, SURFACE_BG};

        // Paint the window background across the ENTIRE canvas. The root UI
        // under the kittest harness is inset from the true screen rect, so a
        // panel fill would leave an unpainted clear-color ring around the
        // golden image. Painting on the root layer itself keeps the card
        // above it (same-layer shapes render in submission order) while the
        // layer painter's clip rect spans the full canvas.
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        // Center the card vertically within the layout area: half the
        // leftover space above, the card (2 × 12 px margin + 36 px button =
        // 60 px), the rest below.
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space((ui.available_height() - 60.0) / 2.0);
            egui::Frame::new()
                .fill(SURFACE)
                .corner_radius(RADIUS_MD)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    let play = egui::Button::new(egui::RichText::new("Play").color(INK))
                        .fill(BRAND_500)
                        .corner_radius(RADIUS_SM)
                        .min_size(egui::vec2(120.0, 36.0));
                    ui.add(play);
                });
        });
    }

    // --- Golden baselines --------------------------------------------------------

    #[test]
    fn dark_play_card_matches_golden_baseline() {
        snapshot_dark_play_card("play_card_dark");
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
                        label: "All Tracks",
                        count: None,
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

    // --- Generated cover block renders headlessly (handoff issue 14) ---------------

    /// The generated colour block is a *user-loaded* texture (built through
    /// `ctx.load_texture` inside the frame), the exact category the pinned
    /// egui 0.35 must keep rendering in headless snapshots (the 0.36
    /// regression in the workspace notes). Behavior test, not a golden:
    /// resolve the block for one identity through the shared-cache seam and
    /// look for its derived colour among the playerbar's output pixels —
    /// the color the user actually sees on the 56×56 cover square. Tolerance
    /// ±2 per channel absorbs driver-level dithering on flat fills.
    #[test]
    fn generated_cover_block_renders_in_a_headless_snapshot() {
        use riff_gui::ui::cover_placeholder::{generated_colour, lookup_cover_texture};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::{self, PlayerBarContent};
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

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

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

                // A full miss on the shared cover cache resolves the
                // identity's generated block, exactly as the app's views do.
                let mut textures = std::collections::HashMap::new();
                let mut lru_keys = Vec::new();
                let block =
                    lookup_cover_texture(&mut textures, &mut lru_keys, ui.ctx(), true, IDENTITY);
                let palette = Palette::dark();
                let content = PlayerBarContent {
                    cover: Some(block),
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
                };
                let mut cache = IconCache::new();
                playerbar::show_player_bar(
                    ui,
                    &mut cache,
                    &palette,
                    &content,
                    &mut riff_gui::ui::playerbar::SeekReadouts::default(),
                    &mut Vec::new(),
                );
            });
        theme::install(&harness.ctx, &Palette::dark());
        harness.run();

        let frame = harness.render().unwrap();
        let colour = generated_colour(IDENTITY, true);
        assert!(
            count_pixels(&frame, colour) > 0,
            "the generated block's derived colour {colour:?} must appear in the \
             headless render — user-loaded textures must not be dropped"
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(riff_gui::ui::chrome::MIN_WINDOW_SIZE)
            .with_pixels_per_point(1.0)
            .build_ui(draw_shell_chrome);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("shell_chrome_dark");
    }

    fn draw_shell_chrome(ui: &mut egui::Ui) {
        use riff_gui::ui::chrome::{TitleBarContent, show_titlebar};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::{self, Palette, SURFACE_BG};

        // Full-canvas background (determinism rule): the stage reads as the
        // window background while the chrome panels sit on surface tokens.
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let content = TitleBarContent {
            scan_status: None,
            theme_dark: true,
            advanced_mode: false,
            active_nav: Some(riff_gui::ui::chrome::NavDestination::Library),
        };
        let mut cache = IconCache::new();

        // Top chrome strip: merged frameless titlebar at TITLEBAR_H.
        egui::Panel::top("titlebar")
            .exact_size(theme::TITLEBAR_H)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                show_titlebar(ui, &mut cache, &palette, &content, &mut Vec::new());
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_sidebar);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("sidebar_dark");
    }

    fn draw_sidebar(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::{Icon, IconCache};
        use riff_gui::ui::sidebar::{self, TreeRow};
        use riff_gui::ui::theme::{Palette, SIDEBAR_W, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let mut cache = IconCache::new();

        egui::Panel::left("sidebar")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| {
                sidebar::section_header(ui, &palette, "Library");

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
                        &palette,
                        TreeRow {
                            indent_level: 0,
                            icon,
                            label,
                            count: Some(count),
                            selected,
                            now_playing: false,
                            playing: false,
                            disclosure: None,
                        },
                    );
                }
                ui.add_space(8.0);

                sidebar::section_header(ui, &palette, "Smart Lists");
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
                        &palette,
                        TreeRow {
                            indent_level: 0,
                            icon: Some(Icon::Sparkles),
                            label: name,
                            count: Some(count),
                            selected: i == 1,
                            now_playing: false,
                            playing: false,
                            disclosure: None,
                        },
                    );
                }
                ui.add_space(8.0);

                sidebar::section_header(ui, &palette, "Playlists");
                sidebar::playlist_row(
                    ui,
                    &mut cache,
                    &palette,
                    "Focus Mix",
                    "Focus Mix (12)",
                    false,
                );
                sidebar::playlist_row(ui, &mut cache, &palette, "Workout", "Workout (3)", true);

                // A nested track row pair showing the indent scale in action.
                sidebar::tree_row(
                    ui,
                    &mut cache,
                    &palette,
                    TreeRow {
                        indent_level: 1,
                        icon: None,
                        label: "01. Moonlight Sonata",
                        count: None,
                        selected: false,
                        now_playing: true,
                        playing: false,
                        disclosure: None,
                    },
                );
                sidebar::tree_row(
                    ui,
                    &mut cache,
                    &palette,
                    TreeRow {
                        indent_level: 2,
                        icon: None,
                        label: "02. Für Elise",
                        count: None,
                        selected: false,
                        now_playing: false,
                        playing: false,
                        disclosure: None,
                    },
                );

                // The Add-folder / last-scan footer.
                sidebar::sidebar_footer(ui, &mut cache, &palette, Some("Last scan 5m ago"));
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui(draw_playerbar);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("playerbar_dark");
    }

    fn draw_playerbar(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::{self, PlayerBarContent};
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let content = PlayerBarContent {
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
        };
        let mut cache = IconCache::new();
        playerbar::show_player_bar(
            ui,
            &mut cache,
            &palette,
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 360.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_queue_panel);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("queue_panel_dark");
    }

    fn draw_queue_panel(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::now_playing::UpNextEntry;
        use riff_gui::ui::playerbar;
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
        playerbar::show_queue_panel(ui, &mut cache, &palette, &entries, &mut Vec::new());
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(library_stage_size())
            .with_pixels_per_point(1.0)
            .build_ui(draw_library_hero);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("library_hero_dark");
    }

    fn draw_library_hero(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::library::empty_state_hero;
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let mut cache = IconCache::new();
        empty_state_hero(ui, &mut cache, &Palette::dark());
    }

    /// The populated-library track list: the styled 40px rows the explorer
    /// lists tracks with — "Artist - Title" labels on the row seam the flat
    /// list renders through, one selected and one now-playing (idle, so
    /// nothing animates between runs).
    #[test]
    fn library_track_list_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(library_stage_size())
            .with_pixels_per_point(1.0)
            .build_ui(draw_library_track_list);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("library_track_list_dark");
    }

    fn draw_library_track_list(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::sidebar::{self, TreeRow};
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let mut cache = IconCache::new();

        // Same row shape `RiffApp::render_track_row` produces for the flat
        // list: indent 0, no leading glyph, "Artist - Title" label.
        let rows = [
            ("Daft Punk - One More Time", false, false),
            ("Radiohead - Weird Fishes", false, true), // now-playing, idle
            ("Miles Davis - So What", true, false),    // selected
            ("Portishead - Roads", false, false),
            ("Burial - Archangel", false, false),
            ("Nils Frahm - Says", false, false),
        ];
        for (label, selected, now_playing) in rows {
            sidebar::tree_row(
                ui,
                &mut cache,
                &palette,
                TreeRow {
                    indent_level: 0,
                    icon: None,
                    label,
                    count: None,
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(
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
            ))
            .with_pixels_per_point(1.0)
            .build_ui(draw_now_playing);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("now_playing_dark");
    }

    fn draw_now_playing(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::now_playing::{self, NowPlayingContent, UpNextEntry};
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
            &palette,
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(
                riff_gui::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .x
                    - theme::SIDEBAR_W,
                840.0,
            ))
            .with_pixels_per_point(1.0)
            .build_ui(draw_settings_modal);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("settings_dark");
    }

    fn draw_settings_modal(ui: &mut egui::Ui) {
        use riff_backend::app::state::{LibraryStatus, WatchState};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::settings::{self, LibraryRow, SettingsContent, SettingsSection};
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let content = SettingsContent {
            libraries: vec![LibraryRow {
                path: "C:\\Users\\stink\\Music".into(),
                status: LibraryStatus::Scanned(1284),
                watch: WatchState::Enabled,
                indexed_tracks: 1284,
            }],
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
            missing_artwork_strategy: Default::default(),
            last_scan: Some(riff_backend::app::store::FullScanSummary {
                // Rendered immediately, so the relative stamp reads "just
                // now" deterministically.
                at: std::time::SystemTime::now(),
                files: 1284,
                errors: 3,
            }),
        };
        let mut cache = IconCache::new();
        settings::show_settings_modal(ui, &mut cache, &palette, &content, SettingsSection::Library);
    }

    // --- Content top bar (design-handoff issue 06) ------------------------------
    //
    // The second content strip above the library stage: orange wordmark,
    // "Search or jump to…" field, and the list/grid view toggles. Rendered
    // idle (empty query so the hint text shows, list layout active) so the
    // snapshot is deterministic.

    #[test]
    fn top_bar_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::TOPBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui(draw_top_bar);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("top_bar_dark");
    }

    fn draw_top_bar(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;
        use riff_gui::ui::topbar;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut query = String::new();
        let mut actions = Vec::new();

        egui::Panel::top("top_bar")
            .exact_size(theme::TOPBAR_H)
            .show(ui, |ui| {
                topbar::show_top_bar(
                    ui,
                    &mut cache,
                    &palette,
                    &mut query,
                    topbar::TopBarContent {
                        layout: riff_backend::app::state::BrowserLayout::List,
                    },
                    &mut actions,
                );
            });
    }

    // --- Three-pane explorer baselines (design-handoff issue 15) -------------------
    //
    // The missing column goldens: browser column, detail column, and the
    // selection panel — the three panes of the restructured explorer. The
    // list/grid toggle state and the top-bar search are pinned by
    // `top_bar_dark`; the grid state gets its own baseline below.

    /// The browser column (the explorer's first pane) at the width the
    /// three-pane layout gives it: the A–Z sort control, the artist
    /// variant's genre chip row (one filter engaged), and list rows with
    /// placeholder thumbnail slots, secondary detail lines, one selected and
    /// one now-playing row. Rendered idle so the snapshot is deterministic.
    #[test]
    fn browser_column_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 420.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_browser_column);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("browser_column_dark");
    }

    fn draw_browser_column(ui: &mut egui::Ui) {
        use riff_backend::domain::GenreCount;
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
        browser::show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
    }

    /// The detail column (the explorer's middle pane) at album level: the
    /// breadcrumb trail, the album header with its subtitle, and the
    /// `# / Title / Plays / Time` track table — one favorite, one selected,
    /// one now-playing (idle, so nothing animates). The harness is wide
    /// enough that the Time column clears the right edge — a clipped
    /// golden would bake truncation into the baseline.
    #[test]
    fn detail_column_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(480.0, 420.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_detail_column);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("detail_column_dark");
    }

    fn draw_detail_column(ui: &mut egui::Ui) {
        use riff_gui::ui::detail::{self, Crumb, DetailColumn, TrackRow};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
        let tracks = [
            TrackRow {
                key: "t1".to_string(),
                number: Some(1),
                title: "Ready Let's Go".to_string(),
                plays: 12,
                duration: Some(std::time::Duration::from_secs(201)),
                favorite: false,
                selected: false,
                now_playing: false,
            },
            TrackRow {
                key: "t2".to_string(),
                number: Some(2),
                title: "Music Is Math".to_string(),
                plays: 34,
                duration: Some(std::time::Duration::from_secs(322)),
                favorite: true,
                selected: false,
                now_playing: false,
            },
            TrackRow {
                key: "t3".to_string(),
                number: Some(3),
                title: "Beware the Friendly Stranger".to_string(),
                plays: 5,
                duration: Some(std::time::Duration::from_secs(27)),
                favorite: false,
                selected: true,
                now_playing: false,
            },
            TrackRow {
                key: "t4".to_string(),
                number: Some(4),
                title: "Gyroscope".to_string(),
                plays: 21,
                duration: Some(std::time::Duration::from_secs(207)),
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
        detail::show_detail_column(ui, &mut cache, &palette, column, &mut Vec::new());
    }

    /// The selection panel (the explorer's third pane): the SELECTION
    /// header with its kind chip, the 268×200 placeholder art block, the
    /// album title over its artist · year line, the Play album action, and
    /// the details grid. Rendered without art so no texture load is
    /// involved — the generated-colour placeholder path is pinned by the
    /// headless behavior test above.
    #[test]
    fn selection_panel_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_selection_panel);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("selection_panel_dark");
    }

    fn draw_selection_panel(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::selection::{self, SelectionDetail, SelectionPanel};
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
        };
        selection::show_selection_panel(ui, &mut cache, &palette, panel, &mut Vec::new());
    }

    // --- Grid toggle state (design-handoff issue 15) -----------------------------
    //
    // The top bar with the grid layout engaged: the grid toggle carries the
    // brand tint and the wordmark/search field stay put. Complements
    // `top_bar_dark`, which pins the list state.

    #[test]
    fn top_bar_grid_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::TOPBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui(draw_top_bar_grid);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("top_bar_grid_dark");
    }

    fn draw_top_bar_grid(ui: &mut egui::Ui) {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::SURFACE_BG;
        use riff_gui::ui::topbar;

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut query = String::new();
        let mut actions = Vec::new();

        egui::Panel::top("top_bar_grid")
            .exact_size(theme::TOPBAR_H)
            .show(ui, |ui| {
                topbar::show_top_bar(
                    ui,
                    &mut cache,
                    &palette,
                    &mut query,
                    topbar::TopBarContent {
                        layout: riff_backend::app::state::BrowserLayout::Grid,
                    },
                    &mut actions,
                );
            });
    }
}
