//! The Application Store read side, narrowed to what the Audio Engine asks for.

use riff_persistence::errors::StoreError;
use riff_persistence::store::LibraryQueryStore;
use riff_persistence::track::{Track, TrackId};
use riff_playback::infra::ports::PlaybackLibrary;

use super::sqlite::SqliteStore;

/// The Application Store as the Audio Engine reads it: the two queries a
/// playback load and a **Queue Fill** make, and nothing else.
///
/// This is a **named adapter, not a blanket impl**, on purpose. A
/// `impl<T: LibraryQueryStore> PlaybackLibrary for T` would admit the store by
/// implication — convenient at the Composition Root, but it makes the narrowing
/// unobservable from outside `riff-playback` and leaves the port unfakeable, so
/// every test of anything above the engine has to implement all 35
/// `LibraryQueryStore` methods to exercise these two (ADR 0012). With a real
/// adapter the seam has one implementation on each side of it: this one in
/// production, a two-method fake in tests.
///
/// It lives in this crate because it implements a port `riff-playback` defines —
/// ADR 0009's membership rule — and beside the store because the store is all
/// it wraps. No query is re-derived here: both reads are the store's own, and
/// the canonical path-ascending order `all_track_ids` returns is the store's
/// contract (ADR 0003), pinned against real `SQLite` by
/// `riff-infra/tests/store_tests.rs::test_all_track_ids_are_canonically_path_ordered`.
pub struct StorePlaybackLibrary {
    store: SqliteStore,
}

impl StorePlaybackLibrary {
    /// Narrow `store` to the Audio Engine's Library read port.
    #[must_use]
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }
}

impl PlaybackLibrary for StorePlaybackLibrary {
    /// The store's own single-Track resolve — the one read behind a `TrackId`
    /// (the load, a resume's re-open, the gapless pre-decode).
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
        LibraryQueryStore::get_track(&self.store, id)
    }

    /// The store's own flat id list, path-ascending, passed through verbatim:
    /// a **Queue Fill** takes it as the store ordered it.
    fn library_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
        LibraryQueryStore::all_track_ids(&self.store)
    }
}
