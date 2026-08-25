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
    use riff::ui::fonts::{self, INTER_FACES};
    use riff::ui::theme::{self, Palette};

    // --- Harness plumbing ------------------------------------------------------

    /// Deterministic font definitions for golden rendering: the vendored
    /// Inter faces only. Unlike [`riff::ui::fonts::font_definitions`] this
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
    /// [`riff::ui::theme`], so the image pins the Issue 01 foundation:
    /// window background, surface fill, brand-500 fill, ink text, and both
    /// radius steps, with the button label in Inter Medium.
    fn draw_play_card(ui: &mut egui::Ui) {
        use riff::ui::theme::{BRAND_500, INK, RADIUS_MD, RADIUS_SM, SURFACE, SURFACE_BG};

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

    // --- Shell chrome baseline (Issue 06) ----------------------------------------

    /// The unified shell chrome at exact token dimensions: 56px titlebar
    /// (wordmark, drag region, window + view controls), 280px sidebar,
    /// 88px playerbar strip, and the central stage. The harness renders at
    /// exactly [`riff::ui::chrome::MIN_WINDOW_SIZE`], so the golden pins both
    /// the panel sizes and the chrome-fitting minimum window.
    #[test]
    fn shell_chrome_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(riff::ui::chrome::MIN_WINDOW_SIZE)
            .with_pixels_per_point(1.0)
            .build_ui(draw_shell_chrome);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("shell_chrome_dark");
    }

    fn draw_shell_chrome(ui: &mut egui::Ui) {
        use riff::ui::chrome::{TitleBarContent, show_titlebar};
        use riff::ui::icons::IconCache;
        use riff::ui::theme::{self, Palette, SURFACE_BG};

        // Full-canvas background (determinism rule): the stage reads as the
        // window background while the chrome panels sit on surface tokens.
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let content = TitleBarContent {
            scan_status: None,
            theme_dark: true,
            advanced_mode: false,
            active_nav: Some(riff::ui::chrome::NavDestination::Library),
        };
        let mut cache = IconCache::new();

        // Top chrome strip: merged frameless titlebar at TITLEBAR_H.
        egui::Panel::top("titlebar")
            .exact_size(theme::TITLEBAR_H)
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                show_titlebar(ui, &mut cache, &palette, &content, &mut Vec::new());
            });

        // Left chrome column: sidebar at SIDEBAR_W with representative
        // browser content (search + Library/Folders nav).
        let mut search = String::new();
        egui::Panel::left("sidebar")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .show_inside(ui, |ui| {
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
            .show_inside(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.weak("player bar");
                });
            });

        // Main stage: the active View's surface over the window background.
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.background))
            .show_inside(ui, |_| {});
    }

    // --- Sidebar baseline (Issue 07) ---------------------------------------------

    /// The restyled sidebar at its exact 280px token width: search box with
    /// focus-ring border, segmented Library/Folders control, 40px tree rows on
    /// the three-level indent scale, styled smart playlists ×4, and playlist
    /// rows. Rendered idle (no hover, nothing playing) so the snapshot is
    /// deterministic; the equalizer animation itself is covered headlessly in
    /// `ui_tests`.
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
        use riff::ui::icons::{Icon, IconCache};
        use riff::ui::sidebar::{self, SidebarNav, TreeRow};
        use riff::ui::theme::{Palette, SIDEBAR_W, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let mut cache = IconCache::new();

        egui::Panel::left("sidebar")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(12)))
            .show_inside(ui, |ui| {
                let mut query = String::from("beeth");
                sidebar::search_box(ui, &mut cache, &palette, &mut query);
                ui.add_space(10.0);

                sidebar::segmented_nav(ui, &palette, Some(SidebarNav::Library));
                ui.add_space(12.0);

                for (label, icon, selected) in [
                    ("All Tracks", Some(Icon::ListMusic), true),
                    ("Artists", Some(Icon::Library), false),
                ] {
                    sidebar::tree_row(
                        ui,
                        &mut cache,
                        &palette,
                        TreeRow {
                            indent_level: 0,
                            icon,
                            label,
                            selected,
                            now_playing: false,
                            playing: false,
                            disclosure: None,
                        },
                    );
                }
                ui.add_space(12.0);

                sidebar::section_header(ui, &palette, "Smart Playlists");
                for (i, name) in ["Recently Added", "Most Played", "Never Played", "Lost Gems"]
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
                            selected: i == 1,
                            now_playing: false,
                            playing: false,
                            disclosure: None,
                        },
                    );
                }
                ui.add_space(12.0);

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
                        selected: false,
                        now_playing: false,
                        playing: false,
                        disclosure: None,
                    },
                );
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
        use riff::ui::icons::IconCache;
        use riff::ui::playerbar::{self, PlayerBarContent};
        use riff::ui::theme::{Palette, SURFACE_BG};

        // Full-canvas background (determinism rule).
        let background = ui.ctx().layer_painter(egui::LayerId::background());
        background.rect_filled(ui.ctx().content_rect(), 0.0, SURFACE_BG);

        let palette = Palette::dark();
        let content = PlayerBarContent {
            cover: None,
            playback: riff::domain::PlaybackState::Playing,
            position: std::time::Duration::from_mins(2),
            total: Some(std::time::Duration::from_secs(245)),
            volume: 0.65,
            muted: false,
            shuffle: true,
            repeat: riff::domain::RepeatMode::None,
            queue_position: "3/12",
            advanced: false,
        };
        let mut cache = IconCache::new();
        playerbar::show_player_bar(
            ui,
            &mut cache,
            &palette,
            &content,
            &mut riff::ui::playerbar::SeekReadouts::default(),
            &mut Vec::new(),
        );
    }

    // --- Library stage baselines (Issue 09) ---------------------------------------

    /// The Library stage canvas at the minimum window: 520×456 = 800−280 wide
    /// × 600−56−88 high, so both goldens pin the hero and the track list at
    /// the smallest real estate they must fit without clipping.
    fn library_stage_size() -> egui::Vec2 {
        egui::vec2(
            riff::ui::chrome::MIN_WINDOW_SIZE.x - theme::SIDEBAR_W,
            riff::ui::chrome::MIN_WINDOW_SIZE.y - theme::TITLEBAR_H - theme::PLAYERBAR_H,
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
        use riff::ui::icons::IconCache;
        use riff::ui::library::empty_state_hero;
        use riff::ui::theme::{Palette, SURFACE_BG};

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
        use riff::ui::icons::IconCache;
        use riff::ui::sidebar::{self, TreeRow};
        use riff::ui::theme::{Palette, SURFACE_BG};

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
                riff::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .x
                    - theme::SIDEBAR_W,
                riff::ui::chrome::viewport_builder()
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
        use riff::ui::icons::IconCache;
        use riff::ui::now_playing::{self, NowPlayingContent, UpNextEntry};
        use riff::ui::theme::{Palette, SURFACE_BG};

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
                    id: riff::domain::TrackId("a.flac".to_owned()),
                    label: "The Midnight - Sunset".to_owned(),
                },
                UpNextEntry {
                    id: riff::domain::TrackId("b.flac".to_owned()),
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
            &mut riff::ui::playerbar::SeekReadouts::default(),
            &mut Vec::new(),
        );
    }

    // --- Settings stage baseline (Issue 11) -----------------------------------------

    /// The restyled Settings stage rendered at its full column height: the
    /// Back bar, the Music Libraries card with a per-path Readiness dot
    /// beside Scan / Watch / trash and the Add Library + Scan All row, the
    /// destructive ghost Clear Library action, the Preferences card driven by
    /// 36×20 toggle switches (Advanced mode checked, as in the mockup), and
    /// the muted info lines. The stage scrolls in the real app (the launch
    /// main stage is shorter than the full column), so this golden pins the
    /// ENTIRE column at the launch width instead of a scrolled slice.
    /// Rendered idle so the snapshot is deterministic.
    #[test]
    fn settings_dark_matches_golden_baseline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(
                riff::ui::chrome::viewport_builder()
                    .inner_size
                    .expect("launch size is configured")
                    .x
                    - theme::SIDEBAR_W,
                // Tall enough that the whole Settings column fits without
                // scrolling — every section must be visible in the baseline.
                840.0,
            ))
            .with_pixels_per_point(1.0)
            .build_ui(draw_settings_stage);
        theme::install(&harness.ctx, &Palette::dark());
        harness.ctx.set_fonts(inter_only_font_definitions());
        harness.run();
        harness.snapshot("settings_dark");
    }

    fn draw_settings_stage(ui: &mut egui::Ui) {
        use riff::app::state::{LibraryStatus, WatchState};
        use riff::ui::icons::IconCache;
        use riff::ui::settings::{self, LibraryRow, SettingsContent};
        use riff::ui::theme::{Palette, SURFACE_BG};

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
        };
        let mut cache = IconCache::new();
        settings::show_settings_stage(ui, &mut cache, &palette, &content);
    }
}
