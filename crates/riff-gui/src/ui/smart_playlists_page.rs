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
//! - *Ticket 18 (draft + save/cancel + reasons)* is what this module is now:
//!   dirtiness derived by comparing the buffers against [`SmartPlaylistOriginal`],
//!   the one [`SmartPlaylistsEditor::reconcile`] that drops a draft when it no
//!   longer belongs, the third [`RuleChildDraft`] variant that carries an
//!   Unreadable Rule at its stored position, and every refusal
//!   ([`SmartPlaylistRefusal`]) with the reason rendered under the row that
//!   caused it. The tag editor's single-slot, self-naming copy is the
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
use crate::ui::rule_row::{
    RuleDraft, RuleGap, RuleIntent, RulePanel, RulePickerValues, ValueGap, rule_row,
};
use crate::ui::settings::{
    LibraryPaneColumns, SettingsAction, SettingsContent, settings_pane_columns,
};
use crate::ui::sidebar::{self, TreeRow};
use riff_backend::app::playlist_name::builtin_smart_list_name_conflict;
use riff_backend::app::store::{CustomSmartPlaylist, SmartPlaylistTree};
use riff_backend::domain::field_catalog::{ValueType, spec};
use riff_backend::domain::{
    Combine, MatchableField, PlaylistId, RootBox, Rule, RuleBox, RuleChild,
};
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

/// Why the editor's Save was refused, and where the reason belongs — the row
/// that caused it, under the name field, or over the whole form.
///
/// Ticket 14's [`RuleGap`] is how a Rule row says what it is missing and the
/// Unreadable Rule's own written words are how it says it cannot be read; this
/// type only carries the resulting sentence to the place that paints it. The
/// tag editor's one per-draft slot is the precedent being *shared* (the
/// reason travels with the draft, it is cleared only by a clean pass), not
/// copied: ticket 18's reasons land next to the thing that caused them, which
/// is why there are three placements and not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartPlaylistRefusal {
    /// Painted under the Rule row at this slot: the row is half-filled (the
    /// reason names what is missing) or Unreadable (the reason names it the
    /// way the row's own words do).
    Row {
        /// The slot of the row the reason belongs to.
        slot: ChildSlot,
        /// The sentence, ready to paint.
        reason: String,
    },
    /// Painted under the name field: the name is missing, or one of the six
    /// Built-in Smart Playlists already answers to it.
    Name(String),
    /// Painted after every row and above the Save line, where no row and no
    /// field owns it: the Application Store refused the write as a whole and
    /// this is its own string, verbatim.
    Save(String),
}

/// What the editor's live match line shows: how many Tracks the draft tree
/// matches, or why no number can be shown (ticket 17).
///
/// The distinction between the two reason variants is where the reason came
/// from, and the pane paints them differently: a draft the listener has not
/// finished blocks the count (a hint, phrased with the same words a refused
/// Save uses), while a tree the engine cannot compute is a broken answer and
/// says so in the store's own words, verbatim — never as a number, and never
/// as "0 songs".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartPlaylistMatchLine {
    /// The draft tree was counted: the Tracks it matches.
    Matches(usize),
    /// The draft cannot be counted as it stands: a half-filled Rule named by
    /// what it is missing, or an Unreadable Rule. The sentence is the block
    /// Save refuses the same tree with.
    Uncountable(String),
    /// The engine could not compute the tree: the store's own reason, the
    /// same reason the listing shows for the same tree.
    Failed(String),
}

impl SmartPlaylistMatchLine {
    /// The words the line shows: the count in a sentence, or the reason
    /// exactly as it was given.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Matches(1) => "Matches 1 song".to_string(),
            Self::Matches(count) => format!("Matches {count} songs"),
            Self::Uncountable(reason) | Self::Failed(reason) => reason.clone(),
        }
    }
}

/// Why the committed walk cannot spell a tree.
enum DraftBlock {
    /// The draft holds a stored Rule this build cannot read — for which no
    /// tree can be written, and none is invented.
    Unreadable,
    /// The first Rule row whose committed answers are not whole, and what it
    /// is still missing.
    Gap(RuleGap),
}

