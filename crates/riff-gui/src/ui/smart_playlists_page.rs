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
//!   [`SmartPlaylistsEditor::root`] is the whole tree, one
//!   [`RuleBoxDraft`] holding Rules and Boxes in each Box's one interleaved
//!   sibling order, and this module draws it depth-first. The add-box
//!   control disappears at the seventh level (the Root Box is level 1) and no
//!   message is ever shown for the level nobody can reach.
//! - *Ticket 17 (the live match count)* hangs its count line off this pane's
//!   header slot; the row identity contract (slot index) is what it keys on —
//!   widened by ticket 16 to a [`ChildSlot`] (the Box's path plus the index).
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
use crate::ui::rule_box::{BoxPath, ChildSlot, RuleBoxDraft, RuleChildDraft};
use crate::ui::rule_row::{RuleDraft, RuleIntent, RulePanel, RulePickerValues, rule_row};
use crate::ui::settings::{
    LibraryPaneColumns, SettingsAction, SettingsContent, settings_pane_columns,
};
use crate::ui::sidebar::{self, TreeRow};
use riff_backend::app::store::CustomSmartPlaylist;
use riff_backend::domain::field_catalog::{ValueType, spec};
use riff_backend::domain::{Combine, MatchableField, PlaylistId, RootBox};
use riff_gui_components::button::{TextButton, Variant, text_button, text_button_size};
use riff_gui_components::ghost_icon_button::ghost_icon_button;
use riff_gui_components::option_list::{OptionChoice, OptionList, option_list};
use riff_gui_components::text_field::{TextField, text_field};
use riff_gui_theme::icons::{Icon, IconCache};
use riff_gui_theme::theme::geometry::rule_box;
use riff_gui_theme::theme::geometry::rule_row::{HEADER_H, HEADER_LABEL_PAD, LIST_TOP_GAP, WELL_H};
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
    /// The tree being written: the Root Box, holding Rules and nested Rule
    /// Boxes in one interleaved sibling order each (ticket 16). The Root Box
    /// is level 1, and a path holds at most seven Boxes — a *displayed*
    /// editor rule the paint answers by hiding the add-box control, never a
    /// check the model or the save makes.
    pub root: RuleBoxDraft,
    /// Which Box's combine list is open, if any. One pane-level path rather
    /// than a flag per Box, which is what makes "one list at a time" true by
    /// construction instead of by a walk over the tree.
    pub open_combine: Option<BoxPath>,
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
        let mut root = RuleBoxDraft::empty(Combine::All);
        root.add_rule(&BoxPath::root());
        Self {
            selected: None,
            name: String::new(),
            root,
            open_combine: None,
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

    /// Append one blank Rule row to the Box `parent` names — the Root Box by
    /// default in a fresh draft.
    pub fn add_rule(&mut self, parent: &BoxPath) {
        self.root.add_rule(parent);
    }

    /// Append one new Box, holding one blank Rule, to the Box `parent` names.
    /// The seven-level cap is not a refusal here: the paint does not draw the
    /// control where [`BoxPath::accepts_a_box`] is `false`, so there is
    /// nothing to reject and no message to show.
    pub fn add_box(&mut self, parent: &BoxPath) {
        self.root.add_box(parent);
    }

    /// Remove the child at `parent`/`index` — a Rule, or a whole Box **with
    /// everything inside it**.
    pub fn remove_child(&mut self, parent: &BoxPath, index: usize) {
        self.root.remove_child(parent, index);
        // A combine list whose Box just left the tree has nothing left to
        // answer for; clearing the dangling path keeps the one-list-open
        // invariant true instead of leaving a stale path to never match.
        if let Some(open) = &self.open_combine
            && self.root.box_at(open).is_none()
        {
            self.open_combine = None;
        }
    }

    /// Choose the combine for the Box at `path` and close its list — the
    /// choice is the decision, and the list has no more to offer once made.
    pub fn set_combine(&mut self, path: &BoxPath, combine: Combine) {
        self.root.set_combine(path, combine);
        if self.open_combine.as_ref() == Some(path) {
            self.open_combine = None;
        }
    }

    /// Load a saved list into the editor as the draft to edit: its name, and
    /// its whole stored tree — every nested Box kept, in the stored sibling
    /// order (ticket 16; ticket 15's flattening skip is gone), never
    /// flattened and never renumbered. Any tree depth loads: the seven-level
    /// cap is an editor rule, so a deeper tree a newer riff wrote still reads
    /// back and can still be saved.
    ///
    /// The unreadable side list of a [`SmartPlaylistTree`] stays ticket 18's
    /// sibling state: this load takes the interpretable [`RootBox`], and a
    /// box draft holds its children as a `Vec` precisely so ticket 18 can set
    /// an Unreadable Rule placeholder at its stored position without
    /// restructuring.
    ///
    /// [`SmartPlaylistTree`]: riff_backend::app::store::SmartPlaylistTree
    pub fn load_saved(&mut self, list: &CustomSmartPlaylist, root: Option<&RootBox>) {
        self.selected = Some(list.id.clone());
        self.name.clone_from(&list.name);
        self.root = root.map_or_else(
            || RuleBoxDraft::empty(Combine::All),
            RuleBoxDraft::from_root_box,
        );
        self.open_combine = None;
        self.picker = None;
        self.pending_picker = None;
        self.refusal = None;
    }

    /// One row's library picker opened for `field` at `slot`: remember the
    /// read the host owes (once, through [`Self::pending_picker`]) and close
    /// every other row's picker — the pane shows one picker list at a time,
    /// because two would answer to one shared value list of the wrong field.
    pub fn picker_opened(&mut self, field: MatchableField, slot: &ChildSlot) {
        self.root.close_value_panels_except(slot);
        if let Some(boxed) = self.root.box_at_mut(&slot.parent)
            && let Some(RuleChildDraft::Rule(draft)) = boxed.children.get_mut(slot.index)
        {
            draft.open = Some(RulePanel::Value);
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

/// One thing a Rule row reported this frame that is the *tree's* to answer,
/// not the row's own: a library picker opened (the pane shows one list at a
/// time and remembers the read it owes) or closed (drop what was read).
enum PickerEvent {
    /// The row at `slot` opened its picker for `field`.
    Opened {
        /// The slot of the row that reported it.
        slot: ChildSlot,
        /// The field whose Library values to read.
        field: MatchableField,
    },
    /// The open picker closed. At most one can be open, so nothing names it.
    Closed,
}

/// The right column: the name typed as the Rules are written, the Rule tree
/// itself (ticket 14's rows inside ticket 16's Boxes, this pane the host),
/// and the Save/Cancel line whose *meanings* are ticket 18's — the buttons
/// and their intents are here.
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

    // The tree (ticket 16): one box model, drawn depth-first from the Root
    // Box — level 1 — every Box's children and controls one indent step in.
    // The step is the design system's, not this function's.
    ui.spacing_mut().indent = rule_box::INDENT;
    let reduce_motion = content.reduce_motion;
    let mut picker_events = Vec::new();
    {
        // Disjoint borrows for the walk: the tree is drawn from `root` while
        // the picker prop and the open-combine path travel beside it.
        let SmartPlaylistsEditor {
            root,
            open_combine,
            picker,
            ..
        } = editor;
        draw_rule_box(
            ui,
            cache,
            palette,
            reduce_motion,
            &BoxPath::root(),
            None,
            root,
            open_combine,
            picker,
            &mut picker_events,
            actions,
        );
    }
    // The picker's cross-tree duty, after the walk: one library picker list
    // at a time, and the read is remembered once for `RiffApp`'s apply step.
    for event in picker_events {
        match event {
            PickerEvent::Opened { slot, field } => editor.picker_opened(field, &slot),
            PickerEvent::Closed => editor.picker_closed(),
        }
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

/// One Rule Box, drawn depth-first: its combine control (the line a nested
/// Box's remove control shares), then every child in the box's one
/// interleaved sibling order — a Rule row or a recursively drawn Box — and
/// finally the box's own add controls.
///
/// `own_slot` is the Box's slot in its parent's list, `None` for the Root Box,
/// which has no parent and so nothing to be removed from. The seven-level cap
/// is drawn exactly here and nowhere else: at the deepest level
/// [`BoxPath::accepts_a_box`] answers `false` and the add-box control is
/// simply not painted — no rejection, no message, and nothing for the model
/// or the save to enforce.
#[expect(
    clippy::too_many_arguments,
    reason = "one tree walk, threaded through recursively"
)]
fn draw_rule_box(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    path: &BoxPath,
    own_slot: Option<&ChildSlot>,
    boxed: &mut RuleBoxDraft,
    open_combine: &mut Option<BoxPath>,
    picker: &mut Option<SmartPlaylistPickerValues>,
    picker_events: &mut Vec<PickerEvent>,
    actions: &mut Vec<SettingsAction>,
) {
    let box_id = egui::Id::new(("smart_playlist_box", path));
    let listed = open_combine.as_ref() == Some(path);
    let line = box_control_line(
        ui,
        cache,
        palette,
        box_id,
        combine_label(boxed.combine),
        listed,
        own_slot.is_some(),
    );
    let mut now_listed = listed;
    if line.combine_pressed {
        *open_combine = if listed { None } else { Some(path.clone()) };
        now_listed = !listed;
        if now_listed {
            line.response.scroll_to_me(Some(egui::Align::Center));
        }
    }
    if line.remove_pressed
        && let Some(slot) = own_slot
    {
        actions.push(SettingsAction::RemoveSmartPlaylistChild {
            parent: slot.parent.clone(),
            index: slot.index,
        });
    }
    if now_listed && let Some(combine) = draw_combine_list(ui, palette, box_id, boxed.combine) {
        actions.push(SettingsAction::SetSmartPlaylistCombine {
            path: path.clone(),
            combine,
        });
    }

    ui.add_space(theme::SPACE_XS);
    ui.indent(box_id.with("contents"), |ui| {
        let child_count = boxed.children.len();
        for index in 0..child_count {
            match &mut boxed.children[index] {
                RuleChildDraft::Rule(draft) => {
                    draw_rule_child(
                        ui,
                        cache,
                        palette,
                        path,
                        index,
                        draft,
                        picker,
                        picker_events,
                        actions,
                    );
                }
                RuleChildDraft::Box(nested) => {
                    let child_path = path.child(index);
                    let slot = ChildSlot::new(path.clone(), index);
                    draw_rule_box(
                        ui,
                        cache,
                        palette,
                        reduce_motion,
                        &child_path,
                        Some(&slot),
                        nested,
                        open_combine,
                        picker,
                        picker_events,
                        actions,
                    );
                    ui.add_space(theme::SPACE_SM);
                }
            }
        }
        // The box's own add controls, under the children they append to — a
        // Rule, and a Box while this level may hold one.
        draw_box_add_controls(ui, cache, palette, reduce_motion, path, box_id, actions);
    });
}

/// A Box's add controls, in one line under its children: "Add rule" always,
/// "Add box" while [`BoxPath::accepts_a_box`] holds.
///
/// The cap is drawn here and nowhere else: at the seventh level the add-box
/// control is **absent** rather than refusing — no message is ever shown for
/// the level nobody can reach, and nothing behind it rejects.
fn draw_box_add_controls(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    path: &BoxPath,
    box_id: egui::Id,
    actions: &mut Vec<SettingsAction>,
) {
    ui.horizontal(|ui| {
        if pane_button(
            ui,
            cache,
            palette,
            reduce_motion,
            "Add rule",
            box_id.with("add_rule"),
            Some(Icon::Plus),
            Variant::Secondary,
        ) {
            actions.push(SettingsAction::AddSmartPlaylistRule {
                parent: path.clone(),
            });
        }
        ui.add_space(theme::SPACE_SM);
        if path.accepts_a_box()
            && pane_button(
                ui,
                cache,
                palette,
                reduce_motion,
                "Add box",
                box_id.with("add_box"),
                Some(Icon::Plus),
                Variant::Secondary,
            )
        {
            actions.push(SettingsAction::AddSmartPlaylistBox {
                parent: path.clone(),
            });
        }
    });
}

/// One Rule child: ticket 14's row, edited in place, with the row's own
/// remove control sharing its line.
///
/// The row is a **vertical** child of the line, not a horizontal sibling of
/// the remove control: the lists the row opens are drawn in its own flow, and
/// a horizontal parent would lay the whole list out sideways. Its width is
/// the line minus the remove control's square, so nothing is overlapped.
#[expect(clippy::too_many_arguments, reason = "one row in a threaded tree walk")]
fn draw_rule_child(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    path: &BoxPath,
    index: usize,
    draft: &mut RuleDraft,
    picker: &mut Option<SmartPlaylistPickerValues>,
    picker_events: &mut Vec<PickerEvent>,
    actions: &mut Vec<SettingsAction>,
) {
    let row_id = egui::Id::new(("smart_playlist_rule_row", path, index));
    let prop = if draft.open == Some(RulePanel::Value) {
        picker.as_ref().map(SmartPlaylistPickerValues::prop)
    } else {
        None
    };
    let mut remove_pressed = false;
    let outcome = ui
        .horizontal_top(|ui| {
            let row_w = (ui.available_width() - rule_box::REMOVE_W - theme::SPACE_SM).max(0.0);
            let outcome = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(row_w, ui.available_height()),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt(row_id),
                    |ui| rule_row(ui, cache, palette, row_id, draft, prop.as_ref()),
                )
                .inner;
            ui.add_space(theme::SPACE_SM);
            remove_pressed = remove_button(
                ui,
                cache,
                palette,
                row_id.with("remove_rule"),
                "Remove rule",
            );
            outcome
        })
        .inner;
    let mut opened = false;
    for intent in &outcome.intents {
        draft.apply(intent);
        match intent {
            RuleIntent::PanelToggled { open, .. } if *open => opened = true,
            RuleIntent::PickerOpened { field } => {
                picker_events.push(PickerEvent::Opened {
                    slot: ChildSlot::new(path.clone(), index),
                    field: *field,
                });
            }
            RuleIntent::PickerClosed => picker_events.push(PickerEvent::Closed),
            _ => {}
        }
    }
    // Host duty handed from ticket 14: egui 0.35 never auto-scrolls a
    // keyboard-focused row inside this clipped pane, so the row that just
    // opened a list scrolls itself into view here.
    if opened {
        outcome.response.scroll_to_me(Some(egui::Align::Center));
    }
    if remove_pressed {
        actions.push(SettingsAction::RemoveSmartPlaylistChild {
            parent: path.clone(),
            index,
        });
    }
}

/// The Box's combine control: the current choice's word with a disclosure
/// chevron, opening the two-word list below it. A real button under its own
/// id — the same shape ticket 14's row headers keep — so it is reachable by
/// pointer, by keyboard and through the a11y tree.
fn box_header(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    label: &str,
    open: bool,
) -> bool {
    let width = header_width(ui, palette, label);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEADER_H), egui::Sense::hover());
    text_button(
        ui,
        cache,
        palette,
        false,
        &TextButton {
            id,
            rect,
            label,
            a11y: label,
            tooltip: None,
            icon: Some(if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            }),
            small: true,
            variant: Variant::Secondary,
            enabled: true,
        },
    )
}

