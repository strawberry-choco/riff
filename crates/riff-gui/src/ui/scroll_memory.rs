//! Scroll Memory across the Library Sections (scroll-memory spec).
//!
//! Each of the four Library Sections — All Tracks, Artists, Albums, Genres —
//! remembers its own root-list scroll position for as long as the app runs.
//! A Section slot records a scroll offset plus a content fingerprint — the
//! search query, the A–Z / Z–A sort direction, and the library generation at
//! the moment the offset was saved. Restoring a slot whose fingerprint no
//! longer matches resets to the top instead of restoring a stale offset.
//!
//! Drill columns (an artist's Albums, a Genre's Artists, a Genre artist's
//! Albums) and the Tracks column reset to the top on every selection change:
//! the Scroll Memory holds no drill offsets by design. A selection epoch is
//! bumped on every browser/drill row selection, so a drill column can detect
//! ANY selection change — including re-selecting the same entity — and force
//! the top. Between selection changes the column's scroll is egui's natural
//! per-widget state under the column's own stable salt.
//!
//! Everything is in-memory: restarting the app starts every list at the top,
//! nothing writes to the Application Store, and nothing crosses the session
//! boundary (user story 21). The library generation is observed from the
//! drained backend event inbox ([`BackendEvent::LibraryChanged`] / the
//! [`BackendEvent::InitialSnapshot`]) — a committed rescan bumps it, which
//! turns every slot's fingerprint stale and resets the affected lists.

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
pub const SALT_ALL_TRACKS: &str = "scroll:all-tracks";
pub const SALT_ARTISTS: &str = "scroll:artists";
pub const SALT_ALBUMS: &str = "scroll:albums";
pub const SALT_GENRES: &str = "scroll:genres";
/// Drill-column salts: stable identities distinct from the Section slots.
pub const SALT_DRILL_ARTIST_ALBUMS: &str = "scroll:drill-artist-albums";
pub const SALT_DRILL_GENRE_ARTISTS: &str = "scroll:drill-genre-artists";
pub const SALT_DRILL_GENRE_ARTIST_ALBUMS: &str = "scroll:drill-genre-artist-albums";
pub const SALT_TRACKS_COLUMN: &str = "scroll:tracks-column";

/// The scroll salt that keys a Section's root list.
#[must_use]
pub fn section_salt(section: LibrarySection) -> &'static str {
    match section {
        LibrarySection::AllTracks => SALT_ALL_TRACKS,
        LibrarySection::Artists => SALT_ARTISTS,
        LibrarySection::Albums => SALT_ALBUMS,
        LibrarySection::Genres => SALT_GENRES,
    }
}

/// The content identity a Section slot's offset was captured against: the
/// search query, the sort direction, and the library generation (issue 06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentFingerprint {
    pub query: String,
    pub sort_desc: bool,
    pub library_generation: u64,
}

impl ContentFingerprint {
    #[must_use]
    pub fn new(query: &str, sort_desc: bool, library_generation: u64) -> Self {
        Self {
            query: query.to_owned(),
            sort_desc,
            library_generation,
        }
    }
}

/// One Section's remembered scroll offset plus the fingerprint it was
/// captured under; a fingerprint mismatch on the next frame restore resets
/// to the top.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionSlot {
    pub offset: f32,
    pub fingerprint: ContentFingerprint,
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
    #[must_use]
    pub fn salt(self) -> &'static str {
        match self {
            Self::ArtistAlbums => SALT_DRILL_ARTIST_ALBUMS,
            Self::GenreArtists => SALT_DRILL_GENRE_ARTISTS,
            Self::GenreArtistAlbums => SALT_DRILL_GENRE_ARTIST_ALBUMS,
            Self::TracksColumn => SALT_TRACKS_COLUMN,
        }
    }
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
    /// Bumped on every browser/drill row selection, so a drill column can
    /// detect ANY selection change — including re-selecting the same entity.
    selection_epoch: u64,
    /// The selection epoch each drill slot last reset at.
    drill_epochs: [u64; 4],
    /// The highest Library generation observed in the drained backend events.
    library_generation: u64,
}

impl ScrollMemory {
    /// Fold drained backend events into the observed library generation: a
    /// committed rescan bumps it, which turns every slot's fingerprint stale.
    pub fn note_backend_events(&mut self, events: &[BackendEvent]) {
        for event in events {
            match event {
                BackendEvent::LibraryChanged { generation } => {
                    self.library_generation = (*generation).max(self.library_generation);
                }
                BackendEvent::InitialSnapshot {
                    library_generation, ..
                } => {
                    self.library_generation = (*library_generation).max(self.library_generation);
                }
                _ => {}
            }
        }
    }

    /// The currently observed library generation, for fingerprint checks.
    #[must_use]
    pub fn library_generation(&self) -> u64 {
        self.library_generation
    }

    /// Bump the selection epoch: called on every browser/drill row selection,
    /// so the drill and Tracks columns reset on the next frame.
    pub fn note_selection_change(&mut self) {
        self.selection_epoch = self.selection_epoch.saturating_add(1);
    }

    /// The offset a Section's root list starts this frame: the saved offset
    /// when the slot's fingerprint matches the current content, zero on a
    /// stale slot (or when none is saved yet) — a reset, never a dead offset.
    #[must_use]
    pub fn section_start(&self, section: LibrarySection, fingerprint: &ContentFingerprint) -> f32 {
        match &self.slots[slot_index(section)] {
            Some(slot) if slot.fingerprint == *fingerprint => slot.offset,
            _ => 0.0,
        }
    }

    /// Record a Section's root list frame: the actual offset and the
    /// fingerprint it was applied under become the slot.
    pub fn record_section(
        &mut self,
        section: LibrarySection,
        offset: f32,
        fingerprint: ContentFingerprint,
    ) {
        self.slots[slot_index(section)] = Some(SectionSlot {
            offset,
            fingerprint,
        });
    }

    /// A drill/Tracks column's start offset for this frame: `Some(0.0)` on
    /// the frame a selection change reaches the column (it resets to the
    /// top — there is no per-selection memory), `None` once the column has
    /// seen the current epoch (egui's natural state continues).
    #[must_use]
    pub fn drill_start(&mut self, slot: DrillSlot) -> Option<f32> {
        if self.drill_epochs[slot as usize] == self.selection_epoch {
            None
        } else {
            self.drill_epochs[slot as usize] = self.selection_epoch;
            Some(0.0)
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
