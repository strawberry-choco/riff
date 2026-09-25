//! Scroll Memory across the Library Sections (scroll-memory spec).
//!
//! Each of the four Library Sections — All Tracks, Artists, Albums, Genres —
//! remembers its own root-list scroll position for as long as the app runs.
//! A Section slot records a scroll offset plus a content fingerprint — the
//! search query, the A–Z / Z–A sort direction, and the library generation at
//! the moment the offset was saved. Restoring a slot whose fingerprint no
//! longer matches resets to the top instead of restoring a stale offset. The
//! fingerprint is the module's own: a site passes the query and the sort it is
//! rendering, never a generation, so the store's counter reaches no UI code
//! through this seam (CONTEXT.md's `SessionViews` rule).
//!
//! Drill columns (an artist's Albums, a Genre's Artists, a Genre artist's
//! Albums) and the Tracks column reset to the top on every selection change:
//! the Scroll Memory holds no drill offsets by design. A selection epoch is
//! bumped on every entity selection, so a drill column can detect ANY
//! selection change — including re-selecting the same entity — and force the
//! top. Between selection changes the column's scroll is egui's natural
//! per-widget state under the column's own stable salt.
//!
//! # The protocol is the module's, not the render site's
//!
//! A site declares **where** it renders — [`ListSlot::Section`] for a root
//! list, [`ListSlot::Drill`] for a column — and asks for a control
//! ([`ScrollMemory::begin_section`] / [`ScrollMemory::begin_drill`]), then
//! records what the frame ended at. Which of the two protocol shapes applies,
//! which salt keys the list, and whether a selection at that slot is drill
//! bookkeeping are all answered here: a site cannot pick the wrong shape, and
//! the two slots whose rows select Tracks rather than entities
//! (`AllTracks`, `TracksColumn`) are exempted by [`selection_bumps`] instead of
//! by each site remembering to omit a call.
//!
//! Sections and Drill Columns stay two operations because they are two domain
//! terms with genuinely different reset semantics: one remembers a position
//! per content identity, the other forces the top on every selection.
//!
//! Everything is in-memory: restarting the app starts every list at the top,
//! nothing writes to the Application Store, and nothing crosses the session
//! boundary (user story 21). The library generation is observed from the
//! drained backend event inbox ([`BackendEvent::LibraryChanged`]) — a
//! committed rescan bumps it, which turns every slot's fingerprint stale and
//! resets the affected lists.

use riff_backend::app::events::BackendEvent;
use riff_backend::app::state::LibrarySection;

/// The four Library Sections in slot order — the index a section's slot
/// lives at. Matches `LibrarySection`'s variants one to one.
const SECTIONS: [LibrarySection; 4] = [
    LibrarySection::AllTracks,
    LibrarySection::Artists,
    LibrarySection::Albums,
    LibrarySection::Genres,
];

/// Stable per-slot scroll salts: the browser lists key egui's scroll state by
/// content identity instead of column position, so no two Sections (or drill
/// contexts) can accidentally share scroll state (user story 20). The old
/// shared `"browser_list_rows"` positional salt is never used for a
/// Section-scoped list.
const SALT_ALL_TRACKS: &str = "scroll:all-tracks";
const SALT_ARTISTS: &str = "scroll:artists";
const SALT_ALBUMS: &str = "scroll:albums";
const SALT_GENRES: &str = "scroll:genres";
/// Drill-column salts: stable identities distinct from the Section slots.
const SALT_DRILL_ARTIST_ALBUMS: &str = "scroll:drill-artist-albums";
const SALT_DRILL_GENRE_ARTISTS: &str = "scroll:drill-genre-artists";
const SALT_DRILL_GENRE_ARTIST_ALBUMS: &str = "scroll:drill-genre-artist-albums";
const SALT_TRACKS_COLUMN: &str = "scroll:tracks-column";