/// The state a draft was opened from: the name and the tree as the store last
/// served them (or the blank starting draft a new list opens as).
///
/// Dirtiness is derived by comparing the current buffers against this — never
/// a `dirty` flag — so nothing is dirty by construction when a list is
/// opened, and a switch of lists (which replaces the whole draft) cannot
/// leave a stale "modified" mark behind. This is the tag editor's
/// `fields`/`originals` pair, at the grain of a whole tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartPlaylistOriginal {
    /// The name as opened.
    pub name: String,
    /// The tree as opened, Unreadable rows included at their stored positions.
    pub root: RuleBoxDraft,
}

/// What one Save needs from the store, once the draft has passed every check
/// the frontend owns: the list to update (`None` when the save creates it),
/// the trimmed name, and the whole tree. One call writes all three (D12), so
/// there is no second step that could half-apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartPlaylistSavePlan {
    /// The saved list this draft was written against, or `None` while the
    /// list is new.
    pub id: Option<PlaylistId>,
    /// The name, trimmed the way the store trims it before writing.
    pub name: String,
    /// The whole tree, spelled for the store's wholesale replace. It always
    /// holds at least one Rule and no Unreadable Rule: the checks that
    /// produced this plan refused otherwise.
    pub tree: SmartPlaylistTree,
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
    /// The state this draft was opened from — what [`Self::is_dirty`]
    /// compares against, and what a successful Save re-stamps.
    pub original: SmartPlaylistOriginal,
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
    /// Why the last Save was refused, and where the reason belongs — under
    /// the offending row, under the name field, or above the Save line. The
    /// single slot is the tag editor's shape (`draft.error`), shared, and it
    /// is cleared only by a Save that passes every local check.
    pub refusal: Option<SmartPlaylistRefusal>,
    /// What the live match line shows: the count of the draft's committed
    /// tree, or why no number can be shown. Written by the app's per-frame
    /// refresh through [`SessionViews::custom_list_match_count`](riff_backend::app::views::SessionViews::custom_list_match_count)
    /// ([`riff_backend::app::views::SessionViews`]) — the one read this line
    /// costs — and painted by the pane. `None` until the first refresh has
    /// run for the draft in hand; a replaced draft carries no answer from
    /// the draft it replaced.
    pub match_line: Option<SmartPlaylistMatchLine>,
}

