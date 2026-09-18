//! The selection panel (design-handoff issue 10), since the elastic-column
//! task the **inspector**: the stage's rightmost column, shown only while a
//! selection exists. A readout of the selected entity or track — its art,
//! title, subtitle, and a details list — with the orange primary play
//! action (**Play album** for an entity readout, **Play** for a track
//! readout that plays just that one track), plus **Add to Queue** in its
//! inspector form. A selection readout, not a view: it follows the live
//! selection, and the Now Playing view stays untouched.
//!
//! Pure widget seam, same discipline as [`crate::ui::browser`] and
//! [`crate::ui::detail`]: the widget paints from [`Palette`] tokens and
//! reports [`SelectionAction`]s instead of mutating app state; `app.rs`
//! applies them. Rendered headlessly in `tests/ui_tests.rs`.

use eframe::egui;
use riff_backend::domain::TrackId;
use std::path::PathBuf;

use super::icons::IconCache;
use super::theme::Palette;

/// What the user did to the selection panel this frame; `app.rs` applies
/// these to the sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionAction {
    /// The panel's primary play action: for an entity readout, start the
    /// selection's tracks from the top in order; for a single-track readout
    /// (`SelectionPanel::single`), play just that one track.
    PlayAlbum,
    /// The panel's **Add to Queue**: append the selection's track batch to
    /// the end of the playback queue, following the context menu's per-track
    /// queue precedent.
    Queue,
    /// Save the open inline editor's draft through the Tag Edits seam
    /// (the Save button or Enter).
    SaveTagEdit,
    /// Discard the open inline editor's draft (the Cancel button or Escape).
    CancelTagEdit,
    /// Enter edit mode for the current readout: the user clicked a tag row
    /// and the app opens the per-selection draft.
    StartEdit,
}

/// One item of the details list: the display values the app resolved from
/// the store — the widget formats, it never re-derives.
#[derive(Debug, Clone)]
pub struct SelectionDetail {
    pub label: String,
    pub value: String,
}

/// The seven editable tag fields, in the modal's stable order (Duration is
/// derived data and is never a tag row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    TrackNumber,
}

impl TagField {
    /// The seven fields in the stable modal order the tag section renders.
    pub const ALL: [TagField; 7] = [
        TagField::Title,
        TagField::Artist,
        TagField::Album,
        TagField::AlbumArtist,
        TagField::Genre,
        TagField::Year,
        TagField::TrackNumber,
    ];

    /// The row's display label, as the modal named the field.
    pub fn label(self) -> &'static str {
        match self {
            TagField::Title => "Title",
            TagField::Artist => "Artist",
            TagField::Album => "Album",
            TagField::AlbumArtist => "Album Artist",
            TagField::Genre => "Genre",
            TagField::Year => "Year",
            TagField::TrackNumber => "Track Number",
        }
    }

    /// The field's index into draft buffers and the model's stable order.
    pub const fn index(self) -> usize {
        match self {
            TagField::Title => 0,
            TagField::Artist => 1,
            TagField::Album => 2,
            TagField::AlbumArtist => 3,
            TagField::Genre => 4,
            TagField::Year => 5,
            TagField::TrackNumber => 6,
        }
    }
}

/// The resolved display state of one tag row: the shared value, the orange
/// `(different)` state, or the grey `(none)` state (a tag no track carries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagRowState {
    Value,
    Different,
    None,
}

/// One resolved tag row: the stable field, the display state (which drives
/// the color: ink / warning / muted ink — never a hardcoded color), the
/// resolved display text (`<value>` / `(different)` / `(none)`), and the
/// per-track original values in track order — the diff bases the inline
/// editor's draft owns (tickets 02/03), never fabricated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    pub field: TagField,
    pub state: TagRowState,
    pub text: String,
    pub originals: Vec<Option<String>>,
}

