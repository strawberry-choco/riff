//! The elastic column stage (elastic-column spec): the Library view's
//! content area, rendered inside the `CentralPanel`. A sequence of side-by-side
//! list columns derived from the active section and the current drill-down
//! path, plus the collapsible inspector as the rightmost column when a
//! selection exists. Single-list stages (search, playlists, smart lists, the
//! folder tree, All Tracks) render one full-width column.
//!
//! Child module of `ui::app` so the pane methods keep direct access to
//! [`RiffApp`]'s fields, exactly like the methods they sit beside.
//!
//! # The two entity-Column drains
//!
//! The six entity-Columns — three Section roots and three Drill Columns — used
//! to each open their action drain with its own copy of the same
//! selecting-action-resets-scroll block, and to re-supply their Section, their
//! depth and their Scroll Memory slot on every single action. They are **not
//! byte-identical**, and this file says so rather than discovering it mid-edit:
//! normalised, they were two shapes, not one.
//!
//! * A **Section root** (`drain_root_actions`) selects at depth 0, owns a
//!   Section slot, flips the section-wide `browser_sort_desc`, and does **not**
//!   reset a scroll on that flip.
//! * A **Drill Column** (`drain_drill_actions`) selects at depth 1 or 2, owns a
//!   Drill slot, flips the drill-wide `drill_sort_desc`, and **does** reset its
//!   own slot on that flip — the control lives above the list, so the change is
//!   content identity the list's own egui state cannot see.
//!
//! So the collapse target is **two bindings**, and the difference between them
//! is a RULE, which is why it stays in the rule. What differs between two
//! Columns of the same shape is a datum: a [`ColumnIdentity`] — Section, depth,
//! Scroll Memory slot — stated once at the render site and read by both the
//! scroll handshake and the drain. The guard that used to be six copied
//! statements is now [`ColumnIdentity::note_scroll_for`], a method on that
//! datum, so a new Column inherits it by declaring its identity and cannot
//! forget it.

use eframe::egui;
use riff_backend::app::store::SortDirection;
use riff_backend::domain::{Album, PlaylistId, SmartPlaylistKind, TrackId};
use std::sync::Arc;

use riff_backend::app::state::{
    BrowseMode, BrowserSelection, LibrarySection, LibrarySession, PlaybackSession, TrackSort,
};

