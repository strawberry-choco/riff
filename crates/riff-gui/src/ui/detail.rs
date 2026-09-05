//! The detail column (design-handoff issue 09): the middle pane of the
//! three-pane explorer. A breadcrumb trail over the drilled path, the album
//! header with **Play all** and **Shuffle**, and the album's track table
//! (`# / Title / Plays / Time`) with a per-row favorite control.
//!
//! Pure widget seam, same discipline as [`crate::ui::browser`]: widgets
//! paint from [`Palette`] tokens and report [`DetailAction`]s instead of
//! mutating app state; `app.rs` applies them. Rendered headlessly in
//! `tests/ui_tests.rs`.

use eframe::egui;

use super::icons::IconCache;
use super::theme::Palette;

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
    /// The tagged track number; `None` falls back to the row's 1-based
    /// position in the table.
    pub number: Option<u32>,
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

/// One frame of the detail column: what to render and how.
pub struct DetailColumn<'a> {
    /// The drilled path, root first, current level last.
    pub breadcrumb: &'a [Crumb],
    /// The album header block; `None` above the album level (artist and
    /// genre detail render entity rows instead).
    pub header: Option<&'a AlbumHeader>,
    /// The album's track table (`# / Title / Plays / Time`); empty above
    /// the album level.
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

/// Render the detail column and append observed [`DetailAction`]s.
pub fn show_detail_column(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    column: DetailColumn<'_>,
    actions: &mut Vec<DetailAction>,
) {
    breadcrumb(ui, palette, column.breadcrumb, actions);
    if let Some(header) = column.header {
        album_header(ui, palette, header, actions);
    }
    if !column.tracks.is_empty() {
        track_table(ui, cache, palette, column.tracks, actions);
    }
    for row in column.rows {
        let response = super::browser::detail_entity_row(ui, cache, palette, row);
        if response.clicked() {
            actions.push(DetailAction::SelectRow(row.key.clone()));
        }
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
            ui.spacing_mut().item_spacing.x = 12.0;
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

/// Column width of the track table's `#` column: two to three digits of
/// room without shoving the titles right.
const NUMBER_COL_W: f32 = 28.0;

/// Column width of the track table's `Plays` and `Time` columns so every
/// row's values line up.
const VALUE_COL_W: f32 = 52.0;

/// Column width of the track table's favorite control.
const FAVORITE_COL_W: f32 = 24.0;

/// The album's track table: `# / Title / Plays / Time`, one selectable row
/// per track, each with its favorite control. Single click selects; double
/// click starts the track — the same gestures every track listing in the
/// app speaks.
fn track_table(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    tracks: &[TrackRow],
    actions: &mut Vec<DetailAction>,
) {
    egui::Grid::new("detail_track_table")
        .num_columns(5)
        .spacing([12.0, 2.0])
        .show(ui, |ui| {
            let head = |text: &str| {
                egui::RichText::new(text)
                    .text_style(egui::TextStyle::Small)
                    .color(palette.ink_3)
            };
            ui.add_sized([FAVORITE_COL_W, 18.0], egui::Label::new(""));
            ui.add_sized([NUMBER_COL_W, 18.0], egui::Label::new(head("#")));
            ui.add(egui::Label::new(head("Title")));
            ui.add_sized([VALUE_COL_W, 18.0], egui::Label::new(head("Plays")));
            ui.add_sized([VALUE_COL_W, 18.0], egui::Label::new(head("Time")));
            ui.end_row();

            for (i, track) in tracks.iter().enumerate() {
                track_row(ui, cache, palette, i, track, actions);
                ui.end_row();
            }
        });
}

/// One row of the track table. The title cell is the row's click target
/// and doubles as its accessibility label.
fn track_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    index: usize,
    track: &TrackRow,
    actions: &mut Vec<DetailAction>,
) {
    favorite_control(ui, cache, palette, track, actions);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a listing position is a small non-negative count"
    )]
    let number = track.number.unwrap_or_else(|| index as u32 + 1).to_string();
    ui.add_sized(
        [NUMBER_COL_W, 20.0],
        egui::Label::new(
            egui::RichText::new(number)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        ),
    );

    let title_color = if track.now_playing {
        palette.brand_primary
    } else {
        palette.ink
    };
    let title = egui::Label::new(
        egui::RichText::new(&track.title)
            .text_style(egui::TextStyle::Body)
            .color(title_color),
    )
    .sense(egui::Sense::click());
    let response = ui.add(title);
    // The title cell is a plain label — egui paints no focused state for it,
    // so the keyboard-focus ring is painted here (handoff issue 16).
    if let Some(ring) =
        super::theme::focus_ring_stroke(palette, ui.memory(|m| m.has_focus(response.id)))
    {
        ui.painter().rect_stroke(
            response.rect,
            super::theme::RADIUS_SM,
            ring,
            egui::StrokeKind::Inside,
        );
    }
    if response.clicked() {
        actions.push(DetailAction::SelectTrack(track.key.clone()));
    }
    if response.double_clicked() {
        actions.push(DetailAction::SelectTrack(track.key.clone()));
        actions.push(DetailAction::PlayTrack(track.key.clone()));
    }

    ui.add_sized(
        [VALUE_COL_W, 20.0],
        egui::Label::new(
            egui::RichText::new(track.plays.to_string())
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_2),
        ),
    );
    let time = track
        .duration
        .map_or_else(|| "\u{2014}".to_string(), super::playerbar::format_duration);
    ui.add_sized(
        [VALUE_COL_W, 20.0],
        egui::Label::new(
            egui::RichText::new(time)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_2),
        ),
    );
}

/// The row's favorite control (handoff issue 09): a heart in the brand tint
/// when the track IS a favorite, muted when not. Clicking reports
/// [`DetailAction::SetFavorite`] with the flag's NEW value — the caller
/// commits exactly that through the store's favorite setter.
fn favorite_control(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    track: &TrackRow,
    actions: &mut Vec<DetailAction>,
) {
    let (label, tint) = if track.favorite {
        ("Remove from Favorites", palette.brand_primary)
    } else {
        ("Add to Favorites", palette.ink_3)
    };
    let texture = cache.texture(ui.ctx(), super::icons::Icon::Heart, 14.0, tint);
    let button = egui::Button::image(egui::Image::new((texture, egui::vec2(14.0, 14.0))));
    let response = ui
        .add_sized([FAVORITE_COL_W, 20.0], button)
        .on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if response.clicked() {
        actions.push(DetailAction::SetFavorite {
            key: track.key.clone(),
            favorite: !track.favorite,
        });
    }
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
        ui.spacing_mut().item_spacing.x = 4.0;
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
