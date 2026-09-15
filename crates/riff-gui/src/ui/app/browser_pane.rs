//! The elastic column stage (elastic-column spec): the Library view's
//! content area, rendered inside the `CentralPanel`. A sequence of side-by-side
//! list columns derived from the active section and the current drill-down
//! path, plus the collapsible inspector as the rightmost column when a
//! selection exists. Single-list stages (search, playlists, smart lists, the
//! folder tree, All Tracks) render one full-width column.
//!
//! Child module of `ui::app` so the pane methods keep direct access to
//! [`RiffApp`]'s fields, exactly like the methods they sit beside.

use eframe::egui;
use riff_backend::domain::{Artist, PlaylistId, SmartPlaylistKind, TrackId};
use std::path::PathBuf;
use std::sync::Arc;

use riff_backend::app::state::{
    BrowseMode, BrowserLayout, BrowserSelection, LibrarySection, LibrarySession, PlaybackSession,
};

use super::super::browser;
use super::super::theme;
use super::{
    ColumnKind, RiffApp, apply_browser_action, apply_detail_action, apply_drill_action,
    column_plan, column_widths, request_cover_intent, resolve_detail_content, resolve_inspector,
    smart_list_openable,
};

/// The stage's single-column states: stages that are not a LIBRARY section's
/// facet drill render exactly one full-width listing column. `Search` is the
/// flat search-results listing; the rest are the existing single renderers.
enum SingleStage {
    /// An active search query with results: the flat listing for the query.
    Search,
    /// An opened user playlist.
    UserPlaylist(PlaylistId),
    /// An opened read-only smart list.
    SmartPlaylist(SmartPlaylistKind),
    /// The Folders browse mode's folder tree.
    Folders,
}

