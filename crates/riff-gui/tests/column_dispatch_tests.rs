//! The entity-Column dispatch's identity, asserted at its own interface.
//!
//! The module is pure, deterministic and in-process — like the Scroll Memory
//! beside it — so its decisions are answerable directly: which Section a
//! Column belongs to, the depth its rows select at, the Scroll Memory slot it
//! owns, and therefore which slot a selecting action notes. No store, no
//! rendering, no frame.
//!
//! What this file is really about is the **guard becoming structural**. Before
//! it, six render sites each opened their drain with the same
//! `if action.selects_a_row() { note_selection_in(<a slot each site spelled
//! out>) }` block, so a new Column had to remember the block *and* pick the
//! right slot. Now the slot is part of the Column's identity, declared once,
//! and the guard is a method on it: a Column cannot forget the guard because
//! there is no place to forget it, and it cannot name the wrong slot because
//! the Scroll Memory wiring and the guard read the *same* value — which is the
//! first test's whole claim.

use riff_backend::app::state::LibrarySection;
use riff_gui::ui::browser::BrowserAction;
use riff_gui::ui::column::ColumnIdentity;
use riff_gui::ui::menu::ListMenuIntent;
use riff_gui::ui::scroll_memory::{DrillSlot, ListSlot};

/// Every entity-Column in the app, with the identity it states once.
///
/// **This list is the change's whole claim, and it is a list, not six copies
/// of a block.** Adding a seventh entity-Column means adding one line here and
/// one line at its render site — there is no drain to copy, because the drain
/// asks this identity which slot it owns.
fn every_entity_column() -> Vec<(&'static str, ColumnIdentity)> {
    vec![
        (
            "Artists root",
            ColumnIdentity::root(LibrarySection::Artists),
        ),
        ("Albums root", ColumnIdentity::root(LibrarySection::Albums)),
        ("Genres root", ColumnIdentity::root(LibrarySection::Genres)),
        (
            "artist's Albums drill",
            ColumnIdentity::drill(LibrarySection::Artists, 1, DrillSlot::ArtistAlbums),
        ),
        (
            "genre's Artists drill",
            ColumnIdentity::drill(LibrarySection::Genres, 1, DrillSlot::GenreArtists),
        ),
        (
            "genre artist's Albums drill",
            ColumnIdentity::drill(LibrarySection::Genres, 2, DrillSlot::GenreArtistAlbums),
        ),
    ]
}

/// Every action shape an entity-Column's drain can be handed, and whether it
/// moves the row selection.
fn every_action_shape() -> Vec<(&'static str, BrowserAction, bool)> {
    vec![
        ("a click", BrowserAction::Select("a".to_string()), true),
        (
            "a right-click that opened a menu and chose nothing",
            BrowserAction::ContextMenu {
                key: "a".to_string(),
                intents: Vec::new(),
            },
            true,
        ),
        (
            "a right-click that also chose an item",
            BrowserAction::ContextMenu {
                key: "a".to_string(),
                intents: vec![ListMenuIntent::Play],
            },
            true,
        ),
        ("the sort control", BrowserAction::ToggleSort, false),
    ]
}

/// The guard is a property of the identity, not a statement each site makes.
///
/// For EVERY Column, a selecting action notes **that Column's own slot** — the
/// very value its Scroll Memory wiring was built with — and a non-selecting
/// action notes nothing. This is the test that fails if a new Column is wired to
/// the wrong slot: the slot the guard names and the slot the Column declared
/// are the same value here, so they cannot drift apart the way two hand-written
/// mentions of a slot could.
#[test]
fn a_selecting_action_notes_the_columns_own_slot_and_nothing_else_does() {
    for (name, column) in every_entity_column() {
        for (shape, action, selects) in every_action_shape() {
            let noted = column.note_scroll_for(&action);
            if selects {
                assert_eq!(
                    noted,
                    Some(column.slot()),
                    "{name}: {shape} selects, so the guard must note the Column's own \
                     slot {:?}",
                    column.slot()
                );
            } else {
                assert_eq!(
                    noted, None,
                    "{name}: {shape} selects nothing, so the guard must move nothing"
                );
            }
        }
    }
}