/// The inline editor's per-selection draft: the seven field buffers the
/// widget renders, prefilled from the resolved readout, plus the selection's
/// identity and the save-flow states the app layer owns. The widget edits
/// the buffers and reports Save/Cancel; the app layer owns the draft's
/// lifecycle — created when editing starts, discarded when the selection
/// leaves, submitted through the Tag Edits seam (ADR 0006).
#[derive(Debug, Clone)]
pub struct TagDraft {
    /// Which readout the draft belongs to: a Track's per-track draft, or an
    /// Album's dirty-only batch draft.
    pub kind: DraftKind,
    /// The single Track being edited (a [`DraftKind::Track`] draft); also the
    /// staleness key for track drafts.
    pub track_id: TrackId,
    /// The track's file path — the durable-change target (Track drafts).
    pub path: PathBuf,
    /// The album's track targets, one `(track, path)` per Track in store
    /// order (a [`DraftKind::Album`] draft). Empty for a Track draft.
    pub album_tracks: Vec<(TrackId, PathBuf)>,
    /// The seven field buffers in [`TagField::ALL`] order.
    pub fields: [String; 7],
    /// The readout value each buffer started from — an untouched buffer is
    /// exactly its original, so nothing is ever "dirty by construction" (a
    /// `(different)` / `(none)` row opens as an empty buffer against an
    /// empty original).
    pub originals: [String; 7],
    /// The inline failure reason: a failed save keeps the draft open with it.
    pub error: Option<String>,
    /// Whether a save is in flight: Save is disabled and the bar spins.
    pub saving: bool,
    /// The Album batch's outcome tallies while its requests are outstanding
    /// (None for Track drafts and idled Album drafts): the Save bar spins
    /// until [`BatchStatus::done`] and then shows the summary line.
    pub batch: Option<BatchStatus>,
    /// One-shot focus request for the first tag field (Issue 04's entry
    /// point): the "Edit Tags" context item opens the draft and asks the
    /// next rendered frame to focus Title; consumed the frame it lands.
    pub focus_first: bool,
}

/// Which readout the inline editor's draft belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftKind {
    /// A single Track readout: one durable change on Save.
    Track,
    /// An Album readout: saving submits one request per album Track, with
    /// only the dirty fields present — N durable changes (ticket 03).
    Album,
}

/// The Album batch's outcome tallies, resolved from polled outcomes in
/// submission order (the worker serializes the batch). The Save bar spins
/// while requests are outstanding and renders the summary line on completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStatus {
    pub total: usize,
    pub saved: usize,
    pub failed: usize,
    /// The first failure reason, kept only for the orange summary line.
    pub first_failure: Option<String>,
}

impl BatchStatus {
    /// Whether every request of the batch has landed.
    pub fn done(&self) -> bool {
        self.saved + self.failed == self.total
    }

    /// The completion line: "Saved N of M tracks", or "Saved N of M tracks —
    /// k failed: <reason>" for a partial failure. `None` while requests are
    /// outstanding.
    pub fn summary(&self) -> Option<String> {
        if self.done() {
            let base = format!("Saved {} of {} tracks", self.saved, self.total);
            Some(match (&self.first_failure, self.failed) {
                (Some(reason), _) => format!("{base} — {} failed: {reason}", self.failed),
                (None, 1..) => format!("{base} — {} failed", self.failed),
                (None, 0) => base,
            })
        } else {
            None
        }
    }
}

impl TagDraft {
    /// Open the editor for a track readout from its resolved rows: every
    /// buffer prefills from the displayed value, grey `(none)` rows opening
    /// empty (an unseen value can never be saved as if it were typed).
    pub fn for_track(track_id: TrackId, path: PathBuf, tags: &[TagRow]) -> Self {
        let mut draft = Self::blank(tags);
        draft.kind = DraftKind::Track;
        draft.track_id = track_id;
        draft.path = path;
        draft
    }

    /// Open the editor for an album readout from its resolved rows: the
    /// buffers prefill from the displayed aggregation (grey `(different)` /
    /// `(none)` rows open empty), and the per-track targets carry the album's
    /// tracks in store order for the dirty-only batch.
    pub fn for_album(album_tracks: Vec<(TrackId, PathBuf)>, tags: &[TagRow]) -> Self {
        let mut draft = Self::blank(tags);
        draft.kind = DraftKind::Album;
        draft.album_tracks = album_tracks;
        draft
    }

    /// Both constructors' shared shape: the prefilled buffers against
    /// identical originals, target-less and idle.
    fn blank(tags: &[TagRow]) -> Self {
        let mut fields = vec![String::new(); TagField::ALL.len()];
        let mut originals = vec![String::new(); TagField::ALL.len()];
        for row in tags {
            if row.state == TagRowState::Value {
                fields[row.field.index()].clone_from(&row.text);
                originals[row.field.index()].clone_from(&row.text);
            }
        }
        Self {
            kind: DraftKind::Track,
            track_id: TrackId(String::new()),
            path: PathBuf::new(),
            album_tracks: Vec::new(),
            fields: fields.try_into().expect("the seven tag fields stay seven"),
            originals: originals
                .try_into()
                .expect("the seven tag fields stay seven"),
            error: None,
            saving: false,
            batch: None,
            focus_first: false,
        }
    }

