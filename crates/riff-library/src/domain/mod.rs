//! Library domain types re-exported from riff-persistence.

pub mod errors;

pub use errors::LibraryError;
pub use riff_persistence::playlist::{Playlist, PlaylistId};
pub use riff_persistence::store::{
    LOST_GEMS_THRESHOLD, LibraryMutationStore, LibraryQueryStore, PlaylistEntry, PlaylistStore,
    ScalarSettings, Settings, StoreChanged, StoreGeneration, WatchState,
};
pub use riff_persistence::track::{
    Album, Artist, CoverSource, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};

/// Repeat mode for the queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    None,
    All,
    One,
}