/// The two shapes, stated as the data they are: a Section root's rows select at
/// depth 0 and own a Section slot, a Drill Column's at 1 or 2 and own a Drill
/// slot. A root has no drill slot, which is exactly why a root's sort flip does
/// not reset a scroll and a drill's does — the difference between the two
/// dispatch bindings, readable here as one absent value rather than as two
/// hand-written match arms.
#[test]
fn a_root_and_a_drill_differ_only_in_the_data_they_state() {
    let (_, root) = every_entity_column()[0];
    let (_, drill) = every_entity_column()[3];

    assert_eq!(root.level(), 0, "a root's rows select at the root");
    assert_eq!(
        root.slot(),
        ListSlot::Section(LibrarySection::Artists),
        "and its Scroll Memory slot is its own Section"
    );
    assert_eq!(
        root.drill_slot(),
        None,
        "a root owns no drill slot, which is why its sort flip resets no scroll"
    );

    assert_eq!(drill.level(), 1, "a drill's rows select below the root");
    assert_eq!(
        drill.slot(),
        ListSlot::Drill(DrillSlot::ArtistAlbums),
        "and its Scroll Memory slot is its own drill slot"
    );
    assert_eq!(
        drill.drill_slot(),
        Some(DrillSlot::ArtistAlbums),
        "which its sort flip names, so the reset lands on its own list"
    );
}

/// The six identities themselves, pinned. A Column wired to the wrong Section
/// or the wrong depth would pass the guard test above — it would still be
/// self-consistent — and would select the wrong entity, so the triples are
/// stated here rather than left to the render sites to agree about.
#[test]
fn the_six_entity_columns_state_these_identities() {
    let stated: Vec<(&str, LibrarySection, usize, ListSlot)> = every_entity_column()
        .into_iter()
        .map(|(name, column)| (name, column.section(), column.level(), column.slot()))
        .collect();

    assert_eq!(
        stated,
        vec![
            (
                "Artists root",
                LibrarySection::Artists,
                0,
                ListSlot::Section(LibrarySection::Artists),
            ),
            (
                "Albums root",
                LibrarySection::Albums,
                0,
                ListSlot::Section(LibrarySection::Albums),
            ),
            (
                "Genres root",
                LibrarySection::Genres,
                0,
                ListSlot::Section(LibrarySection::Genres),
            ),
            (
                "artist's Albums drill",
                LibrarySection::Artists,
                1,
                ListSlot::Drill(DrillSlot::ArtistAlbums),
            ),
            (
                "genre's Artists drill",
                LibrarySection::Genres,
                1,
                ListSlot::Drill(DrillSlot::GenreArtists),
            ),
            (
                "genre artist's Albums drill",
                LibrarySection::Genres,
                2,
                ListSlot::Drill(DrillSlot::GenreArtistAlbums),
            ),
        ],
        "each entity-Column states its Section, its depth and its Scroll Memory slot \
         once, and they are these"
    );
}

/// The identity is the *only* thing the guard reads. A Column cannot be given a
/// second slot, or a per-action override, because there is no parameter to pass
/// one: this is the compile-time half of the previous test, and it is the
/// reason a new Column inherits the reset behaviour by stating its identity
/// rather than by remembering a block.
#[test]
fn the_guard_reads_the_identity_and_the_action_and_nothing_else() {
    // Two Columns differing ONLY in their slot get different answers from the
    // same action, which is what "the slot is the identity's" means.
    let click = BrowserAction::Select("a".to_string());
    let (_, artists_root) = every_entity_column()[0];
    let (_, albums_root) = every_entity_column()[1];

    assert_ne!(
        artists_root.note_scroll_for(&click),
        albums_root.note_scroll_for(&click),
        "two roots given the same action answer with their OWN slots, so the slot \
         cannot be a constant in the guard"
    );

    // And a Column's answer does not depend on which action selected, only on
    // whether one did — so the rule is about the ACTION, the slot is the
    // IDENTITY's, and the two questions are separately answered.
    assert_eq!(
        artists_root.note_scroll_for(&click),
        artists_root.note_scroll_for(&BrowserAction::ContextMenu {
            key: "a".to_string(),
            intents: Vec::new(),
        }),
        "a click and a right-click select the same way, so the guard agrees"
    );
}
