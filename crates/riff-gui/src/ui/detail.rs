//! The detail column widget (design-handoff issue 09): what the elastic
//! stage's Tracks column paints. A bare track list — one shared 40px
//! [`super::sidebar::tree_row`] per track, the same shape the All Tracks
//! list speaks, favorite control included — with the `Plays · Time` cluster
//! on the right.
//!
//! The column states no identity of its own. The breadcrumb trail and the
//! album header that used to open it are gone, and nothing took their place
//! here: the album is already named on its selected row in the column that
//! listed it, and again in the right inspector's readout, so a third copy of
//! the title above the list repeated what two neighbouring surfaces already
//! said. What is left in this column is the one surface that acts — track
//! rows carry the favorite control and anchor the shared Track menu.
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
//! ([`super::menu::TrackMenuReport`]), answered by the same per-app Track-menu
//! host every other Track row reaches. It travels in its own channel
//! ([`DetailReport::TrackMenu`]) rather than as a [`DetailAction`], because
//! answering it needs the Playlist Store and the Inline Tag Editor, which the
//! host holds and this column's applier does not: a separate report type makes
//! that structural instead of a no-op arm. That report is also where this
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

/// What the user did to the detail column this frame that `app.rs` answers
/// through [`apply_detail_action`](crate::ui::app::apply_detail_action): the
/// reports that move a selection, start a Track, and commit a Favourite.
///
/// A Track row's **menu** report is deliberately not one of these — it is a
/// [`DetailReport::TrackMenu`], answered by the per-app Track-menu host. See
/// [`DetailReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailAction {
    /// A track-table row was selected, by its [`crate::riff_backend::domain::TrackId`]
    /// key.
    SelectTrack(String),
    /// A track-table row asked to start playing, by its id key.
    PlayTrack(String),
    /// The row's favorite control toggled the track's flag to `favorite`,
    /// by its id key.
    SetFavorite { key: String, favorite: bool },
    /// The track sort control chose `sort` as the listing's new order. The
    /// caller owns the session state and re-sorts the rows it hands over;
    /// the widget only reports the choice.
    TrackSortSelected(riff_backend::app::state::TrackSort),
}

/// Everything the detail column reported this frame, in the order it happened.
///
/// One channel, two kinds, because the two kinds are answered by two different
/// owners and must not be confused for each other: a [`DetailAction`] goes to
/// the detail-action applier, and a [`super::menu::TrackMenuReport`] goes to
/// the per-app Track-menu host. Keeping them in one `Vec` preserves the order
/// the user acted in, which a pair of separate channels would not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailReport {
    /// A report the detail-action applier answers.
    Action(DetailAction),
    /// A Track row's menu report, for the per-app Track-menu host.
    TrackMenu(super::menu::TrackMenuReport),
}

impl From<DetailAction> for DetailReport {
    fn from(action: DetailAction) -> Self {
        Self::Action(action)
    }
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
    /// The album's track list (the shared 40px track row, one per track);
    /// empty when the selection has none.
    pub tracks: &'a [TrackRow],
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
    /// The listing's sort mode, when the host offers the sort control at all:
    /// `Some` renders the track sort button above the list (only while the
    /// listing has tracks) and reports choices through
    /// [`DetailAction::TrackSortSelected`]; `None` is the plain rendering
    /// path (the widget tests that render bare columns). The rows themselves
    /// are sorted by the host before they arrive — the widget formats, it
    /// never re-orders.
    pub sort: Option<riff_backend::app::state::TrackSort>,
}

impl<'a> DetailColumn<'a> {
    /// An empty frame: no tracks, so the column paints nothing but its empty
    /// state. The construction path every listing starts from.
    pub fn empty(empty_title: &'a str, empty_hint: &'a str) -> Self {
        Self {
            tracks: &[],
            track_menu: None,
            empty_title,
            empty_hint,
            sort: None,
        }
    }
}

