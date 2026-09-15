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
                        cover: None,
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
        use riff_gui::ui::theme::{Palette, SURFACE_BG, TEXTURE_TINT};

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
                            cover: None,
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
                            cover: None,
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
                        cover: None,
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
                        cover: None,
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
                    cover: None,
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(riff_gui::ui::theme::COLUMN_WIDTH, 420.0))
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
        use riff_gui::ui::detail::{self, Crumb, DetailColumn};
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
        let tracks = geogaddi_tracks();
        let column = DetailColumn {
            breadcrumb: &crumbs,
            header: Some(&header),
            tracks: &tracks,
            ..DetailColumn::empty("", "")
        };
        detail::show_detail_column(ui, &mut cache, &palette, column, &mut Vec::new());
    }

    /// The dummy Geogaddi track table shared by the detail-column golden and
    /// the elastic drilled compositions: one favorite, one selected, one
    /// now-playing (idle, so nothing animates).
    fn geogaddi_tracks() -> Vec<riff_gui::ui::detail::TrackRow> {
        use riff_gui::ui::detail::TrackRow;
        vec![
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(riff_gui::ui::theme::INSPECTOR_WIDTH, 640.0))
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
            single: false,
            // The golden pins the panel's single Play album action — the
            // rendering the original fixed pane used; the inspector's Play /
            // Add to Queue row (`queue: true`) is pinned by the
            // `elastic_all_tracks_inspector_dark` composition below.
            queue: false,
        };
        selection::show_selection_panel(ui, &mut cache, &palette, panel, &mut Vec::new());
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1180.0, 420.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_elastic_artists_drilled);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("elastic_artists_drilled_dark");
    }

    fn draw_elastic_artists_drilled(ui: &mut egui::Ui) {
        use riff_backend::domain::GenreCount;
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::detail::{self, Crumb, DetailColumn};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
                browser::show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
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
                browser::show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
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
            detail::show_detail_column(ui, &mut cache, &palette, column, &mut Vec::new());
        });
    }

    /// The elastic stage's Genres drill-down composition: the four list
    /// columns the stage sizes side by side — the Genres root · the
    /// artists-in-genre column · the albums-in-genre column · the Tracks
    /// column (breadcrumb `Genres / Electronic / Autechre / Tri Repetae`).
    #[test]
    fn elastic_genres_drilled_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1460.0, 420.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_elastic_genres_drilled);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("elastic_genres_drilled_dark");
    }

    fn draw_elastic_genres_drilled(ui: &mut egui::Ui) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::detail::{self, Crumb, DetailColumn, TrackRow};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
                browser::show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
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
                browser::show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
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
                browser::show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
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
                    number: Some(1),
                    title: "Drane".to_string(),
                    plays: 14,
                    duration: Some(std::time::Duration::from_secs(377)),
                    favorite: false,
                    selected: false,
                    now_playing: false,
                },
                TrackRow {
                    key: "g2".to_string(),
                    number: Some(2),
                    title: "Eutow".to_string(),
                    plays: 27,
                    duration: Some(std::time::Duration::from_secs(255)),
                    favorite: true,
                    selected: false,
                    now_playing: false,
                },
                TrackRow {
                    key: "g3".to_string(),
                    number: Some(3),
                    title: "C/Pach".to_string(),
                    plays: 8,
                    duration: Some(std::time::Duration::from_secs(237)),
                    favorite: false,
                    selected: true,
                    now_playing: false,
                },
                TrackRow {
                    key: "g4".to_string(),
                    number: Some(4),
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
            detail::show_detail_column(ui, &mut cache, &palette, column, &mut Vec::new());
        });
    }

    /// The elastic stage's All Tracks composition: the flat Tracks listing
    /// beside the collapsible inspector (the selection panel's Play / Add to
    /// Queue variant) — one selected and one now-playing (idle) track row,
    /// the inspector at [`riff_gui::ui::theme::INSPECTOR_WIDTH`].
    #[test]
    fn elastic_all_tracks_inspector_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui(draw_elastic_all_tracks_inspector);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("elastic_all_tracks_inspector_dark");
    }

    fn draw_elastic_all_tracks_inspector(ui: &mut egui::Ui) {
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::selection::{self, SelectionDetail, SelectionPanel};
        use riff_gui::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
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
                    browser::show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
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
                            &palette,
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

    // TEMP DEBUG: bisect the red-text corruption.
    #[test]
    fn debug_red_check() {
        use riff_backend::domain::GenreCount;
        use riff_gui::ui::browser::{self, BrowserColumn, BrowserItem};
        use riff_gui::ui::icons::IconCache;

        fn red_count(image: &image::RgbaImage) -> usize {
            image
                .pixels()
                .filter(|p| p.0[0] > 200 && p.0[1] < 110 && p.0[2] < 110)
                .count()
        }

        fn base_rows() -> Vec<BrowserItem> {
            [
                ("Boards of Canada", "12 albums", false, false),
                ("Daft Punk", "9 albums", true, false),
                ("Miles Davis", "31 albums", false, false),
                ("Portishead", "5 albums", false, true),
            ]
            .into_iter()
            .map(|(label, detail, selected, now_playing)| BrowserItem {
                key: label.to_string(),
                label: label.to_string(),
                detail: Some(detail.to_string()),
                thumbnail: None,
                selected,
                now_playing,
            })
            .collect()
        }

        fn genres() -> Vec<GenreCount> {
            vec![
                GenreCount {
                    genre: "Electronic".to_string(),
                    tracks: 42,
                },
                GenreCount {
                    genre: "Jazz".to_string(),
                    tracks: 31,
                },
            ]
        }

        fn run_variant<F: FnMut(&mut egui::Ui)>(size: egui::Vec2, mut draw: F) -> usize {
            let mut harness = egui_kittest::Harness::builder()
                .with_size(size)
                .with_pixels_per_point(1.0)
                .build_ui(move |ui| draw(ui));
            theme::install(&harness.ctx, &Palette::dark());
            harness.ctx.set_fonts(inter_only_font_definitions());
            harness.run();
            let frame = harness.render().unwrap();
            red_count(&frame)
        }

        // A: root ui, full width, one browser column with chips.
        let a = run_variant(egui::vec2(1180.0, 420.0), |ui| {
            let background = ui.ctx().layer_painter(egui::LayerId::background());
            background.rect_filled(ui.ctx().content_rect(), 0.0, theme::SURFACE_BG);
            let palette = Palette::dark();
            let mut cache = IconCache::new();
            let items = base_rows();
            let genres = genres();
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
        });
        eprintln!("A (root ui, full width, 1 column): red={a}");

        // B: same column inside a 280-wide child ui.
        let b = run_variant(egui::vec2(1180.0, 420.0), |ui| {
            let background = ui.ctx().layer_painter(egui::LayerId::background());
            background.rect_filled(ui.ctx().content_rect(), 0.0, theme::SURFACE_BG);
            let palette = Palette::dark();
            let mut cache = IconCache::new();
            ui.allocate_ui_with_layout(
                egui::vec2(280.0, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let items = base_rows();
                    let genres = genres();
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
                },
            );
        });
        eprintln!("B (280 child ui, 1 column): red={b}");

        // C: 280-wide root-ui harness (like browser_column_dark) as control.
        let c = run_variant(egui::vec2(280.0, 420.0), |ui| {
            let background = ui.ctx().layer_painter(egui::LayerId::background());
            background.rect_filled(ui.ctx().content_rect(), 0.0, theme::SURFACE_BG);
            let palette = Palette::dark();
            let mut cache = IconCache::new();
            let items = base_rows();
            let genres = genres();
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
        });
        eprintln!("C (280 root ui control): red={c}");

        // D: two browser columns side by side (horizontal stage), no detail column.
        let d = run_variant(egui::vec2(1180.0, 420.0), |ui| {
            use riff_gui::ui::app::column_widths;
            let background = ui.ctx().layer_painter(egui::LayerId::background());
            background.rect_filled(ui.ctx().content_rect(), 0.0, theme::SURFACE_BG);
            let palette = Palette::dark();
            let mut cache = IconCache::new();
            let gaps = 1;
            let separator_w = ui
                .style()
                .separator_style(&Default::default(), Default::default())
                .spacing;
            let widths = column_widths(
                (ui.available_width() - separator_w * gaps as f32).max(0.0),
                2,
                false,
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, width) in widths.iter().copied().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            let items = base_rows();
                            let genres = genres();
                            let mut provider = |i: usize| items.get(i).cloned();
                            let column = BrowserColumn {
                                layout: riff_backend::app::state::BrowserLayout::List,
                                sort_desc: false,
                                show_sort: i == 0,
                                genres: if i == 0 { &genres } else { &[] },
                                genre_filter: if i == 0 { Some("Electronic") } else { None },
                                total: items.len(),
                                item: &mut provider,
                                empty_title: "",
                                empty_hint: "",
                            };
                            browser::show_browser_column(
                                ui,
                                &mut cache,
                                &palette,
                                column,
                                &mut Vec::new(),
                            );
                        },
                    );
                }
            });
        });
        eprintln!("D (2 browser columns): red={d}");

        // E: the full three-column artists composition.
        let e = run_variant(egui::vec2(1180.0, 420.0), draw_elastic_artists_drilled);
        eprintln!("E (full 3-col artists composition): red={e}");

        // F: one browser column with 12 rows (text volume).
        let f = run_variant(egui::vec2(1180.0, 420.0), |ui| {
            let background = ui.ctx().layer_painter(egui::LayerId::background());
            background.rect_filled(ui.ctx().content_rect(), 0.0, theme::SURFACE_BG);
            let palette = Palette::dark();
            let mut cache = IconCache::new();
            let items: Vec<BrowserItem> = (0..12)
                .map(|i| BrowserItem {
                    key: format!("artist-{i}"),
                    label: format!("Artist Number {i}"),
                    detail: Some(format!("{i} albums")),
                    thumbnail: None,
                    selected: i == 1,
                    now_playing: i == 2,
                })
                .collect();
            let genres = genres();
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
        });
        eprintln!("F (1 column, 12 rows): red={f}");
    }

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