/// The scroll salt that keys a Section's root list.
fn section_salt(section: LibrarySection) -> &'static str {
    match section {
        LibrarySection::AllTracks => SALT_ALL_TRACKS,
        LibrarySection::Artists => SALT_ARTISTS,
        LibrarySection::Albums => SALT_ALBUMS,
        LibrarySection::Genres => SALT_GENRES,
    }
}

/// The content identity a Section slot's offset was captured against: the
/// search query, the sort direction, and the library generation at the moment
/// the offset was saved (issue 06).
///
/// Private on purpose. A render site passes the ingredients it already holds —
/// the query and the sort — and the generation is the module's own observation
/// of the drained event inbox, so nothing outside can build, compare, or
/// mis-time a fingerprint. The constraint that the offset a frame starts from
/// and the offset it records must be "the same value" is gone because the
/// [`SectionVisit`] token carries the identity from one call to the other, and
/// no caller holds one to mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentFingerprint {
    query: String,
    sort_desc: bool,
    library_generation: u64,
}

/// One Section's remembered scroll offset plus the fingerprint it was
/// captured under; a fingerprint mismatch on the next frame restore resets
/// to the top.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionSlot {
    offset: f32,
    fingerprint: ContentFingerprint,
}

/// The drill/Tracks-column slots, each with its own stable salt and its own
/// reset bookkeeping. In a fixed order for the epoch array's indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum DrillSlot {
    /// An artist's Albums column (Artists level 1).
    ArtistAlbums = 0,
    /// A Genre's Artists column (Genres level 1).
    GenreArtists = 1,
    /// A Genre artist's Albums column (Genres level 2).
    GenreArtistAlbums = 2,
    /// The Tracks column (a selected album's or artist's track listing).
    TracksColumn = 3,
}

impl DrillSlot {
    /// The column's stable scroll salt, distinct from every Section slot.
    fn salt(self) -> &'static str {
        match self {
            Self::ArtistAlbums => SALT_DRILL_ARTIST_ALBUMS,
            Self::GenreArtists => SALT_DRILL_GENRE_ARTISTS,
            Self::GenreArtistAlbums => SALT_DRILL_GENRE_ARTIST_ALBUMS,
            Self::TracksColumn => SALT_TRACKS_COLUMN,
        }
    }
}

/// **Where** a list renders this frame — the declaration a render site makes,
/// and everything the module needs to pick the protocol shape from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSlot {
    /// A Section's root list.
    Section(LibrarySection),
    /// A Drill Column or the Tracks column, at a stable slot.
    Drill(DrillSlot),
}

/// Scroll handling for one list column this frame, as the widgets' seam:
/// the stable per-slot salt that keys the column's egui `ScrollArea` state
/// and the offset to start the frame at (the Scroll Memory's saved value, or
/// 0 on a reset). `start: None` leaves egui's own state untouched — a drill
/// column between selection changes.
#[derive(Debug, Clone, Copy)]
pub struct ScrollControl {
    pub salt: &'static str,
    pub start: Option<f32>,
}

/// One Section root's open visit: the Section plus the content identity the
/// offset was handed out under. [`ScrollMemory::end_section`] takes it back,
/// so a site cannot record an offset under a fingerprint other than the one it
/// rendered with — the pair travels together instead of being re-derived twice
/// and compared.
#[derive(Debug)]
pub struct SectionVisit {
    section: LibrarySection,
    fingerprint: ContentFingerprint,
}

/// The in-memory scroll record. One slot per Section (the root lists), the
/// per-drill reset bookkeeping, and the observed library generation. The
/// single source of truth between frames: before a Section's root list
/// renders, its saved offset (or zero when the fingerprint is stale) is
/// applied; after the frame, the actual offset is read back and recorded,
/// making the memory independent of egui's internal widget state.
#[derive(Debug, Default)]
pub struct ScrollMemory {
    /// The four Section slots, indexed by [`SECTIONS`] position.
    slots: [Option<SectionSlot>; 4],
    /// Bumped on every entity selection, so a drill column can detect ANY
    /// selection change — including re-selecting the same entity.
    selection_epoch: u64,
    /// The selection epoch each drill slot last reset at.
    drill_epochs: [u64; 4],
    /// The highest Library generation observed in the drained backend events.
    library_generation: u64,
}

