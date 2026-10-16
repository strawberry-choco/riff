//! The Smart Playlists page — Settings' editor for Custom Smart Playlists
//! (ticket 15; ADR 0021: rules are collection data, Settings is only where
//! they are edited).
//!
//! Two columns while the pane is wide enough to split — the listener's saved
//! Custom Smart Playlists down the left, the Rules they are writing on the
//! right, and the name typed as they write — collapsing to one stacked column
//! below [`MIN_TWO_COL_W`](riff_gui_theme::theme::geometry::settings::MIN_TWO_COL_W),
//! exactly the branch the Library pane already draws ([`settings_pane_columns`]).
//!
//! **The editor state is frontend-local and this module owns its shape.**
//! [`SmartPlaylistsEditor`] is one `RiffApp` field, the same owner-held-draft
//! pattern as the inline tag editor: nothing here is persisted, which is what
//! keeps "a list appears in the sidebar only at Save" true, and what keeps the
//! last-selected Settings section (also `RiffApp`-local) free of any new
//! stored state.
//!
//! **The handoff.** Tickets 16–20 build *inside* this pane, and the boundary
//! is deliberate:
//!
//! - *Ticket 16 (Rule Box nesting)* owns the representation of a nested Box —
//!   the draft list here is the Root Box's siblings only, and a Box child of
//!   the loaded tree is documented at the load site as 16's to represent.
//! - *Ticket 17 (the live match count)* hangs its count line off this pane's
//!   header slot; the row identity contract (slot index) is what it keys on.
//! - *Ticket 18 (draft + save/cancel + reasons)* owns dirtiness, the
//!   reconcile hook, the Unreadable Rule sibling state, and every refusal
//!   decision. This pane only puts the Save and Cancel buttons down and emits
//!   their typed intents, and it OWNS the *rendering* half of the refusal
//!   slot ([`SmartPlaylistsEditor::refusal`]) handed from ticket 14 — the tag
//!   editor's single-slot, self-naming copy painted after all rows is the
//!   precedent being shared, not its state.
//! - *Ticket 19 (reorder)* keys drags off the same row identity; ticket 14's
//!   row hands back its response rect for exactly that.
//!
//! The two duties ticket 14's row cannot perform itself are performed here:
//! an opened selector list scrolls its row into view (`scroll_to_me`, which
//! egui 0.35 does not do for a clipped, keyboard-focused row), and the picker
//! read is requested once per [`RuleIntent::PickerOpened`], never per frame —
//! the answer lands through `RiffApp`'s apply step and reaches the row as the
//! [`RulePickerValues`] prop.

use crate::ui::browser;
use crate::ui::rule_row::{RuleDraft, RuleIntent, RulePanel, RulePickerValues, rule_row};
use crate::ui::settings::{
    LibraryPaneColumns, SettingsAction, SettingsContent, settings_pane_columns,
};
use crate::ui::sidebar::{self, TreeRow};
use riff_backend::app::store::CustomSmartPlaylist;
use riff_backend::domain::field_catalog::{ValueType, spec};
use riff_backend::domain::{MatchableField, PlaylistId, RootBox, RuleChild};
use riff_gui_components::button::{TextButton, Variant, text_button, text_button_size};
use riff_gui_components::text_field::{TextField, text_field};
use riff_gui_theme::icons::{Icon, IconCache};
use riff_gui_theme::theme::geometry::rule_row::WELL_H;
use riff_gui_theme::theme::geometry::settings::SECTION_GAP;
use riff_gui_theme::theme::{self, Palette};

/// The Library values the pane read for one open picker, held as the editor's
/// own buffers (the row takes them as a borrowed prop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartPlaylistPickerValues {
    /// The text half — `picker_texts` for a pickable text field.
    Texts(Vec<String>),
    /// The numeric half — `picker_numbers` for a pickable numeric field.
    Numbers(Vec<i64>),
}

impl SmartPlaylistPickerValues {
    /// The same values as the prop [`rule_row`] takes.
    #[must_use]
    pub fn prop(&self) -> RulePickerValues<'_> {
        match self {
            Self::Texts(values) => RulePickerValues::Texts(values),
            Self::Numbers(values) => RulePickerValues::Numbers(values),
        }
    }
}

