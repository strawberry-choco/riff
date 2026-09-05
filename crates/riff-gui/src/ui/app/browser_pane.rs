//! The browser column pane (design-handoff issue 08): the first pane of the
//! three-pane explorer, rendered in its own left panel between the top bar
//! and the player bar. One listing per sidebar section — search results, an
//! opened playlist or smart list, the folder tree, or the selected LIBRARY
//! section's entity listing (Artists / Albums / Genres through the
//! [`crate::ui::browser`] widget seam).
//!
//! Child module of `ui::app` so the pane methods keep direct access to
//! [`RiffApp`]'s fields, exactly like the methods they sit beside.

use eframe::egui;
use riff_backend::domain::{Artist, TrackId};
use std::path::PathBuf;
use std::sync::Arc;

use riff_backend::app::state::{
    BrowseMode, LibrarySection, LibrarySession, PlaybackSession, ViewMode,
};

use super::super::browser;
use super::{RiffApp, apply_browser_action, request_cover_intent, smart_list_openable};

impl RiffApp {
    /// The browser column's left panel (handoff issue 08): a 320px pane
    /// right of the nav sidebar, between the top bar and the player bar —
    /// first pane of the three-pane explorer. Library content only: the
    /// Settings and Now Playing stages replace it. Panels are laid out in
    /// show order, so this lands after the player bar's bottom strip and
    /// keeps the pane clear of both bars.
    pub(super) fn render_browser_column_panel(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
    ) {
        if library.view_mode != ViewMode::Library {
            return;
        }
        egui::Panel::left("browser_column")
            .exact_size(crate::ui::theme::BROWSER_W)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(8)))
            .show(ui, |ui| {
                self.render_browser_pane(ui, library, playback);
            });
    }

    /// The browser column pane (handoff issue 08). Whatever the sidebar nav
    /// has open renders here: search results, an opened playlist or smart
    /// list, the folder tree, or the selected LIBRARY section's listing.
    fn render_browser_pane(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &PlaybackSession,
    ) {
        // Search gating reads the store's match count (the same query the
        // flat view's projection totals use) — never the in-memory mirror.
        let query = library.search_query.clone();
        let has_results = query.is_empty() || self.views.search_has_matches(&query);
        // A list only renders when not searching; search shows matching
        // tracks only. The Advanced-only lists (Never Played, Lost Gems)
        // close once Advanced mode flips off.
        let open_playlist = self.smart_playlist_view.filter(|kind| {
            query.is_empty() && smart_list_openable(*kind, library.ui_flags.advanced_mode)
        });
        let open_user_playlist = self.playlist_view.clone().filter(|_| query.is_empty());

        if !has_results && !query.is_empty() {
            browser::empty_state(
                ui,
                &self.theme.active,
                "No tracks found",
                &format!("Nothing in your library matches '{query}'."),
            );
        } else if let Some(pid) = open_user_playlist {
            self.render_playlist_view(ui, library, playback, &pid);
        } else if let Some(kind) = open_playlist {
            self.render_smart_playlist_view(ui, library, playback, kind);
        } else {
            match library.browse_mode {
                BrowseMode::Library => match library.library_section {
                    LibrarySection::AllTracks => {
                        self.render_flat_view(ui, library, playback, &query);
                    }
                    LibrarySection::Artists => self.render_artists_browser(ui, library),
                    LibrarySection::Albums => self.render_albums_browser(ui, library),
                    LibrarySection::Genres => self.render_genres_browser(ui, library),
                },
                BrowseMode::Folders => self.render_folder_tree(ui, library, playback, &query),
            }
        }
    }

    /// The Artists variant (handoff issue 08): every artist as a row with a
    /// small cover thumbnail (open decision 3: the first album's cover),
    /// with the A–Z sort control and the genre filter chips above the list.
    /// Selecting a row stores the identity the detail column (issue 09)
    /// resolves; the album hierarchy it drills into lands there.
    fn render_artists_browser(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        use riff_backend::app::state::BrowserSelection;

        let palette = self.theme.active;
        let genre = library.genre_filter.clone();
        let sort_desc = library.browser_sort_desc;

        let genres = self.views.genres();
        let artists: Arc<[Artist]> = match &genre {
            Some(g) => self.views.artists_in_genre(g),
            None => self.views.artists(),
        };
        // artists() is name-ascending; the sort control flips the render
        // order only, the store keeps the canonical ordering.
        let mut names: Vec<String> = artists.iter().map(|a| a.name.clone()).collect();
        if sort_desc {
            names.reverse();
        }
        let selected = match &library.browser_selection {
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
            let name = names.get(i)?;
            // The row's small cover thumbnail: the first album's cover
            // (open decision 3), requested through the first album's first
            // track — the Cover Service resolves by track path, the texture
            // comes from the UI LRU. A full miss resolves the generated
            // colour block (issue 14) through the same cache. Repeat reads
            // hit the projection cache.
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
                        textures,
                        lru_keys,
                        &ctx,
                        palette.dark,
                        &tid.0,
                    )
                });
            Some(browser::BrowserItem {
                key: name.clone(),
                label: name.clone(),
                detail: None,
                thumbnail,
                selected: selected.as_deref() == Some(name.as_str()),
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
            genres: &genres,
            genre_filter: genre.as_deref(),
            total: names.len(),
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
        let selected = match &library.browser_selection {
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
            // resolves the generated colour block (issue 14).
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
                        textures,
                        lru_keys,
                        &ctx,
                        palette.dark,
                        &tid.0,
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
    /// the sort control. Selecting a genre stores the identity the detail
    /// column (issue 09) resolves into that genre's artists and albums.
    fn render_genres_browser(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        use riff_backend::app::state::BrowserSelection;

        let palette = self.theme.active;
        let sort_desc = library.browser_sort_desc;
        let genres = self.views.genres();
        let total = genres.len();
        let selected = match &library.browser_selection {
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
            // facade serves repeat windows from cache.
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
            // A full miss resolves the generated colour block (issue 14).
            let thumbnail = Some(crate::ui::cover_placeholder::lookup_cover_texture(
                textures,
                lru_keys,
                &ctx,
                palette.dark,
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
