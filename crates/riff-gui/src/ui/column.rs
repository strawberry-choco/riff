//! One entity-Column's identity: the Section it belongs to, the depth its
//! rows select at, and the Scroll Memory slot it owns — stated **once**, and
//! then never supplied again.
//!
//! A Section root and a Drill Column are both "a list of entities in the
//! browser stage", and both answer the same three actions. What used to differ
//! between them was spelled out at six render sites: each one opened its drain
//! with the same `if action.selects_a_row() { note_selection_in(<a slot this
//! site wrote out>) }` block, and each supplied its own Section, its own
//! depth, and its own slot — in three places for a drill (the scroll handshake,
//! the guard, and the sort-flip reset) and in two for a root. A new entity-
//! Column therefore had to remember the block *and* pick the right slot, and a
//! wrong slot is a rendering bug nothing reports.
//!
//! [`ColumnIdentity`] is that declaration as data. The guard becomes a method on
//! it, so it is a property of the Column rather than a statement six drains each
//! repeat, and the Scroll Memory wiring and the guard read the *same* value — so
//! they cannot disagree. The dispatch that consumes it lives beside the app
//! (`app/browser_pane.rs`), because it reaches the Transport and the Session
//! Views seam; this module names neither, so it stays pure and in-process and
//! its decisions are answerable directly, beside
//! [`crate::ui::scroll_memory`].

use riff_backend::app::state::LibrarySection;

use crate::ui::browser::BrowserAction;
use crate::ui::scroll_memory::{DrillSlot, ListSlot};

/// What one entity-Column IS, declared once at its render site and carried into
/// its drain.
///
/// The two constructors are the two shapes the app has — a Section root and a
/// Drill Column — and there is no third: what differs between two Columns of
/// the same shape is *data*, not code, which is what makes "add a Column" a
/// one-line change rather than a block to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnIdentity {
    /// A Section's root entity listing (Artists / Albums / Genres). Its rows
    /// select at depth 0, and the Scroll Memory slot is the Section's own.
    Root {
        /// The Section this root lists.
        section: LibrarySection,
    },
    /// A Drill Column below a root. Its rows select at `level`, and the Scroll
    /// Memory slot is a drill slot of its own.
    Drill {
        /// The Section the drill runs under — the one whose stage it is.
        section: LibrarySection,
        /// The depth this Column's rows select at: 1 or 2.
        level: usize,
        /// This Column's own drill slot.
        slot: DrillSlot,
    },
}

impl ColumnIdentity {
    /// A Section's root entity listing.
    #[must_use]
    pub fn root(section: LibrarySection) -> Self {
        Self::Root { section }
    }

    /// A Drill Column, `level` deep under `section`, owning `slot`.
    #[must_use]
    pub fn drill(section: LibrarySection, level: usize, slot: DrillSlot) -> Self {
        Self::Drill {
            section,
            level,
            slot,
        }
    }

    /// The Section this Column belongs to.
    #[must_use]
    pub fn section(self) -> LibrarySection {
        match self {
            Self::Root { section } | Self::Drill { section, .. } => section,
        }
    }

    /// The depth this Column's rows select at. A root's is 0 — its rows land
    /// the path at the top, so drill-down restarts from the Section.
    #[must_use]
    pub fn level(self) -> usize {
        match self {
            Self::Root { .. } => 0,
            Self::Drill { level, .. } => level,
        }
    }

    /// The Scroll Memory slot this Column owns. This is the ONE value both the
    /// scroll handshake and the selection guard read, so a Column's list and its
    /// guard cannot name different slots.
    #[must_use]
    pub fn slot(self) -> ListSlot {
        match self {
            Self::Root { section } => ListSlot::Section(section),
            Self::Drill { slot, .. } => ListSlot::Drill(slot),
        }
    }

    /// The drill slot a sort flip resets, when this Column has one.
    ///
    /// `None` for a Section root, and that absence is the whole difference
    /// between the two dispatch bindings: a drill's A–Z control lives above its
    /// list, so the flip is content identity the list's own egui state cannot
    /// see and the slot is forced back to the top; a root's flip is not reset
    /// that way. The rule is one binding's arm; the fact that a root is exempt
    /// is a missing value here rather than a second copy of the arm.
    #[must_use]
    pub fn drill_slot(self) -> Option<DrillSlot> {
        match self {
            Self::Root { .. } => None,
            Self::Drill { slot, .. } => Some(slot),
        }
    }

    /// The Scroll Memory slot `action` notes, or `None` for an action that
    /// moves no selection.
    ///
    /// **The guard, as a property of the Column.** `selects_a_row` is the one
    /// predicate that answers "did the selection move?", and a right-click
    /// moves it exactly as a click does — so a right-click that moved the
    /// selection but not the scroll would leave the list pointing at the wrong
    /// place. The answer is this Column's own slot, taken from the same value
    /// its scroll handshake used.
    #[must_use]
    pub fn note_scroll_for(self, action: &BrowserAction) -> Option<ListSlot> {
        action.selects_a_row().then_some(self.slot())
    }
}
