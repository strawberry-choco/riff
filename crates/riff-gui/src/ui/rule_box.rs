//! The Rule Box draft model — the tree around ticket 14's Rule rows (Smart
//! Playlists ticket 16).
//!
//! The editor's in-progress tree is a [`RuleBoxDraft`]: one box holding
//! [`RuleChildDraft`]s in **one interleaved sibling order**, exactly the order
//! the store numbers and the pane draws. A Rule Box inside a Rule Box is the
//! same type again, so nesting is unbounded in the type and the depth cap
//! ([`MAX_RULE_BOX_LEVELS`]) is carried where it belongs: a *displayed*
//! editor rule, asked once by the paint through [`BoxPath::accepts_a_box`].
//! Nothing here counts levels on the way in or out — a tree deeper than the
//! cap still loads, still converts, and is still spellable, because a newer
//! riff's tree must answer rather than be refused.
//!
//! **The save path must not enforce the cap either.** [`RuleBoxDraft::to_root_box`]
//! spells the listener's tree for ticket 18's one-call Save, and it refuses
//! only what has no stored form at all: a half-filled Rule row makes it answer
//! [`None`] (never a silent drop), while an *empty* box converts as an empty
//! box — refusing that is ticket 18's save decision, not the model's.
//!
//! A box holds a `Vec` of children rather than parallel Rule and Box lists
//! because the store numbers siblings together and the pane draws one list;
//! the same shape is what lets a future [`RuleChildDraft`] variant (ticket
//! 18's Unreadable Rule placeholder) sit at its stored position without
//! restructuring anything.
//!
//! No egui, no store, no palette: this module is pure state and walks, which
//! is why its whole surface is testable with no context at all.

use riff_backend::domain::{Combine, RootBox, RuleBox, RuleChild};

use crate::ui::rule_row::{RuleDraft, RulePanel};

/// The most Rule Boxes a path from the Root Box may hold.
///
/// A level is a Rule Box and the Root Box is level 1, so a path holds at
/// most this many boxes and the add-box control disappears at the deepest
/// level rather than rejecting a save (story 33). It is an **editor rule and
/// a displayed one only**: nothing in the store or the query compiler counts
/// levels, and nothing in this module enforces the cap — the one place it is
/// asked is [`BoxPath::accepts_a_box`], which is what the pane's paint
/// consults before drawing the control. A tree deeper than this still loads,
/// still converts and still answers.
pub const MAX_RULE_BOX_LEVELS: usize = 7;

/// Where a Rule Box sits in the editor's draft: the child indices walked
/// from the Root Box.
///
/// The Root Box is the empty path, so a path of `n` indices names a box at
/// level `n + 1`. A path is *address*, not identity: it is derived from the
/// current shape of the tree, which is why nothing that can go stale — a
/// store id, a pointer — appears here (the store's ids never reach the
/// frontend at all).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct BoxPath(Vec<usize>);

impl BoxPath {
    /// The Root Box's path — the empty walk.
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// The path of the `index`-th child of the box this path names. Used for
    /// both a nested Box and a Rule row's parent slot; which one the child is
    /// is the caller's to know from its own walk.
    #[must_use]
    pub fn child(&self, index: usize) -> Self {
        let mut indices = self.0.clone();
        indices.push(index);
        Self(indices)
    }

    /// The child indices this path walks.
    #[must_use]
    pub fn indices(&self) -> &[usize] {
        &self.0
    }

    /// This box's level: the Root Box is level 1, one step per nested Box.
    #[must_use]
    pub const fn level(&self) -> usize {
        self.0.len() + 1
    }

    /// Whether a Rule Box may be added inside the box this path names — the
    /// one place [`MAX_RULE_BOX_LEVELS`] is asked. `false` means the pane
    /// draws no add-box control there: the cap is displayed by absence, and
    /// no message is ever shown for trying to exceed it.
    #[must_use]
    pub const fn accepts_a_box(&self) -> bool {
        self.level() < MAX_RULE_BOX_LEVELS
    }
}

/// One child slot: the path of its parent Box and the child's index in that
/// box's interleaved sibling list.
///
/// A Rule row's identity in the editor is its slot — ticket 14's "row
/// identity is the slot index", widened by ticket 16 to name the Box the
/// index belongs to. Ticket 17's count and ticket 19's reorder key on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChildSlot {
    /// The Box whose sibling list holds this child.
    pub parent: BoxPath,
    /// The child's index in the parent's one interleaved order.
    pub index: usize,
}

