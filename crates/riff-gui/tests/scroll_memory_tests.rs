//! The Scroll Memory's protocol, asserted at its own interface.
//!
//! The module is pure, deterministic and in-process, so its decisions — which
//! slot a list reads, whether a selection is drill bookkeeping, when an offset
//! goes stale — are answerable directly. What a frame still has to prove is
//! that egui applied the offset the module handed out; that survives in the
//! whole-frame suite at the workspace root (`tests/ui_tests.rs`), one test per
//! protocol shape.
//!
//! A render site declares WHERE it renders — a named Section root, or a Drill
//! Column — and passes its content (the query and the sort direction). Which
//! protocol shape applies, which salt keys the list, what the content identity
//! is, and whether a selection at that slot is drill bookkeeping are all
//! answered here.

use riff_backend::app::events::BackendEvent;
use riff_backend::app::state::LibrarySection;
use riff_gui::ui::scroll_memory::{DrillSlot, ListSlot, ScrollMemory};

/// Render one Section root list through the module and hand back the offset it
/// started at, exactly as a render site does: declare the slot and the content,
/// render, record what the frame ended at.
fn visit_section(
    memory: &mut ScrollMemory,
    section: LibrarySection,
    query: &str,
    sort_desc: bool,
    actual_offset: f32,
) -> Option<f32> {
    let (control, visit) = memory.begin_section(section, query, sort_desc);
    let start = control.start;
    memory.end_section(visit, actual_offset);
    start
}

#[test]
fn a_section_root_restores_its_own_slot_and_leaves_another_section_at_the_top() {
    // The frame test this replaces scrolled two roots and compared painted row
    // positions; the decision underneath it is that slots are keyed per
    // Section, so each root returns to the place it left.
    let mut memory = ScrollMemory::default();

    // Scroll the Artists root deep, then the Genres root shallow.
    assert_eq!(
        visit_section(&mut memory, LibrarySection::Artists, "", false, 1_200.0),
        Some(0.0),
        "a Section with nothing saved starts at the top"
    );
    assert_eq!(
        visit_section(&mut memory, LibrarySection::Genres, "", false, 400.0),
        Some(0.0),
        "and so does the next Section — no slot is shared"
    );

    // Reselecting each Section restores its own offset, not the other's.
    assert_eq!(
        visit_section(&mut memory, LibrarySection::Artists, "", false, 1_200.0),
        Some(1_200.0),
        "Artists returns exactly where it was left"
    );
    assert_eq!(
        visit_section(&mut memory, LibrarySection::Genres, "", false, 400.0),
        Some(400.0),
        "Genres remembers its own, shallower place"
    );
}

#[test]
fn a_content_change_resets_only_the_list_whose_content_moved() {
    // Query and sort are the content a saved offset belongs to: a new query is
    // new content, so the offset is not the listener's place any more.
    let mut memory = ScrollMemory::default();

    visit_section(&mut memory, LibrarySection::Albums, "", false, 900.0);

    assert_eq!(
        memory
            .begin_section(LibrarySection::Albums, "geo", false)
            .0
            .start,
        Some(0.0),
        "a different query resets to the top rather than restoring a stale offset"
    );
    assert_eq!(
        memory
            .begin_section(LibrarySection::Albums, "", true)
            .0
            .start,
        Some(0.0),
        "and so does a different sort direction"
    );
    assert_eq!(
        memory
            .begin_section(LibrarySection::Albums, "", false)
            .0
            .start,
        Some(900.0),
        "still remembered for the content it was saved under"
    );
    assert_eq!(
        memory
            .begin_section(LibrarySection::Artists, "geo", false)
            .0
            .start,
        Some(0.0),
        "another Section is untouched by this one's query"
    );
}

#[test]
fn a_recorded_section_keeps_the_content_it_was_saved_under() {
    // Nothing outside the module holds a content identity, so the constraint
    // that the start call and the record call must agree is gone: the visit
    // token carries it from one to the other.
    let mut memory = ScrollMemory::default();

    let (control, visit) = memory.begin_section(LibrarySection::AllTracks, "one", false);
    assert_eq!(control.start, Some(0.0));
    memory.end_section(visit, 640.0);

    // The offset recorded under query "one" must not resurface under "two".
    assert_eq!(
        memory
            .begin_section(LibrarySection::AllTracks, "two", false)
            .0
            .start,
        Some(0.0)
    );
    assert_eq!(
        memory
            .begin_section(LibrarySection::AllTracks, "one", false)
            .0
            .start,
        Some(640.0)
    );
}

#[test]
fn a_committed_library_change_stales_every_saved_offset() {
    // The generation the module observed rides in the content identity, so a
    // rescan that commits rows resets the lists whose content moved — and a
    // Section that re-renders after the change records under the new one.
    let mut memory = ScrollMemory::default();
    visit_section(&mut memory, LibrarySection::Genres, "", false, 700.0);

    memory.note_backend_events(&[BackendEvent::LibraryChanged { generation: 1 }]);

    assert_eq!(
        memory
            .begin_section(LibrarySection::Genres, "", false)
            .0
            .start,
        Some(0.0),
        "the saved offset belongs to the pre-scan listing"
    );
    assert_eq!(
        visit_section(&mut memory, LibrarySection::Genres, "", false, 300.0),
        Some(0.0),
        "the reset frame renders from the top"
    );
    assert_eq!(
        memory
            .begin_section(LibrarySection::Genres, "", false)
            .0
            .start,
        Some(300.0),
        "and the offset is remembered again under the post-scan identity"
    );
}

