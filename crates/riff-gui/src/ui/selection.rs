//! The selection panel (design-handoff issue 10), since the elastic-column
//! task the **inspector**: the stage's rightmost column, shown only while a
//! selection exists. A READOUT of the selected entity or track — its art,
//! title, subtitle, the kind of thing it is, its tag rows, and a details
//! list. A selection readout, not a view: it follows the live selection, and
//! the Now Playing view stays untouched.
//!
//! A readout displays; it does not act. It used to carry a Play action and an
//! Add to Queue action; both are gone, with the buttons that reported them and
//! the two [`SelectionAction`] variants they carried. An entity's actions are
//! reachable from that entity's own context menu, and there is nothing here to
//! press. The one way INWARD is a tag row: clicking one opens the Inline Tag
//! Editor, an album batch draft applies to every Track of the album, and Save
//! and Cancel are unchanged.
//!
//! Pure widget seam, same discipline as [`crate::ui::browser`] and
//! [`crate::ui::detail`]: the widget paints from [`Palette`] tokens and
//! reports [`SelectionAction`]s instead of mutating app state; `app.rs`
//! applies them. Rendered headlessly in `tests/ui_tests.rs`.

use eframe::egui;
use riff_backend::domain::TrackId;
use std::path::PathBuf;

use super::icons::IconCache;
use super::theme::geometry::inspector::{ART_H, TAG_FIELD_H};
use super::theme::{self, Palette};

/// What the user did to the selection panel this frame; `app.rs` applies
/// these to the sessions.
///
/// Every variant here is TAG EDITING. The panel used to report a play action
/// and a queue action as well; both are gone with the buttons that reported
/// them, because an entity's actions live on that entity's own context menu and
/// a readout does not act.
///
/// The `Edit` postfix is kept deliberately. Three tag-editor intents are all
/// this enum holds now, so the variants look redundantly suffixed — but the
/// suffix is what tells them apart from the other action enums a host matches
/// in the same breath, and renaming public variants to satisfy a lint the
/// deletion merely exposed would be churn, not clarity.
#[expect(
    clippy::enum_variant_names,
    reason = "the Edit postfix is the seam's naming convention, kept stable"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionAction {
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
    /// The Track's measured gain in dB — edited like any other field, but
    /// traveling its own write path: `ReplayGain` is not Metadata.
    ReplayGainTrackGain,
    /// The Track's measured peak, a bare linear ratio.
    ReplayGainTrackPeak,
    /// The Album's shared gain in dB — one fact across the Album's Tracks;
    /// an edit on an Album readout is a Batch Tag Edit over every member.
    ReplayGainAlbumGain,
    /// The Album's shared peak, a bare linear ratio.
    ReplayGainAlbumPeak,
}

impl TagField {
    /// The eleven fields in the stable modal order the tag section renders:
    /// the seven Metadata fields, then the four `ReplayGain` values —
    /// which ride the same rows and the same door, but never the Metadata
    /// write path.
    pub const ALL: [TagField; 11] = [
        TagField::Title,
        TagField::Artist,
        TagField::Album,
        TagField::AlbumArtist,
        TagField::Genre,
        TagField::Year,
        TagField::TrackNumber,
        TagField::ReplayGainTrackGain,
        TagField::ReplayGainTrackPeak,
        TagField::ReplayGainAlbumGain,
        TagField::ReplayGainAlbumPeak,
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
            TagField::ReplayGainTrackGain => "Track Gain (dB)",
            TagField::ReplayGainTrackPeak => "Track Peak",
            TagField::ReplayGainAlbumGain => "Album Gain (dB)",
            TagField::ReplayGainAlbumPeak => "Album Peak",
        }
    }

    /// Whether this field is one of the `ReplayGain` values: validated and
    /// clamped differently from the metadata numerics, and written through
    /// `ReplayGain`'s own path rather than the Metadata one.
    pub const fn is_replaygain(self) -> bool {
        matches!(
            self,
            TagField::ReplayGainTrackGain
                | TagField::ReplayGainTrackPeak
                | TagField::ReplayGainAlbumGain
                | TagField::ReplayGainAlbumPeak
        )
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
            TagField::ReplayGainTrackGain => 7,
            TagField::ReplayGainTrackPeak => 8,
            TagField::ReplayGainAlbumGain => 9,
            TagField::ReplayGainAlbumPeak => 10,
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

/// The inline editor's per-selection draft: one field buffer per
/// [`TagField::ALL`] entry the widget renders, prefilled from the resolved
/// readout, plus the selection's
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
    /// The field buffers in [`TagField::ALL`] order.
    pub fields: Vec<String>,
    /// The readout value each buffer started from — an untouched buffer is
    /// exactly its original, so nothing is ever "dirty by construction" (a
    /// `(different)` / `(none)` row opens as an empty buffer against an
    /// empty original).
    pub originals: Vec<String>,
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
            fields,
            originals,
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

    /// Whether any buffer differs from the readout.
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
    /// What KIND of thing is being read out, already named for display:
    /// "Album", "Artist", "Genre", or "Track". The caller resolves it from
    /// whatever selected the readout, so this seam never learns the app's own
    /// kind type. The header chip says exactly this, which is why it is passed
    /// as a name rather than derived here.
    pub kind: &'a str,
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
}