/// Whether a selection at `at` is drill bookkeeping. Two slots render Tracks
/// rather than entities — the All Tracks root's rows and the Tracks column's —
/// and a Track selection opens nothing below it, so it bumps nothing. Every
/// other slot's selection opens or reopens a Drill Column. The rule lives here
/// so no render site has to know it.
fn selection_bumps(at: ListSlot) -> bool {
    let selects_tracks = matches!(
        at,
        ListSlot::Section(LibrarySection::AllTracks) | ListSlot::Drill(DrillSlot::TracksColumn)
    );
    !selects_tracks
}

impl ScrollMemory {
    /// Fold drained backend events into the observed library generation: a
    /// committed rescan bumps it, which turns every slot's fingerprint stale.
    pub fn note_backend_events(&mut self, events: &[BackendEvent]) {
        for event in events {
            if let BackendEvent::LibraryChanged { generation } = event {
                self.library_generation = (*generation).max(self.library_generation);
            }
        }
    }

    /// Begin a Section's root list: the control to render with, and the visit
    /// to hand back to [`Self::end_section`]. The site passes only its slot and
    /// its content — the current query and sort direction — and the module
    /// composes the content identity with the generation it observed itself.
    /// The start is the saved offset when that identity matches the slot's, and
    /// zero on a stale slot (or when none is saved yet) — a reset, never a dead
    /// offset.
    pub fn begin_section(
        &mut self,
        section: LibrarySection,
        query: &str,
        sort_desc: bool,
    ) -> (ScrollControl, SectionVisit) {
        let fingerprint = ContentFingerprint {
            query: query.to_owned(),
            sort_desc,
            library_generation: self.library_generation,
        };
        let start = match &self.slots[slot_index(section)] {
            Some(slot) if slot.fingerprint == fingerprint => slot.offset,
            _ => 0.0,
        };
        (
            ScrollControl {
                salt: section_salt(section),
                start: Some(start),
            },
            SectionVisit {
                section,
                fingerprint,
            },
        )
    }

    /// Record the end of a Section's frame: the offset the list actually ended
    /// at becomes the slot, under the content identity it was rendered with.
    pub fn end_section(&mut self, visit: SectionVisit, offset: f32) {
        self.slots[slot_index(visit.section)] = Some(SectionSlot {
            offset,
            fingerprint: visit.fingerprint,
        });
    }

    /// Begin a Drill Column: its own stable salt, and the start offset for this
    /// frame — `Some(0.0)` on the frame a selection reaches the column (it
    /// resets to the top; there is no per-selection memory), `None` once the
    /// column has seen the current epoch, which leaves egui's own state to
    /// continue.
    pub fn begin_drill(&mut self, slot: DrillSlot) -> ScrollControl {
        let start = if self.drill_epochs[slot as usize] == self.selection_epoch {
            None
        } else {
            self.drill_epochs[slot as usize] = self.selection_epoch;
            Some(0.0)
        };
        ScrollControl {
            salt: slot.salt(),
            start,
        }
    }

    /// Note that a selection happened in this slot. The module decides whether
    /// that is drill bookkeeping ([`selection_bumps`]); either way the site
    /// states only where it is, never which protocol applies.
    pub fn note_selection_in(&mut self, at: ListSlot) {
        if selection_bumps(at) {
            self.selection_epoch = self.selection_epoch.saturating_add(1);
        }
    }
}

/// The slot array index for a Section, via the fixed [`SECTIONS`] order.
fn slot_index(section: LibrarySection) -> usize {
    SECTIONS
        .iter()
        .position(|candidate| *candidate == section)
        .expect("every LibrarySection variant has a slot")
}
