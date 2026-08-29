//! Re-exports of the Application Store contract from riff-persistence.

pub use riff_persistence::errors::StoreError;
pub use riff_persistence::store::{
    LOST_GEMS_THRESHOLD, LibraryMutationStore, LibraryQueryStore, PlaylistEntry, PlaylistStore,
    ScalarSettings, Settings, StoreChanged, StoreGeneration, WatchState,
};