/// The Smart Playlists pane's editor: one unsaved draft of one list, held by
/// `RiffApp` and owned here. Fresh frames mutate nothing; a gesture does.
///
/// The Default is the **new, empty, unsaved draft** — which is exactly what
/// the sidebar's `+` and the page's own button both open, so "opens a new
/// draft" is one call ([`Self::start_new`]) with no second state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartPlaylistsEditor {
    /// The saved list this draft was written against, or `None` while the
    /// draft is a new list nothing has saved yet.
    pub selected: Option<PlaylistId>,
    /// The name being typed. It reaches the store only through ticket 18's
    /// Save — the sidebar shows a list only after that write.
    pub name: String,
    /// The Root Box's Rules, in the listener's one shared sibling order.
    /// Row identity is the slot index, per ticket 14's host contract.
    /// Nested Rule Boxes are ticket 16's to represent beside this list.
    pub drafts: Vec<RuleDraft>,
    /// The values read for the currently open library picker, if one is open.
    pub picker: Option<SmartPlaylistPickerValues>,
    /// The field a row reported [`RuleIntent::PickerOpened`] for this frame
    /// and has not been answered yet: `RiffApp`'s apply step reads the values
    /// once and clears this. Nothing on the paint path reaches a store.
    pub pending_picker: Option<MatchableField>,
    /// Why the last write was refused, painted after all rows and above the
    /// Save line — the single-slot, self-naming rendering the tag editor is
    /// the precedent for. This pane renders it; deciding what fills it is
    /// ticket 18's.
    pub refusal: Option<String>,
}

impl Default for SmartPlaylistsEditor {
    fn default() -> Self {
        Self {
            selected: None,
            name: String::new(),
            drafts: vec![RuleDraft::blank()],
            picker: None,
            pending_picker: None,
            refusal: None,
        }
    }
}

impl SmartPlaylistsEditor {
    /// Switch to a **new, empty, unsaved** draft: no list is selected, no
    /// name is typed, one blank Rule row is ready to write, and nothing is
    /// written anywhere — the list first appears in the sidebar at Save,
    /// which is ticket 18's, not this call's.
    pub fn start_new(&mut self) {
        *self = Self::default();
    }

    /// Whether this draft is a new list (nothing selected, nothing saved
    /// against it yet).
    #[must_use]
    pub fn is_new(&self) -> bool {
        self.selected.is_none()
    }

    /// Append one blank Rule row — the shape ticket 16 grows into a box-aware
    /// "add Rule or Box".
    pub fn add_rule(&mut self) {
        self.drafts.push(RuleDraft::blank());
    }

    /// Load a saved list into the editor as the draft to edit: its name, and
    /// the Root Box's Rules, in the stored sibling order.
    ///
    /// Only [`RuleChild::Rule`] children become rows today: a nested
    /// [`RuleChild::Box`] has no draft representation until ticket 16 gives
    /// the pane box nesting, and silently flattening one would rewrite the
    /// listener's tree on the next Save — so it is skipped here, loudly owned
    /// there, and no in-repo editor can have produced such a tree yet.
    pub fn load_saved(&mut self, list: &CustomSmartPlaylist, root: Option<&RootBox>) {
        let drafts = root.map_or_else(Vec::new, |root| {
            root.children()
                .iter()
                .filter_map(|child| match child {
                    RuleChild::Rule(rule) => Some(RuleDraft::from_rule(rule)),
                    RuleChild::Box(_) => None,
                })
                .collect()
        });
        self.selected = Some(list.id.clone());
        self.name.clone_from(&list.name);
        self.drafts = drafts;
        self.picker = None;
        self.pending_picker = None;
        self.refusal = None;
    }

    /// One row's library picker opened for `field` at `row`: remember the
    /// read the host owes (once, through [`Self::pending_picker`]) and close
    /// any other row's picker — the pane shows one picker list at a time,
    /// because two would answer to one shared value list of the wrong field.
    pub fn picker_opened(&mut self, field: MatchableField, row: usize) {
        if let Some(other) = self.drafts.get_mut(row) {
            other.open = Some(RulePanel::Value);
        }
        for (index, draft) in self.drafts.iter_mut().enumerate() {
            if index != row && draft.open == Some(RulePanel::Value) {
                draft.open = None;
            }
        }
        self.pending_picker = Some(field);
        self.picker = None;
    }

    /// The library picker closed: drop the values, keep nothing.
    pub fn picker_closed(&mut self) {
        self.picker = None;
        self.pending_picker = None;
    }