impl ChildSlot {
    /// The slot at `index` inside the Box at `parent`.
    #[must_use]
    pub const fn new(parent: BoxPath, index: usize) -> Self {
        Self { parent, index }
    }
}

/// One child of a Rule Box draft, in the box's one shared sibling order.
///
/// The two variants are the two things a listener can put in a box; ticket
/// 18's Unreadable Rule placeholder is a future sibling here, which is why a
/// box holds these rather than one `Vec<RuleDraft>` per box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleChildDraft {
    /// A leaf test, edited by ticket 14's row.
    Rule(RuleDraft),
    /// A nested box, holding more of both.
    Box(RuleBoxDraft),
}

/// One Rule Box's draft: how it combines its children, whether its combine
/// list is open, and its children in one interleaved order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleBoxDraft {
    /// Whether everything inside must match or any one thing may.
    pub combine: Combine,
    /// Which inline list is open — presentation state that travels with the
    /// box. The pane keeps at most one box's list open by holding the open
    /// path itself, so this flag is written for no one else; it exists so a
    /// cloned draft is self-describing.
    pub open: bool,
    /// The box's children, Rules and Boxes interleaved in the order they were
    /// put in.
    pub children: Vec<RuleChildDraft>,
}

impl RuleBoxDraft {
    /// A box that combines with `combine` and holds nothing yet.
    #[must_use]
    pub const fn empty(combine: Combine) -> Self {
        Self {
            combine,
            open: false,
            children: Vec::new(),
        }
    }

    /// The draft that re-shows a stored tree — the Root Box's and every nested
    /// box's children, in their stored order, never flattened.
    #[must_use]
    pub fn from_root_box(root: &RootBox) -> Self {
        Self::from_rule_box(root.inner())
    }

    /// The walk behind [`Self::from_root_box`].
    fn from_rule_box(boxed: &RuleBox) -> Self {
        Self {
            combine: boxed.combine(),
            open: false,
            children: boxed
                .children()
                .iter()
                .map(|child| match child {
                    RuleChild::Rule(rule) => RuleChildDraft::Rule(RuleDraft::from_rule(rule)),
                    RuleChild::Box(nested) => RuleChildDraft::Box(Self::from_rule_box(nested)),
                })
                .collect(),
        }
    }

    /// The tree this draft spells, or `None` while any Rule row is
    /// half-filled — the shape ticket 18's one-call Save writes.
    ///
    /// `None` rather than a partial tree: a conversion that skipped a
    /// half-filled row would silently rewrite the listener's work on the next
    /// Save, and ticket 18 names the offending row from its own walk before
    /// it ever asks this. An empty box converts as an empty box (the save's
    /// refusal to store one is ticket 18's decision), and the depth cap is
    /// not consulted: a tree deeper than [`MAX_RULE_BOX_LEVELS`] spells back
    /// exactly as it loaded.
    #[must_use]
    pub fn to_root_box(&self) -> Option<RootBox> {
        self.to_rule_box().map(RootBox::new)
    }

    /// The walk behind [`Self::to_root_box`].
    fn to_rule_box(&self) -> Option<RuleBox> {
        let children = self
            .children
            .iter()
            .map(|child| match child {
                RuleChildDraft::Rule(draft) => draft.rule().map(RuleChild::Rule),
                RuleChildDraft::Box(nested) => nested.to_rule_box().map(RuleChild::Box),
            })
            .collect::<Option<Vec<_>>>()?;
        Some(RuleBox::new(self.combine).with_children(children))
    }

