//! The selection panel (design-handoff issue 10), since the elastic-column
//! task the **inspector**: the stage's rightmost column, shown only while a
//! selection exists. A readout of the selected entity or track — its art,
//! title, subtitle, and a details list — with the orange **Play album**
//! action, plus **Add to Queue** in its inspector form. A selection readout,
//! not a view: it follows the live selection, and the Now Playing view stays
//! untouched.
//!
//! Pure widget seam, same discipline as [`crate::ui::browser`] and
//! [`crate::ui::detail`]: the widget paints from [`Palette`] tokens and
//! reports [`SelectionAction`]s instead of mutating app state; `app.rs`
//! applies them. Rendered headlessly in `tests/ui_tests.rs`.

use eframe::egui;

use super::icons::IconCache;
use super::theme::Palette;

/// What the user did to the selection panel this frame; `app.rs` applies
/// these to the sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionAction {
    /// The panel's **Play album**: start the selected album's tracks from
    /// the top, in order.
    PlayAlbum,
    /// The panel's **Add to Queue**: append the selection's track batch to
    /// the end of the playback queue, following the context menu's per-track
    /// queue precedent.
    Queue,
}

/// One item of the details list: the display values the app resolved from
/// the store — the widget formats, it never re-derives.
#[derive(Debug, Clone)]
pub struct SelectionDetail {
    pub label: String,
    pub value: String,
}

/// One frame of the selection panel: what to render and how. `title: None`
/// is the clear empty state — nothing has been selected yet.
pub struct SelectionPanel<'a> {
    /// The album's cover texture from the UI's texture LRU; `None` renders
    /// the neutral placeholder block.
    pub art: Option<&'a egui::TextureHandle>,
    /// The album title; `None` renders the empty state.
    pub title: Option<&'a str>,
    /// `"Artist · Year"`-style secondary line.
    pub subtitle: Option<&'a str>,
    /// The details list rows, resolved by the caller.
    pub details: &'a [SelectionDetail],
    /// Whether the quick-action row renders **Add to Queue** beside **Play
    /// album** (the elastic-column inspector). `false` keeps the panel's
    /// original single Play album action — the rendering the `selection_panel`
    /// golden pins.
    pub queue: bool,
}

/// The empty state's copy: what the panel says before any album has been
/// selected. The header still renders — the pane never blanks out.
const EMPTY_TITLE: &str = "Nothing selected";
const EMPTY_HINT: &str = "Select an album in the browser to see it here.";

/// Art block height (design: the 268×200 cover block under the header).
const ART_H: f32 = 200.0;
/// Height of the Play album button (design: the 32px action row).
const PLAY_H: f32 = 32.0;

/// Render the selection panel and append observed [`SelectionAction`]s.
pub fn show_selection_panel(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    panel: SelectionPanel<'_>,
    actions: &mut Vec<SelectionAction>,
) {
    header(ui, palette, panel.title.is_some());
    if let Some(title) = panel.title {
        album_art(ui, palette, panel.art);
        ui.add_space(12.0);
        ui.heading(title);
        if let Some(subtitle) = panel.subtitle {
            ui.label(
                egui::RichText::new(subtitle)
                    .text_style(egui::TextStyle::Small)
                    .color(palette.ink_2),
            );
        }
        ui.add_space(12.0);
        if panel.queue {
            action_row(ui, cache, palette, actions);
        } else {
            play_album_button(ui, cache, palette, actions);
        }
        ui.add_space(12.0);
        details_list(ui, palette, panel.details);
    } else {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(EMPTY_TITLE).color(palette.ink_2));
        ui.label(
            egui::RichText::new(EMPTY_HINT)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
    }
}

/// The `SELECTION` header row: the muted caps label with the selection
/// kind's chip at the right edge (design: `Album` on a raised pill).
fn header(ui: &mut egui::Ui, palette: &Palette, with_chip: bool) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new("SELECTION")
                    .text_style(egui::TextStyle::Small)
                    .color(palette.ink_3),
            );
        });
        if with_chip {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let chip = egui::Button::new(
                    egui::RichText::new("Album")
                        .text_style(egui::TextStyle::Small)
                        .color(palette.ink),
                )
                .fill(palette.surface_2)
                .corner_radius(super::theme::RADIUS_SM);
                ui.add(chip);
            });
        }
    });
}