    /// Answer this frame's pending picker read with the values the store
    /// served, and mark it answered.
    pub fn picker_answered(&mut self, values: SmartPlaylistPickerValues) {
        self.pending_picker = None;
        self.picker = Some(values);
    }
}

/// Which picker values a row asked for: the catalog's `Number` fields read
/// through the numeric picker, every other pickable field through the text
/// one. The question is asked of the catalog, never restated here.
#[must_use]
pub fn picker_is_numeric(field: MatchableField) -> bool {
    matches!(spec(field).value_type(), ValueType::Number)
}

/// Draw the Smart Playlists section's pane: the listener's lists down the
/// left, the Rules being written on the right, collapsing to one stacked
/// column below the two-column boundary the Library pane already honors.
///
/// `content.custom_smart_playlists` is the frame's enumeration through the
/// Session Views seam; `editor` is the `RiffApp`-held draft. The pane mutates
/// only the draft's typed buffers (in place, the row's own contract) and
/// reports every gesture as a typed [`SettingsAction`] for `RiffApp`'s apply
/// step — Save, Cancel and their meanings included, which stay ticket 18's.
pub fn smart_playlists_pane(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    editor: &mut SmartPlaylistsEditor,
    actions: &mut Vec<SettingsAction>,
) {
    match settings_pane_columns(ui.available_width()) {
        LibraryPaneColumns::Stacked { width } => {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(width, ui.available_height()),
                    ))
                    .id_salt("smart-playlists-column"),
                |ui| {
                    lists_column(ui, cache, palette, content, editor, actions);
                    ui.add_space(SECTION_GAP);
                    editor_column(ui, cache, palette, content, editor, actions);
                },
            );
        }
        LibraryPaneColumns::TwoColumns { left, right } => {
            ui.horizontal_top(|ui| {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(left, ui.available_height()),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt(("smart-playlists-column", 0)),
                    |ui| {
                        lists_column(ui, cache, palette, content, editor, actions);
                    },
                );
                // The left scope's used rect advances the horizontal flow, so
                // the cursor already names the right column's left edge — the
                // exact idiom the Library pane allocates its columns with.
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(right, ui.available_height()),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt(("smart-playlists-column", 1)),
                    |ui| {
                        editor_column(ui, cache, palette, content, editor, actions);
                    },
                );
            });
        }
    }
}

/// The left column: the listener's saved lists, and the page's own way to
/// start one — so the listener who never finds the sidebar `+` still has a
/// route, and the empty state is never a dead end.
fn lists_column(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    editor: &SmartPlaylistsEditor,
    actions: &mut Vec<SettingsAction>,
) {
    sidebar::section_header(ui, palette, "Smart Playlists");
    ui.add_space(theme::SPACE_SM);
    if pane_button(
        ui,
        cache,
        palette,
        content.reduce_motion,
        "New smart playlist",
        egui::Id::new("smart_playlists_new"),
        Some(Icon::Plus),
        Variant::Primary,
    ) {
        actions.push(SettingsAction::NewSmartPlaylist);
    }
    ui.add_space(theme::SPACE_MD);

    if content.custom_smart_playlists.is_empty() {
        browser::empty_state(
            ui,
            palette,
            "No smart playlists yet",
            "Press \"New smart playlist\" above to write the first one.",
        );
        return;
    }

    for list in &content.custom_smart_playlists {
        let row = sidebar::tree_row(
            ui,
            cache,
            palette,
            content.reduce_motion,
            TreeRow::nav(
                list.name.as_str(),
                Some(Icon::Sparkles),
                None,
                editor.selected.as_ref() == Some(&list.id),
            ),
        );
        if row.response.clicked() {
            actions.push(SettingsAction::SelectSmartPlaylist(list.id.clone()));
        }
    }
}

