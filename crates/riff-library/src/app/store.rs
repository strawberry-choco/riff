//! Re-exports of the Application Store contract from riff-persistence.

pub use riff_persistence::errors::StoreError;
pub use riff_persistence::store::{
    FullScanSummary, LOST_GEMS_THRESHOLD, LibraryCounts, LibraryMutationStore, LibraryQueryStore,
    PlaylistEntry, PlaylistStore, ScalarSettings, Settings, StoreChanged, StoreGeneration,
    WatchState,
};