impl RiffApp {
    /// The elastic column stage: the Library view's whole content area. The
    /// stage lays the section's list columns side by side — the number of
    /// columns follows the section and the drill-down path (the pure
    /// [`column_plan`]), each at its sized width ([`column_widths`]) with a
    /// thin hairline separator between — then the collapsible inspector as
    /// the rightmost column only while a selection exists. No horizontal
    /// scrolling: narrow windows shrink columns toward their floors.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a separator count is a small non-negative number"
    )]
    pub(super) fn render_elastic_stage(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
        // Single-stage gating, mirroring the old browser-pane dispatch:
        // an active search query takes over the whole stage (its results
        // are the flat listing), then an opened playlist or smart list,
        // then the Folders tree; the LIBRARY sections go through the
        // column plan.
        let query = library.search_query.clone();
        let has_results = query.is_empty() || self.views.search_has_matches(&query);
        if !has_results && !query.is_empty() {
            browser::empty_state(
                ui,
                &self.theme.active,
                "No tracks found",
                &format!("Nothing in your library matches '{query}'."),
            );
            return;
        }
        let single = if !query.is_empty() {
            Some(SingleStage::Search)
        } else if let Some(pid) = self.playlist_view.clone() {
            Some(SingleStage::UserPlaylist(pid))
        } else if let Some(kind) = self
            .smart_playlist_view
            .filter(|kind| smart_list_openable(*kind, library.ui_flags.advanced_mode))
        {
            Some(SingleStage::SmartPlaylist(kind))
        } else if library.browse_mode == BrowseMode::Folders {
            Some(SingleStage::Folders)
        } else {
            None
        };

        let plan: Vec<ColumnKind> = match &single {
            None => column_plan(library.library_section, &library.browser_path),
            Some(SingleStage::Search) => vec![ColumnKind::Flat],
            Some(_) => vec![ColumnKind::Single],
        };

        // The inspector follows the live selection: the selected track when
        // a track row was single-clicked (in any track listing), otherwise
        // the deepest path entity — and collapses away completely when
        // nothing is selected.
        let inspector = resolve_inspector(&mut self.views, library);

        let available = ui.available_width();
        // Each hairline separator between the columns consumes the style's
        // separator spacing (6 px) in the horizontal layout: subtract one
        // per gap so the sized columns end exactly at the stage's right
        // edge — otherwise the last column / the inspector would over-
        // allocate past it and be clipped by the panel's clip rect.
        let gaps = plan.len().saturating_sub(1) + usize::from(inspector.visible);
        let separator_w = ui
            .style()
            .separator_style(
                &egui::widget_style::Classes::default(),
                egui::widget_style::WidgetState::default(),
            )
            .spacing;
        let list_widths = column_widths(
            (available - separator_w * gaps as f32).max(0.0),
            plan.len(),
            inspector.visible,
        );

        // `horizontal_top` (not `horizontal`): the plain horizontal variant
        // sizes its row to `interact_size.y` and only grows with content,
        // which would collapse every stage column to one 18 px row.
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, kind) in plan.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                stage_column_scope(ui, list_widths[i], ("stage-column", i), |ui| {
                    self.render_stage_column(ui, library, playback, &query, single.as_ref(), *kind);
                });
            }
            if inspector.visible {
                ui.separator();
                stage_column_scope(ui, theme::INSPECTOR_WIDTH, "inspector", |ui| {
                    self.render_inspector(ui, library);
                });
            }
        });
    }

    /// Render one list column of the elastic stage inside its
    /// width-constrained child ui. Root columns keep the existing section
    /// renderers (A–Z sort, genre chips, list/grid toggle); the drill
    /// columns are `BrowserColumn`s over the seam's genre-scoped queries;
    /// the Tracks column is the existing `DetailColumn` shape.
    fn render_stage_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        query: &str,
        single: Option<&SingleStage>,
        kind: ColumnKind,
    ) {
        match kind {
            ColumnKind::Root => match library.library_section {
                LibrarySection::Artists => self.render_artists_browser(ui, library),
                LibrarySection::Albums => self.render_albums_browser(ui, library),
                LibrarySection::Genres => self.render_genres_browser(ui, library),
                LibrarySection::AllTracks => {
                    unreachable!("the All Tracks section plans a Flat column")
                }
            },
            ColumnKind::Flat => self.render_flat_view(ui, library, playback, query),
            ColumnKind::Single => match single {
                Some(SingleStage::UserPlaylist(pid)) => {
                    self.render_playlist_view(ui, library, playback, pid);
                }
                Some(SingleStage::SmartPlaylist(kind)) => {
                    self.render_smart_playlist_view(ui, library, playback, *kind);
                }
                Some(SingleStage::Folders) => self.render_folder_tree(ui, library, playback, query),
                _ => unreachable!("a Single column renders only for the single-list stages"),
            },
            ColumnKind::ArtistAlbums => {
                let Some(BrowserSelection::Artist(artist)) = library.browser_path.first() else {
                    return;
                };
                let artist = artist.clone();
                let albums = self.views.artist_albums(&artist);
                self.render_albums_drill_column(
                    ui,
                    library,
                    &albums,
                    1,
                    LibrarySection::Artists,
                    "No albums yet",
                    "This artist has no albums in your library.",
                );
            }
            ColumnKind::GenreArtists => {
                let Some(BrowserSelection::Genre(genre)) = library.browser_path.first() else {
                    return;
                };
                let genre = genre.clone();
                self.render_genre_artists_column(ui, library, &genre);
            }
            ColumnKind::GenreArtistAlbums => {
                let (Some(BrowserSelection::Genre(genre)), Some(BrowserSelection::Artist(artist))) =
                    (library.browser_path.first(), library.browser_path.get(1))
                else {
                    return;
                };
                let (genre, artist) = (genre.clone(), artist.clone());
                let albums = self.views.artist_albums_in_genre(&artist, &genre);
                self.render_albums_drill_column(
                    ui,
                    library,
                    &albums,
                    2,
                    LibrarySection::Genres,
                    "No albums in this genre",
                    "This artist has no albums carrying this genre.",
                );
            }
            ColumnKind::Tracks => self.render_tracks_column(ui, library, playback),
        }
    }

    /// One albums drill column (Artists level 1 and Genres level 2 share the
    /// same row shape): a `BrowserColumn` (always list, no sort, no genre
    /// chips) over `albums`, with the album row's cover thumbnail, `Artist ·
    /// Year` detail line, and `(album artist, title)` composite key.
    /// Highlighting reads `library.browser_path[level]`; a row selection
    /// lands at that level via [`apply_drill_action`].
    #[allow(clippy::too_many_arguments)]
    fn render_albums_drill_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        albums: &[riff_backend::domain::Album],
        level: usize,
        section: LibrarySection,
        empty_title: &str,
        empty_hint: &str,
    ) {
        let palette = self.theme.active;
        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let covers = &self.covers;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        let total = albums.len();
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let album = albums.get(i)?;
            // The album's cover, requested through its first track — the
            // same flow the root columns use; a full miss resolves the
            // music-icon placeholder tile.
            let thumbnail = album.tracks.first().map(|tid| {
                request_cover_intent(
                    textures.contains_key(&tid.0),
                    covers.as_ref(),
                    tid.clone(),
                    PathBuf::from(&tid.0),
                );
                crate::ui::cover_placeholder::lookup_cover_texture(
                    textures, lru_keys, &ctx, &palette, &tid.0,
                )
            });
            let detail = album.year.map_or_else(
                || album.artist.clone(),
                |y| format!("{} \u{b7} {y}", album.artist),
            );
            let selected = matches!(
                library.browser_path.get(level),
                Some(BrowserSelection::Album { artist, title })
                    if artist == &album.artist && title == &album.title
            );
            Some(browser::BrowserItem {
                key: format!("{}\u{1f}{}", album.artist, album.title),
                label: album.title.clone(),
                detail: Some(detail),
                thumbnail,
                selected,
                now_playing: false,
            })
        };
        let column = browser::BrowserColumn {
            layout: BrowserLayout::List,
            sort_desc: false,
            show_sort: false,
            genres: &[],
            genre_filter: None,
            total,
            item: &mut item,
            empty_title,
            empty_hint,
        };
        browser::show_browser_column(ui, &mut self.icons, &palette, column, &mut actions);
        for action in actions {
            match action {
                browser::BrowserAction::Select(key) => {
                    apply_drill_action(section, level, key, library);
                }
                browser::BrowserAction::ToggleSort | browser::BrowserAction::SetGenreFilter(_) => {}
            }
        }
    }

    /// The Genres section's artists-in-genre column (level 1): every artist
    /// carrying the genre, with their genre-scoped album count. Rows select
    /// the artist at level 1.
    fn render_genre_artists_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        genre: &str,
    ) {
        let palette = self.theme.active;
        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        let artists = views.artists_in_genre(genre);
        let total = artists.len();
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let artist = artists.get(i)?;
            // The artist's genre-scoped albums: the count line, and the
            // cover through the first one's first track.
            let albums = views.artist_albums_in_genre(&artist.name, genre);
            let detail = match albums.len() {
                1 => "1 album".to_string(),
                n => format!("{n} albums"),
            };
            let thumbnail = albums
                .first()
                .and_then(|album| album.tracks.first())
                .map(|tid| {
                    request_cover_intent(
                        textures.contains_key(&tid.0),
                        covers.as_ref(),
                        tid.clone(),
                        PathBuf::from(&tid.0),
                    );
                    crate::ui::cover_placeholder::lookup_cover_texture(
                        textures, lru_keys, &ctx, &palette, &tid.0,
                    )
                });
            let selected = matches!(
                library.browser_path.get(1),
                Some(BrowserSelection::Artist(name)) if name == &artist.name
            );
            Some(browser::BrowserItem {
                key: artist.name.clone(),
                label: artist.name.clone(),
                detail: Some(detail),
                thumbnail,
                selected,
                now_playing: false,
            })
        };
        let column = browser::BrowserColumn {
            layout: BrowserLayout::List,
            sort_desc: false,
            show_sort: false,
            genres: &[],
            genre_filter: None,
            total,
            item: &mut item,
            empty_title: "No artists in this genre",
            empty_hint: "This genre has no artists in your library.",
        };
        browser::show_browser_column(ui, &mut self.icons, &palette, column, &mut actions);
        for action in actions {
            match action {
                browser::BrowserAction::Select(key) => {
                    apply_drill_action(LibrarySection::Genres, 1, key, library);
                }
                browser::BrowserAction::ToggleSort | browser::BrowserAction::SetGenreFilter(_) => {}
            }
        }
    }

    /// The Tracks column (the stage's last column): the existing
    /// `DetailColumn` shape — breadcrumb trail, album header with Play all /
    /// Shuffle, and the album's track list. Entity listings are their own columns
    /// now, so the widget receives no rows.
    fn render_tracks_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
        let content = resolve_detail_content(&mut self.views, library);
        // The play batch the album header's actions start: the album's
        // tracks in store order (genre-scoped in the Genres path).
        let album_tracks: Vec<TrackId> = content
            .tracks
            .iter()
            .map(|row| TrackId(row.key.clone()))
            .collect();
        let mut actions = Vec::new();
        crate::ui::detail::show_detail_column(
            ui,
            &mut self.icons,
            &self.theme.active,
            crate::ui::detail::DetailColumn {
                breadcrumb: &content.breadcrumb,
                header: content.header.as_ref(),
                tracks: &content.tracks,
                rows: &[],
                empty_title: "Nothing here yet",
                empty_hint: "This selection has nothing to show.",
            },
            &mut actions,
        );
        for action in actions {
            apply_detail_action(
                action,
                library,
                playback,
                self.transport.as_ref(),
                self.library_mutations.as_mut(),
                &album_tracks,
            );
        }
    }

    /// The Artists variant (handoff issue 08): every artist as a row with a
    /// small cover thumbnail (open decision 3: the first album's cover) and
    /// the A–Z sort control above the list. Selecting a row drills into the
    /// artist at the root level — the path restarts from here.
    fn render_artists_browser(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        use riff_backend::app::state::BrowserSelection;

        let palette = self.theme.active;
        let genre = library.genre_filter.clone();
        let sort_desc = library.browser_sort_desc;

        let artists: Arc<[Artist]> = match &genre {
            Some(g) => self.views.artists_in_genre(g),
            None => self.views.artists(),
        };
        // artists() is name-ascending; the sort control flips the render
        // order only, the store keeps the canonical ordering. The album
        // count rides along so the row's detail line can match the Albums
        // browser's `Artist · Year` and Genres' `N tracks` shape.
        let mut rows: Vec<(&str, usize)> = artists
            .iter()
            .map(|a| (a.name.as_str(), a.albums.len()))
            .collect();
        if sort_desc {
            rows.reverse();
        }
        // The root column highlights the path's first entry: the artist the
        // listener is drilled into.
        let selected = match library.browser_path.first() {
            Some(BrowserSelection::Artist(name)) => Some(name.clone()),
            _ => None,
        };

        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let (name, album_count) = *rows.get(i)?;
            // The row's small cover thumbnail: the first album's cover
            // (open decision 3), requested through the first album's first
            // track — the Cover Service resolves by track path, the texture
            // comes from the UI LRU. A full miss resolves the generated
            // colour block (issue 14) through the same cache. Repeat reads
            // hit the projection cache. The detail line shows the album
            // count, matching the shape Albums and Genres rows already use.
            let detail = match album_count {
                1 => "1 album".to_string(),
                n => format!("{n} albums"),
            };
            let albums = views.artist_albums(name);
            let thumbnail = albums
                .first()
                .and_then(|album| album.tracks.first())
                .map(|tid| {
                    request_cover_intent(
                        textures.contains_key(&tid.0),
                        covers.as_ref(),
                        tid.clone(),
                        PathBuf::from(&tid.0),
                    );
                    crate::ui::cover_placeholder::lookup_cover_texture(
                        textures, lru_keys, &ctx, &palette, &tid.0,
                    )
                });
            Some(browser::BrowserItem {
                key: name.to_owned(),
                label: name.to_owned(),
                detail: Some(detail),
                thumbnail,
                selected: selected.as_deref() == Some(name),
                now_playing: false,
            })
        };
        let (empty_title, empty_hint) = if genre.is_some() {
            (
                "No artists in this genre",
                "Clear the genre filter above to see every artist.",
            )
        } else {
            (
                "No artists yet",
                "Add a folder from the sidebar to start scanning your library.",
            )
        };
        let column = browser::BrowserColumn {
            layout: library.browser_layout,
            sort_desc,
            show_sort: true,
            genres: &[],
            genre_filter: genre.as_deref(),
            total: rows.len(),
            item: &mut item,
            empty_title,
            empty_hint,
        };
        browser::show_browser_column(ui, &mut self.icons, &palette, column, &mut actions);
        for action in actions {
            apply_browser_action(action, library);
        }
    }

    /// The Albums variant (handoff issue 08): every album in the library as
    /// a row with its artist and year plus its first track's cover. The flat
    /// listing derives from the per-artist album tables via
    /// [`browser::flat_slot`] — no whole-library album query exists, and the
    /// prefix-sum table means only the visible slots' artists are fetched.
    fn render_albums_browser(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        use riff_backend::app::state::BrowserSelection;

        let palette = self.theme.active;
        let genre = library.genre_filter.clone();
        let sort_desc = library.browser_sort_desc;

        let artists: Arc<[Artist]> = match &genre {
            Some(g) => self.views.artists_in_genre(g),
            None => self.views.artists(),
        };
        // Prefix-sum table over per-artist album counts (the genre-filtered
        // counts come from the filtered projection); flat_slot maps the
        // listing index through, flipping to Z–A when the sort is reversed.
        let mut counts: Vec<usize> = Vec::with_capacity(artists.len() + 1);
        counts.push(0);
        for artist in artists.iter() {
            let n = match &genre {
                Some(g) => self.views.artist_albums_in_genre(&artist.name, g).len(),
                None => artist.albums.len(),
            };
            let last = counts.last().copied().unwrap_or(0);
            counts.push(last + n);
        }
        let total = counts.last().copied().unwrap_or(0);
        // The root column highlights the path's first entry: the album the
        // listener is drilled into.
        let selected = match library.browser_path.first() {
            Some(BrowserSelection::Album { artist, title }) => {
                Some((artist.clone(), title.clone()))
            }
            _ => None,
        };

        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let (ai, slot) = browser::flat_slot(&counts, i, sort_desc)?;
            let artist = artists.get(ai)?;
            let albums = match &genre {
                Some(g) => views.artist_albums_in_genre(&artist.name, g),
                None => views.artist_albums(&artist.name),
            };
            let album = albums.get(slot)?;
            // The album's cover, requested through its first track — the
            // same flow the artist rows and track listings use; a full miss
            // resolves the music-icon placeholder tile.
            let thumbnail = album
                .tracks
                .first()
                .map(|tid| {
                    request_cover_intent(
                        textures.contains_key(&tid.0),
                        covers.as_ref(),
                        tid.clone(),
                        PathBuf::from(&tid.0),
                    );
                    crate::ui::cover_placeholder::lookup_cover_texture(
                        textures, lru_keys, &ctx, &palette, &tid.0,
                    )
                    .into()
                })
                .unwrap_or_default();
            let detail = album.year.map_or_else(
                || album.artist.clone(),
                |y| format!("{} \u{b7} {y}", album.artist),
            );
            Some(browser::BrowserItem {
                key: format!("{}\u{1f}{}", album.artist, album.title),
                label: album.title.clone(),
                detail: Some(detail),
                thumbnail,
                selected: selected
                    .as_ref()
                    .is_some_and(|(a, t)| *a == album.artist && *t == album.title),
                now_playing: false,
            })
        };
        let (empty_title, empty_hint) = if genre.is_some() {
            (
                "No albums in this genre",
                "Clear the genre filter above to see every album.",
            )
        } else {
            (
                "No albums yet",
                "Add a folder from the sidebar to start scanning your library.",
            )
        };
        let column = browser::BrowserColumn {
            layout: library.browser_layout,
            sort_desc,
            show_sort: true,
            genres: &[],
            genre_filter: genre.as_deref(),
            total,
            item: &mut item,
            empty_title,
            empty_hint,
        };
        browser::show_browser_column(ui, &mut self.icons, &palette, column, &mut actions);
        for action in actions {
            apply_browser_action(action, library);
        }
    }

    /// The Genres variant (handoff issue 08): every genre with its track
    /// count from the genre read model (handoff issue 02), ordered A–Z by
    /// the sort control. Selecting a genre drills into it at the root level.
    fn render_genres_browser(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        use riff_backend::app::state::BrowserSelection;

        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let genres = self.views.genres();
        let total = genres.len();
        // The root column highlights the path's first entry: the genre the
        // listener is drilled into.
        let selected = match library.browser_path.first() {
            Some(BrowserSelection::Genre(genre)) => Some(genre.clone()),
            _ => None,
        };

        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let genre = genres.get(if sort_desc { total - 1 - i } else { i })?;
            Some(browser::BrowserItem {
                key: genre.genre.clone(),
                label: genre.genre.clone(),
                detail: Some(format!("{} tracks", genre.tracks)),
                thumbnail: None,
                selected: selected.as_deref() == Some(genre.genre.as_str()),
                now_playing: false,
            })
        };
        let column = browser::BrowserColumn {
            layout: library.browser_layout,
            sort_desc,
            show_sort: true,
            genres: &[],
            genre_filter: None,
            total,
            item: &mut item,
            empty_title: "No genres yet",
            empty_hint: "Genres come from your tracks' tags \u{2014} add music and rescan.",
        };
        browser::show_browser_column(ui, &mut self.icons, &palette, column, &mut actions);
        for action in actions {
            apply_browser_action(action, library);
        }
    }

    /// The flat track listing's grid mode (handoff issue 08): the same
    /// paged tracks the list shows, as cover tiles. Tiles select the track
    /// (the double-click-to-play gesture stays a list-mode gesture until
    /// the detail column provides the album header's Play all / Shuffle).
    pub(super) fn render_flat_grid(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        query: &str,
        current_track: Option<&TrackId>,
    ) {
        let palette = self.theme.active;
        let selected = library.selected_track.clone();

        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        let first_page = views.track_list(query, 0);
        let total = first_page.total;
        let mut page: Option<riff_backend::app::views::TrackListPage> = Some(first_page);
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            // Refetch only when the row leaves the page in hand; the
            // seam serves repeat windows from cache.
            if page.as_ref().is_none_or(|p| p.start + p.rows.len() <= i) {
                page = Some(views.track_list(query, i));
            }
            let p = page.as_ref()?;
            let track = p.rows.get(i - p.start)?;
            request_cover_intent(
                textures.contains_key(&track.id.0),
                covers.as_ref(),
                track.id.clone(),
                track.file_path.clone(),
            );
            // A full miss resolves the music-icon placeholder tile.
            let thumbnail = Some(crate::ui::cover_placeholder::lookup_cover_texture(
                textures,
                lru_keys,
                &ctx,
                &palette,
                &track.id.0,
            ));
            Some(browser::BrowserItem {
                key: track.id.0.clone(),
                label: track.metadata.display_title(&track.file_path),
                detail: Some(track.metadata.display_artist()),
                thumbnail,
                selected: selected.as_ref() == Some(&track.id),
                now_playing: current_track == Some(&track.id),
            })
        };
        let column = browser::BrowserColumn {
            layout: riff_backend::app::state::BrowserLayout::Grid,
            sort_desc: false,
            show_sort: false,
            genres: &[],
            genre_filter: None,
            total,
            item: &mut item,
            empty_title: "No tracks yet",
            empty_hint: "Add a folder from the sidebar to start scanning your library.",
        };
        browser::show_browser_column(ui, &mut self.icons, &palette, column, &mut actions);
        for action in actions {
            match action {
                browser::BrowserAction::Select(key) => {
                    library.selected_track = Some(TrackId(key));
                }
                other => apply_browser_action(other, library),
            }
        }
    }
}

/// Allocate a width-constrained child ui for one stage column. The explicit
/// id salt gives every column a distinct *stable* id: sibling child uis made
/// through [`egui::Ui::allocate_ui_with_layout`] all share the parent's
/// `"child"` salt, so persistent-id widgets inside them — each column's
/// `ScrollArea`, which ids itself via [`egui::Ui::make_persistent_id`] —
/// would collide, sharing scroll state between columns and drawing egui's
/// id-collision overlays.
fn stage_column_scope(
    ui: &mut egui::Ui,
    width: f32,
    salt: impl egui::AsIdSalt,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, ui.available_height()),
        egui::Sense::hover(),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min))
            .id_salt(salt),
        add_contents,
    );
}