/// The album's art: the cover texture stretched over the design's 268×200
/// rounded block when one is loaded, a raised neutral block otherwise.
fn album_art(ui: &mut egui::Ui, palette: &Palette, art: Option<&egui::TextureHandle>) {
    let size = egui::vec2(ui.available_width(), ART_H);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if let Some(texture) = art {
        let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
        ui.painter().image(texture.id(), rect, uv, palette.ink);
    } else {
        ui.painter()
            .rect_filled(rect, super::theme::RADIUS_MD, palette.surface_2);
    }
}

/// The **Play album** action: a full-width orange button — the brand fill
/// with its foreground ink — reporting [`SelectionAction::PlayAlbum`]. The
/// visible text doubles as the accessibility label.
fn play_album_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    actions: &mut Vec<SelectionAction>,
) {
    let width = ui.available_width();
    action_button(
        ui,
        cache,
        palette,
        width,
        QuickAction {
            text: "Play album",
            icon: super::icons::Icon::Play,
            label: "Play the whole album",
            action: SelectionAction::PlayAlbum,
        },
        actions,
    );
}

/// The inspector's quick-action row: **Play album** and **Add to Queue**
/// side by side, each half the panel width. The visible text doubles as the
/// accessibility labels; the queue action follows the context menu's
/// per-track queue precedent.
fn action_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    actions: &mut Vec<SelectionAction>,
) {
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let half = (width - 8.0) / 2.0;
        action_button(
            ui,
            cache,
            palette,
            half,
            QuickAction {
                text: "Play album",
                icon: super::icons::Icon::Play,
                label: "Play the whole album",
                action: SelectionAction::PlayAlbum,
            },
            actions,
        );
        action_button(
            ui,
            cache,
            palette,
            half,
            QuickAction {
                text: "Add to Queue",
                icon: super::icons::Icon::ListMusic,
                label: "Add this selection's tracks to the queue",
                action: SelectionAction::Queue,
            },
            actions,
        );
    });
}

/// One quick-action button's spec: the visible text (doubling as the
/// accessibility label), its icon, the hover label, and the action it
/// reports.
struct QuickAction {
    text: &'static str,
    icon: super::icons::Icon,
    label: &'static str,
    action: SelectionAction,
}

/// One primary action button at `width` wide: the brand fill with its
/// foreground ink, reporting `spec.action` when clicked.
fn action_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    width: f32,
    spec: QuickAction,
    actions: &mut Vec<SelectionAction>,
) {
    let QuickAction {
        text,
        icon,
        label,
        action,
    } = spec;
    let texture = cache.texture(ui.ctx(), icon, 14.0, palette.on_brand);
    let button = egui::Button::image_and_text(
        egui::Image::new((texture, egui::vec2(14.0, 14.0))),
        egui::RichText::new(text)
            .text_style(egui::TextStyle::Body)
            .color(palette.on_brand),
    )
    .fill(palette.brand_primary)
    .corner_radius(super::theme::RADIUS_SM);
    if ui
        .add_sized([width, PLAY_H], button)
        .on_hover_text(label)
        .clicked()
    {
        actions.push(action);
    }
}

/// Vertical gap between two detail items.
const DETAILS_ITEM_GAP: f32 = 8.0;

/// The details list: one muted label on its own line, its value on the next,
/// both hugging the panel's left edge; the gap between items keeps the
/// stacked readout scannable (design: the label above the value it names).
fn details_list(ui: &mut egui::Ui, palette: &Palette, details: &[SelectionDetail]) {
    ui.label(
        egui::RichText::new("DETAILS")
            .text_style(egui::TextStyle::Small)
            .color(palette.ink_3),
    );
    ui.add_space(4.0);
    for detail in details {
        ui.label(
            egui::RichText::new(&detail.label)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
        ui.label(
            egui::RichText::new(&detail.value)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink),
        );
        ui.add_space(DETAILS_ITEM_GAP);
    }
}
