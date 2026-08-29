//! Scan-side Track construction: drives the [`MetadataReader`] port over
//! discovered file paths to produce domain [`Track`]s ready for an
//! [`LibraryMutationStore`] batch commit.
//!
//! This is app-layer logic on purpose: it orchestrates a port over pure
//! domain values and owns scan policy (per-file failures never abort a
//! scan; first-add stamping). The filesystem walk itself lives in
//! infrastructure ([`crate::infra::AudioFileScanner`]); the durable commit
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
                tracks.push(Track {
                    id,
                    file_path: path,
                    metadata,
                    duration: Some(duration),
                    sample_rate: Some(audio_format.sample_rate),
                    channels: Some(audio_format.channels),
                    play_count: 0,
                    last_played: None,
                    // Stamp first-add time once, at scan time. This (not
                    // the file mtime) drives "Recently Added".
                    date_added: Some(SystemTime::now()),
                    // The store derives search_text from metadata at write
                    // time; freshly scanned tracks carry none yet.
                    search_text: String::new(),
                });
            }
            Err(e) => {
                tracing::error!("Failed to read metadata for {:?}: {}", path, e);
            }
        }
    }
    tracks
}