    /// Whether the field's buffer differs from the value the readout showed.
    pub fn is_dirty(&self, field: TagField) -> bool {
        self.fields[field.index()] != self.originals[field.index()]
    }

    /// Whether any of the seven buffers differ from the readout.
    pub fn any_dirty(&self) -> bool {
        TagField::ALL.iter().any(|&field| self.is_dirty(field))
    }
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
    /// The tag section rows (Title → Track Number), resolved by the caller —
    /// empty for readouts without tag rows (Artist / Genre entities). The
    /// widget renders the resolved text and state only; it never re-derives
    /// aggregation.
    pub tags: &'a [TagRow],
    /// The open inline editor's draft when the readout is being edited.
    /// Present, the seven tag rows render as in-place fields with the Save
    /// bar; `None` renders the read-only tag section. The widget edits the
    /// draft's buffers and reports the bar's actions — the app layer owns the
    /// draft and the request.
    pub editor: Option<&'a mut TagDraft>,
    /// Whether the readout is a single track (a track row single-clicked in
    /// any listing). The primary action then reads **Play** and plays just
    /// that one track; entity readouts read **Play album** and start the
    /// whole batch.
    pub single: bool,
    /// Whether the quick-action row renders **Add to Queue** beside the
    /// primary play action (the elastic-column inspector). `false` keeps the
    /// panel's original single Play album action — the rendering the
    /// `selection_panel` golden pins.
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
///
/// The header stays pinned and the readout under it scrolls: the body is
/// routinely taller than the inspector column (the 200 px art block plus
/// seven tag rows and the details grid), and the stage's column clips rather
/// than grows, which would leave the tail of the metadata unreachable.
pub fn show_selection_panel(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    panel: SelectionPanel<'_>,
    actions: &mut Vec<SelectionAction>,
) {
    header(ui, palette, panel.title.is_some());
    egui::ScrollArea::vertical()
        .id_salt("selection_panel_readout")
        .auto_shrink(false)
        .show(ui, |ui| {
            readout(ui, cache, palette, panel, actions);
        });
}

/// The panel's body: art, title line, quick actions, tag section, details.
fn readout(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    mut panel: SelectionPanel<'_>,
    actions: &mut Vec<SelectionAction>,
) {
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
            action_row(ui, cache, palette, panel.single, actions);
        } else {
            primary_play_button(ui, cache, palette, panel.single, actions);
        }
        ui.add_space(12.0);
        if let Some(editor) = panel.editor.as_deref_mut() {
            editor_section(ui, palette, editor, actions);
        } else if !panel.tags.is_empty() {
            tag_section(ui, palette, panel.tags, actions);
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

/// The primary play action: a full-width orange button — the brand fill with
/// its foreground ink — reporting [`SelectionAction::PlayAlbum`]. A track
/// readout (`single`) reads **Play** and plays just that one track; an entity
/// readout reads **Play album**. The visible text doubles as the
/// accessibility label.
fn primary_play_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    single: bool,
    actions: &mut Vec<SelectionAction>,
) {
    let width = ui.available_width();
    action_button(
        ui,
        cache,
        palette,
        width,
        play_quick_action(single),
        actions,
    );
}

/// The inspector's quick-action row: the primary play action and **Add to
/// Queue** side by side, each half the panel width. The visible texts double
/// as the accessibility labels; the queue action follows the context menu's
/// per-track queue precedent.
fn action_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    single: bool,
    actions: &mut Vec<SelectionAction>,
) {
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let half = (width - 8.0) / 2.0;
        action_button(ui, cache, palette, half, play_quick_action(single), actions);
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

/// The primary play action's spec: for a single-track readout it reads
/// **Play** and plays just that one track; for an entity readout it reads
/// **Play album** and starts the whole selection.
fn play_quick_action(single: bool) -> QuickAction {
    if single {
        QuickAction {
            text: "Play",
            icon: super::icons::Icon::Play,
            label: "Play this track",
            action: SelectionAction::PlayAlbum,
        }
    } else {
        QuickAction {
            text: "Play album",
            icon: super::icons::Icon::Play,
            label: "Play the whole album",
            action: SelectionAction::PlayAlbum,
        }
    }
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

/// The inline editor: one in-place field per tag field, prefilled from the
/// draft's buffers, an inline error line, and the Save bar (Save, spinner
/// while a write is in flight, Cancel). The widget edits the buffers and
/// reports the bar's actions; it never submits — the app layer owns the
/// request. **Enter** saves and **Esc** discards (REQ-UI-007); Tab moves
/// between fields through egui's default focus traversal.
fn editor_section(
    ui: &mut egui::Ui,
    palette: &Palette,
    draft: &mut TagDraft,
    actions: &mut Vec<SelectionAction>,
) {
    ui.label(
        egui::RichText::new("TAGS")
            .text_style(egui::TextStyle::Small)
            .color(palette.ink_3),
    );
    ui.add_space(4.0);
    for field in TagField::ALL {
        ui.label(
            egui::RichText::new(field.label())
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
        let response = ui.add(
            egui::TextEdit::singleline(&mut draft.fields[field.index()])
                .desired_width(ui.available_width()),
        );
        // The entry point (Issue 04) focuses the editor's first tag field;
        // the request lands on the next frame, so this one-shot flag is
        // consumed here rather than carrying state across frames.
        if field == TagField::Title && draft.focus_first {
            response.request_focus();
            draft.focus_first = false;
        }
        ui.add_space(DETAILS_ITEM_GAP);
    }

    if let Some(error) = &draft.error {
        ui.colored_label(palette.error, error);
        ui.add_space(DETAILS_ITEM_GAP);
    }

    // A batch save is in flight while any of its requests is outstanding
    // (ticket 03): the bar spins and Save stays disabled. An Album draft
    // with nothing dirty submits nothing, so its Save stays disabled too.
    let batch_in_flight = draft.batch.as_ref().is_some_and(|b| !b.done());
    let save_enabled =
        !(draft.saving || batch_in_flight || draft.kind == DraftKind::Album && !draft.any_dirty());

    ui.horizontal(|ui| {
        if ui
            .add_enabled(save_enabled, egui::Button::new("Save"))
            .clicked()
        {
            actions.push(SelectionAction::SaveTagEdit);
        }
        if ui.button("Cancel").clicked() {
            actions.push(SelectionAction::CancelTagEdit);
        }
        if draft.saving || batch_in_flight {
            ui.spinner();
        }
    });

    // The batch's outcome line lands under the bar once every request has:
    // "Saved N of M tracks", or "... — k failed: <reason>" in the warning
    // token. The widget formats the resolved tallies only.
    if let Some(batch) = &draft.batch
        && let Some(summary) = batch.summary()
    {
        let color = if batch.failed > 0 {
            palette.warning
        } else {
            palette.ink
        };
        ui.label(
            egui::RichText::new(summary)
                .text_style(egui::TextStyle::Small)
                .color(color),
        );
    }

    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        actions.push(SelectionAction::SaveTagEdit);
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(SelectionAction::CancelTagEdit);
    }
}

/// The tag section: one row per tag field — the field label on its own
/// muted line, the resolved display text beside it colored by state: ink for
/// a shared value, the warning token for `(different)`, muted ink for
/// `(none)`. Mirrors the details grid's label-above-value rhythm so the
/// readout stays scannable. The value is clickable: it reports
/// [`SelectionAction::StartEdit`] so the app opens the per-selection draft
/// (the same entry the retired modal's context item leads to).
fn tag_section(
    ui: &mut egui::Ui,
    palette: &Palette,
    tags: &[TagRow],
    actions: &mut Vec<SelectionAction>,
) {
    ui.label(
        egui::RichText::new("TAGS")
            .text_style(egui::TextStyle::Small)
            .color(palette.ink_3),
    );
    ui.add_space(4.0);
    for row in tags {
        let color = match row.state {
            TagRowState::Value => palette.ink,
            TagRowState::Different => palette.warning,
            TagRowState::None => palette.ink_3,
        };
        ui.label(
            egui::RichText::new(row.field.label())
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
        if ui
            .add(
                egui::Label::new(
                    egui::RichText::new(&row.text)
                        .text_style(egui::TextStyle::Small)
                        .color(color),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            actions.push(SelectionAction::StartEdit);
        }
        ui.add_space(DETAILS_ITEM_GAP);
    }
}

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
