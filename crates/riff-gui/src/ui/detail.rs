//! The detail column (design-handoff issue 09): the middle pane of the
//! three-pane explorer. A breadcrumb trail over the drilled path, the album
//! header as a readout of the album's title over its `Artist · Year` line,
//! and the album's track list: one shared 40px [`super::sidebar::tree_row`]
//! per track — the same shape the All Tracks list speaks, favorite control
//! included — with the `Plays · Time` cluster on the right.
//!
//! A readout displays; it does not act. The header carries no buttons and no
//! context menu anchor — the album's actions are reached from its row in the
//! Albums column, so this column has no button surface of its own.
//!
//! # The Track menu, and this column's one accepted rough edge
//!
//! Every track row anchors the shared Track menu ([`super::menu::track_menu`]),
//! so a Track's actions do not depend on which Column happens to be showing
//! it. The popup is egui's own transient one, keyed by the row's own response
//! identity rather than by an index, so these virtualized rows need no per-row
//! open state — and the attach sits inside the branch that MATERIALIZES a row,
//! because a row that was never created has nothing to open a menu on.
//!
//! The menu is BUILT per row, from the host's [`TrackMenuFactory`], because one
//! of its props is a fact about the row and not about the column: a Track's
//! Favourite flag. An album's Tracks are Favourited independently, so a column
//! that handed over one finished menu would label every row's Favourite item the
//! same way and be wrong on all but one of them. The heart on the row stays, and
//! the menu's Favourite item is a second door to the change it makes.
//!
//! What the menu reports is that it OPENED, and opening is what makes the
//! Track the selection — the same report every other Track row's menu makes
//! ([`DetailAction::TrackMenu`]), answered by the same two appliers the host
//! already runs for every other Track row. That report is also where this
//! column's one accepted rough edge comes from, so it is written out here
//! rather than left to be rediscovered as a bug:
//!
//! **Right-clicking a Track in this column selects it, which switches the
//! Detail Panel from the Album readout to the Track readout, so the Album's
//! Tag Aggregation and Batch Tag Edit are hidden until the Album is selected
//! again.** That follows directly from right-click selecting, it is recoverable
//! (the Album is selected again from its row in the Albums column), and it is
//! recorded as accepted rather than worked around.
//!
//! Pure widget seam, same discipline as [`crate::ui::browser`]: widgets
//! paint from [`Palette`] tokens and report [`DetailAction`]s instead of
//! mutating app state; `app.rs` applies them. Rendered headlessly in
//! `tests/ui_tests.rs`.

use eframe::egui;

use super::icons::IconCache;
use super::theme::{self, Palette};

/// One segment of the breadcrumb trail: the path from the browser column's
/// section down to the entity now in the detail column (e.g. `Artists /
/// Boards of Canada / Geogaddi`).
#[derive(Debug, Clone)]
pub struct Crumb {
    pub label: String,
}

/// What the user did to the detail column this frame; `app.rs` applies
/// these to the sessions and the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailAction {
    /// A breadcrumb segment at level `index` was clicked (0 = the section
    /// root): the caller climbs the selection path back to that level.
    Crumb(usize),
    /// An entity row (an album under an artist, an artist under a genre)
    /// was clicked, by its key. The caller resolves the level from the
    /// current selection — the widget stays identity-agnostic.
    SelectRow(String),
    /// A track-table row was selected, by its [`crate::riff_backend::domain::TrackId`]
    /// key.
    SelectTrack(String),
    /// A track-table row asked to start playing, by its id key.
    PlayTrack(String),
    /// A right-click opened this track row's Track menu, carrying the row's
    /// [`crate::riff_backend::domain::TrackId`] key and whatever was chosen
    /// from the menu in click order.
    ///
    /// The counterpart of [`crate::ui::browser::BrowserAction::ContextMenu`],
    /// and it arrives on the frame the menu OPENS — with `intents` empty,
    /// because OPENING is what moves the selection and CHOOSING is a separate
    /// event. The key travels with the report so the host can select the Track
    /// AND act on it from one value; a right-click that opens a menu and is
    /// then dismissed chose nothing and has still selected.
    TrackMenu {
        key: String,
        intents: Vec<super::menu::TrackMenuIntent>,
    },
    /// The row's favorite control toggled the track's flag to `favorite`,
    /// by its id key.
    SetFavorite { key: String, favorite: bool },
}

