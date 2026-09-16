//! Bounded Session Projections over Application Store query results.
//!
//! A Session Projection is a bounded in-memory view of store query results
//! used while rendering the UI; it is never authoritative (ADR 0002). Each
//! projection caches a total count plus only the currently visible row
//! windows (`LIMIT`/`OFFSET` ranges) until invalidated by the session-local
//! Store generation counter, which bumps after every committed mutation.
//! Stale reads are possible only between a committed write and the next
//! refresh, which generation invalidation makes explicit.
//!
//! One file per projection: each owns its [`GenerationCache`] instances and
//! the staleness contract its views ride on, so understanding one view's
//! caching contract means reading one small file. The module name is the
//! seam — everything re-exported here keeps the historical
//! `app::projection::` import paths.

mod browsing;
mod counts;
mod folders;
mod genres;
mod hit_list;
mod hits;
mod playlists;
mod queue;
mod smart;
mod track_list;

pub use browsing::BrowsingProjection;
pub use counts::CountsProjection;
pub use folders::FolderProjection;
pub use genres::GenreProjection;
pub use hit_list::HitListProjection;
pub use hits::HitProjection;
pub use playlists::{PlaylistEntryRow, PlaylistProjection, PlaylistView};
pub use queue::PlaybackQueue;
pub use smart::SmartPlaylistsProjection;
pub use track_list::{ProjectionKey, TrackListProjection, WINDOW_SIZE};
