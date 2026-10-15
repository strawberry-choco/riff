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
use riff_backend::domain::{Album, PlaylistId, SmartPlaylistKind};
use std::sync::Arc;

use riff_backend::app::state::{
    BrowseMode, BrowserSelection, LibrarySection, LibrarySession, PlaybackSession, TrackSort,
};
use riff_gui_components::menu::{PlaylistKey, TrackMenu};

use super::super::browser;
use super::super::column::ColumnIdentity;
use super::super::scroll_memory::{DrillSlot, ScrollMemory};
use super::{
    COVER_THUMB, ColumnKind, RiffApp, RowCovers, apply_collection_menu, apply_detail_action,
    apply_entity_selection, column_plan, resolve_detail_content, resolve_inspector,
    smart_list_openable,
};
use riff_gui_theme::theme::Palette;

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
    /// [`column_plan`]), each at its sized width
    /// ([`column_widths`](super::column_widths)) with a thin hairline separator
    /// between — then the collapsible inspector as the rightmost column only
    /// while a selection exists. No horizontal scrolling: narrow windows shrink
    /// columns toward their floors.
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
        let inspector = resolve_inspector(&mut self.views, library, self.now);
        let inspector_visible = inspector.visible;

        // The stage's width allocation, separator accounting, zero-gap
        // composition, stable child identities, and inspector placement are all
        // owned by the shared geometry seam — the golden harness calls the very
        // same helper, so production and goldens cannot disagree on a column
        // edge. This call site supplies only the column content.
        riff_gui_components::stage::show_elastic_stage(
            ui,
            plan.len(),
            inspector_visible,
            |ui, slot| match slot {
                riff_gui_components::stage::StageSlot::Column(i) => {
                    self.render_stage_column(
                        ui,
                        library,
                        playback,
                        &query,
                        single.as_ref(),
                        plan[i],
                    );
                }
                riff_gui_components::stage::StageSlot::Inspector => {
                    self.render_inspector(ui, library);
                }
            },
        );
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
    ///
    /// The name-hit decision is the store's, asked through the read seam
    /// beside the album name-hit read it already uses — the drill holds no
    /// hit rule of its own. The empty-query short-circuit stays *before* the
    /// read, exactly like the album drill's, so an empty query spends no
    /// store read and no cache entry.
    fn artist_drill_albums(&mut self, artist: &str, query: &str) -> Vec<Album> {
        let all: Vec<Album> = self.views.artist_albums(artist).to_vec();
        if query.is_empty() || self.views.artist_is_name_hit(artist, query) {
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
        // The four handles this column's rows ask for art through, named once
        // for the whole listing: a row calls `thumbnail` with a track id and
        // never names the Cover Cache's halves.
        let mut covers = RowCovers::new(
            self.covers.as_ref(),
            &mut self.cover_cache,
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ui.ctx(),
            palette,
        );
        let total = albums.len();
        let mut item = |i: usize| -> Option<browser::BrowserItem> {
            let album = albums.get(i)?;
            // The album's cover, requested through its first track — the
            // same flow the root columns use; a full miss resolves the
            // music-icon placeholder tile.
            let thumbnail = album
                .tracks
                .first()
                .map(|tid| covers.thumbnail(tid, COVER_THUMB));
            let selected = matches!(
                library.browser_path.get(column_id.level()),
                Some(BrowserSelection::Album { artist, title })
                    if artist == &album.artist && title == &album.title
            );
            Some(album_item(album, selected, thumbnail))
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
        let mut paint = column_paint(ui, &mut self.icons, palette, library.ui_flags.reduce_motion);
        let actions = render_drill_column(&mut paint, column, column_id, &mut self.scroll_memory);
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
        let views = &mut self.views;
        let mut covers = RowCovers::new(
            self.covers.as_ref(),
            &mut self.cover_cache,
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ui.ctx(),
            palette,
        );
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
                .map(|tid| covers.thumbnail(tid, COVER_THUMB));
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
        let mut paint = column_paint(ui, &mut self.icons, palette, library.ui_flags.reduce_motion);
        let actions = render_drill_column(&mut paint, column, column_id, &mut self.scroll_memory);
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
        let views = &mut self.views;
        let mut covers = RowCovers::new(
            self.covers.as_ref(),
            &mut self.cover_cache,
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ui.ctx(),
            palette,
        );
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
            let thumbnail = album
                .tracks
                .first()
                .map(|tid| covers.thumbnail(tid, COVER_THUMB));
            let selected = matches!(
                library.browser_path.get(2),
                Some(BrowserSelection::Album { artist, title })
                    if artist == &album.artist && title == &album.title
            );
            Some(album_item(&album, selected, thumbnail))
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
        let mut paint = column_paint(ui, &mut self.icons, palette, library.ui_flags.reduce_motion);
        let actions = render_drill_column(&mut paint, column, column_id, &mut self.scroll_memory);
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
        let options: Vec<(PlaylistKey, String)> = playlists
            .iter()
            .map(|p| (super::track_menu::playlist_id_to_key(&p.id), p.name.clone()))
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
        let track_menu = |row: &crate::ui::detail::TrackRow| TrackMenu {
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
                    let track_id = super::track_menu::track_key_to_id(report.key);
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

        let views = &mut self.views;
        let mut covers = RowCovers::new(
            self.covers.as_ref(),
            &mut self.cover_cache,
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ui.ctx(),
            palette,
        );
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
            let thumbnail = views
                .artist_first_track(&name)
                .as_ref()
                .map(|tid| covers.thumbnail(tid, COVER_THUMB));
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
        let mut paint = column_paint(ui, &mut self.icons, palette, library.ui_flags.reduce_motion);
        let actions = render_root_column(
            &mut paint,
            column,
            column_id,
            query,
            sort_desc,
            &mut self.scroll_memory,
        );
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

        let views = &mut self.views;
        let mut covers = RowCovers::new(
            self.covers.as_ref(),
            &mut self.cover_cache,
            &mut self.cover_textures,
            &mut self.cover_lru_keys,
            ui.ctx(),
            palette,
        );
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
                .map(|tid| covers.thumbnail(tid, COVER_THUMB).into())
                .unwrap_or_default();
            Some(album_item(
                &album,
                selected
                    .as_ref()
                    .is_some_and(|(a, t)| *a == album.artist && *t == album.title),
                thumbnail,
            ))
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
        let mut paint = column_paint(ui, &mut self.icons, palette, library.ui_flags.reduce_motion);
        let actions = render_root_column(
            &mut paint,
            column,
            column_id,
            query,
            sort_desc,
            &mut self.scroll_memory,
        );
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
        let mut paint = column_paint(ui, &mut self.icons, palette, library.ui_flags.reduce_motion);
        let actions = render_root_column(
            &mut paint,
            column,
            column_id,
            query,
            sort_desc,
            &mut self.scroll_memory,
        );
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
                        self.collection_menu(library, playback),
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
                        self.collection_menu(library, playback),
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

// --- The two entity-Column render protocols ----------------------------
//
// Each of the six entity-Columns closes its render with the same
// scroll-handshake, and the handshake is where the protocol can be spelled
// wrong in ways nothing catches: a site that forgets `end_section` loses
// its scroll position silently, a site that hands back an offset it never
// rendered with saves it under the wrong content, and a drill site that
// reaches for a slot it has no claim on panics at runtime rather than at
// compile time. So the four statements are written twice — once per Column
// kind — and a Column states WHERE it is and WHAT it renders while these
// two answer HOW.
//
// The action half is not here. [`RiffApp::drain_root_actions`] and
// [`RiffApp::drain_drill_actions`] own it, and they must run AFTER the
// column is painted: they take `&mut self`, and the `item` closure inside
// a [`browser::BrowserColumn`] holds `&mut self.views`,
// `&mut self.cover_cache`, `&mut self.cover_textures`,
// `&mut self.cover_lru_keys` and `&self.covers` at once, so no `&mut self`
// method is reachable while it is alive. Both functions therefore report
// the actions out and the caller drains them on the next statement.
//
// Both are free functions over explicit disjoint borrows rather than
// methods on `RiffApp`, for the same reason [`RowCovers`](super::RowCovers)
// and the Frame's `FrameParts` are: the render borrows the icons, the
// palette and the Scroll Memory, none of which the caller can hand over as a
// whole.

/// What one Column paints with, stated once per listing and passed in.
///
/// The six entity-Column render sites each spelled the same four handles for
/// a column they were about to describe: the ui, the icon cache, the palette,
/// and the session's temporal policy. That prefix is identical at all six and
/// is not the interesting part of any of them, so it becomes one value — a site
/// states WHAT its column is, and this carries HOW it paints.
///
/// It is built per listing and passed **into** the paint, never shared across a
/// call that closes over `&mut self`: the `item` closure inside a
/// [`browser::BrowserColumn`] already holds `&mut self.views`, the Cover
/// Cache's two halves and the LRU keys at once, so a bundle that outlived that
/// closure would need a field of `self` the closure already holds. See the
/// module section above, and the precedent [`RowCovers`]
/// documents for the same reason.
///
/// The Scroll Memory is deliberately NOT in here: it is the one handle the
/// paint hands to [`ScrollMemory::begin_section`] and then takes straight back,
/// so keeping it outside the bundle is what keeps the handshake's borrow short
/// and the drain's `&mut self` reachable afterwards.
pub(super) struct ColumnPaint<'a> {
    /// The column's own ui.
    pub(super) ui: &'a mut egui::Ui,
    /// The glyph cache the rows and the sort control draw through.
    pub(super) icons: &'a mut riff_gui_theme::icons::IconCache,
    /// The tokens the column paints from (ADR 0004).
    pub(super) palette: Palette,
    /// Whether hover washes and tweens snap to their settled state.
    pub(super) reduce_motion: bool,
}

/// Build the paint bundle out of the four handles every site already holds.
///
/// A free function rather than a `&mut self` method for the same reason
/// [`RowCovers::new`] is one: the call sites are inside listings whose `item`
/// closures already borrow `self.views`, `self.cover_cache`,
/// `self.cover_textures` and `self.cover_lru_keys`, so a `&mut self` builder
/// would ask for a second whole-`self` borrow the closures cannot release.
/// Explicit disjoint field borrows are the shape that compiles, and naming the
/// set here means a seventh column site passes four fields and never lists them.
pub(super) fn column_paint<'a>(
    ui: &'a mut egui::Ui,
    icons: &'a mut riff_gui_theme::icons::IconCache,
    palette: Palette,
    reduce_motion: bool,
) -> ColumnPaint<'a> {
    ColumnPaint {
        ui,
        icons,
        palette,
        reduce_motion,
    }
}

/// Paint one **Section root** column and report its actions.
///
/// The scroll-handshake for a Section root, in one place: the site declares
/// its [`ColumnIdentity`] and passes its content (the current query and
/// sort direction), the Scroll Memory composes the content identity and
/// answers with the offset to start at, and the offset the list actually
/// ended at travels straight back to [`ScrollMemory::end_section`] under
/// the visit the start call returned — so a site cannot record an offset
/// under content other than what it rendered with.
///
/// The actions come back out rather than being drained here: see the
/// module section above for why the drains must run after the paint.
pub(super) fn render_root_column(
    paint: &mut ColumnPaint<'_>,
    column: browser::BrowserColumn<'_>,
    column_id: ColumnIdentity,
    query: &str,
    sort_desc: bool,
    scroll_memory: &mut ScrollMemory,
) -> Vec<browser::BrowserAction> {
    let mut actions = Vec::new();
    let ColumnPaint {
        ui,
        icons,
        palette,
        reduce_motion,
    } = paint;
    let (control, visit) =
        scroll_memory.begin_section(column_id.section(), query, u8::from(sort_desc));
    let actual = browser::show_browser_column_scrolled(
        ui,
        icons,
        palette,
        *reduce_motion,
        column,
        Some(control),
        &mut actions,
    );
    scroll_memory.end_section(visit, actual);
    actions
}

/// Paint one **Drill Column** and report its actions.
///
/// The same handshake shape, with the two differences a drill's own: it
/// takes no content identity, because a drill column resets to the top on
/// every selection change and the Scroll Memory holds no drill offsets by
/// design; and it hands the actual offset back to nobody.
///
/// The discard is the protocol, not an oversight. Nothing downstream reads
/// a drill's offset — [`RiffApp::drain_drill_actions`] resets this
/// Column's own slot on a sort flip, and every other frame leaves egui's
/// natural state under the drill's stable salt to continue — so the
/// returned offset is dropped on purpose. A drill that started recording
/// would grow a per-selection memory the module deliberately does not
/// have, and would do it under a Section's slot it has no claim on.
///
/// The drill slot is asked of the identity the site stated, and the
/// `expect` is the One place that claim is made: every drill site states
/// a drill identity, and a root reaching this function is a compile-time
/// impossibility rather than a runtime surprise.
pub(super) fn render_drill_column(
    paint: &mut ColumnPaint<'_>,
    column: browser::BrowserColumn<'_>,
    column_id: ColumnIdentity,
    scroll_memory: &mut ScrollMemory,
) -> Vec<browser::BrowserAction> {
    let mut actions = Vec::new();
    let ColumnPaint {
        ui,
        icons,
        palette,
        reduce_motion,
    } = paint;
    let control = scroll_memory.begin_drill(
        column_id
            .drill_slot()
            .expect("a drill Column has a drill slot"),
    );
    let _actual = browser::show_browser_column_scrolled(
        ui,
        icons,
        palette,
        *reduce_motion,
        column,
        Some(control),
        &mut actions,
    );
    actions
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

/// One album row: the store's identity key, its label, its year detail, and
/// the caller's thumbnail and selection.
///
/// The key's `\u{1f}` separator is not a formatting choice. It **is** the
/// store's album identity — `(album artist, title)` joined — and it is what
/// [`super::entity_row`] splits to classify this row as an Album, which is why
/// the three album listings (the Albums root, an artist's Albums drill, and a
/// genre's Albums drill) build their rows here rather than each spelling the
/// format itself. One construction is what keeps a row's identity and its
/// consumer linked: the key a click selects by and the key the collection menu
/// dispatches on are then the same bytes by construction, not by convention.
///
/// That is what preserves the invariant [`super::entity_row`]'s doc states —
/// a row can never be SELECTED as one thing and PLAYED as another. Were the
/// construction and the split to drift, an Album's right-click would offer
/// Track actions and an Artist's would offer Album measure items, and nothing
/// in the type system would say so.
///
/// `selected` and `thumbnail` stay the caller's, because they are the two
/// facts a listing knows and an Album does not: which row the drill-down path
/// is on, and which cover has landed for this frame. Everything else about
/// the row is a fact about the Album, so it is derived here.
///
/// Pure over its arguments, which is why it is a free function rather than a
/// method: it reads no listing state, so the three columns can share it while
/// their `item` closures hold `&mut self.views` and the Cover Cache's two
/// halves (the same borrow shape [`RowCovers`] documents).
fn album_item(
    album: &Album,
    selected: bool,
    thumbnail: Option<egui::TextureHandle>,
) -> browser::BrowserItem {
    browser::BrowserItem {
        key: format!("{}\u{1f}{}", album.artist, album.title),
        label: album.title.clone(),
        // The artist · year line, as the albums column and the inspector's
        // album readout both read it: the artist alone before a year, and the
        // year joined to it once there is one.
        detail: Some(album.year.map_or_else(
            || album.artist.clone(),
            |y| format!("{} \u{b7} {y}", album.artist),
        )),
        thumbnail,
        selected,
        // An album row is never the track currently loaded: `now_playing` is a
        // Track-row fact, and an album listing's rows carry it as false.
        now_playing: false,
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

#[cfg(test)]
mod tests {
    use riff_backend::app::state::TrackSort;

    use super::sort_tracks;

    /// The display→canonical index permutation [`sort_tracks`] produces over
    /// the `(0..n)` indices — the EXACT shape the user-playlist view feeds it
    /// after the S1 fold (`let mut sorted = (0..entries.len()).collect();
    /// sort_tracks(&mut sorted, |&i| title(i), sort)`), so ordering the playlist
    /// and ordering this owner are one fact, not two.
    ///
    /// `title_key` returns the RAW title and lets [`sort_tracks`] own the single
    /// case-insensitive compare, matching the playlist's raw-title hand-off.
    fn order_for(titles: &[&str], sort: TrackSort) -> Vec<usize> {
        let mut order: Vec<usize> = (0..titles.len()).collect();
        sort_tracks(&mut order, |&i| titles[i].to_owned(), sort);
        order
    }

    /// The number modes are the canonical order and its exact mirror.
    #[test]
    fn number_modes_are_the_identity_and_its_exact_reverse() {
        let titles = ["Solo", "Duo", "Trio", "Quartet"];
        assert_eq!(order_for(&titles, TrackSort::NumberAsc), vec![0, 1, 2, 3]);
        assert_eq!(order_for(&titles, TrackSort::NumberDesc), vec![3, 2, 1, 0]);
    }

    /// Every mode is a PERMUTATION of the canonical indices, and — the
    /// invariant the playlist view's drag-to-reorder rests on — a sort FLIP
    /// keeps each canonical row naming the same row: the row rendered at
    /// display `pos` under Asc is the row rendered at the mirror position under
    /// Desc. Nothing is lost, duplicated, or re-homed to a different track.
    #[test]
    fn a_sort_flip_keeps_every_order_entry_pointing_at_the_same_row() {
        let titles = ["a", "b", "c", "d", "e"];
        let asc = order_for(&titles, TrackSort::NumberAsc);
        let desc = order_for(&titles, TrackSort::NumberDesc);
        for (pos, &canonical_under_asc) in asc.iter().enumerate() {
            assert_eq!(
                desc[asc.len() - 1 - pos],
                canonical_under_asc,
                "flipping the sort mirrors every row's position; the canonical \
                 index it carries is unchanged"
            );
        }
        // Both orders cover the canonical set exactly once — each `order[i]` is
        // a real row, and each row is addressed by exactly one position.
        for order in [asc, desc] {
            let mut as_set = order.clone();
            as_set.sort_unstable();
            assert_eq!(as_set, (0..titles.len()).collect::<Vec<_>>());
        }
    }

    /// The title modes compare case-insensitively — a lower-case-first title
    /// sorts ahead of an upper-case one that would win a byte-order compare —
    /// and ties keep the canonical order (stable) under BOTH directions.
    #[test]
    fn title_modes_are_case_insensitive_and_stable_on_ties() {
        // Case-insensitivity: `apple` (lower) sorts before `Banana` (upper),
        // where a byte compare would put `Banana` ('B' = 66) first.
        let titles = ["Banana", "apple"];
        assert_eq!(
            order_for(&titles, TrackSort::TitleAsc),
            vec![1, 0],
            "the compare is case-insensitive, not byte-order"
        );

        // Stability: two entries share the title "Echo" and keep their canonical
        // relative order in each direction (a stable sort preserves input order
        // among equal keys, even under the reversed comparator).
        let tied = ["Delta", "Echo", "echo", "Alpha"];
        // lower = [delta, echo, echo, alpha]; asc = alpha, delta, echo, echo.
        assert_eq!(
            order_for(&tied, TrackSort::TitleAsc),
            vec![3, 0, 1, 2],
            "case-folded ties keep canonical order"
        );
        // Reverse comparator: the two echoes lead (keeping input order 1 before
        // 2), then delta, then alpha.
        assert_eq!(
            order_for(&tied, TrackSort::TitleDesc),
            vec![1, 2, 0, 3],
            "the reverse arm mirrors the compare but still keeps ties stable"
        );
    }
}