/// What a Box's control line reported this frame.
struct BoxLineReport {
    /// The combine control was pressed.
    combine_pressed: bool,
    /// The remove control was pressed — never true for the Root Box, which
    /// draws none.
    remove_pressed: bool,
    /// The line's own response, the scroll target when the list opens.
    response: egui::Response,
}

/// One Box's control line: its combine control, and — for a nested Box — the
/// remove control that takes the whole Box, children and all, away.
fn box_control_line(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    box_id: egui::Id,
    label: &str,
    listed: bool,
    removable: bool,
) -> BoxLineReport {
    let mut remove_pressed = false;
    let line = ui.horizontal_top(|ui| {
        let pressed = box_header(ui, cache, palette, box_id.with("combine"), label, listed);
        if removable {
            ui.add_space(theme::SPACE_SM);
            remove_pressed =
                remove_button(ui, cache, palette, box_id.with("remove_box"), "Remove box");
        }
        pressed
    });
    BoxLineReport {
        combine_pressed: line.inner,
        remove_pressed,
        response: line.response,
    }
}

/// The two combine words, drawn inline under the control while its list is
/// open; reports the word clicked, if any.
fn draw_combine_list(
    ui: &mut egui::Ui,
    palette: &Palette,
    box_id: egui::Id,
    combine: Combine,
) -> Option<Combine> {
    ui.add_space(LIST_TOP_GAP);
    let options = [Combine::All, Combine::Any].map(|combine| OptionChoice {
        key: combine,
        label: combine_label(combine),
    });
    option_list(
        ui,
        palette,
        &OptionList {
            id: box_id.with("combine-list"),
            options: &options,
            current: Some(&combine),
        },
    )
}

/// A combine control is as wide as its label plus its glyph and padding,
/// bounded by the pane — the measurement the Rule row's own headers share,
/// `HEADER_LABEL_PAD` included.
fn header_width(ui: &egui::Ui, palette: &Palette, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        theme::styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS),
        palette.ink,
    );
    (galley.size().x + HEADER_LABEL_PAD).min(ui.available_width().max(0.0))
}

/// The words for a Box's combine choice — the listener's own phrase, spelled
/// once for the control and for the list's two options.
fn combine_label(combine: Combine) -> &'static str {
    match combine {
        Combine::All => "All of the following",
        Combine::Any => "Any of the following",
    }
}

/// The square remove control closing a Rule's line or a nested Box's header:
/// an always-visible ghost glyph with its own hit area and a11y name — a
/// control that appears only on hover is a control the golden and the a11y
/// tree cannot be held to.
fn remove_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    label: &str,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(rule_box::REMOVE_W, HEADER_H),
        egui::Sense::hover(),
    );
    let hit = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2(rule_box::REMOVE_W, rule_box::REMOVE_W),
    );
    ghost_icon_button(ui, cache, palette, hit, id, Icon::Trash, label, false)
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