/// The album header block: the album title over its muted artist · year
/// line. A readout, with nothing to press: the album's playback actions live
/// in the Albums column's row menu, so the header neither renders a button
/// nor anchors a context menu.
#[derive(Debug, Clone)]
pub struct AlbumHeader {
    pub title: String,
    /// `"Artist · Year"`-style secondary line; `None` renders only the
    /// title.
    pub subtitle: Option<String>,
}

/// One row of the album track table: the display values the app resolved
/// from the store's `Track` — the widget formats, it never re-derives.
#[derive(Debug, Clone)]
pub struct TrackRow {
    /// The track's [`riff_backend::domain::TrackId`] key — the selection,
    /// playback, and favorite identity for the row.
    pub key: String,
    pub title: String,
    /// Finished plays, straight from the store's play history.
    pub plays: u32,
    /// `None` renders the dash (unknown duration).
    pub duration: Option<std::time::Duration>,
    pub favorite: bool,
    pub selected: bool,
    /// Whether this row IS the track currently loaded in the player.
    pub now_playing: bool,
}

/// The row's right-aligned value cluster, `Plays · Time`.
impl TrackRow {
    fn meta(&self) -> super::sidebar::RowMeta {
        super::sidebar::RowMeta {
            plays: Some(self.plays),
            time: self.duration,
        }
    }
}

/// The host's per-row Track-menu builder: given one row, hand back the menu
/// that row's right-click should open. A named alias because it appears in two
/// signatures — [`DetailColumn::track_menu`] and `track_list` — and the lifetime
/// it binds (`'a`, the column's own) is the whole point of it: the menu a row
/// paints borrows the frame's resolved playlist targets for as long as the
/// column lives, and gets its own per-row facts handed in as the argument.
pub type TrackMenuFactory<'a> = dyn Fn(&TrackRow) -> super::menu::TrackMenu<'a> + 'a;

/// One frame of the detail column: what to render and how.
pub struct DetailColumn<'a> {
    /// The drilled path, root first, current level last.
    pub breadcrumb: &'a [Crumb],
    /// The album header block; `None` above the album level (artist and
    /// genre detail render entity rows instead).
    pub header: Option<&'a AlbumHeader>,
    /// The album's track list (the shared 40px track row, one per track);
    /// empty above the album level.
    pub tracks: &'a [TrackRow],
    /// The entity rows below the album level: an artist's albums, or a
    /// genre's artists (the browser column's row shape, drilled down).
    pub rows: &'a [super::browser::BrowserItem],
    /// The Track menu ONE track row anchors, as a factory over the row.
    ///
    /// A factory and not one finished menu, because a Track menu is not all
    /// column-wide: a Track's Favourite flag belongs to the ROW, and an album's
    /// Tracks are Favourited independently, so handing this column a single menu
    /// would put one row's answer on every row. Everything else — the playlist
    /// add targets, which the host resolves up front because the menu is PAINTED
    /// from them, and whether these rows are playlist entries at all — is a fact
    /// about the frame, and the host keeps writing it in the factory it hands
    /// over. That is the same shape [`crate::ui::browser::BrowserColumn`] uses
    /// for its per-row item, and for the same reason: these rows are virtualized
    /// and the widget can only see a row once it has materialized one.
    ///
    /// `None` anchors no menu at all, which is the plain rendering path: the
    /// goldens and the widget tests that have no host to resolve playlist
    /// targets from. The app always hands one, because a Track's actions must
    /// not depend on which Column is showing it.
    pub track_menu: Option<&'a TrackMenuFactory<'a>>,
    /// Friendly empty-state title when there is nothing to render.
    pub empty_title: &'a str,
    /// Friendly empty-state hint when there is nothing to render.
    pub empty_hint: &'a str,
}

impl<'a> DetailColumn<'a> {
    /// A breadcrumb-only frame: no header, no rows. The construction path
    /// every level starts from.
    pub fn empty(empty_title: &'a str, empty_hint: &'a str) -> Self {
        Self {
            breadcrumb: &[],
            header: None,
            tracks: &[],
            rows: &[],
            track_menu: None,
            empty_title,
            empty_hint,
        }
    }
}

