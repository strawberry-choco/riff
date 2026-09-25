//! riff-infra — the adapter crate.
//!
//! Every port implementation and every native/external dependency lives here:
//! the `SQLite` Application Store, the symphonia decoder, the cpal output, the
//! lofty metadata reader/writer, the image cover loader, the walkdir scanner,
//! and the notify watcher. It implements the ports defined in `riff-persistence`,
//! `riff-library`, and `riff-playback`; the dependency arrow points at the
//! slices, never the reverse.
//!
//! Internal module seams (store / audio / media / filesystem) are preserved so
//! the crate can be split further later without redesign if compile times ever
//! demand it.

pub mod audio;
pub mod filesystem;
pub mod media;
pub mod store;
