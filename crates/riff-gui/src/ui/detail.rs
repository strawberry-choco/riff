//! The detail column (design-handoff issue 09): the middle pane of the
//! three-pane explorer. A breadcrumb trail over the drilled path, the album
//! header with **Play all** and **Shuffle**, and the album's track list: one
//! shared 40px [`super::sidebar::tree_row`] per track — the same shape the
//! All Tracks list speaks, favorite control included — with the
//! `Plays · Time` cluster on the right.
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
    /// The album header's **Play all**: start the album's tracks from the
    /// top, in order.
    PlayAll,
    /// The album header's **Shuffle**: start the album's tracks shuffled.
    Shuffle,
    /// An entity row (an album under an artist, an artist under a genre)
    /// was clicked, by its key. The caller resolves the level from the
    /// current selection — the widget stays identity-agnostic.
    SelectRow(String),
    /// A track-table row was selected, by its [`crate::riff_backend::domain::TrackId`]
    /// key.
    SelectTrack(String),
    /// A track-table row asked to start playing, by its id key.
    PlayTrack(String),
    /// The row's favorite control toggled the track's flag to `favorite`,
    /// by its id key.
    SetFavorite { key: String, favorite: bool },
}

/// The album header block: the album title over its muted artist · year
/// line, with the two playback actions beside it.
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
        album_header(ui, palette, header, actions);
    }
    if !column.tracks.is_empty() {
        track_list(ui, cache, palette, scroll, column.tracks, actions);
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

/// The album header: title over the muted subtitle, **Play all** and
/// **Shuffle** at the right edge.
fn album_header(
    ui: &mut egui::Ui,
    palette: &Palette,
    header: &AlbumHeader,
    actions: &mut Vec<DetailAction>,
) {
    ui.allocate_ui(egui::vec2(ui.available_width(), 64.0), |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_LG;
            ui.vertical(|ui| {
                ui.heading(&header.title);
                if let Some(subtitle) = &header.subtitle {
                    ui.label(
                        egui::RichText::new(subtitle)
                            .text_style(egui::TextStyle::Small)
                            .color(palette.ink_2),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if action_button(ui, palette, "Shuffle", "Shuffle this album") {
                    actions.push(DetailAction::Shuffle);
                }
                if action_button(ui, palette, "Play all", "Play the whole album") {
                    actions.push(DetailAction::PlayAll);
                }
            });
        });
    });
}

/// One of the header's playback action buttons: the visible text doubles as
/// the accessibility label. Returns whether it was clicked.
fn action_button(ui: &mut egui::Ui, palette: &Palette, text: &str, label: &str) -> bool {
    let button = egui::Button::new(
        egui::RichText::new(text)
            .text_style(egui::TextStyle::Small)
            .color(palette.ink),
    )
    .fill(palette.surface_2)
    .corner_radius(super::theme::RADIUS_SM);
    ui.add(button).on_hover_text(label).clicked()
}

/// The album's track list: one shared 40px track row per track, the same
/// [`super::sidebar::tree_row`] shape every other track listing in the app
/// speaks, with its favorite control in the row's leading cell and the
/// `plays · time` cluster on the right. Rows cull to the visible
/// viewport; single click selects, double click starts the track, the same
/// gestures every track listing speaks. A row's favorite toggle lands here as
/// [`DetailAction::SetFavorite`], carrying the flag's NEW value.
fn track_list(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    scroll: Option<super::scroll_memory::ScrollControl>,
    tracks: &[TrackRow],
    actions: &mut Vec<DetailAction>,
) {
    let total = tracks.len();
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
    scroll_area.show_rows(ui, super::sidebar::ROW_H, total, |ui, row_range| {
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
                    disclosure: None,
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
        }
    });
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