/// The empty state's copy: what the panel says before any album has been
/// selected. The header still renders — the pane never blanks out.
const EMPTY_TITLE: &str = "Nothing selected";
const EMPTY_HINT: &str = "Select an album in the browser to see it here.";

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
    // The chip rides with the title: with nothing selected there is no kind to
    // name, and the empty state is not a readout of anything.
    header(ui, palette, panel.title.is_some().then_some(panel.kind));
    egui::ScrollArea::vertical()
        .id_salt("selection_panel_readout")
        .auto_shrink(false)
        .show(ui, |ui| {
            readout(ui, cache, palette, panel, actions);
        });
}

/// The panel's body: art, title line, tag section, details.
///
/// The rhythm is the readout's and nothing else's: the art, then the title
/// block, then the tag section, then the details grid, with the design's
/// [`theme::SPACE_LG`] at each BOUNDARY between them. The action row that used
/// to sit between the title block and the tag rows is gone, and so is the
/// divider that separated it — the space that divider occupied is the title
/// block's own trailing space, so removing the buttons closed the gap under
/// the title rather than opening a hole.
///
/// A boundary is a place where a section ENDS and the next one begins, so a
/// gap goes with one and only where there is one. A readout with no tag rows —
/// an Artist or a Genre — was being handed both of the twelve-pixel gaps back
/// to back with nothing between them, which put its first section a full gap
/// lower than every other readout's: 33 px of blank under the subtitle against
/// 21 px on a readout that has tags, and the panel's largest hole standing in
/// the one column that had nothing to fill it.
fn readout(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    mut panel: SelectionPanel<'_>,
    actions: &mut Vec<SelectionAction>,
) {
    if let Some(title) = panel.title {
        album_art(ui, palette, panel.art);
        ui.add_space(theme::SPACE_LG);
        ui.heading(title);
        if let Some(subtitle) = panel.subtitle {
            ui.label(
                egui::RichText::new(subtitle)
                    .text_style(egui::TextStyle::Small)
                    .color(palette.ink_2),
            );
        }
        if panel.editor.is_some() || !panel.tags.is_empty() {
            ui.add_space(theme::SPACE_LG);
            if let Some(editor) = panel.editor.as_deref_mut() {
                editor_section(ui, cache, palette, editor, actions);
            } else {
                tag_section(ui, palette, panel.tags, actions);
            }
        }
        ui.add_space(theme::SPACE_LG);
        details_list(ui, palette, panel.details);
    } else {
        ui.add_space(theme::SPACE_LG);
        ui.label(egui::RichText::new(EMPTY_TITLE).color(palette.ink_2));
        ui.label(
            egui::RichText::new(EMPTY_HINT)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
    }
}

/// The `SELECTION` header row: the muted caps label with the readout KIND's
/// chip at the right edge.
///
/// The chip is a plain label. It used to be an `egui::Button` with no click
/// handler at all, wearing a raised pill — a control that invited a press and
/// did nothing when given one, and whose text was the hardcoded string
/// "Album", so every kind of readout announced itself as an Album. A readout
/// displays; a label is what a readout wears. `kind` is `None` when there is no
/// readout, which is the only case in which the chip is not drawn.
fn header(ui: &mut egui::Ui, palette: &Palette, kind: Option<&str>) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new("SELECTION")
                    .text_style(egui::TextStyle::Small)
                    .color(palette.ink_3),
            );
        });
        if let Some(kind) = kind {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(kind).text_style(egui::TextStyle::Small));
            });
        }
    });
}

/// The album's art: the cover texture stretched over the design's 268×200
/// rounded block when one is loaded, a raised neutral block otherwise.
fn album_art(ui: &mut egui::Ui, palette: &Palette, art: Option<&egui::TextureHandle>) {
    let size = egui::vec2(ui.available_width(), ART_H);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    super::artwork::paint(
        ui.painter(),
        palette,
        &super::artwork::Artwork {
            rect,
            texture: art.map(egui::TextureHandle::id),
            fit: super::artwork::Fit::Fill,
            tint: palette.ink,
            placeholder: Some(super::artwork::Placeholder::Well {
                radius: super::theme::RADIUS_MD,
            }),
            border: None,
        },
    );
}