#[test]
fn an_empty_event_batch_leaves_saved_offsets_current() {
    // Only a Library generation move goes stale; draining a frame with nothing
    // in it must not cost the listener their place.
    let mut memory = ScrollMemory::default();
    visit_section(&mut memory, LibrarySection::Artists, "", false, 800.0);

    memory.note_backend_events(&[]);
    memory.note_backend_events(&[BackendEvent::PlaylistsChanged { generation: 1 }]);

    assert_eq!(
        memory
            .begin_section(LibrarySection::Artists, "", false)
            .0
            .start,
        Some(800.0),
        "an unrelated event does not reset a Section"
    );
}

#[test]
fn a_drill_column_resets_once_per_selection_and_then_keeps_eguis_state() {
    // Between selection changes the column's scroll is egui's own, which is
    // why the drill answer is `None` and not `Some(0.0)`.
    let mut memory = ScrollMemory::default();

    assert_eq!(memory.begin_drill(DrillSlot::ArtistAlbums).start, None);
    memory.note_selection_in(ListSlot::Drill(DrillSlot::ArtistAlbums));
    assert_eq!(
        memory.begin_drill(DrillSlot::ArtistAlbums).start,
        Some(0.0),
        "the frame a selection reaches the column forces the top"
    );
    assert_eq!(
        memory.begin_drill(DrillSlot::ArtistAlbums).start,
        None,
        "the column has seen this epoch; egui continues"
    );
}

#[test]
fn re_selecting_the_same_entity_still_resets_the_drill_column() {
    // The epoch is a selection counter, not a selection identity: clicking the
    // artist that is already selected is still a change of context for the
    // column below it.
    let mut memory = ScrollMemory::default();
    memory.begin_drill(DrillSlot::GenreArtists);

    memory.note_selection_in(ListSlot::Section(LibrarySection::Genres));
    assert_eq!(
        memory.begin_drill(DrillSlot::GenreArtists).start,
        Some(0.0),
        "the new selection reaches the column"
    );

    memory.note_selection_in(ListSlot::Section(LibrarySection::Genres));
    assert_eq!(
        memory.begin_drill(DrillSlot::GenreArtists).start,
        Some(0.0),
        "and so does selecting again, even the same row"
    );
}

#[test]
fn one_selection_resets_every_drill_column_that_can_be_on_screen() {
    let mut memory = ScrollMemory::default();
    memory.note_selection_in(ListSlot::Section(LibrarySection::Artists));

    for slot in [
        DrillSlot::ArtistAlbums,
        DrillSlot::GenreArtists,
        DrillSlot::GenreArtistAlbums,
    ] {
        assert_eq!(
            memory.begin_drill(slot).start,
            Some(0.0),
            "each column resets on the frame the selection lands"
        );
    }
}

#[test]
fn the_module_decides_which_slots_selection_bookkeeping_belongs_in() {
    // The rule, in one place: a selection that opens or reopens a Drill Column
    // bumps the epoch; a Track selection does not, because nothing below it
    // resets. All Tracks' rows and the Tracks column's rows select Tracks, so
    // bookkeeping is not theirs — and the two sites that render them never had
    // to know that.
    let mut memory = ScrollMemory::default();
    memory.begin_drill(DrillSlot::TracksColumn);
    memory.begin_drill(DrillSlot::ArtistAlbums);

    // A Track selection: no bookkeeping.
    memory.note_selection_in(ListSlot::Section(LibrarySection::AllTracks));
    memory.note_selection_in(ListSlot::Drill(DrillSlot::TracksColumn));
    assert_eq!(
        memory.begin_drill(DrillSlot::ArtistAlbums).start,
        None,
        "a Track selection leaves every column on egui's own state"
    );

    // An entity selection: bookkeeping.
    memory.note_selection_in(ListSlot::Drill(DrillSlot::ArtistAlbums));
    assert_eq!(
        memory.begin_drill(DrillSlot::ArtistAlbums).start,
        Some(0.0),
        "drilling into an artist resets the column it just opened"
    );
    assert_eq!(
        memory.begin_drill(DrillSlot::TracksColumn).start,
        Some(0.0),
        "and the Tracks column, which resets on any selection change"
    );
}

#[test]
fn each_slot_keys_eguis_state_by_its_own_identity() {
    // User story 20: no two lists can accidentally share scroll state, because
    // salts are per slot rather than positional.
    let mut memory = ScrollMemory::default();

    let artists = memory.begin_section(LibrarySection::Artists, "", false).0;
    let albums = memory.begin_section(LibrarySection::Albums, "", false).0;
    let drill = memory.begin_drill(DrillSlot::ArtistAlbums);
    let tracks = memory.begin_drill(DrillSlot::TracksColumn);

    assert_eq!(artists.salt, "scroll:artists");
    assert_eq!(albums.salt, "scroll:albums");
    assert_eq!(drill.salt, "scroll:drill-artist-albums");
    assert_eq!(tracks.salt, "scroll:tracks-column");
    let salts = [artists.salt, albums.salt, drill.salt, tracks.salt];
    for salt in salts {
        assert_eq!(
            salts.iter().filter(|candidate| **candidate == salt).count(),
            1,
            "{salt} identifies exactly one slot"
        );
    }
}
