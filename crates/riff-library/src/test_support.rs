//! Shared-ownership forwarding implementations for the collection
//! capability's loudness-analysis and file-write ports: any
//! [`LoudnessAnalyzer`], [`MetadataWriter`], or [`ReplayGainWriter`] behind a
//! shared lock cell implements its port, so a suite that hands the Tag Edit
//! worker its write port or the `ReplayGain` Pass its analyzer keeps a
//! handle to the recording mock behind it instead of re-spelling the port as
//! a lock-and-delegate adapter.
//!
//! They are test-only by convention — nothing in production constructs one —
//! and they sit in this crate because the orphan rule pins an implementation
//! of these ports for `std`'s `Arc<Mutex<T>>` to the crate that owns the
//! ports.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::app::errors::LibraryError;
use crate::infra::ports::{
    LoudnessAnalyzer, MetadataWriter, ReplayGainTags, ReplayGainWriter, TagEdit, TrackLoudness,
};

// The distinction from the narrow-port ADR (ADR 0012,
// `docs/adr/0012-narrow-port-gets-a-real-adapter.md`): these are test-support
// forwarding implementations at the ports' own width for shared ownership —
// they narrow nothing, no seam goes hypothetical, and no production path
// constructs one. ADR 0012 forbids the other thing: a production blanket
// impl that coerces a wide port into a narrow one and thereby erases a real
// seam. Each forwards its port's every method by locking and delegating, and
// the forwarding knowledge dies when the port dies.

/// [`MetadataWriter`] for any metadata writer behind a shared lock cell: the
/// seam under test owns one handle of the cell and the test keeps the other,
/// so the mock's recording stays readable after the hand-over. Forwards the
/// port at its own width by locking and delegating — see the section comment
/// above for the ADR 0012 distinction.
impl<T: MetadataWriter + ?Sized> MetadataWriter for Arc<Mutex<T>> {
    fn write_tags(&self, path: &Path, edit: &TagEdit) -> Result<(), LibraryError> {
        self.lock().unwrap().write_tags(path, edit)
    }
}

/// [`ReplayGainWriter`] for any `ReplayGain` writer behind a shared lock
/// cell — the same shared-ownership forwarding implementation as the
/// Metadata write port's, for the same reason and under the same ADR 0012
/// distinction: the port's own width, test support only, nothing narrowed,
/// nothing hypothetical.
impl<T: ReplayGainWriter + ?Sized> ReplayGainWriter for Arc<Mutex<T>> {
    fn write_replaygain(&self, path: &Path, tags: &ReplayGainTags) -> Result<(), LibraryError> {
        self.lock().unwrap().write_replaygain(path, tags)
    }
}

/// [`LoudnessAnalyzer`] for any loudness analyzer behind a shared lock cell —
/// the same shared-ownership forwarding implementation as the two file-write
/// ports', for the same reason and under the same ADR 0012 distinction: the
/// port's own width, test support only, nothing narrowed, nothing
/// hypothetical.
impl<T: LoudnessAnalyzer + ?Sized> LoudnessAnalyzer for Arc<Mutex<T>> {
    fn measure_track(&self, path: &Path) -> Result<TrackLoudness, LibraryError> {
        self.lock().unwrap().measure_track(path)
    }
}