/// The inline editor: one in-place field per tag field, prefilled from the
/// draft's buffers, an inline error line, and the Save bar (Save, spinner
/// while a write is in flight, Cancel). The widget edits the buffers and
/// reports the bar's actions; it never submits — the app layer owns the
/// request. **Enter** saves and **Esc** discards (REQ-UI-007); Tab moves
/// between fields through egui's default focus traversal.
fn editor_section(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
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
        // One shared field owner for every typed value in the shell: the same
        // well, the same focus ring, the same truncation. A tag row carries no
        // leading glyph and no clear affordance, and Escape belongs to the
        // editor below (it discards the whole draft) rather than emptying the
        // row the cursor happens to sit on.
        //
        // The well is a full [`TAG_FIELD_H`] tall rather than the bare
        // one-line interact height: the inner text rides a 4px inset, so a
        // one-line-tall well left no room and the text clipped on the vertical
        // axis. TAG_FIELD_H is the shared input-well height, so a tag field
        // sits and reads like the search boxes.
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), TAG_FIELD_H),
            egui::Sense::hover(),
        );
        let response = super::text_field::text_field(
            ui,
            cache,
            palette,
            &mut draft.fields[field.index()],
            &super::text_field::TextField {
                id: egui::Id::new(("tag_editor_field", field.index())),
                rect,
                hint: "",
                leading_icon: None,
                clear_label: None,
                dismiss_on_escape: false,
            },
        );
        // The entry point (Issue 04) focuses the editor's first tag field;
        // the request lands on the next frame, so this one-shot flag is
        // consumed here rather than carrying state across frames.
        if field == TagField::Title && draft.focus_first {
            response.request_focus();
            draft.focus_first = false;
        }
        ui.add_space(theme::SPACE_MD);
    }

    if let Some(error) = &draft.error {
        ui.colored_label(palette.error, error);
        ui.add_space(theme::SPACE_MD);
    }

    // A batch save is in flight while any of its requests is outstanding
    // (ticket 03): the bar spins and Save stays disabled. An Album draft
    // with nothing dirty submits nothing, so its Save stays disabled too.
    let batch_in_flight = draft.batch.as_ref().is_some_and(|b| !b.done());
    let save_enabled =
        !(draft.saving || batch_in_flight || draft.kind == DraftKind::Album && !draft.any_dirty());

    save_bar(ui, cache, palette, draft, save_enabled, actions);

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
        ui.add_space(theme::SPACE_MD);
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
        ui.add_space(theme::SPACE_MD);
    }
}

/// The editor's Save bar: the affirmative Primary save — disabled while a write
/// is in flight, and for an Album draft while nothing is dirty, so a
/// double-submit is impossible — the neutral Cancel, and the spinner while a
/// batch is outstanding. The bar reports actions; submitting is the host's.
fn save_bar(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    draft: &TagDraft,
    save_enabled: bool,
    actions: &mut Vec<SelectionAction>,
) {
    let batch_in_flight = draft.batch.as_ref().is_some_and(|batch| !batch.done());
    ui.horizontal(|ui| {
        // One button owner: an affirmative Primary save that a disabled state
        // strips of both click and focus, and a neutral Cancel.
        let (rect, _) = ui.allocate_exact_size(
            super::button::text_button_size(ui, palette, "Save", false),
            egui::Sense::hover(),
        );
        if super::button::text_button(
            ui,
            cache,
            palette,
            &super::button::TextButton {
                id: egui::Id::new("tag_editor_save"),
                rect,
                label: "Save",
                a11y: "Save",
                tooltip: None,
                icon: None,
                small: false,
                variant: super::button::Variant::Primary,
                enabled: save_enabled,
            },
        ) {
            actions.push(SelectionAction::SaveTagEdit);
        }
        let (rect, _) = ui.allocate_exact_size(
            super::button::text_button_size(ui, palette, "Cancel", false),
            egui::Sense::hover(),
        );
        if super::button::text_button(
            ui,
            cache,
            palette,
            &super::button::TextButton {
                id: egui::Id::new("tag_editor_cancel"),
                rect,
                label: "Cancel",
                a11y: "Cancel",
                tooltip: None,
                icon: None,
                small: false,
                variant: super::button::Variant::Caption,
                enabled: true,
            },
        ) {
            actions.push(SelectionAction::CancelTagEdit);
        }
        if draft.saving || batch_in_flight {
            ui.spinner();
        }
    });
}