impl Default for SmartPlaylistsEditor {
    fn default() -> Self {
        let mut root = RuleBoxDraft::empty(Combine::All);
        root.add_rule(&BoxPath::root());
        Self {
            selected: None,
            name: String::new(),
            original: SmartPlaylistOriginal {
                name: String::new(),
                root: root.clone(),
            },
            root,
            open_combine: None,
            picker: None,
            pending_picker: None,
            refusal: None,
            match_line: None,
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

    /// Whether the listener has changed anything since the draft was opened
    /// or last saved — **derived** by comparing the current buffers against
    /// [`Self::original`], never tracked as a flag. A freshly opened list is
    /// therefore clean by construction, and so is a freshly saved one.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.name != self.original.name || self.root != self.original.root
    }

    /// Save refused the draft's whole right to exist, or opened a new one:
    /// discard it. This is Cancel's meaning and the reconcile's outcome —
    /// the same one call, so the two cannot disagree about what "discarded"
    /// leaves behind (the fresh new-list draft, nothing selected).
    pub fn discard(&mut self) {
        *self = Self::default();
    }

    /// Cancel: discard the draft. Nothing is written, whatever was typed.
    pub fn cancel(&mut self) {
        self.discard();
    }

    /// The **single reconcile hook**: drop the draft when it no longer
    /// belongs. A draft belongs to the Smart Playlists page and, when it has
    /// one, to a saved list that still exists; anything else is a draft
    /// outliving the thing it was written for, and it goes. Enforced here,
    /// not by call-site discipline — `RiffApp::ui` calls this once per frame,
    /// before anything draws, and no gesture clears a draft itself.
    ///
    /// The two cases, and what they are:
    ///
    /// - **the page is gone** (`on_page` false: another Settings section, or
    ///   the whole Settings view left behind): unsaved edits are discarded on
    ///   leave, exactly as the inline tag editor discards them on a selection
    ///   change;
    /// - **its list is gone** (selected, but the enumeration no longer names
    ///   it — deleted from the sidebar while open): the draft can only be
    ///   written against a list that is not there any more, so it cannot
    ///   stay.
    ///
    /// Switching to another list is *not* a drop and does not belong here: it
    /// is a whole-draft **replacement** through [`Self::load_saved`] (the tag
    /// editor's `open_track` on a row click), and a replacement cannot leave
    /// the previous draft behind to outlive anything.
    pub fn reconcile(&mut self, on_page: bool, lists: &[CustomSmartPlaylist]) {
        if !on_page {
            self.discard();
            return;
        }
        if let Some(id) = &self.selected
            && !lists.iter().any(|list| &list.id == id)
        {
            self.discard();
        }
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

    /// Move the child at `parent`/`from` to position `to` in that Box's one
    /// interleaved sibling order — the drag-and-drop reorder, whose no-op
    /// rule (self-drops, out-of-range gestures) lives in
    /// [`RuleBoxDraft::move_child`]. The draft renumbers by construction: a
    /// Box holds its Rules and Boxes in one `Vec`, so the list's order *is*
    /// the shared `position` sequence Save spells.
    pub fn move_child(&mut self, parent: &BoxPath, from: usize, to: usize) {
        let reordered = self.root.move_child(parent, from, to);
        // A Box strictly under a reordered list answers to a different
        // address once its siblings shift, so an open combine list down
        // there would name whichever Box now sits at that address — close it
        // rather than paint a different Box's list (the one-list-open
        // invariant `remove_child` keeps for the dangling case). Only a real
        // reorder does that: an empty gesture (the model answers `false`)
        // renumbers nothing. The drag wiring reports a self-drop on every
        // plain press-release of a draggable row — egui begins a drag on the
        // press under a drag-only sense — and without this gate that empty
        // drop would close the combine list the very same click had just
        // opened, leaving a nested Box's control unopenable by pointer.
        if reordered
            && let Some(open) = &self.open_combine
            && open != parent
            && open.indices().starts_with(parent.indices())
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
    /// order, never flattened and never renumbered, and **every Unreadable
    /// Rule put back at the position it was stored in** (ticket 18), so it is
    /// shown where the listener can see it and preserved across a later Save
    /// rather than silently vanishing. Any tree depth loads: the seven-level
    /// cap is an editor rule, so a deeper tree a newer riff wrote still reads
    /// back and can still be saved.
    ///
    /// `tree` of `None` is a degraded read (ticket 11's reason channel): the
    /// list exists but its tree could not be served, so the draft opens
    /// empty — the listener can rewrite the rules, and Save then writes what
    /// they wrote. Nothing is invented for the missing half.
    ///
    /// The whole draft is replaced, and [`Self::original`] is stamped from
    /// what was loaded: a freshly opened list is clean by construction, and
    /// whatever the previous draft held is gone — switching lists discards
    /// unsaved edits, the inline tag editor's own rule.
    pub fn load_saved(&mut self, list: &CustomSmartPlaylist, tree: Option<&SmartPlaylistTree>) {
        self.selected = Some(list.id.clone());
        self.name.clone_from(&list.name);
        self.root = tree.map_or_else(
            || RuleBoxDraft::empty(Combine::All),
            |tree| RuleBoxDraft::from_root_box(tree.root()),
        );
        if let Some(tree) = tree {
            self.root.insert_unreadable(tree.unreadable());
        }
        self.open_combine = None;
        self.picker = None;
        self.pending_picker = None;
        self.refusal = None;
        // The live match line is an answer about the draft being replaced:
        // it goes with it, and the next refresh derives the line for the
        // draft in hand.
        self.match_line = None;
        self.original = SmartPlaylistOriginal {
            name: self.name.clone(),
            root: self.root.clone(),
        };
    }

    /// The tree this draft **counts** as it stands — every Rule row spelled
    /// from its committed answers, every Box's combine, in the stored shape —
    /// or the reason it cannot be counted at all.
    ///
    /// The **committed** answers, not the live buffers: a well's buffer
    /// changes on every keystroke ([`RuleDraft::value`] is the row's typing
    /// state), so a shape derived from buffers would move per keystroke and
    /// re-query with it — the one thing the live line must not do. What a
    /// value accepted on Enter or blur, a selection, or a picker choice
    /// lands is [`RuleDraft::committed_value`], and that is what this walk
    /// spells. A value typed but not yet committed is therefore not part of
    /// the counted tree: the line keeps the previous answer (or block) until
    /// the commit, which is the cadence rule, not an omission.
    ///
    /// The block, when there is one, is ticket 18's own ladder in its own
    /// order: an Unreadable Rule first — the one row a listener cannot
    /// resolve by completing it — then the first half-filled row, named by
    /// what it is missing ([`RuleGap`]). The sentences are [`gap_reason`]'s
    /// and `UNREADABLE_BLOCK_REASON`'s, the same words a refused Save
    /// paints, so the line and Save teach one vocabulary.
    ///
    /// `Err` is the sentence to paint **instead of a number** — never a zero
    /// standing in for a count, and never a tree riff cannot spell.
    pub fn committed_tree(&self) -> Result<SmartPlaylistTree, String> {
        if first_unreadable(&self.root, &BoxPath::root()).is_some() {
            return Err(UNREADABLE_BLOCK_REASON.to_string());
        }
        // The walk's own Unreadable arm is unreachable through the check
        // above, exactly as `prepare_save`'s spelling arm is — but a draft
        // that cannot spell a tree must never be counted, so it reports the
        // block rather than falling through.
        committed_rule_box(&self.root)
            .map(|root| SmartPlaylistTree::new(RootBox::new(root)))
            .map_err(|block| match block {
                DraftBlock::Unreadable => UNREADABLE_BLOCK_REASON.to_string(),
                DraftBlock::Gap(gap) => gap_reason(gap).to_string(),
            })
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

    /// The **single Save entry point**: decide whether the draft can be
    /// written, and hand back what to write — or record why not, in
    /// [`Self::refusal`], and hand back nothing.
    ///
    /// Pure with respect to the store: it consults the draft, the catalog
    /// (through ticket 14's row) and the application layer's built-in-name
    /// check, and writes nothing. The caller performs the one store call the
    /// plan answers.
    ///
    /// The refusals, in the order they are asked — an earlier one hides a
    /// later one, which is why the order is fixed here rather than left to
    /// whichever check a caller ran first:
    ///
    /// 1. **an Unreadable Rule**, whoever else is also wrong: it is the one
    ///    row a listener cannot resolve by completing it — it must be deleted
    ///    — and re-inserting it would fail inside the store's `CHECK` list
    ///    with no explanation. Pointed at, preserved, never dropped;
    /// 2. **a half-filled Rule**, the first in draw order, named by what it
    ///    is missing ([`RuleGap`]);
    /// 3. **the name**: missing, or one of the six Built-in Smart Playlists'
    ///    display names. The cross-table half of the name rule (a name
    ///    already held by a sidebar row) lives in the store's mutation path
    ///    and comes back as its refusal, below;
    /// 4. the store, which can still refuse what the frontend cannot see: a
    ///    name another row holds, or an empty Box (a box with no children at
    ///    all — the tree checks above refuse a *row*, not an empty *box*).
    ///
    /// A clean plan clears the refusal **before** the write, the tag editor's
    /// own shape: a stale reason from an earlier refusal never outlives the
    /// attempt that resolved it, and the store's answer (success or its own
    /// refusal) is what the listener is left with.
    #[must_use]
    pub fn prepare_save(&mut self) -> Option<SmartPlaylistSavePlan> {
        if let Some(slot) = first_unreadable(&self.root, &BoxPath::root()) {
            self.refusal = Some(SmartPlaylistRefusal::Row {
                slot,
                reason: UNREADABLE_BLOCK_REASON.to_string(),
            });
            return None;
        }
        if let Some((slot, gap)) = first_incomplete(&self.root, &BoxPath::root()) {
            self.refusal = Some(SmartPlaylistRefusal::Row {
                slot,
                reason: gap_reason(gap).to_string(),
            });
            return None;
        }
        let name = self.name.trim().to_string();
        if name.is_empty() {
            self.refusal = Some(SmartPlaylistRefusal::Name(
                "this list needs a name".to_string(),
            ));
            return None;
        }
        if let Some(reason) = builtin_smart_list_name_conflict(&name) {
            self.refusal = Some(SmartPlaylistRefusal::Name(reason));
            return None;
        }
        let Some(root) = self.root.to_root_box() else {
            // Unreachable through the checks above — every half-filled row
            // and every Unreadable Rule was caught first — but a draft that
            // cannot spell a tree must never be reported as savable.
            self.refusal = Some(SmartPlaylistRefusal::Save(
                "this rule set cannot be saved as it stands".to_string(),
            ));
            return None;
        };
        self.refusal = None;
        Some(SmartPlaylistSavePlan {
            id: self.selected.clone(),
            name,
            tree: SmartPlaylistTree::new(root),
        })
    }

    /// The store's answer to [`Self::prepare_save`]: the list now exists, or
    /// exists under this id. The draft **stays open** as the draft of that
    /// list — the listener keeps writing on the page, and the name written is
    /// the trimmed one the store holds, so the buffer reads back exactly what
    /// the sidebar row now says.
    pub fn mark_saved(&mut self, id: PlaylistId) {
        self.selected = Some(id);
        self.name = self.name.trim().to_string();
        self.original = SmartPlaylistOriginal {
            name: self.name.clone(),
            root: self.root.clone(),
        };
        self.refusal = None;
    }

    /// The store refused the write as a whole. Its words are rendered
    /// verbatim above the Save line: the refusal reaches the listener exactly
    /// as the store phrased it, double-prefix and all — no reason is rewritten
    /// or shortened on the way to the screen.
    pub fn refuse_save(&mut self, reason: String) {
        self.refusal = Some(SmartPlaylistRefusal::Save(reason));
    }
}

/// The words a row this build cannot read wears, and the reason Save gives
/// for refusing while one is in the tree. The row's own label is the spec's
/// copy; the reason opens with the same words so the pointing is unambiguous,
/// and then names the way out the spec names — the row's own delete control.
pub const UNREADABLE_ROW_WORDS: &str = "riff can't understand this rule";
const UNREADABLE_BLOCK_REASON: &str =
    "riff can't understand this rule — delete the row to save the list";

/// The slot of the first Unreadable Rule in draw order, or `None` when the
/// tree holds none.
fn first_unreadable(boxed: &RuleBoxDraft, path: &BoxPath) -> Option<ChildSlot> {
    for (index, child) in boxed.children.iter().enumerate() {
        match child {
            RuleChildDraft::Unreadable(_) => return Some(ChildSlot::new(path.clone(), index)),
            RuleChildDraft::Box(nested) => {
                if let Some(slot) = first_unreadable(nested, &path.child(index)) {
                    return Some(slot);
                }
            }
            RuleChildDraft::Rule(_) => {}
        }
    }
    None
}

/// The first half-filled Rule in draw order and what it is missing, or `None`
/// when every row is a whole Rule.
fn first_incomplete(boxed: &RuleBoxDraft, path: &BoxPath) -> Option<(ChildSlot, RuleGap)> {
    for (index, child) in boxed.children.iter().enumerate() {
        match child {
            RuleChildDraft::Rule(draft) => {
                if let Some(gap) = draft.gap() {
                    return Some((ChildSlot::new(path.clone(), index), gap));
                }
            }
            RuleChildDraft::Box(nested) => {
                if let Some(found) = first_incomplete(nested, &path.child(index)) {
                    return Some(found);
                }
            }
            RuleChildDraft::Unreadable(_) => {}
        }
    }
    None
}

/// What a half-filled row is missing, in the listener's own terms — one
/// sentence per [`RuleGap`], so the reason names the missing part rather than
/// reporting that "a rule is incomplete".
fn gap_reason(gap: RuleGap) -> &'static str {
    match gap {
        RuleGap::Field => "this rule needs a field",
        RuleGap::Comparison => "this rule needs a comparison",
        RuleGap::Value(ValueGap::Missing) => "this rule needs a value",
        RuleGap::Value(ValueGap::Unparseable) => {
            "this rule's value can't be read as this field's kind"
        }
    }
}

/// The committed walk's spelling of one Box: its combine, and its children —
/// Rules from their committed answers, Boxes recursively — or the first block
/// in draw order.
fn committed_rule_box(boxed: &RuleBoxDraft) -> Result<RuleBox, DraftBlock> {
    let mut children = Vec::with_capacity(boxed.children.len());
    for child in &boxed.children {
        children.push(match child {
            RuleChildDraft::Rule(draft) => RuleChild::Rule(committed_rule(draft)?),
            RuleChildDraft::Box(nested) => RuleChild::Box(committed_rule_box(nested)?),
            RuleChildDraft::Unreadable(_) => return Err(DraftBlock::Unreadable),
        });
    }
    Ok(RuleBox::new(boxed.combine).with_children(children))
}

/// The Rule one row's **committed** answers spell, or the part they are
/// still missing — the same questions [`RuleDraft::gap`] and
/// [`RuleDraft::rule`] ask, taken over the committed value
/// ([`RuleDraft::committed_value`]) instead of the parse of the live
/// buffers.
///
/// The committed value can hold no unparseable answer: a commit only ever
/// lands a value the row parsed against the field's own type, so the only
/// value gap this walk can find is a missing one.
fn committed_rule(draft: &RuleDraft) -> Result<Rule, DraftBlock> {
    let Some(field) = draft.field else {
        return Err(DraftBlock::Gap(RuleGap::Field));
    };
    let Some(comparison) = draft.comparison else {
        return Err(DraftBlock::Gap(RuleGap::Comparison));
    };
    if !spec(field).permits(comparison) {
        return Err(DraftBlock::Gap(RuleGap::Comparison));
    }
    let value = if draft.value_widgets().is_empty() {
        None
    } else {
        Some(
            draft
                .committed_value()
                .cloned()
                .ok_or(DraftBlock::Gap(RuleGap::Value(ValueGap::Missing)))?,
        )
    };
    let rule = if draft.negated {
        Rule::new(field, comparison, value).negated()
    } else {
        Rule::new(field, comparison, value)
    };
    Ok(rule)
}

/// The live match line: the count, or the reason no number can be shown.
///
/// The count is secondary text. A draft the listener has not finished blocks
/// it in the hint tone — its sentence is the same one a refused Save uses, so
/// it reads as what is still owed, not as an error — while a tree the engine
/// cannot compute is a broken answer and wears the error tone.
fn match_line_label(ui: &mut egui::Ui, palette: &Palette, line: &SmartPlaylistMatchLine) {
    let color = match line {
        SmartPlaylistMatchLine::Matches(_) => palette.ink_2,
        SmartPlaylistMatchLine::Uncountable(_) => palette.ink_3,
        SmartPlaylistMatchLine::Failed(_) => palette.error,
    };
    ui.label(
        egui::RichText::new(line.text())
            .color(color)
            .size(theme::TEXT_XS),
    );
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

/// Drag-and-drop payload for one row of the rule tree: the row's index in
/// its Box, and **which Box's sibling list it came from**. Private to this
/// module — the walk only ever sees a drop already checked against its own
/// list (see [`draggable_rule_row`]).
///
/// The list identity is what makes a cross-Box drop fail safe. A pane
/// renders several sibling lists at once and each starts at zero, so a bare
/// index released over one Box's row would read as a legal move inside
/// another Box's — the gesture would silently move a Rule across a Box
/// boundary while looking like a success. Carrying the source list and
/// refusing a payload from any other list makes that read impossible: the
/// drop simply changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RuleRowDrag {
    /// The Box whose one interleaved sibling list the row belongs to.
    list: BoxPath,
    /// The row's index in that list.
    index: usize,
}

/// One rule-tree row (a Rule's line or a nested Box's control line) wrapped
/// in the shared drag mechanism ([`sidebar::drag_row`]), **scoped to the
/// sibling list it lives in** — `list`/`index` are that row's own slot.
///
/// The scoping is the point: the wrapper constructs the payload from `list`
/// and accepts a payload only from the same `list`, so a drag can never move
/// a child across a Box boundary, and nothing at a call site can forget the
/// check. Everything else — the hit-area registered before the row paints,
/// the floating copy, the stale-payload cleanup — is the shared mechanism;
/// see its doc for why the ordering cannot be reversed.
fn draggable_rule_row<R>(
    ui: &mut egui::Ui,
    palette: &Palette,
    list: &BoxPath,
    index: usize,
    id: egui::Id,
    slot_size: egui::Vec2,
    paint: impl FnOnce(&mut egui::Ui) -> (R, egui::Response),
) -> sidebar::DragRow<RuleRowDrag, R> {
    let payload = RuleRowDrag {
        list: list.clone(),
        index,
    };
    let held_list = list.clone();
    sidebar::drag_row(
        ui,
        id,
        slot_size,
        palette,
        payload,
        move |incoming: &RuleRowDrag| incoming.list == held_list,
        paint,
    )
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
    // The name's own refusal, under the field it is about: the name is
    // missing, or a Built-in Smart Playlist already answers to it.
    if let Some(SmartPlaylistRefusal::Name(reason)) = &editor.refusal {
        ui.add_space(theme::SPACE_XS);
        ui.colored_label(palette.error, reason);
    }

    // The live match line (ticket 17): how many Tracks the committed tree
    // matches, or why no number can be shown. It paints what the app's
    // refresh last wrote — nothing on this paint path reaches a store — and
    // it is silent only on a frame before that refresh has run.
    if let Some(line) = &editor.match_line {
        ui.add_space(theme::SPACE_XS);
        match_line_label(ui, palette, line);
    }
    ui.add_space(theme::SPACE_MD);

    sidebar::section_header(ui, palette, "Rules");
    ui.add_space(theme::SPACE_SM);

    // The tree (ticket 16): one box model, drawn depth-first from the Root
    // Box — level 1 — every Box's children and controls one indent step in.
    // The step is the design system's, not this function's. The refusal
    // travels beside the tree as an immutable borrow so each row can paint
    // the reason that belongs to it, exactly where that row is drawn.
    ui.spacing_mut().indent = rule_box::INDENT;
    let reduce_motion = content.reduce_motion;
    let mut picker_events = Vec::new();
    {
        // Disjoint borrows for the walk: the tree is drawn from `root` while
        // the picker prop, the open-combine path and the refusal travel
        // beside it.
        let SmartPlaylistsEditor {
            root,
            open_combine,
            picker,
            refusal,
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
            refusal.as_ref(),
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

    // The store's own refusal of the whole write — a name another sidebar row
    // holds, an empty Box — rendered after the rows it is about and above the
    // Save line that asked, its words verbatim. The tag editor's single-slot
    // position; the two row-shaped refusals are painted under their rows.
    if let Some(SmartPlaylistRefusal::Save(reason)) = &editor.refusal {
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
    refusal: Option<&SmartPlaylistRefusal>,
    picker_events: &mut Vec<PickerEvent>,
    actions: &mut Vec<SettingsAction>,
) {
    let box_id = egui::Id::new(("smart_playlist_box", path));
    let listed = open_combine.as_ref() == Some(path);
    // A nested Box's control line is the Box's own row in its parent's
    // sibling list (ticket 19): dragging it moves the whole child — the Box
    // leaves with everything inside it, the same unit its remove control
    // takes. The Root Box has no parent slot, so it neither drags nor
    // accepts a drop. Same list only, enforced inside the wrapper.
    let (line, dropped) =
        draw_box_line(ui, cache, palette, box_id, boxed.combine, listed, own_slot);
    // A drag released over this Box's line moves the dragged sibling to this
    // Box's position; the floating copy reports nothing. A payload from
    // another sibling list never reaches here (the wrapper refuses it), and
    // a drop on the row it came from is the model's no-op.
    if let Some(held) = dropped
        && let Some(slot) = own_slot
    {
        actions.push(SettingsAction::MoveSmartPlaylistChild {
            parent: slot.parent.clone(),
            from: held.index,
            to: slot.index,
        });
    }
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
                        refusal,
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
                        refusal,
                        picker_events,
                        actions,
                    );
                    ui.add_space(theme::SPACE_SM);
                }
                RuleChildDraft::Unreadable(_) => {
                    draw_unreadable_child(ui, cache, palette, path, index, refusal, actions);
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
///
/// The row drags up and down among its parent Box's siblings (ticket 19)
/// through [`draggable_rule_row`], whose hit-area registration happens
/// BEFORE the row paints — the one ordering that keeps the row's controls
/// clickable, the shared text field included. The drag's one order is the
/// canonical one, so unlike a Playlist's entries the drag is always enabled;
/// there is no sorted view here to disable it for.
#[expect(clippy::too_many_arguments, reason = "one row in a threaded tree walk")]
fn draw_rule_child(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    path: &BoxPath,
    index: usize,
    draft: &mut RuleDraft,
    picker: &mut Option<SmartPlaylistPickerValues>,
    refusal: Option<&SmartPlaylistRefusal>,
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
    let drag = ui
        .horizontal_top(|ui| {
            let row_w = (ui.available_width() - rule_box::REMOVE_W - theme::SPACE_SM).max(0.0);
            let drag = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(row_w, ui.available_height()),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt(row_id),
                    |ui| {
                        draggable_rule_row(
                            ui,
                            palette,
                            path,
                            index,
                            row_id.with("drag"),
                            egui::vec2(row_w, HEADER_H),
                            |ui| {
                                let outcome =
                                    rule_row(ui, cache, palette, row_id, draft, prop.as_ref());
                                let response = outcome.response.clone();
                                (outcome, response)
                            },
                        )
                    },
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
            drag
        })
        .inner;
    let outcome = drag.inner;
    // A drag released over this row moves the dragged child to this row's
    // position — but only a child of the same Box can reach here: the
    // wrapper refuses any payload from another sibling list, and the model
    // drops a self-drop. A row painted as the floating copy reports nothing.
    if !drag.in_flight
        && let Some(held) = &drag.dropped
    {
        actions.push(SettingsAction::MoveSmartPlaylistChild {
            parent: path.clone(),
            from: held.index,
            to: index,
        });
    }
    let mut opened = false;
    // The floating copy takes no part in interaction, so its intents are not
    // the listener's decisions.
    if !drag.in_flight {
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
    // The row's own refusal, directly under the row it is about: a half-filled
    // Rule named by what it is missing. The tag editor places its one reason
    // above the Save bar and names the field in the sentence; here the
    // sentence is painted at the row, which is the ticket's own requirement.
    if let Some(SmartPlaylistRefusal::Row { slot, reason }) = refusal
        && slot == &ChildSlot::new(path.clone(), index)
    {
        ui.add_space(theme::SPACE_XS);
        ui.colored_label(palette.error, reason);
    }
}

/// One Unreadable Rule, drawn at the stored position it was read from: the
/// row's own words, the remove control that is the spec's only way out, and —
/// while Save has refused because of it — the reason under the row.
///
/// The row is not a Rule and is never rendered as one: ticket 14's row would
/// have to invent a field, a comparison and a value for ids this build cannot
/// map. It is words and a delete control, which is exactly the state the
/// listener has to act on: riff can show the row and take it away, and cannot
/// edit what it cannot read.
fn draw_unreadable_child(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    path: &BoxPath,
    index: usize,
    refusal: Option<&SmartPlaylistRefusal>,
    actions: &mut Vec<SettingsAction>,
) {
    let row_id = egui::Id::new(("smart_playlist_unreadable_row", path, index));
    let mut remove_pressed = false;
    ui.horizontal_top(|ui| {
        let row_w = (ui.available_width() - rule_box::REMOVE_W - theme::SPACE_SM).max(0.0);
        // A real label, not a painted run of text: the row's words have to be
        // in the accessibility tree for the golden and the a11y queries to
        // hold the row, and the refusal under it is asserted against its rect.
        ui.add_sized(
            egui::vec2(row_w, HEADER_H),
            egui::Label::new(
                egui::RichText::new(UNREADABLE_ROW_WORDS)
                    .color(palette.ink_2)
                    .size(theme::TEXT_XS),
            ),
        );
        ui.add_space(theme::SPACE_SM);
        remove_pressed = remove_button(
            ui,
            cache,
            palette,
            row_id.with("remove_rule"),
            "Remove rule",
        );
    });
    if remove_pressed {
        actions.push(SettingsAction::RemoveSmartPlaylistChild {
            parent: path.clone(),
            index,
        });
    }
    if let Some(SmartPlaylistRefusal::Row { slot, reason }) = refusal
        && slot == &ChildSlot::new(path.clone(), index)
    {
        ui.add_space(theme::SPACE_XS);
        ui.colored_label(palette.error, reason);
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

/// One Box's control line, wrapped in the shared row drag when the Box is a
/// child of another Box. `own_slot` is the Box's slot in its parent's
/// sibling list — `None` for the Root Box, which is no list's child, so it
/// neither drags nor accepts a drop. The second half of the answer is the
/// payload a drag released over the line carried, already checked against
/// this list (and `None` while the line is the floating copy).
fn draw_box_line(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    box_id: egui::Id,
    combine: Combine,
    listed: bool,
    own_slot: Option<&ChildSlot>,
) -> (BoxLineReport, Option<std::sync::Arc<RuleRowDrag>>) {
    if let Some(slot) = own_slot {
        let drag = draggable_rule_row(
            ui,
            palette,
            &slot.parent,
            slot.index,
            box_id.with("drag"),
            egui::vec2(ui.available_width(), HEADER_H),
            |ui| {
                let line = box_control_line(
                    ui,
                    cache,
                    palette,
                    box_id,
                    combine_label(combine),
                    listed,
                    true,
                );
                let response = line.response.clone();
                (line, response)
            },
        );
        if drag.in_flight {
            (drag.inner, None)
        } else {
            (drag.inner, drag.dropped)
        }
    } else {
        (
            box_control_line(
                ui,
                cache,
                palette,
                box_id,
                combine_label(combine),
                listed,
                false,
            ),
            None,
        )
    }
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