/// Render the detail column and append observed [`DetailAction`]s. No
/// scroll memory: the track list keeps its own positional state (the seam's
/// plain rendering path — goldens and widget tests).
pub fn show_detail_column(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    column: DetailColumn<'_>,
    actions: &mut Vec<DetailAction>,
) {
    show_detail_column_scrolled(ui, cache, palette, column, None, actions);
}

/// The app's render path: like [`show_detail_column`], but the track list's
/// `ScrollArea` takes a [`ScrollControl`] so the Tracks column resets to the
/// top on a selection change (the Scroll Memory holds no Tracks-column
/// offset). See [`super::scroll_memory`].
pub fn show_detail_column_scrolled(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    column: DetailColumn<'_>,
    scroll: Option<super::scroll_memory::ScrollControl>,
    actions: &mut Vec<DetailAction>,
) {
    // Resolved before the fields move out of `column` below.
    let has_content =
        column.header.is_some() || !column.tracks.is_empty() || !column.rows.is_empty();
    breadcrumb(ui, palette, column.breadcrumb, actions);
    if let Some(header) = column.header {
        album_header(ui, palette, header);
    }
    if !column.tracks.is_empty() {
        track_list(
            ui,
            cache,
            palette,
            scroll,
            column.tracks,
            column.track_menu,
            actions,
        );
    }
    for row in column.rows {
        let response = super::browser::detail_entity_row(ui, cache, palette, row);
        if response.clicked() {
            actions.push(DetailAction::SelectRow(row.key.clone()));
        }
    }
    // Nothing to render (no album selected yet, or a level with no entries):
    // the column says so under its breadcrumb instead of going blank. Every
    // app call site passes copy for this case.
    if !has_content {
        super::browser::empty_state(ui, palette, column.empty_title, column.empty_hint);
    }
}

/// The album header: the title over the muted subtitle, and nothing else.
/// It used to hang **Play all** and **Shuffle** off its right edge, painting
/// through a shared [`super::button::Variant::Secondary`] painter that had no
/// other reader; both the buttons and the painter are gone with them.
///
/// What replaces them is spacing, not a control. The 64px band and its
/// vertical centring stay — they are what places the block and what the track
/// list below is laid out against — but the horizontal gap that separated the
/// block from the buttons goes with them, and the two lines that remain are
/// one stack separated by the tightest step on the scale: a title and its own
/// subtitle read as one thing, so the gap between them is now stated rather
/// than inherited from the style.
fn album_header(ui: &mut egui::Ui, palette: &Palette, header: &AlbumHeader) {
    ui.allocate_ui(egui::vec2(ui.available_width(), 64.0), |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = theme::SPACE_XS;
                ui.heading(&header.title);
                if let Some(subtitle) = &header.subtitle {
                    ui.label(
                        egui::RichText::new(subtitle)
                            .text_style(egui::TextStyle::Small)
                            .color(palette.ink_2),
                    );
                }
            });
        });
    });
}

