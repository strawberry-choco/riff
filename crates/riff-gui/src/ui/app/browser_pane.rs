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
use riff_backend::domain::{Album, Artist, GenreCount, PlaylistId, SmartPlaylistKind, TrackId};
use std::path::PathBuf;

use riff_backend::app::state::{
    BrowseMode, BrowserSelection, LibrarySection, LibrarySession, PlaybackSession,
};

use super::super::browser;
use super::{
    COVER_THUMB, CollectionMenuEffects, ColumnKind, RiffApp, apply_browser_action,
    apply_collection_menu, apply_detail_action, apply_drill_action, column_plan,
    request_cover_intent, resolve_detail_content, resolve_inspector, smart_list_openable,
};

/// The stage's single-column states: stages that are not a LIBRARY section's
/// facet drill render exactly one full-width listing column. The user
/// playlist, smart list, and Folders views are the existing single
/// renderers; an active search query is no longer one of them (issue 04) —
/// it filters the open section's own columns instead.
enum SingleStage {
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
    pub(super) fn render_elastic_stage(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
        // Single-stage gating: an opened playlist or smart list, then the
        // Folders tree. The single-list search stage is gone (issue 04): a
        // query no longer takes over the stage — each LIBRARY section's own
        // columns filter to hits instead, and the Folders tree keeps its
        // pruned shape under a query.
        let query = library.search_query.clone();
        let single = if let Some(pid) = self.playlist_view.clone() {
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
            Some(_) => vec![ColumnKind::Single],
        };

        // The inspector follows the live selection: the selected track when
        // a track row was single-clicked (in any track listing), otherwise
        // the deepest path entity — and collapses away completely when
        // nothing is selected.
        let inspector = resolve_inspector(&mut self.views, library);
        let inspector_visible = inspector.visible;

        // The stage's width allocation, separator accounting, zero-gap
        // composition, stable child identities, and inspector placement are all
        // owned by the shared geometry seam — the golden harness calls the very
        // same helper, so production and goldens cannot disagree on a column
        // edge. This call site supplies only the column content.
        super::super::stage::show_elastic_stage(ui, plan.len(), inspector_visible, |ui, slot| {
            match slot {
                super::super::stage::StageSlot::Column(i) => {
                    self.render_stage_column(
                        ui,
                        library,
                        playback,
                        &query,
                        single.as_ref(),
                        plan[i],
                    );
                }
                super::super::stage::StageSlot::Inspector => {
                    self.render_inspector(ui, library);
                }
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
                LibrarySection::Artists => {
                    self.render_artists_browser(ui, library, playback, query);
                }
                LibrarySection::Albums => {
                    self.render_albums_browser(ui, library, playback, query);
                }
                LibrarySection::Genres => {
                    self.render_genres_browser(ui, library, playback, query);
                }
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
                let albums = self.artist_drill_albums(&artist, query);
                let (empty_title, empty_hint): (&str, String) = if query.is_empty() {
                    (
                        "No albums yet",
                        "This artist has no albums in your library.".to_string(),
                    )
                } else {
                    (
                        "No matching albums",
                        format!("Nothing in this view matches '{query}'."),
                    )
                };
                self.render_albums_drill_column(
                    ui,
                    library,
                    playback,
                    &albums,
                    1,
                    LibrarySection::Artists,
                    empty_title,
                    &empty_hint,
                );
            }
            ColumnKind::GenreArtists => {
                let Some(BrowserSelection::Genre(genre)) = library.browser_path.first() else {
                    return;
                };
                let genre = genre.clone();
                self.render_genre_artists_column(ui, library, playback, &genre);
            }
            ColumnKind::GenreArtistAlbums => {
                let (Some(BrowserSelection::Genre(genre)), Some(BrowserSelection::Artist(artist))) =
                    (library.browser_path.first(), library.browser_path.get(1))
                else {
                    return;
                };
                let (genre, artist) = (genre.clone(), artist.clone());
                self.render_genre_album_drill_column(ui, library, playback, &artist, &genre);
            }
            ColumnKind::Tracks => self.render_tracks_column(ui, library, query),
        }
    }

    /// The artist drill's album listing under the current query (issue 04):
    /// a name-hit artist drills to *all* its albums (the downward name-hit
    /// expansion); a track-hit artist's hit albums show — the albums that
    /// are themselves name-hits or carry a matching track. With no query the
    /// full album table serves unchanged.
    fn artist_drill_albums(&mut self, artist: &str, query: &str) -> Vec<Album> {
        let all: Vec<Album> = self.views.artist_albums(artist).to_vec();
        if query.is_empty() || artist_name_hits(artist, query) {
            return all;
        }
        // Track-hit artist: every album that is itself a hit (its own name
        // matched, or a member track matched) shows; the others stay out.
        all.into_iter()
            .filter(|album| {
                self.views
                    .album_is_name_hit(&album.artist, &album.title, query)
                    || !self
                        .views
                        .album_hit_tracks(&album.artist, &album.title, query)
                        .is_empty()
            })
            .collect()
    }

    /// One albums drill column (Artists level 1 and Genres level 2 share the
    /// same row shape): a `BrowserColumn` (always list, no sort, no genre
    /// chips) over `albums`, with the album row's cover thumbnail, `Artist ·
    /// Year` detail line, and `(album artist, title)` composite key.
    /// Highlighting reads `library.browser_path[level]`; a row selection
    /// lands at that level via [`apply_drill_action`], and so does a
    /// right-click on the row — via [`apply_collection_menu`], which also acts
    /// on the album. `playback` travels with them because the Shuffle item
    /// completes in the playback session's queue.
    #[allow(clippy::too_many_arguments)]
    fn render_albums_drill_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
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
        let in_flight = &mut self.cover_in_flight;
        let in_flight_keys = &mut self.cover_in_flight_keys;
        let ctx = ui.ctx().clone();
        let total = albums.len();
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let album = albums.get(i)?;
            // The album's cover, requested through its first track — the
            // same flow the root columns use; a full miss resolves the
            // music-icon placeholder tile.
            let thumbnail = album.tracks.first().map(|tid| {
                request_cover_intent(
                    textures,
                    in_flight,
                    in_flight_keys,
                    covers.as_ref(),
                    tid.clone(),
                    PathBuf::from(&tid.0),
                    COVER_THUMB,
                );
                crate::ui::artwork::lookup_cover_texture(
                    textures,
                    lru_keys,
                    &ctx,
                    &palette,
                    &tid.0,
                    COVER_THUMB,
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
            sort_desc: false,
            show_sort: false,
            total,
            item: &mut item,
            virtualize: false,
            empty_title,
            empty_hint,
        };
        // A drill column resets to the top on every selection change — the
        // Scroll Memory holds no drill offsets by design, and it is the module
        // that knows this slot's selections are drill bookkeeping. Between
        // changes its scroll is egui's natural state under the drill's own
        // stable salt.
        let control = self
            .scroll_memory
            .begin_drill(crate::ui::scroll_memory::DrillSlot::ArtistAlbums);
        let _actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            column,
            Some(control),
            &mut actions,
        );
        for action in actions {
            // One predicate answers "did the selection move?", for a click and
            // for a right-click alike — see `BrowserAction::selects_a_row`. It
            // is asked HERE, above the match, so no arm can be added that
            // selects without also resetting the drill's scroll.
            if action.selects_a_row() {
                self.scroll_memory
                    .note_selection_in(crate::ui::scroll_memory::ListSlot::Drill(
                        crate::ui::scroll_memory::DrillSlot::ArtistAlbums,
                    ));
            }
            match action {
                browser::BrowserAction::Select(key) => {
                    apply_drill_action(section, level, key, library);
                }
                browser::BrowserAction::ContextMenu { key, intents } => {
                    apply_collection_menu(
                        &key,
                        &intents,
                        section,
                        level,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                browser::BrowserAction::ToggleSort => {}
            }
        }
    }

    /// The Genres section's artists-in-genre column (level 1): every artist
    /// carrying the genre, with their genre-scoped album count. Rows select
    /// the artist at level 1, and a right-click on one both selects it and
    /// acts on it. Renders one window in hand (issue 05).
    #[expect(
        clippy::too_many_lines,
        reason = "one paged genre-artists listing with its right-click dispatch"
    )]
    fn render_genre_artists_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        genre: &str,
    ) {
        use riff_backend::app::store::SortDirection;

        let palette = self.theme.active;
        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let in_flight = &mut self.cover_in_flight;
        let in_flight_keys = &mut self.cover_in_flight_keys;
        let ctx = ui.ctx().clone();
        // One window in hand (paginate-browse-columns issue 05): opening a
        // large genre reads only the visible artists, so a common genre like
        // "Rock" no longer loads every artist that touches it at once.
        let total = views
            .artists_in_genre_page(genre, SortDirection::Ascending, 0)
            .total;
        let mut browse_page: Option<riff_backend::app::views::HitPage<Artist>> = None;
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            if browse_page
                .as_ref()
                .is_none_or(|p| p.start + p.rows.len() <= i)
            {
                browse_page = Some(views.artists_in_genre_page(genre, SortDirection::Ascending, i));
            }
            let page = browse_page.as_ref()?;
            let artist = page.rows.get(i - page.start)?;
            // The artist's genre-scoped albums: the count line, and the
            // cover through the first one's first track. This per-artist
            // fetch stays full — it only ever runs for rows on screen.
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
                        textures,
                        in_flight,
                        in_flight_keys,
                        covers.as_ref(),
                        tid.clone(),
                        PathBuf::from(&tid.0),
                        COVER_THUMB,
                    );
                    crate::ui::artwork::lookup_cover_texture(
                        textures,
                        lru_keys,
                        &ctx,
                        &palette,
                        &tid.0,
                        COVER_THUMB,
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
            sort_desc: false,
            show_sort: false,
            total,
            item: &mut item,
            virtualize: false,
            empty_title: "No artists in this genre",
            empty_hint: "This genre has no artists in your library.",
        };
        // Drill column: reset to the top on every selection change (no
        // per-selection memory); egui's natural state between changes.
        let control = self
            .scroll_memory
            .begin_drill(crate::ui::scroll_memory::DrillSlot::GenreArtists);
        let _actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            column,
            Some(control),
            &mut actions,
        );
        for action in actions {
            if action.selects_a_row() {
                self.scroll_memory
                    .note_selection_in(crate::ui::scroll_memory::ListSlot::Drill(
                        crate::ui::scroll_memory::DrillSlot::GenreArtists,
                    ));
            }
            match action {
                browser::BrowserAction::Select(key) => {
                    apply_drill_action(LibrarySection::Genres, 1, key, library);
                }
                browser::BrowserAction::ContextMenu { key, intents } => {
                    apply_collection_menu(
                        &key,
                        &intents,
                        LibrarySection::Genres,
                        1,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                browser::BrowserAction::ToggleSort => {}
            }
        }
    }

    /// The Genres section's artist-albums drill column (level 2): one artist's
    /// albums holding the genre, each carrying its matching track ids, in
    /// canonical browsing order — one window in hand (issue 05), so drilling
    /// deeper reads only the visible albums. The row shape matches the
    /// shared album drill column: cover thumbnail, "Artist · Year" detail,
    /// `(album artist, title)` composite key; a row selection lands at level
    /// 2 via [`apply_drill_action`], and a right-click lands there too.
    #[expect(
        clippy::too_many_lines,
        reason = "one paged genre-albums listing with its right-click dispatch"
    )]
    fn render_genre_album_drill_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        artist: &str,
        genre: &str,
    ) {
        use riff_backend::app::store::SortDirection;

        let palette = self.theme.active;
        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let in_flight = &mut self.cover_in_flight;
        let in_flight_keys = &mut self.cover_in_flight_keys;
        let ctx = ui.ctx().clone();
        let total = views
            .artist_albums_in_genre_page(artist, genre, SortDirection::Ascending, 0)
            .total;
        let mut browse_page: Option<riff_backend::app::views::HitPage<Album>> = None;
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            if browse_page
                .as_ref()
                .is_none_or(|p| p.start + p.rows.len() <= i)
            {
                browse_page = Some(views.artist_albums_in_genre_page(
                    artist,
                    genre,
                    SortDirection::Ascending,
                    i,
                ));
            }
            let page = browse_page.as_ref()?;
            let album = page.rows.get(i - page.start)?;
            // The album's cover, requested through its first track — the
            // same flow the root columns use; a full miss resolves the
            // music-icon placeholder tile.
            let thumbnail = album.tracks.first().map(|tid| {
                request_cover_intent(
                    textures,
                    in_flight,
                    in_flight_keys,
                    covers.as_ref(),
                    tid.clone(),
                    PathBuf::from(&tid.0),
                    COVER_THUMB,
                );
                crate::ui::artwork::lookup_cover_texture(
                    textures,
                    lru_keys,
                    &ctx,
                    &palette,
                    &tid.0,
                    COVER_THUMB,
                )
            });
            let detail = album.year.map_or_else(
                || album.artist.clone(),
                |y| format!("{} \u{b7} {y}", album.artist),
            );
            let selected = matches!(
                library.browser_path.get(2),
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
            sort_desc: false,
            show_sort: false,
            total,
            item: &mut item,
            virtualize: false,
            empty_title: "No albums in this genre",
            empty_hint: "This artist has no albums carrying this genre.",
        };
        // Drill column: reset to the top on every selection change (no
        // per-selection memory); egui's natural state between changes.
        let control = self
            .scroll_memory
            .begin_drill(crate::ui::scroll_memory::DrillSlot::GenreArtistAlbums);
        let _actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            column,
            Some(control),
            &mut actions,
        );
        for action in actions {
            if action.selects_a_row() {
                self.scroll_memory
                    .note_selection_in(crate::ui::scroll_memory::ListSlot::Drill(
                        crate::ui::scroll_memory::DrillSlot::GenreArtistAlbums,
                    ));
            }
            match action {
                browser::BrowserAction::Select(key) => {
                    apply_drill_action(LibrarySection::Genres, 2, key, library);
                }
                browser::BrowserAction::ContextMenu { key, intents } => {
                    apply_collection_menu(
                        &key,
                        &intents,
                        LibrarySection::Genres,
                        2,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                browser::BrowserAction::ToggleSort => {}
            }
        }
    }

    /// The Tracks column (the stage's last column): the existing
    /// `DetailColumn` shape, a bare list of the selected album's tracks. It
    /// opens with no readout of its own — the breadcrumb trail and the album
    /// header it used to carry are gone — so which album the list belongs to
    /// is read from the albums column's selected row and from the inspector,
    /// not from anything painted in here. Entity listings are their own
    /// columns now, so the widget receives no rows.
    ///
    /// The TRACKS are not readouts: each row carries the shared Track menu, so
    /// a Track's actions do not depend on which Column happens to be showing
    /// it. The column resolves what the menu needs from the FRAME once — the
    /// playlist targets especially, because the menu is PAINTED from them before
    /// anything can be chosen from it — and hands the widget a per-row factory
    /// rather than one finished menu, because one of the menu's props is a fact
    /// about a single row: the widget reports the outcome as a
    /// [`crate::ui::detail::DetailAction`], and the two appliers every other
    /// Track row reaches are called below.
    ///
    /// # The accepted rough edge, in the host's words
    ///
    /// Those appliers include [`apply_track_menu_open`](super::apply_track_menu_open),
    /// so opening one of these menus makes that Track the selection — the same
    /// report a flat-list Track row's menu makes. In this column that has one
    /// visible consequence, recorded here so the next reader learns it from the
    /// code rather than reporting it as a regression:
    ///
    /// **Right-clicking a Track in this column selects it, which switches the
    /// Detail Panel from the Album readout to the Track readout, so the Album's
    /// Tag Aggregation and Batch Tag Edit are hidden until the Album is
    /// selected again.** It follows directly from right-click selecting, it is
    /// recoverable (the Album is selected again from its row in the Albums
    /// column), and the spec accepts it rather than working around it.
    fn render_tracks_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        query: &str,
    ) {
        let content = resolve_detail_content(&mut self.views, library, query);
        let (empty_title, empty_hint): (&str, String) = if query.is_empty() {
            (
                "Nothing here yet",
                "This selection has nothing to show.".to_string(),
            )
        } else {
            (
                "No matching tracks",
                format!("Nothing in this view matches '{query}'."),
            )
        };
        // The playlist list is resolved UP FRONT, before the widget renders:
        // the menu is painted from it on the frame it opens, which is the same
        // frame the listener may choose from it. A Track in this column is in
        // no Playlist, so the menu offers the ADD targets and no removal.
        let playlists = self.views.playlists();
        let options: Vec<(PlaylistId, String)> = playlists
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        // `playable` and `editable` are true of every row here by
        // construction, and that is not an optimistic guess: a row only exists
        // because the store resolved that Track for this album's listing, and
        // the column has always offered its rows unconditionally — a
        // double-click has always started one without asking whether the file
        // is still there. The reduced menu stays the playlist entry's
        // property, which is the one surface that flags a vanished file.
        //
        // The menu is a FACT ABOUT A ROW, though, not about the column, and the
        // one prop that says so is `favorite`: an album's Tracks are Favourited
        // independently, so a single menu value for the whole column would put
        // the same Favourite wording on every row and be wrong on all but one of
        // them. So the widget is handed a factory over the row and the host
        // still owns every flag — it is the same `BrowserColumn::item` shape a
        // virtualized listing already uses to hand out per-row data.
        let track_menu = |row: &crate::ui::detail::TrackRow| crate::ui::menu::TrackMenu {
            playable: true,
            editable: true,
            favorite: row.favorite,
            playlists: &options,
            remove_from_playlist: false,
        };
        let mut actions = Vec::new();
        // The Tracks column resets to the top whenever the selection feeding
        // it changes (a new album or artist selected anywhere in the browser);
        // it never remembers a position (issue 05). Its own rows select Tracks,
        // which the module knows is not drill bookkeeping.
        let control = self
            .scroll_memory
            .begin_drill(crate::ui::scroll_memory::DrillSlot::TracksColumn);
        crate::ui::detail::show_detail_column_scrolled(
            ui,
            &mut self.icons,
            &self.theme.active,
            crate::ui::detail::DetailColumn {
                tracks: &content.tracks,
                rows: &[],
                track_menu: Some(&track_menu),
                empty_title,
                empty_hint: &empty_hint,
            },
            Some(control),
            &mut actions,
        );
        for action in actions {
            // A Track menu is NOT a second dispatch: it is reported here and
            // answered by the SAME two appliers every other Track row reaches,
            // through the same effects bag. Routing it through
            // `apply_detail_action` instead would be the fork this column
            // must not have — two surfaces that answer a right-click
            // "similarly" rather than identically.
            match action {
                crate::ui::detail::DetailAction::TrackMenu { key, intents } => {
                    let track_id = TrackId(key);
                    // OPENING is what selects, whether or not anything was
                    // chosen from the menu.
                    super::apply_track_menu_open(
                        &track_id,
                        super::TrackMenuOpen::Opened,
                        &mut library.selected_track,
                    );
                    if intents.is_empty() {
                        continue;
                    }
                    // The Track itself, resolved at dispatch time through the
                    // same seam the Detail Panel's Track readout uses; it is
                    // what "Edit Tags" needs to open its draft, and its absence
                    // is what reduces that one item.
                    let track = self.views.selected_track(&track_id);
                    let mut effects = super::TrackMenuEffects {
                        track_id: &track_id,
                        track: track.as_ref(),
                        selected_track: &mut library.selected_track,
                        tag_editor: &mut self.tag_editor,
                        transport: self.transport.as_ref(),
                        playlist_store: self.playlist_store.as_mut(),
                        library_mutations: self.library_mutations.as_mut(),
                        remove_from_playlist: None,
                    };
                    for intent in intents {
                        super::apply_track_menu_intent(intent, &mut effects);
                    }
                }
                action => apply_detail_action(
                    action,
                    library,
                    self.transport.as_ref(),
                    self.library_mutations.as_mut(),
                ),
            }
        }
    }

    /// The Artists variant (handoff issue 08): every artist as a row with a
    /// small cover thumbnail (open decision 3: the first album's cover) and
    /// the A–Z sort control above the list. Selecting a row drills into the
    /// artist at the root level — the path restarts from here.
    ///
    /// Under a query the root lists only hit artists in canonical hit order
    /// (the A–Z sort control is hidden), each row carrying its hit-album
    /// count. The genre chip filter is not part of this column (issue 04
    /// keeps search display-independent): no genre chips render, and the
    /// listing is never genre-filtered.
    #[allow(clippy::too_many_lines, reason = "one paged artists-browser listing")]
    fn render_artists_browser(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        query: &str,
    ) {
        use riff_backend::app::state::BrowserSelection;
        use riff_backend::app::store::SortDirection;

        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let hit = !query.is_empty();
        // The direction is part of the paged read's query signature: the
        // store applies it in SQL so page offsets stay aligned when the sort
        // reverses (no in-memory reversal of an ascending copy).
        let direction = if sort_desc {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        };
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
        let in_flight = &mut self.cover_in_flight;
        let in_flight_keys = &mut self.cover_in_flight_keys;
        let ctx = ui.ctx().clone();
        // Anchor read: sizes the row range with the authoritative total
        // (computed before the closure borrows `views` mutably).
        let total = if hit {
            views.hit_artists_page(query, 0).total
        } else {
            views.artists_page(direction, 0).total
        };
        // One page cached in hand per frame, refetched only when the row
        // walks out of it (the flat-grid pattern) — the whole artist list
        // never materializes on first view.
        let mut hit_page: Option<riff_backend::app::views::HitPage<Artist>> = None;
        let mut browse_page: Option<riff_backend::app::views::HitPage<Artist>> = None;
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let (name, album_count) = if hit {
                if hit_page
                    .as_ref()
                    .is_none_or(|p| p.start + p.rows.len() <= i)
                {
                    hit_page = Some(views.hit_artists_page(query, i));
                }
                let page = hit_page.as_ref()?;
                let artist = page.rows.get(i - page.start)?;
                (artist.name.clone(), artist.albums.len())
            } else {
                if browse_page
                    .as_ref()
                    .is_none_or(|p| p.start + p.rows.len() <= i)
                {
                    browse_page = Some(views.artists_page(direction, i));
                }
                let page = browse_page.as_ref()?;
                let artist = page.rows.get(i - page.start)?;
                (artist.name.clone(), artist.albums.len())
            };
            // The row's small cover thumbnail: the first album's first
            // track (open decision 3), resolved through the seam's per-artist
            // first-track cache — never a per-row album read. The Cover
            // Service resolves by track path, the texture comes from the UI
            // LRU. A full miss resolves the generated colour block (issue 14)
            // through the same cache.
            let detail = match album_count {
                1 => "1 album".to_string(),
                n => format!("{n} albums"),
            };
            let thumbnail = views.artist_first_track(&name).map(|tid| {
                request_cover_intent(
                    textures,
                    in_flight,
                    in_flight_keys,
                    covers.as_ref(),
                    tid.clone(),
                    PathBuf::from(&tid.0),
                    COVER_THUMB,
                );
                crate::ui::artwork::lookup_cover_texture(
                    textures,
                    lru_keys,
                    &ctx,
                    &palette,
                    &tid.0,
                    COVER_THUMB,
                )
            });
            Some(browser::BrowserItem {
                key: name.clone(),
                label: name.clone(),
                detail: Some(detail),
                thumbnail,
                selected: selected.as_deref() == Some(name.as_str()),
                now_playing: false,
            })
        };
        let (empty_title, empty_hint) = Self::artists_empty_state(query, hit);
        let column = browser::BrowserColumn {
            sort_desc,
            show_sort: !hit,
            total,
            item: &mut item,
            // Virtualized (the idle-CPU fix): the walker reserves default
            // slots for rows above the viewport, so the provider — and with
            // it the per-row paged reads and cover intents — only serves the
            // on-screen window.
            virtualize: true,
            empty_title,
            empty_hint: &empty_hint,
        };
        // Section root list: the site declares WHERE it renders and passes its
        // content; the Scroll Memory composes the identity and picks the shape
        // — saved offset when it matches, else a reset (issue 02).
        let (control, visit) = self.scroll_memory.begin_section(
            riff_backend::app::state::LibrarySection::Artists,
            query,
            sort_desc,
        );
        let actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            column,
            Some(control),
            &mut actions,
        );
        self.scroll_memory.end_section(visit, actual);
        for action in actions {
            // THE GUARD. `selects_a_row` is the one predicate that answers
            // "did the selection move?", and it is asked above the match so
            // that adding an arm which selects cannot leave the scroll stale:
            // a right-click that moved the selection but not the scroll would
            // leave the list pointing at the wrong place. Answered identically
            // by all six entity Columns.
            if action.selects_a_row() {
                self.scroll_memory
                    .note_selection_in(crate::ui::scroll_memory::ListSlot::Section(
                        riff_backend::app::state::LibrarySection::Artists,
                    ));
            }
            match action {
                browser::BrowserAction::ContextMenu { key, intents } => {
                    let section = library.library_section;
                    apply_collection_menu(
                        &key,
                        &intents,
                        section,
                        0,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                other => apply_browser_action(other, library),
            }
        }
    }

    /// The artists column's empty state copy: the query hit-set's message vs
    /// the no-library hint. A tiny helper so the paged builder stays under
    /// the too-many-lines threshold.
    fn artists_empty_state(query: &str, hit: bool) -> (&'static str, String) {
        if hit {
            (
                "No matching artists",
                format!("Nothing in your library matches '{query}'."),
            )
        } else {
            (
                "No artists yet",
                "Add a folder from the sidebar to start scanning your library.".to_string(),
            )
        }
    }

    /// The Albums variant (handoff issue 08): every album in the library as
    /// a row with its artist and year plus its first track's cover. The flat
    /// listing is a paged store read over every album in the canonical
    /// browsing order — the prefix-sum flatten over per-artist tables is gone,
    /// so opening the column fetches only the visible window, each album
    /// already carrying its track ids (paginate-browse-columns issue 03).
    ///
    /// Under a query the root lists only hit albums in canonical hit order
    /// (the A–Z sort control is hidden), an album matching by its own
    /// artist/title or by any member track. The genre chip filter is not
    /// part of this column (issue 04 keeps search display-independent): no
    /// genre chips render, and the listing is never genre-filtered.
    #[expect(
        clippy::too_many_lines,
        reason = "the root renderer's query path (hit rows) stays in one function"
    )]
    fn render_albums_browser(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        query: &str,
    ) {
        use riff_backend::app::state::BrowserSelection;
        use riff_backend::app::store::SortDirection;

        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let hit = !query.is_empty();
        // The direction is part of the paged read's query signature: the
        // store applies it in SQL so page offsets stay aligned when the sort
        // reverses (no in-memory reversal of an ascending copy).
        let direction = if sort_desc {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        };
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
        let in_flight = &mut self.cover_in_flight;
        let in_flight_keys = &mut self.cover_in_flight_keys;
        let ctx = ui.ctx().clone();
        // Anchor read: sizes the row range with the authoritative total
        // (computed before the closure borrows `views` mutably).
        let total = if hit {
            views.hit_albums_page(query, 0).total
        } else {
            views.albums_page(direction, 0).total
        };
        // One page cached in hand per frame, refetched only when the row
        // walks out of it (the flat-grid pattern) — the whole album list
        // never materializes on first view.
        let mut hit_page: Option<riff_backend::app::views::HitPage<Album>> = None;
        let mut browse_page: Option<riff_backend::app::views::HitPage<Album>> = None;
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let album: Album = if hit {
                if hit_page
                    .as_ref()
                    .is_none_or(|p| p.start + p.rows.len() <= i)
                {
                    hit_page = Some(views.hit_albums_page(query, i));
                }
                let page = hit_page.as_ref()?;
                page.rows.get(i - page.start)?.clone()
            } else {
                if browse_page
                    .as_ref()
                    .is_none_or(|p| p.start + p.rows.len() <= i)
                {
                    browse_page = Some(views.albums_page(direction, i));
                }
                let page = browse_page.as_ref()?;
                page.rows.get(i - page.start)?.clone()
            };
            // The album's cover, requested through its first track — the
            // same flow the artist rows and track listings use; a full miss
            // resolves the music-icon placeholder tile.
            let thumbnail = album
                .tracks
                .first()
                .map(|tid| {
                    request_cover_intent(
                        textures,
                        in_flight,
                        in_flight_keys,
                        covers.as_ref(),
                        tid.clone(),
                        PathBuf::from(&tid.0),
                        COVER_THUMB,
                    );
                    crate::ui::artwork::lookup_cover_texture(
                        textures,
                        lru_keys,
                        &ctx,
                        &palette,
                        &tid.0,
                        COVER_THUMB,
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
        let (empty_title, empty_hint): (&str, String) = if hit {
            (
                "No matching albums",
                format!("Nothing in your library matches '{query}'."),
            )
        } else {
            (
                "No albums yet",
                "Add a folder from the sidebar to start scanning your library.".to_string(),
            )
        };
        let column = browser::BrowserColumn {
            sort_desc,
            show_sort: !hit,
            total,
            item: &mut item,
            virtualize: false,
            empty_title,
            empty_hint: &empty_hint,
        };
        // Section root list: the site declares WHERE it renders and passes its
        // content; the Scroll Memory composes the identity and picks the shape
        // — saved offset when it matches, else a reset (issue 02).
        let (control, visit) = self.scroll_memory.begin_section(
            riff_backend::app::state::LibrarySection::Albums,
            query,
            sort_desc,
        );
        let actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            column,
            Some(control),
            &mut actions,
        );
        self.scroll_memory.end_section(visit, actual);
        for action in actions {
            // THE GUARD — see the Artists root's identical comment.
            if action.selects_a_row() {
                self.scroll_memory
                    .note_selection_in(crate::ui::scroll_memory::ListSlot::Section(
                        riff_backend::app::state::LibrarySection::Albums,
                    ));
            }
            match action {
                browser::BrowserAction::ContextMenu { key, intents } => {
                    let section = library.library_section;
                    apply_collection_menu(
                        &key,
                        &intents,
                        section,
                        0,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                other => apply_browser_action(other, library),
            }
        }
    }

    /// The Genres variant (handoff issue 08): every genre with its track
    /// count, ordered A–Z / Z–A by the sort control through a paged store
    /// read — one window in hand, so even a library with extreme genre
    /// diversity opens instantly (paginate-browse-columns issue 04).
    /// Selecting a genre drills into it at the root level.
    ///
    /// The query is threaded in for the stage's uniform signature; hit-scoped
    /// genre counts under a query are the spec's isolated cut (issue 05) and
    /// land separately.
    fn render_genres_browser(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        query: &str,
    ) {
        use riff_backend::app::state::BrowserSelection;
        use riff_backend::app::store::SortDirection;

        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let direction = if sort_desc {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        };
        // The root column highlights the path's first entry: the genre the
        // listener is drilled into.
        let selected = match library.browser_path.first() {
            Some(BrowserSelection::Genre(genre)) => Some(genre.clone()),
            _ => None,
        };

        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        // Anchor read: sizes the row range with the authoritative total.
        let total = views.genres_page(direction, 0).total;
        // One page cached in hand per frame, refetched only when the row
        // walks out of it — the whole genre list never materializes.
        let mut browse_page: Option<riff_backend::app::views::HitPage<GenreCount>> = None;
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            if browse_page
                .as_ref()
                .is_none_or(|p| p.start + p.rows.len() <= i)
            {
                browse_page = Some(views.genres_page(direction, i));
            }
            let page = browse_page.as_ref()?;
            let genre = page.rows.get(i - page.start)?;
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
            sort_desc,
            show_sort: true,
            total,
            item: &mut item,
            virtualize: false,
            empty_title: "No genres yet",
            empty_hint: "Genres come from your tracks' tags \u{2014} add music and rescan.",
        };
        // Section root list: the site declares WHERE it renders; the Scroll
        // Memory picks the shape — saved offset when the fingerprint matches,
        // else a reset (issue 02).
        let (control, visit) = self.scroll_memory.begin_section(
            riff_backend::app::state::LibrarySection::Genres,
            query,
            sort_desc,
        );
        let actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            column,
            Some(control),
            &mut actions,
        );
        self.scroll_memory.end_section(visit, actual);
        for action in actions {
            // THE GUARD — see the Artists root's identical comment.
            if action.selects_a_row() {
                self.scroll_memory
                    .note_selection_in(crate::ui::scroll_memory::ListSlot::Section(
                        riff_backend::app::state::LibrarySection::Genres,
                    ));
            }
            match action {
                browser::BrowserAction::ContextMenu { key, intents } => {
                    let section = library.library_section;
                    apply_collection_menu(
                        &key,
                        &intents,
                        section,
                        0,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                other => apply_browser_action(other, library),
            }
        }
    }
}

/// Whether `artist`'s name is itself a hit for `query` — the literal,
/// case-insensitive substring rule (the query lowercased in Rust) the store
/// applies to its write-time-lowercased `name_lower` column. The artist
/// drill column needs the name-hit decision to pick its listing: a name-hit
/// artist expands into all its albums; a track-hit artist shows only its
/// hit albums.
fn artist_name_hits(artist: &str, query: &str) -> bool {
    !query.is_empty() && artist.to_lowercase().contains(&query.to_lowercase())
}