use super::super::browser;
use super::super::column::ColumnIdentity;
use super::super::scroll_memory::DrillSlot;
use super::{
    COVER_THUMB, CollectionMenuEffects, ColumnKind, RiffApp, apply_collection_menu,
    apply_detail_action, apply_entity_selection, column_plan, cover_texture_for,
    resolve_detail_content, resolve_inspector, smart_list_openable,
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
                let mut albums = self.artist_drill_albums(&artist, query);
                // The drill column's rows are the name-keyed albums the store
                // served in canonical order; the drill sort re-orders the
                // display copy here (the paged genre drills land their
                // direction in the store's ORDER BY instead).
                sort_albums_by_title(&mut albums, library.drill_sort_desc);
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
                // WHAT this Column is, stated once. The `ColumnKind` arm above
                // already fixed it — this site only says so.
                self.render_albums_drill_column(
                    ui,
                    library,
                    playback,
                    &albums,
                    ColumnIdentity::drill(LibrarySection::Artists, 1, DrillSlot::ArtistAlbums),
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
    /// same row shape): a `BrowserColumn` (always list, no genre chips) over
    /// `albums` — already sorted by the caller per the drill sort — with the album row's cover thumbnail, `Artist ·
    /// Year` detail line, and `(album artist, title)` composite key.
    ///
    /// `column` is what this Column IS, and it arrives from the one site that
    /// knows: the stage already chose which `ColumnKind` this is, and that
    /// choice fixes the Section and the depth. Highlighting reads
    /// `library.browser_path[column.level()]`, and a row's selection and its
    /// right-click both land there through the same identity — so this
    /// function, which two Columns share, cannot render one Column's rows and
    /// select them as the other. `playback` travels with them because the
    /// Shuffle item completes in the playback session's queue.
    #[allow(clippy::too_many_arguments)]
    fn render_albums_drill_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        albums: &[riff_backend::domain::Album],
        column_id: ColumnIdentity,
        empty_title: &str,
        empty_hint: &str,
    ) {
        let palette = self.theme.active;
        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let covers = &self.covers;
        let cover_cache = &mut self.cover_cache;
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
                cover_texture_for(
                    cover_cache,
                    covers.as_ref(),
                    textures,
                    lru_keys,
                    &ctx,
                    &palette,
                    tid,
                    COVER_THUMB,
                )
            });
            let detail = album.year.map_or_else(
                || album.artist.clone(),
                |y| format!("{} \u{b7} {y}", album.artist),
            );
            let selected = matches!(
                library.browser_path.get(column_id.level()),
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
            sort_desc: library.drill_sort_desc,
            show_sort: true,
            total,
            item: &mut item,
            virtualize: false,
            empty_title,
            empty_hint,
        };
        // WHAT this Column is, stated ONCE: the Section it runs under, the
        // depth its rows select at, and the Scroll Memory slot it owns. The
        // scroll handshake below and the drain after it read the same value, so
        // the two cannot name different lists.
        //
        // A drill column resets to the top on every selection change — the
        // Scroll Memory holds no drill offsets by design, and it is the module
        // that knows this slot's selections are drill bookkeeping. Between
        // changes its scroll is egui's natural state under the drill's own
        // stable salt. That is the guard's other half: `note_scroll_for` below.
        let control = self.scroll_memory.begin_drill(
            column_id
                .drill_slot()
                .expect("a drill Column has a drill slot"),
        );
        let _actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            library.ui_flags.reduce_motion,
            column,
            Some(control),
            &mut actions,
        );
        self.drain_drill_actions(column_id, actions, library, playback);
    }

    /// The Genres section's artists-in-genre column (level 1): every artist
    /// carrying the genre, with their genre-scoped album count. Rows select
    /// the artist at level 1, and a right-click on one both selects it and
    /// acts on it. Renders one window in hand (issue 05).
    fn render_genre_artists_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        genre: &str,
    ) {
        let palette = self.theme.active;
        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let cover_cache = &mut self.cover_cache;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        // The drill sort's direction is part of the listing's query
        // signature — the store applies it in SQL, so a row index names the
        // same row before and after the flip (no in-memory reversal of an
        // ascending copy).
        let direction = sort_direction(library.drill_sort_desc);
        // One window in hand (paginate-browse-columns issue 05): opening a
        // large genre reads only the visible artists, so a common genre like
        // "Rock" no longer loads every artist that touches it at once. The
        // seam holds the windowing; this view asks for a count and for rows
        // by index.
        let total = views.genre_artist_count(genre, direction);
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let artist = views.genre_artist_row(genre, direction, i)?;
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
                    cover_texture_for(
                        cover_cache,
                        covers.as_ref(),
                        textures,
                        lru_keys,
                        &ctx,
                        &palette,
                        tid,
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
            sort_desc: library.drill_sort_desc,
            show_sort: true,
            total,
            item: &mut item,
            virtualize: false,
            empty_title: "No artists in this genre",
            empty_hint: "This genre has no artists in your library.",
        };
        // WHAT this Column is, stated ONCE — the same three facts the
        // artist's Albums column states, differing only as data.
        let column_id = ColumnIdentity::drill(LibrarySection::Genres, 1, DrillSlot::GenreArtists);
        // A drill column resets to the top on every selection change (no
        // per-selection memory); egui's natural state between changes.
        let control = self.scroll_memory.begin_drill(
            column_id
                .drill_slot()
                .expect("a drill Column has a drill slot"),
        );
        let _actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            library.ui_flags.reduce_motion,
            column,
            Some(control),
            &mut actions,
        );
        self.drain_drill_actions(column_id, actions, library, playback);
    }

    /// The Genres section's artist-albums drill column (level 2): one artist's
    /// albums holding the genre, each carrying its matching track ids, in
    /// canonical browsing order — one window in hand (issue 05), so drilling
    /// deeper reads only the visible albums. The row shape matches the
    /// shared album drill column: cover thumbnail, "Artist · Year" detail,
    /// `(album artist, title)` composite key; a row selection lands at level
    /// 2 — which this Column states once, beside its Scroll Memory slot — and a
    /// right-click lands there too.
    fn render_genre_album_drill_column(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        artist: &str,
        genre: &str,
    ) {
        let palette = self.theme.active;
        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let cover_cache = &mut self.cover_cache;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        // The drill sort's direction is part of the listing's query
        // signature — the store applies it in SQL, so a row index names the
        // same row before and after the flip (no in-memory reversal of an
        // ascending copy).
        let direction = sort_direction(library.drill_sort_desc);
        let total = views.genre_album_count(artist, genre, direction);
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let album = views.genre_album_row(artist, genre, direction, i)?;
            // The album's cover, requested through its first track — the
            // same flow the root columns use; a full miss resolves the
            // music-icon placeholder tile.
            let thumbnail = album.tracks.first().map(|tid| {
                cover_texture_for(
                    cover_cache,
                    covers.as_ref(),
                    textures,
                    lru_keys,
                    &ctx,
                    &palette,
                    tid,
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
            sort_desc: library.drill_sort_desc,
            show_sort: true,
            total,
            item: &mut item,
            virtualize: false,
            empty_title: "No albums in this genre",
            empty_hint: "This artist has no albums carrying this genre.",
        };
        // WHAT this Column is, stated ONCE. The only thing that differs from
        // the genre's Artists column is `2` — the depth this one selects at.
        let column_id =
            ColumnIdentity::drill(LibrarySection::Genres, 2, DrillSlot::GenreArtistAlbums);
        // A drill column resets to the top on every selection change (no
        // per-selection memory); egui's natural state between changes.
        let control = self.scroll_memory.begin_drill(
            column_id
                .drill_slot()
                .expect("a drill Column has a drill slot"),
        );
        let _actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            library.ui_flags.reduce_motion,
            column,
            Some(control),
            &mut actions,
        );
        self.drain_drill_actions(column_id, actions, library, playback);
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
    /// [`crate::ui::detail::DetailReport`], and below this column hands the
    /// Track-menu half of that report to the per-app Track-menu host while the
    /// rest goes to the detail-action applier.
    ///
    /// # The accepted rough edge, in the host's words
    ///
    /// The host's [`TrackMenuHost::right_clicked`](super::TrackMenuHost) is one
    /// of the two things every Track row reaches, so opening one of these menus
    /// makes that Track the selection — the same report a flat-list Track row's
    /// menu makes. In this column that has one
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
        // The rows arrive in the store's canonical album-track order; the
        // session's track sort re-orders the display copy here — the widget
        // formats, it never re-orders. A reversal is the exact reverse of the
        // canonical order; the title modes compare case-insensitively, ties
        // keeping the canonical order.
        let mut tracks = content.tracks;
        sort_tracks(&mut tracks, |row| row.title.clone(), library.track_sort);
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
        let mut reports = Vec::new();
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
            library.ui_flags.reduce_motion,
            crate::ui::detail::DetailColumn {
                tracks: &tracks,
                track_menu: Some(&track_menu),
                empty_title,
                empty_hint: &empty_hint,
                sort: Some(library.track_sort),
            },
            Some(control),
            &mut reports,
        );
        // ONE ordered channel, drained here. Two of its three arms are exactly
        // the drain every other entity Column has — a scroll reset on the sort
        // report, then the applier for everything else — and the third is the
        // Track-menu arm, which was an intercept until the per-app host owned
        // the two handles this column could not reach.
        for report in reports {
            match report {
                crate::ui::detail::DetailReport::TrackMenu(report) => {
                    // A Track menu is NOT this column's dispatch. It is the
                    // per-app Track-menu host's report, answered with the SAME
                    // two methods every other Track row reaches, and it is a
                    // separate report type precisely because it is a different
                    // owner's business: choosing an item needs the Playlist
                    // Store and the Inline Tag Editor, which the host holds and
                    // this column's applier does not. The intercept that used
                    // to sit here existed only to route around those two
                    // missing handles — and the intents were not dropped to
                    // make room for it, they still work, through the host.
                    let track_id = TrackId(report.key);
                    // The Track itself, resolved at dispatch time through the
                    // same seam the Detail Panel's Track readout uses; it is
                    // what "Edit Tags" needs to open its draft, and its absence
                    // is what reduces that one item. A Track in this column is
                    // in no Playlist, so the subject carries no removal.
                    let track = self.views.selected_track(&track_id);
                    let subject = match track.as_ref() {
                        Some(track) => super::TrackMenuSubject::resolved(track, None),
                        None => super::TrackMenuSubject::unresolved(&track_id, None),
                    };
                    // OPENING is what selects, whether or not anything was
                    // chosen from the menu.
                    let mut host = self.track_menu(&mut library.selected_track);
                    host.right_clicked(subject);
                    for intent in report.intents {
                        host.item_chosen(subject, intent);
                    }
                }
                crate::ui::detail::DetailReport::Action(
                    action @ crate::ui::detail::DetailAction::TrackSortSelected(_),
                ) => {
                    // The sort change is content identity the Tracks column's
                    // egui scroll state cannot see — the control lives above
                    // the list — so the slot is forced back to the top, then
                    // the session write goes through the same applier as
                    // every other report.
                    self.scroll_memory
                        .reset_drill_scroll(crate::ui::scroll_memory::DrillSlot::TracksColumn);
                    apply_detail_action(
                        action,
                        library,
                        self.transport.as_ref(),
                        self.library_mutations.as_mut(),
                    );
                }
                crate::ui::detail::DetailReport::Action(action) => apply_detail_action(
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
        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let hit = !query.is_empty();
        // The direction is part of the listing's query signature: the store
        // applies it in SQL, so a row index names the same row before and
        // after the sort reverses (no in-memory reversal of an ascending copy).
        let direction = sort_direction(sort_desc);
        // The root column highlights the path's first entry: the artist the
        // listener is drilled into.
        let selected = match library.browser_path.first() {
            Some(BrowserSelection::Artist(name)) => Some(name.clone()),
            _ => None,
        };

        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        let covers = &self.covers;
        let cover_cache = &mut self.cover_cache;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        // The count read, taken before the closure borrows `views` mutably:
        // its own store read, the value to size the row range with.
        let total = if hit {
            views.hit_artist_count(query)
        } else {
            views.artist_count(direction)
        };
        // One row in hand per frame, by index — the seam refetches only when
        // the row walks out of the window it has (the flat-grid pattern), so
        // the whole artist list never materializes on first view.
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let artist = if hit {
                views.hit_artist_row(query, i)?
            } else {
                views.artist_row(direction, i)?
            };
            let (name, album_count) = (artist.name.clone(), artist.albums.len());
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
            let thumbnail = views.artist_first_track(&name).as_ref().map(|tid| {
                cover_texture_for(
                    cover_cache,
                    covers.as_ref(),
                    textures,
                    lru_keys,
                    &ctx,
                    &palette,
                    tid,
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
        // WHAT this Column is, stated ONCE: the Section it lists, the depth
        // its rows select at (0, so drill-down restarts from the top), and the
        // Scroll Memory slot it owns. The handshake below and the drain after it
        // read the same value, so the two cannot name different lists.
        let column_id = ColumnIdentity::root(LibrarySection::Artists);
        // Section root list: the site declares WHERE it renders and passes its
        // content; the Scroll Memory composes the identity and picks the shape
        // — saved offset when it matches, else a reset (issue 02).
        let (control, visit) =
            self.scroll_memory
                .begin_section(column_id.section(), query, u8::from(sort_desc));
        let actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            library.ui_flags.reduce_motion,
            column,
            Some(control),
            &mut actions,
        );
        self.scroll_memory.end_section(visit, actual);
        self.drain_root_actions(column_id, actions, library, playback);
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
    fn render_albums_browser(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
        query: &str,
    ) {
        use riff_backend::app::state::BrowserSelection;
        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let hit = !query.is_empty();
        // The direction is part of the listing's query signature: the store
        // applies it in SQL, so a row index names the same row before and
        // after the sort reverses (no in-memory reversal of an ascending copy).
        let direction = sort_direction(sort_desc);
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
        let cover_cache = &mut self.cover_cache;
        let textures = &mut self.cover_textures;
        let lru_keys = &mut self.cover_lru_keys;
        let ctx = ui.ctx().clone();
        // The count read, taken before the closure borrows `views` mutably:
        // its own store read, the value to size the row range with.
        let total = if hit {
            views.hit_album_count(query)
        } else {
            views.album_count(direction)
        };
        // One row in hand per frame, by index — the seam refetches only when
        // the row walks out of the window it has, so the whole album list
        // never materializes on first view.
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let album: Arc<Album> = if hit {
                views.hit_album_row(query, i)?
            } else {
                views.album_row(direction, i)?
            };
            // The album's cover, requested through its first track — the
            // same flow the artist rows and track listings use; a full miss
            // resolves the music-icon placeholder tile.
            let thumbnail = album
                .tracks
                .first()
                .map(|tid| {
                    cover_texture_for(
                        cover_cache,
                        covers.as_ref(),
                        textures,
                        lru_keys,
                        &ctx,
                        &palette,
                        tid,
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
        // WHAT this Column is, stated ONCE — the same three facts the Artists
        // root states, differing only as data.
        let column_id = ColumnIdentity::root(LibrarySection::Albums);
        // Section root list: the site declares WHERE it renders and passes its
        // content; the Scroll Memory composes the identity and picks the shape
        // — saved offset when it matches, else a reset (issue 02).
        let (control, visit) =
            self.scroll_memory
                .begin_section(column_id.section(), query, u8::from(sort_desc));
        let actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            library.ui_flags.reduce_motion,
            column,
            Some(control),
            &mut actions,
        );
        self.scroll_memory.end_section(visit, actual);
        self.drain_root_actions(column_id, actions, library, playback);
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
        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let direction = sort_direction(sort_desc);
        // The root column highlights the path's first entry: the genre the
        // listener is drilled into.
        let selected = match library.browser_path.first() {
            Some(BrowserSelection::Genre(genre)) => Some(genre.clone()),
            _ => None,
        };

        let mut actions: Vec<browser::BrowserAction> = Vec::new();
        let views = &mut self.views;
        // The count read: its own store read, the value to size the row
        // range with.
        let total = views.genre_count(direction);
        // One row in hand per frame, by index — the seam refetches only when
        // the row walks out of the window it has, so the whole genre list
        // never materializes.
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let genre = views.genre_row(direction, i)?;
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
        // WHAT this Column is, stated ONCE.
        let column_id = ColumnIdentity::root(LibrarySection::Genres);
        // Section root list: the site declares WHERE it renders; the Scroll
        // Memory picks the shape — saved offset when the fingerprint matches,
        // else a reset (issue 02).
        let (control, visit) =
            self.scroll_memory
                .begin_section(column_id.section(), query, u8::from(sort_desc));
        let actual = browser::show_browser_column_scrolled(
            ui,
            &mut self.icons,
            &palette,
            library.ui_flags.reduce_motion,
            column,
            Some(control),
            &mut actions,
        );
        self.scroll_memory.end_section(visit, actual);
        self.drain_root_actions(column_id, actions, library, playback);
    }

    // --- The two entity-Column drains ---------------------------------------
    //
    // The three Section roots and the three Drill Columns, each stated as a
    // `ColumnIdentity` at its render site and answered here. Six copies of the
    // same guard became one method on the identity; per-action Section and
    // depth are gone.
    //
    // What is left in these two bodies is the ONE thing that genuinely differs
    // between a root and a drill: which sort direction the A–Z control flips,
    // and whether that flip also forces the list back to the top. A drill's
    // control lives above its list, so its flip is content identity the list's
    // egui state cannot see and the slot is reset; a root's is not. Everything
    // else the two share, so it is written once each and neither copy is a
    // third copy of the guard.

    /// Answer one **Section root's** actions.
    ///
    /// `column` is the identity its render site stated: which Section it lists,
    /// the depth its rows select at (0, so drill-down restarts from the top),
    /// and the Scroll Memory slot it owns. Nothing about it is re-supplied per
    /// action, and the guard is asked of it rather than remembered here.
    fn drain_root_actions(
        &mut self,
        column: ColumnIdentity,
        actions: Vec<browser::BrowserAction>,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
        for action in actions {
            if let Some(slot) = column.note_scroll_for(&action) {
                self.scroll_memory.note_selection_in(slot);
            }
            match action {
                browser::BrowserAction::ContextMenu { key, intents } => {
                    apply_collection_menu(
                        &key,
                        &intents,
                        column,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            passes: self.passes.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                browser::BrowserAction::Select(key) => {
                    apply_entity_selection(&key, column, library);
                }
                // A root's A–Z control flips the section-wide direction, and —
                // unlike a drill's — does not force the list back to the top.
                // That asymmetry is the reason there are two bindings and not
                // one; it is stated once, here, rather than six times.
                browser::BrowserAction::ToggleSort => {
                    library.browser_sort_desc = !library.browser_sort_desc;
                }
            }
        }
    }

    /// Answer one **Drill Column's** actions.
    ///
    /// The same three arms as [`Self::drain_root_actions`] and the same guard,
    /// with one difference: the sort flip is the drill-wide direction AND resets
    /// this Column's own slot, because the control lives above the list and the
    /// list's egui state cannot see it. The slot it resets is the identity's
    /// own — the one its scroll handshake used — so the reset cannot land on a
    /// neighbour's list.
    fn drain_drill_actions(
        &mut self,
        column: ColumnIdentity,
        actions: Vec<browser::BrowserAction>,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
        let sort_reset_slot = column.drill_slot();
        for action in actions {
            if let Some(slot) = column.note_scroll_for(&action) {
                self.scroll_memory.note_selection_in(slot);
            }
            match action {
                browser::BrowserAction::ContextMenu { key, intents } => {
                    apply_collection_menu(
                        &key,
                        &intents,
                        column,
                        CollectionMenuEffects {
                            library,
                            playback,
                            transport: self.transport.as_ref(),
                            passes: self.passes.as_ref(),
                            views: &mut self.views,
                        },
                    );
                }
                browser::BrowserAction::Select(key) => {
                    apply_entity_selection(&key, column, library);
                }
                browser::BrowserAction::ToggleSort => {
                    library.drill_sort_desc = !library.drill_sort_desc;
                    if let Some(slot) = sort_reset_slot {
                        self.scroll_memory.reset_drill_scroll(slot);
                    }
                }
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

/// The sort toggle's bool as the store's sort direction — the one spelling
/// of that conversion for the paged columns (the direction is part of each
/// listing's query signature: the store applies it in SQL, so a row index
/// names the same row before and after the flip).
fn sort_direction(desc: bool) -> SortDirection {
    if desc {
        SortDirection::Descending
    } else {
        SortDirection::Ascending
    }
}

/// Sort a track listing in place per `sort`: `NumberAsc` is the canonical
/// order the store served — the no-op anchor; `NumberDesc` is its exact
/// reversal; the title modes compare `title_key` case-insensitively, and
/// being stable sorts, ties keep the canonical order. Shared by the owned
/// `TrackRow` listings here and the borrowed `&Track` listings in `app.rs`.
pub(super) fn sort_tracks<T>(rows: &mut [T], title_key: impl Fn(&T) -> String, sort: TrackSort) {
    match sort {
        TrackSort::NumberAsc => {}
        TrackSort::NumberDesc => rows.reverse(),
        TrackSort::TitleAsc => rows.sort_by_key(|row| title_key(row).to_lowercase()),
        TrackSort::TitleDesc => {
            rows.sort_by_key(|row| std::cmp::Reverse(title_key(row).to_lowercase()));
        }
    }
}

/// Sort an album drill's display copy in place per the drill sort's
/// direction: title A–Z (case-insensitive, stable, so ties keep the store's
/// canonical order) or its Z–A reverse — the same reversed-comparator shape
/// the paged genre drills land in SQL.
pub(super) fn sort_albums_by_title(albums: &mut [riff_backend::domain::Album], desc: bool) {
    if desc {
        albums.sort_by_key(|album| std::cmp::Reverse(album.title.to_lowercase()));
    } else {
        albums.sort_by_key(|album| album.title.to_lowercase());
    }
}