/// The album's track list: one shared 40px track row per track, the same
/// [`super::sidebar::tree_row`] shape every other track listing in the app
/// speaks, with its favorite control in the row's leading cell and the
/// `plays · time` cluster on the right. Rows cull to the visible
/// viewport; single click selects, double click starts the track, and a
/// right-click opens the shared Track menu — the same gestures every track
/// listing speaks. A row's favorite toggle lands here as
/// [`DetailAction::SetFavorite`], carrying the flag's NEW value, and its menu
/// as [`DetailAction::TrackMenu`], carrying the row's key.
///
/// The menu is BUILT PER ROW from the factory the host handed over, because one
/// of its props — the row's Favourite flag — is a fact about this row and not
/// about the column. Building one menu for the list and painting it on every
/// row would be a single answer to a per-row question.
fn track_list(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    scroll: Option<super::scroll_memory::ScrollControl>,
    tracks: &[TrackRow],
    track_menu: Option<&TrackMenuFactory<'_>>,
    actions: &mut Vec<DetailAction>,
) {
    // The track list is a field, painted by the list rather than by each row:
    // these rows are virtualized, so a row-carried fill would stop at the last
    // rendered row and leave the column below it on the card plane. See
    // [`super::browser::show_browser_list`] for the same paint on the listing
    // side. The cursor is already past the breadcrumb and the album header
    // here, so the plane starts below them: the album's own block stays on the
    // card and only the track rows move to the row plane.
    ui.painter().rect_filled(
        egui::Rect::from_min_size(ui.cursor().min, ui.available_size()),
        0.0,
        palette.surface_row,
    );
    let total = tracks.len();
    // `.animated(false)`: the track list is driven by a `ScrollControl`, so it
    // jumps to a named offset (a restored position, or the top after a
    // selection change) and a keep-in-view jump must not lerp. The flag
    // governs programmatic scroll-to offsets only, not wheel feel — the reason
    // is written once on [`ScrollControl`](super::scroll_memory::ScrollControl).
    let mut scroll_area = egui::ScrollArea::vertical()
        .auto_shrink(false)
        .animated(false);
    match scroll {
        Some(control) => {
            scroll_area = scroll_area.id_salt(control.salt);
            if let Some(offset) = control.start {
                scroll_area = scroll_area.vertical_scroll_offset(offset);
            }
        }
        None => scroll_area = scroll_area.id_salt("tracks_column_list"),
    }
    scroll_area.show_rows(
        ui,
        theme::geometry::sidebar::ROW_H,
        total,
        |ui, row_range| {
            for i in row_range {
                let Some(track) = tracks.get(i) else {
                    continue;
                };
                let row = super::sidebar::tree_row(
                    ui,
                    cache,
                    palette,
                    super::sidebar::TreeRow {
                        indent_level: 0,
                        icon: None,
                        cover: None,
                        label: &track.title,
                        count: None,
                        meta: Some(track.meta()),
                        favorite: Some(track.favorite),
                        selected: track.selected,
                        now_playing: track.now_playing,
                        playing: false,
                        art_slot: false,
                    },
                );
                if row.response.clicked() {
                    actions.push(DetailAction::SelectTrack(track.key.clone()));
                }
                if row.response.double_clicked() {
                    actions.push(DetailAction::SelectTrack(track.key.clone()));
                    actions.push(DetailAction::PlayTrack(track.key.clone()));
                }
                if let Some(favorite) = row.favorite_toggled {
                    actions.push(DetailAction::SetFavorite {
                        key: track.key.clone(),
                        favorite,
                    });
                }
                // The row's Track menu, from the shared menu module, so a
                // Track's actions do not depend on which Column is showing it.
                // It hangs off THIS row's own response identity — egui's
                // transient popup — so there is no per-row open state and
                // nothing to keep in step with an index, which is what lets the
                // culling this loop does cost the menu nothing. It is also why
                // the attach belongs here, inside the branch that MATERIALIZES a
                // row: a row outside the visible window is never created, so it
                // is never a candidate.
                if let Some(build_menu) = track_menu {
                    // This row's own menu, from the host's factory — so the
                    // Favourite item on it says what THIS row's click will do.
                    let menu = build_menu(track);
                    let mut intents = Vec::new();
                    if row
                        .response
                        .context_menu(|ui| {
                            super::menu::track_menu(ui, palette, &menu, &mut intents);
                        })
                        .is_some()
                    {
                        // Reported on the frame the menu OPENS, with an empty
                        // intent list: the host selects the Track off that
                        // report, and an item chosen later arrives on the same
                        // report with the key still attached.
                        actions.push(DetailAction::TrackMenu {
                            key: track.key.clone(),
                            intents,
                        });
                    }
                }
            }
        },
    );
}

/// The breadcrumb trail: one button per earlier level (clicking one reports
/// [`DetailAction::Crumb`] with its level), the current level as plain
/// text — the listener is already there.
fn breadcrumb(
    ui: &mut egui::Ui,
    palette: &Palette,
    crumbs: &[Crumb],
    actions: &mut Vec<DetailAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        let last = crumbs.len().saturating_sub(1);
        for (i, crumb) in crumbs.iter().enumerate() {
            if i > 0 {
                ui.label(
                    egui::RichText::new("/")
                        .text_style(egui::TextStyle::Small)
                        .color(palette.ink_3),
                );
            }
            if i == last {
                ui.label(
                    egui::RichText::new(&crumb.label)
                        .text_style(egui::TextStyle::Small)
                        .color(palette.ink),
                );
            } else {
                let button = egui::Button::new(
                    egui::RichText::new(&crumb.label)
                        .text_style(egui::TextStyle::Small)
                        .color(palette.ink_2),
                )
                .frame(false);
                if ui.add(button).clicked() {
                    actions.push(DetailAction::Crumb(i));
                }
            }
        }
    });
}