    /// How many Rules this box and every box below it holds — the same
    /// question [`RuleBox::rule_count`] answers for a stored tree.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| match child {
                RuleChildDraft::Rule(_) => 1,
                RuleChildDraft::Box(nested) => nested.rule_count(),
            })
            .sum()
    }

    /// The box `path` names, if this box holds it.
    #[must_use]
    pub fn box_at(&self, path: &BoxPath) -> Option<&Self> {
        self.descend(path.indices())
    }

    /// The walk behind [`Self::box_at`]: one index deeper per nested Box.
    fn descend(&self, indices: &[usize]) -> Option<&Self> {
        let Some((first, rest)) = indices.split_first() else {
            return Some(self);
        };
        match self.children.get(*first)? {
            RuleChildDraft::Box(nested) => nested.descend(rest),
            RuleChildDraft::Rule(_) => None,
        }
    }

    /// The box `path` names, mutably — where every structural edit lands.
    #[must_use]
    pub fn box_at_mut(&mut self, path: &BoxPath) -> Option<&mut Self> {
        self.descend_mut(path.indices())
    }

    /// The walk behind [`Self::box_at_mut`].
    fn descend_mut(&mut self, indices: &[usize]) -> Option<&mut Self> {
        let Some((first, rest)) = indices.split_first() else {
            return Some(self);
        };
        match self.children.get_mut(*first)? {
            RuleChildDraft::Box(nested) => nested.descend_mut(rest),
            RuleChildDraft::Rule(_) => None,
        }
    }

    /// Append one blank Rule row to the box `parent` names. Does nothing when
    /// the path names no box — a path comes from the paint, so a miss means
    /// the tree moved under the gesture, and inventing a box would be worse
    /// than dropping the click.
    pub fn add_rule(&mut self, parent: &BoxPath) {
        if let Some(boxed) = self.box_at_mut(parent) {
            boxed
                .children
                .push(RuleChildDraft::Rule(RuleDraft::blank()));
        }
    }

    /// Append one new Box, holding one blank Rule, to the box `parent` names.
    ///
    /// The cap is deliberately not enforced here: the paint does not draw the
    /// control where [`BoxPath::accepts_a_box`] is `false`, so there is
    /// nothing to refuse, and this method must stay usable for a tree that
    /// already reads deeper than the cap.
    pub fn add_box(&mut self, parent: &BoxPath) {
        if let Some(boxed) = self.box_at_mut(parent) {
            let mut nested = Self::empty(Combine::All);
            nested
                .children
                .push(RuleChildDraft::Rule(RuleDraft::blank()));
            boxed.children.push(RuleChildDraft::Box(nested));
        }
    }

    /// Remove the child at `parent`/`index` — a Rule, or a whole Box **with
    /// everything inside it**. No emptying first; the removed subtree leaves
    /// with its root.
    pub fn remove_child(&mut self, parent: &BoxPath, index: usize) {
        if let Some(boxed) = self.box_at_mut(parent)
            && index < boxed.children.len()
        {
            boxed.children.remove(index);
        }
    }

    /// Move the child at `parent`/`from` to position `to` in the box's one
    /// interleaved sibling order — the reorder a drag-and-drop gesture
    /// reports as `(from, to)`. The math is
    /// [`reorder_tracks`](riff_backend::app::playlist_manager::reorder_tracks)'s,
    /// over the numbering Rules and Boxes share: remove the dragged child and
    /// reinsert it at `to`, and every child between shifts one step to close
    /// and open the gaps.
    ///
    /// A self-drop (`from == to`) and any out-of-range index change nothing,
    /// so an empty gesture never renumbers, dirties or saves anything. A path
    /// that names no box drops the gesture rather than inventing a box: a
    /// path comes from the paint, so a miss means the tree moved under the
    /// gesture and doing nothing is the only honest answer.
    pub fn move_child(&mut self, parent: &BoxPath, from: usize, to: usize) {
        if let Some(boxed) = self.box_at_mut(parent) {
            if from == to || from >= boxed.children.len() || to >= boxed.children.len() {
                return;
            }
            let dragged = boxed.children.remove(from);
            boxed.children.insert(to, dragged);
        }
    }

    /// Set the combine choice of the box `path` names.
    pub fn set_combine(&mut self, path: &BoxPath, combine: Combine) {
        if let Some(boxed) = self.box_at_mut(path) {
            boxed.combine = combine;
        }
    }

    /// Close every open value picker panel except the row at `keep` — the
    /// pane shows one library picker list at a time, because two would answer
    /// to one shared value list of the wrong field (ticket 14's host duty,
    /// widened to the tree).
    ///
    /// Closing only, never opening: the row that reported the picker has
    /// already opened its own panel through [`RuleDraft::apply`].
    pub fn close_value_panels_except(&mut self, keep: &ChildSlot) {
        self.close_value_panels(&BoxPath::root(), keep);
    }

    /// The walk behind [`Self::close_value_panels_except`].
    fn close_value_panels(&mut self, path: &BoxPath, keep: &ChildSlot) {
        for (index, child) in self.children.iter_mut().enumerate() {
            match child {
                RuleChildDraft::Rule(draft) => {
                    if path == &keep.parent && index == keep.index {
                        continue;
                    }
                    if draft.open == Some(RulePanel::Value) {
                        draft.open = None;
                    }
                }
                RuleChildDraft::Box(nested) => {
                    nested.close_value_panels(&path.child(index), keep);
                }
            }
        }
    }
}