/// The right column: the name typed as the Rules are written, the Rule rows
/// themselves (ticket 14's row, this pane the host), and the Save/Cancel line
/// whose *meanings* are ticket 18's — the buttons and their intents are here.
fn editor_column(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    editor: &mut SmartPlaylistsEditor,
    actions: &mut Vec<SettingsAction>,
) {
    let title = if editor.is_new() {
        "New smart playlist"
    } else {
        "Edit smart playlist"
    };
    ui.label(egui::RichText::new(title).color(palette.ink_2));
    ui.add_space(theme::SPACE_XS);

    let name_w = ui.available_width();
    let (_, name_response) =
        ui.allocate_exact_size(egui::vec2(name_w, WELL_H), egui::Sense::hover());
    text_field(
        ui,
        cache,
        palette,
        &mut editor.name,
        &TextField {
            id: egui::Id::new("smart_playlist_name_field"),
            rect: name_response.rect,
            hint: "Name this playlist",
            leading_icon: None,
            clear_label: None,
            dismiss_on_escape: false,
        },
    );
    ui.add_space(theme::SPACE_MD);

    sidebar::section_header(ui, palette, "Rules");
    ui.add_space(theme::SPACE_SM);

    for index in 0..editor.drafts.len() {
        let row_id = egui::Id::new(("smart_playlist_rule_row", index));
        let picker = if editor.drafts[index].open == Some(RulePanel::Value) {
            editor.picker.as_ref().map(SmartPlaylistPickerValues::prop)
        } else {
            None
        };
        let outcome = rule_row(
            ui,
            cache,
            palette,
            row_id,
            &mut editor.drafts[index],
            picker.as_ref(),
        );
        let mut opened = false;
        for intent in &outcome.intents {
            editor.drafts[index].apply(intent);
            match intent {
                RuleIntent::PanelToggled { open, .. } if *open => opened = true,
                RuleIntent::PickerOpened { field } => {
                    editor.picker_opened(*field, index);
                }
                RuleIntent::PickerClosed => editor.picker_closed(),
                _ => {}
            }
        }
        // Host duty handed from ticket 14: egui 0.35 never auto-scrolls a
        // keyboard-focused row inside this clipped pane, so the row that
        // just opened a list scrolls itself into view here.
        if opened {
            outcome.response.scroll_to_me(Some(egui::Align::Center));
        }
    }

    if pane_button(
        ui,
        cache,
        palette,
        content.reduce_motion,
        "Add rule",
        egui::Id::new("smart_playlists_add_rule"),
        Some(Icon::Plus),
        Variant::Secondary,
    ) {
        actions.push(SettingsAction::AddSmartPlaylistRule);
    }

    // The refusal line renders under the rows it names things among, above
    // the Save line — the tag editor's single-slot shape (ADR: share the
    // pattern, not the state). Who fills it is ticket 18's.
    if let Some(reason) = &editor.refusal {
        ui.add_space(theme::SPACE_SM);
        ui.colored_label(palette.error, reason);
    }
    ui.add_space(theme::SPACE_MD);

    save_cancel_row(ui, cache, palette, content.reduce_motion, actions);
}

/// The editor's Save/Cancel line, as one flow-placed pair: the controls and
/// their typed intents are this pane's, and what Save and Cancel *mean* is
/// ticket 18's.
fn save_cancel_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    actions: &mut Vec<SettingsAction>,
) {
    if pane_button(
        ui,
        cache,
        palette,
        reduce_motion,
        "Save",
        egui::Id::new("smart_playlists_save"),
        None,
        Variant::Primary,
    ) {
        actions.push(SettingsAction::SaveSmartPlaylist);
    }
    if pane_button(
        ui,
        cache,
        palette,
        reduce_motion,
        "Cancel",
        egui::Id::new("smart_playlists_cancel"),
        None,
        Variant::Secondary,
    ) {
        actions.push(SettingsAction::CancelSmartPlaylist);
    }
}

/// One flow-placed shared text button: the size is measured, the slot
/// allocated, and the control registered under its own id — a painted rect
/// is not a control, and every clickable thing in this pane goes through
/// the component's own `ui.interact` with a fresh id.
///
/// The measurement is `text_button_size`'s plus the leading glyph's own
/// room: the component's paint charges 16px of glyph and an 8px gap before
/// the label and centres the pair in the rect, a charge
/// `text_button_size(label)` does not know about — so an icon button sized
/// by its label alone clips its own words at `painter_at`'s rect edges.
/// The `16.0 + 8.0` is the button component's arithmetic, spelled out here
/// exactly as the Settings header's Back control spells it.
#[expect(clippy::too_many_arguments, reason = "one button-paint call")]
fn pane_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    label: &str,
    id: egui::Id,
    icon: Option<Icon>,
    variant: Variant,
) -> bool {
    let mut size = text_button_size(ui, palette, label, false);
    if icon.is_some() {
        size.x += 16.0 + 8.0;
    }
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    text_button(
        ui,
        cache,
        palette,
        reduce_motion,
        &TextButton {
            id,
            rect,
            label,
            a11y: label,
            tooltip: None,
            icon,
            small: false,
            variant,
            enabled: true,
        },
    )
}