/// Render the detail column and append what it observed to `reports`.
///
/// One entry point, not two. This used to be a `None`-passing pass-through
/// beside the real one, and the pass-through had **zero production callers** —
/// every test that used it was a test that had bypassed the composition the
/// function exists to serve. A test that wants this column now passes `None`
/// here, exactly as the app passes `Some(control)`, so the two cannot drift.
pub fn show_detail_column_scrolled(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    column: DetailColumn<'_>,
    scroll: Option<super::scroll_memory::ScrollControl>,
    reports: &mut Vec<DetailReport>,
) {
    // Asked up front, because the fallback at the end of this function is what
    // a column with no tracks renders.
    let has_content = !column.tracks.is_empty();
    if let Some(sort) = column.sort
        && !column.tracks.is_empty()
    {
        // The sort control rides above the list, but only while there is
        // something to reorder — an empty listing hides it, like every other
        // column's sort control.
        if let Some(chosen) = super::browser::track_sort_row(ui, palette, sort) {
            reports.push(DetailAction::TrackSortSelected(chosen).into());
        }
    }
    if !column.tracks.is_empty() {
        track_list(
            ui,
            cache,
            palette,
            reduce_motion,
            scroll,
            column.tracks,
            column.track_menu,
            reports,
        );
    }
    // Nothing to render (no album selected yet, or a selection with no
    // tracks): the column says so at its top instead of going blank. Every
    // app call site passes copy for this case.
    if !has_content {
        super::browser::empty_state(ui, palette, column.empty_title, column.empty_hint);
    }
}

/// The album's track list: one shared 40px track row per track, the same
/// [`super::sidebar::tree_row`] shape every other track listing in the app
/// speaks, with its favorite control in the row's leading cell and the
/// `plays · time` cluster on the right. Rows cull to the visible
/// viewport; single click selects, double click starts the track, and a
/// right-click opens the shared Track menu — the same gestures every track
/// listing speaks. A row's favorite toggle lands here as
/// [`DetailAction::SetFavorite`], carrying the flag's NEW value, and its menu
/// as a [`DetailReport::TrackMenu`], carrying the row's key.
///
/// The menu is BUILT PER ROW from the factory the host handed over, because one
/// of its props — the row's Favourite flag — is a fact about this row and not
/// about the column. Building one menu for the list and painting it on every
/// row would be a single answer to a per-row question.
#[expect(
    clippy::too_many_arguments,
    reason = "the list's own data plus the global theme pair (palette, reduce_motion)"
)]
fn track_list(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    scroll: Option<super::scroll_memory::ScrollControl>,
    tracks: &[TrackRow],
    track_menu: Option<&TrackMenuFactory<'_>>,
    reports: &mut Vec<DetailReport>,
) {
    // The track list is a field, painted by the list rather than by each row:
    // these rows are virtualized, so a row-carried fill would stop at the last
    // rendered row and leave the column below it on the card plane. See
    // [`super::browser::show_browser_list`] for the same paint on the listing
    // side. The rect now starts at the column's top edge — with the breadcrumb
    // and the album header gone, nothing sits above the rows, so the plane runs
    // the full column and the first row lines up with the neighbouring columns'
    // first rows.
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
                    reduce_motion,
                    super::sidebar::TreeRow::track(
                        &track.title,
                        None,
                        Some(track.meta()),
                        Some(track.favorite),
                        track.selected,
                        track.now_playing,
                        false,
                    ),
                );
                if row.response.clicked() {
                    reports.push(DetailAction::SelectTrack(track.key.clone()).into());
                }
                if row.response.double_clicked() {
                    reports.push(DetailAction::SelectTrack(track.key.clone()).into());
                    reports.push(DetailAction::PlayTrack(track.key.clone()).into());
                }
                if let Some(favorite) = row.favorite_toggled {
                    reports.push(
                        DetailAction::SetFavorite {
                            key: track.key.clone(),
                            favorite,
                        }
                        .into(),
                    );
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
                        // report with the key still attached. It is the HOST's
                        // channel, not the detail applier's — answering it needs
                        // handles this column does not hold.
                        reports.push(DetailReport::TrackMenu(super::menu::TrackMenuReport {
                            key: track.key.clone(),
                            intents,
                        }));
                    }
                }
            }
        },
    );
}
