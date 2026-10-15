//! Scan-side Track construction: drives the [`MetadataReader`] port over
//! discovered file paths to produce domain [`Track`]s ready for an
//! [`LibraryMutationStore`](crate::app::store::LibraryMutationStore) batch commit.
//!
//! This is app-layer logic on purpose: it orchestrates a port over pure
//! domain values and owns scan policy (per-file failures never abort a
//! scan; first-add stamping). The filesystem walk itself lives in
//! infrastructure (`riff_infra::filesystem::AudioFileScanner`); the durable commit
//! lives behind the mutation port.

use crate::app::traits::MetadataReader;
use riff_persistence::track::{Track, TrackId};
use std::path::PathBuf;
use std::time::SystemTime;

/// Read metadata for `paths` into Track values, skipping (and logging)
/// per-file failures so a scan never aborts on one bad file. The caller
/// commits the resulting batch as one durable transaction through the
/// store's mutation port.
pub fn build_tracks(paths: Vec<PathBuf>, reader: &dyn MetadataReader) -> Vec<Track> {
    let mut tracks = Vec::new();
    for path in paths {
        match reader.read_all(&path) {
            Ok((metadata, duration, _cover_source, audio_format)) => {
                let id = TrackId::from_path(&path);
                // Stamp first-add time once, at scan time. This (not the file
                // mtime) drives "Recently Added"; a fresh scan never marks
                // favorites and carries no search blob — `Track::fresh_scan`
                // fixes those to a new Track's values.
                tracks.push(Track::fresh_scan(
                    id,
                    path,
                    metadata,
                    Some(duration),
                    Some(audio_format.sample_rate),
                    Some(audio_format.channels),
                    Some(SystemTime::now()),
                ));
            }
            Err(e) => {
                tracing::error!("Failed to read metadata for {:?}: {}", path, e);
            }
        }
    }
    tracks
}
