//! The `ReplayGain` Pass use case: one run of `ReplayGain` measurement, either
//! **targeted** — a Track-menu or Album-menu command naming its Track or
//! Album — or **library-wide**: on demand from Settings or after a Library
//! Scan completes, gated by the Settings checkboxes.
//!
//! The pass composes the two measurement building blocks (the
//! [`LoudnessAnalyzer`] port and [`album_aggregate`]) with the two write
//! ports (the [`ReplayGainWriter`] file write and the store's scan-batch
//! commit) under one rule set, and this module — exercised entirely through
//! fakes — is where that rule set is asserted:
//!
//! - A targeted pass measures only its target and writes only that target's
//!   values: a Track command never touches the Album aggregate.
//! - A library-wide pass measures unmeasured Tracks only, unless Force; a
//!   Force pass re-measures measured Tracks too. Menu-style commands ignore
//!   that gating entirely — they always measure exactly what their label
//!   says.
//! - A library-wide pass that measured any member of an Album recomputes
//!   that Album's aggregate from the members' values (this pass's
//!   measurements plus already-measured members' stored facts — no
//!   re-decode) and rewrites it on all its Tracks.
//! - Every write is file tags first, then the Store facts, and a
//!   file-write failure is reported and leaves the Store untouched (ADR
//!   0014).
//! - Commits are batched ([`PASS_BATCH_SIZE`], the scan's prior art), so an
//!   interrupted pass keeps everything it already committed; resumption is
//!   just running it again.
//! - Measurement always wins over manual edits: the next measurement simply
//!   overwrites, which is why no flag tracks "manually edited".

use crate::app::replaygain::album_aggregate;
use crate::app::store::{LibraryMutationStore, LibraryQueryStore};
use crate::app::traits::{LoudnessAnalyzer, ReplayGainTags, ReplayGainWriter, TrackLoudness};
use riff_persistence::track::{Track, TrackId, TrackMetadata};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// How many measured Tracks share one durable store commit — the scan's
/// batch size, so an interrupted pass keeps every batch it already
/// committed.
pub const PASS_BATCH_SIZE: usize = 10;

/// One run of the `ReplayGain` Pass: what to measure and which values to
/// write.
#[derive(Debug, Clone, PartialEq)]
pub enum PassCommand {
    /// Measure one Track's track values and write exactly those. Never
    /// touches the Album aggregate.
    Track(TrackId),
    /// Measure the Album's aggregate over its current Track set and write
    /// the album pair to every member.
    AlbumAggregate {
        album_artist: String,
        album_title: String,
    },
    /// Measure track values for every Track of the Album individually and
    /// write exactly those. Never touches the Album aggregate.
    AlbumTracks {
        album_artist: String,
        album_title: String,
    },
    /// Measure unmeasured Tracks across the Library (every Track when
    /// `force`), writing whichever of the two value kinds the checkboxes
    /// enable. Only the Settings flows construct this variant; the menu
    /// commands are always targeted.
    LibraryWide {
        track_values: bool,
        album_values: bool,
        force: bool,
    },
}

/// How a finished or interrupted pass reports. `measured` counts the Tracks
/// whose values landed on both file and Store; `skipped` counts the
/// library-wide pass's already-measured Tracks; `failed` counts the Targets
/// whose measurement, file write, or store commit did not land, with the
/// first reason carried for the outcome report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PassReport {
    pub measured: usize,
    pub skipped: usize,
    pub failed: usize,
    pub first_failure: Option<String>,
    pub cancelled: bool,
}

impl PassReport {
    fn fail(&mut self, reason: String) {
        self.failed += 1;
        if self.first_failure.is_none() {
            self.first_failure = Some(reason);
        }
    }
}

/// The `ReplayGain` Pass use case, over the ports it consumes. Construction is
/// manual injection; the worker that drives `run` lives above this seam.
pub struct ReplayGainPass {
    analyzer: Box<dyn LoudnessAnalyzer + Send>,
    writer: Box<dyn ReplayGainWriter + Send>,
    queries: Box<dyn LibraryQueryStore + Send>,
    mutations: Box<dyn LibraryMutationStore + Send>,
}

/// The measured facts of one Track this pass, kept so an Album's aggregate
/// can combine this pass's measurements with already-measured members'
/// stored facts.
struct Measurement {
    track: Track,
    loudness: TrackLoudness,
}

impl ReplayGainPass {
    pub fn new(
        analyzer: Box<dyn LoudnessAnalyzer + Send>,
        writer: Box<dyn ReplayGainWriter + Send>,
        queries: Box<dyn LibraryQueryStore + Send>,
        mutations: Box<dyn LibraryMutationStore + Send>,
    ) -> Self {
        Self {
            analyzer,
            writer,
            queries,
            mutations,
        }
    }

    /// Run one pass to completion (or cancellation), reporting progress as
    /// `(done, total)` measurement completions. Everything the pass already
    /// committed stays committed when it comes back cancelled or partial.
    pub fn run(
        &mut self,
        command: &PassCommand,
        cancel: &AtomicBool,
        mut progress: impl FnMut(usize, usize),
    ) -> PassReport {
        let mut report = PassReport::default();
        match command {
            PassCommand::Track(id) => self.run_track_target(id, cancel, &mut progress, &mut report),
            PassCommand::AlbumAggregate {
                album_artist,
                album_title,
            } => {
                self.run_album_target(
                    album_artist,
                    album_title,
                    Aggregate,
                    cancel,
                    &mut progress,
                    &mut report,
                );
            }
            PassCommand::AlbumTracks {
                album_artist,
                album_title,
            } => {
                self.run_album_target(
                    album_artist,
                    album_title,
                    TrackValues,
                    cancel,
                    &mut progress,
                    &mut report,
                );
            }
            PassCommand::LibraryWide {
                track_values,
                album_values,
                force,
            } => {
                self.run_library_wide(
                    *track_values,
                    *album_values,
                    *force,
                    cancel,
                    &mut progress,
                    &mut report,
                );
            }
        }
        report
    }

    /// A Track command: measure exactly that Track, write exactly its track
    /// pair. The Album aggregate is untouched.
    fn run_track_target(
        &mut self,
        id: &TrackId,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
        report: &mut PassReport,
    ) {
        let track = match self.queries.get_track(id) {
            Ok(Some(track)) => track,
            Ok(None) => {
                report.fail("Track is no longer in the library".to_string());
                return;
            }
            Err(e) => {
                report.fail(format!("failed to resolve the Track: {e}"));
                return;
            }
        };
        progress(0, 1);
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            return;
        }
        let mut batch = Vec::new();
        if self.measure_write_commit(track, TrackValues, &mut batch, report) {
            self.flush(&mut batch, report);
            progress(1, 1);
        }
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
        }
    }

    /// An Album command. `Aggregate` measures every member once, computes
    /// the aggregate over the measured members, and writes the album pair to
    /// each of them; `TrackValues` writes each member's own pair as it is
    /// measured. Either shape never writes the other kind of value.
    fn run_album_target(
        &mut self,
        album_artist: &str,
        album_title: &str,
        shape: TargetShape,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
        report: &mut PassReport,
    ) {
        let members = match self.queries.album_tracks(album_artist, album_title) {
            Ok(members) => members,
            Err(e) => {
                report.fail(format!("failed to resolve the Album: {e}"));
                return;
            }
        };
        if members.is_empty() {
            report.fail("Album is no longer in the library".to_string());
            return;
        }
        progress(0, members.len());

        let mut batch = Vec::new();
        let mut measured = Vec::new();
        let mut done = 0;
        for track in &members {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            match self.analyzer.measure_track(&track.file_path) {
                Ok(loudness) => {
                    measured.push(Measurement {
                        track: track.clone(),
                        loudness,
                    });
                    done += 1;
                    progress(done, members.len());
                }
                Err(e) => {
                    report.fail(format!("{}: {e}", track.file_path.display()));
                }
            }
        }

        match shape {
            TrackValues => {
                for measurement in measured {
                    if cancel.load(Ordering::Relaxed) {
                        report.cancelled = true;
                        break;
                    }
                    let Measurement { track, loudness } = measurement;
                    let tags =
                        ReplayGainTags::track_pair(loudness.track_gain_db, loudness.track_peak);
                    self.write_commit(track, &tags, &mut batch, report);
                }
            }
            Aggregate => {
                let contributions: Vec<_> = measured
                    .iter()
                    .map(|m| (m.loudness, duration_seconds(&m.track)))
                    .collect();
                let Some(aggregate) = album_aggregate(&contributions) else {
                    // Every measurement failed (already reported); the file
                    // writes below must not run on nothing.
                    return;
                };
                let tags =
                    ReplayGainTags::album_pair(aggregate.album_gain_db, aggregate.album_peak);
                for measurement in measured {
                    if cancel.load(Ordering::Relaxed) {
                        report.cancelled = true;
                        break;
                    }
                    self.write_commit(measurement.track, &tags, &mut batch, report);
                }
            }
        }
        self.flush(&mut batch, report);
    }

    /// The library-wide shape: unmeasured Tracks only, unless `force`; only
    /// the requested value kinds are written; any Album that gained a
    /// measurement has its aggregate recomputed and rewritten on all its
    /// Tracks.
    fn run_library_wide(
        &mut self,
        track_values: bool,
        album_values: bool,
        force: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
        report: &mut PassReport,
    ) {
        if !track_values && !album_values {
            // A pass that writes nothing is not a pass; the Settings surface
            // gates the button, and the use case agrees.
            return;
        }

        let ids = match self.queries.all_track_ids() {
            Ok(ids) => ids,
            Err(e) => {
                report.fail(format!("failed to list the Library: {e}"));
                return;
            }
        };

        let mut targets = Vec::new();
        for id in ids {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            match self.queries.get_track(&id) {
                Ok(Some(track)) => {
                    if !force && track.metadata.replaygain_track_gain.is_some() {
                        report.skipped += 1;
                    } else {
                        targets.push(track);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    report.fail(format!("failed to resolve {}: {e}", id.0));
                }
            }
        }

        progress(0, targets.len());
        let mut batch = Vec::new();
        let mut measurements: Vec<Measurement> = Vec::new();
        let mut done = 0;
        for track in &targets {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            match self.analyzer.measure_track(&track.file_path) {
                Ok(loudness) => {
                    if track_values {
                        let tags =
                            ReplayGainTags::track_pair(loudness.track_gain_db, loudness.track_peak);
                        self.write_commit(track.clone(), &tags, &mut batch, report);
                    }
                    measurements.push(Measurement {
                        track: track.clone(),
                        loudness,
                    });
                    done += 1;
                    progress(done, targets.len());
                }
                Err(e) => {
                    report.fail(format!("{}: {e}", track.file_path.display()));
                }
            }
        }
        self.flush(&mut batch, report);

        if album_values && !measurements.is_empty() {
            self.recompute_albums(measurements, cancel, report);
        }
    }

    /// Every Album one of whose members this pass measured gets its
    /// aggregate recomputed — from this pass's measurements plus
    /// already-measured members' stored facts, no re-decode — and rewritten
    /// on all its Tracks, file first, store second.
    fn recompute_albums(
        &mut self,
        measurements: Vec<Measurement>,
        cancel: &AtomicBool,
        report: &mut PassReport,
    ) {
        let mut measured_by_id: HashMap<String, TrackLoudness> = HashMap::new();
        let mut affected: Vec<(String, String)> = Vec::new();
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for Measurement { track, loudness } in &measurements {
            measured_by_id.insert(track.id.0.clone(), *loudness);
            let key = (
                track.metadata.display_album_artist().into_owned(),
                track.metadata.display_album().into_owned(),
            );
            if seen.insert(key.clone()) {
                affected.push(key);
            }
        }

        let mut batch = Vec::new();
        for (album_artist, album_title) in affected {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            let members = match self.queries.album_tracks(&album_artist, &album_title) {
                Ok(members) => members,
                Err(e) => {
                    report.fail(format!("failed to resolve {album_title}: {e}"));
                    continue;
                }
            };
            let contributions: Vec<_> = members
                .iter()
                .filter_map(|member| {
                    let loudness = measured_by_id.get(&member.id.0).copied().or_else(|| {
                        member
                            .metadata
                            .replaygain_track_gain
                            .map(|gain| TrackLoudness {
                                track_gain_db: gain,
                                track_peak: member.metadata.replaygain_track_peak.unwrap_or(0.0),
                            })
                    })?;
                    // A stored value can be non-finite only if it arrived from a
                    // file this analyzer never measured — a foreign `inf dB` tag
                    // read back — since a measurement of riff's own is finite by
                    // the port's contract. Weighing one would add its duration to
                    // the Album's energy weight with no sound behind it, so a
                    // member carrying one is absent from the aggregate exactly as
                    // a member whose measurement failed is.
                    (loudness.track_gain_db.is_finite() && loudness.track_peak.is_finite())
                        .then_some((loudness, duration_seconds(member)))
                })
                .collect();
            let Some(aggregate) = album_aggregate(&contributions) else {
                continue;
            };
            let tags = ReplayGainTags::album_pair(aggregate.album_gain_db, aggregate.album_peak);
            for member in &members {
                if cancel.load(Ordering::Relaxed) {
                    report.cancelled = true;
                    break;
                }
                self.write_commit(member.clone(), &tags, &mut batch, report);
            }
        }
        self.flush(&mut batch, report);
    }

    /// Measure, write the file tags, and queue the Store commit — in that
    /// order, so a file-write failure is reported and leaves the Store
    /// untouched (no phantom values). Returns whether the measurement
    /// landed.
    fn measure_write_commit(
        &mut self,
        track: Track,
        shape: TargetShape,
        batch: &mut Vec<Track>,
        report: &mut PassReport,
    ) -> bool {
        match self.analyzer.measure_track(&track.file_path) {
            Ok(loudness) => {
                let tags = match shape {
                    TrackValues => {
                        ReplayGainTags::track_pair(loudness.track_gain_db, loudness.track_peak)
                    }
                    Aggregate => unreachable!("a Track target never writes an aggregate"),
                };
                self.write_commit(track, &tags, batch, report)
            }
            Err(e) => {
                report.fail(format!("{}: {e}", track.file_path.display()));
                false
            }
        }
    }

    /// Write the file tags and queue the Store commit of the same facts —
    /// file first, store second; a file-write failure leaves the Store
    /// untouched.
    fn write_commit(
        &mut self,
        track: Track,
        tags: &ReplayGainTags,
        batch: &mut Vec<Track>,
        report: &mut PassReport,
    ) -> bool {
        if let Err(e) = self.writer.write_replaygain(&track.file_path, tags) {
            report.fail(format!("{}: {e}", track.file_path.display()));
            return false;
        }
        let mut updated = track;
        apply_replaygain_to_metadata(&mut updated.metadata, tags);
        batch.push(updated);
        if batch.len() >= PASS_BATCH_SIZE {
            self.flush(batch, report);
        }
        true
    }

    /// Commit the queued facts as one durable store change — the scan's
    /// batch commit, which preserves play history — and count the outcome.
    fn flush(&mut self, batch: &mut Vec<Track>, report: &mut PassReport) {
        if batch.is_empty() {
            return;
        }
        let count = batch.len();
        match self.mutations.apply_scan_batch(batch) {
            Ok(_) => report.measured += count,
            Err(e) => {
                // Every fact in the batch failed to land — one reason, `count`
                // failures.
                if report.first_failure.is_none() {
                    report.first_failure =
                        Some(format!("failed to commit the measured values: {e}"));
                }
                report.failed += count;
            }
        }
        batch.clear();
    }
}

/// Which values an Album command writes: the members' own pairs, or the
/// Album's shared pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetShape {
    TrackValues,
    Aggregate,
}
use TargetShape::{Aggregate, TrackValues};

/// A member's energy weight: its duration in seconds. A Track the store
/// knows no duration for weighs nothing — it cannot contribute energy it
/// never declared.
fn duration_seconds(track: &Track) -> f32 {
    track.duration.map_or(0.0, |d| d.as_secs_f32())
}

/// Mirror the written tags into the Track's stored metadata. Only `Some`
/// fields move, and measurement always wins: whatever a manual edit held is
/// simply overwritten.
fn apply_replaygain_to_metadata(metadata: &mut TrackMetadata, tags: &ReplayGainTags) {
    if let Some(gain) = tags.track_gain {
        metadata.replaygain_track_gain = Some(gain);
    }
    if let Some(peak) = tags.track_peak {
        metadata.replaygain_track_peak = Some(peak);
    }
    if let Some(gain) = tags.album_gain {
        metadata.replaygain_album_gain = Some(gain);
    }
    if let Some(peak) = tags.album_peak {
        metadata.replaygain_album_peak = Some(peak);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::errors::LibraryError;
    use crate::app::store::{FullScanSummary, StoreError};
    use riff_persistence::test_support::StubLibraryQueryStore;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // --- Fakes ------------------------------------------------------------------

    /// The fake library behind the query port: tracks by id, album
    /// memberships, and the flat id list, shared with the test for seeding
    /// and assertions.
    #[derive(Default)]
    struct FakeLibrary {
        tracks: Vec<Track>,
        albums: HashMap<(String, String), Vec<String>>,
    }

    impl FakeLibrary {
        fn stub(self) -> StubLibraryQueryStore {
            let tracks: HashMap<String, Track> = self
                .tracks
                .into_iter()
                .map(|t| (t.id.0.clone(), t))
                .collect();
            let albums = Arc::new(self.albums);
            let tracks = Arc::new(tracks);
            let by_id = Arc::clone(&tracks);
            let by_album = Arc::clone(&tracks);
            StubLibraryQueryStore::new(move |id| by_id.get(&id.0).cloned())
                .with_album_tracks(move |artist, title| {
                    albums
                        .get(&(artist.to_string(), title.to_string()))
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|id| by_album.get(id).cloned())
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .with_all_track_ids(move || {
                    // Path-ascending, the canonical flat ordering.
                    let mut ids: Vec<String> = tracks.keys().cloned().collect();
                    ids.sort();
                    ids.into_iter().map(TrackId).collect()
                })
        }
    }

    /// The fake analyzer: canned measurements per path, call recording, and
    /// named failures. `cancel_after` arms a cancellation flag once the
    /// given number of measurements has run, so a test can interrupt a pass
    /// mid-flight.
    struct FakeAnalyzer {
        canned: HashMap<String, TrackLoudness>,
        fail: HashSet<String>,
        calls: Mutex<Vec<String>>,
        cancel_after: Option<(Arc<AtomicBool>, usize)>,
    }

    impl FakeAnalyzer {
        fn new(canned: HashMap<String, TrackLoudness>) -> Self {
            Self {
                canned,
                fail: HashSet::new(),
                calls: Mutex::new(Vec::new()),
                cancel_after: None,
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl LoudnessAnalyzer for FakeAnalyzer {
        fn measure_track(&self, path: &std::path::Path) -> Result<TrackLoudness, LibraryError> {
            let key = path.to_string_lossy().to_string();
            self.calls.lock().unwrap().push(key.clone());
            if let Some((cancel, after)) = &self.cancel_after
                && self.calls.lock().unwrap().len() >= *after
            {
                cancel.store(true, Ordering::Relaxed);
            }
            if self.fail.contains(&key) {
                return Err(LibraryError::Loudness("measurement failed".to_string()));
            }
            Ok(self.canned.get(&key).copied().unwrap_or(TrackLoudness {
                track_gain_db: -6.0,
                track_peak: 0.5,
            }))
        }
    }

    /// The fake tag writer: every accepted write is recorded with the exact
    /// tags it carried; named paths fail.
    struct FakeWriter {
        writes: Mutex<Vec<(String, ReplayGainTags)>>,
        fail: HashSet<String>,
    }

    impl FakeWriter {
        fn new() -> Self {
            Self {
                writes: Mutex::new(Vec::new()),
                fail: HashSet::new(),
            }
        }

        fn writes(&self) -> Vec<(String, ReplayGainTags)> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl ReplayGainWriter for FakeWriter {
        fn write_replaygain(
            &self,
            path: &std::path::Path,
            tags: &ReplayGainTags,
        ) -> Result<(), LibraryError> {
            let key = path.to_string_lossy().to_string();
            if self.fail.contains(&key) {
                return Err(LibraryError::Io("permission denied".to_string()));
            }
            self.writes.lock().unwrap().push((key, *tags));
            Ok(())
        }
    }

    /// The fake store mutations: every scan-batch commit is recorded; an
    /// armed failure fails every commit.
    struct FakeMutations {
        batches: Mutex<Vec<Vec<Track>>>,
        fail: AtomicBool,
    }

    impl FakeMutations {
        fn new() -> Self {
            Self {
                batches: Mutex::new(Vec::new()),
                fail: AtomicBool::new(false),
            }
        }

        fn committed(&self) -> Vec<Vec<Track>> {
            self.batches.lock().unwrap().clone()
        }

        fn committed_flat(&self) -> Vec<Track> {
            self.committed().into_iter().flatten().collect()
        }
    }

    // --- Fixtures ---------------------------------------------------------------

    fn track(path: &str, album: Option<(&str, &str)>) -> Track {
        let mut metadata = TrackMetadata::default();
        if let Some((artist, title)) = album {
            metadata.album = Some(title.to_string());
            metadata.album_artist = Some(artist.to_string());
        }
        Track {
            id: TrackId(path.to_string()),
            file_path: std::path::PathBuf::from(path),
            metadata,
            duration: Some(Duration::from_secs(2)),
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
            favorite: false,
            search_text: String::new(),
        }
    }

    fn loudness(gain_db: f32, peak: f32) -> TrackLoudness {
        TrackLoudness {
            track_gain_db: gain_db,
            track_peak: peak,
        }
    }

    /// The fixture library: three tracks in Album A, one in Album B. The
    /// fake analyzer measures everything at −6 dB / 0.5 unless canned.
    fn fixture() -> (FakeLibrary, HashMap<String, TrackLoudness>) {
        let mut library = FakeLibrary::default();
        library
            .tracks
            .push(track("music/a1.flac", Some(("Artist A", "Album A"))));
        library
            .tracks
            .push(track("music/a2.flac", Some(("Artist A", "Album A"))));
        library
            .tracks
            .push(track("music/a3.flac", Some(("Artist A", "Album A"))));
        library
            .tracks
            .push(track("music/b1.flac", Some(("Artist B", "Album B"))));
        library.albums.insert(
            ("Artist A".to_string(), "Album A".to_string()),
            vec![
                "music/a1.flac".to_string(),
                "music/a2.flac".to_string(),
                "music/a3.flac".to_string(),
            ],
        );
        library.albums.insert(
            ("Artist B".to_string(), "Album B".to_string()),
            vec!["music/b1.flac".to_string()],
        );
        (library, HashMap::new())
    }

    /// The wired pass with handles back to the fakes for assertions.
    struct Wired {
        pass: ReplayGainPass,
        analyzer: Arc<FakeAnalyzer>,
        writer: Arc<FakeWriter>,
        mutations: Arc<FakeMutations>,
    }

    fn wire(library: FakeLibrary, analyzer: FakeAnalyzer) -> Wired {
        let analyzer = Arc::new(analyzer);
        let writer = Arc::new(FakeWriter::new());
        let mutations = Arc::new(FakeMutations::new());
        let pass = ReplayGainPass::new(
            Box::new(SharedAnalyzer(analyzer.clone())),
            Box::new(SharedWriter(writer.clone())),
            Box::new(library.stub()),
            Box::new(SharedMutations(mutations.clone())),
        );
        Wired {
            pass,
            analyzer,
            writer,
            mutations,
        }
    }

    // The ports are object-safe but the fakes are shared with the test for
    // assertions; these wrappers forward through the `Arc`s. (A fake behind
    // an `Arc` cannot be its own `Box<dyn Port>` because the ports take
    // `&self` while the store mutations take `&mut self` — only the
    // mutations need the mutex they already carry.)
    struct SharedAnalyzer(Arc<FakeAnalyzer>);
    impl LoudnessAnalyzer for SharedAnalyzer {
        fn measure_track(&self, path: &std::path::Path) -> Result<TrackLoudness, LibraryError> {
            self.0.measure_track(path)
        }
    }
    struct SharedWriter(Arc<FakeWriter>);
    impl ReplayGainWriter for SharedWriter {
        fn write_replaygain(
            &self,
            path: &std::path::Path,
            tags: &ReplayGainTags,
        ) -> Result<(), LibraryError> {
            self.0.write_replaygain(path, tags)
        }
    }
    struct SharedMutations(Arc<FakeMutations>);
    impl LibraryMutationStore for SharedMutations {
        fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, StoreError> {
            self.0.batches.lock().unwrap().push(tracks.to_vec());
            if self.0.fail.load(Ordering::Relaxed) {
                return Err(StoreError::InvalidOperation("store is dead".to_string()));
            }
            Ok(tracks.len())
        }
        fn record_track_played(
            &mut self,
            _id: &TrackId,
            _played_at: std::time::SystemTime,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn set_track_favorite(
            &mut self,
            _id: &TrackId,
            _favorite: bool,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
        fn apply_tag_refresh(&mut self, _track: &Track) -> Result<(), StoreError> {
            Ok(())
        }
        fn remove_library_path(&mut self, _root: &std::path::Path) -> Result<usize, StoreError> {
            Ok(0)
        }
        fn clear_library(&mut self) -> Result<usize, StoreError> {
            Ok(0)
        }
        fn record_full_scan_completed(
            &mut self,
            _summary: FullScanSummary,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        fn stamp_metadata_version(&mut self, _version: u32) -> Result<(), StoreError> {
            Ok(())
        }
    }

    fn album_tags_of(writes: &[(String, ReplayGainTags)], path: &str) -> Option<ReplayGainTags> {
        // Latest write wins: a member measured this pass is written twice
        // (its track pair, then the album-pair rewrite), and the readout
        // that matters is the last one.
        writes
            .iter()
            .rev()
            .find(|(p, _)| p == path)
            .map(|(_, tags)| *tags)
    }

    fn no_progress() -> impl FnMut(usize, usize) {
        |_, _| {}
    }

    // --- Targeted Track command ---------------------------------------------------

    /// A Track command measures and writes exactly that Track's track
    /// values; the Album aggregate is untouched — no album pair is written
    /// anywhere, and the store commit carries only the track pair.
    #[test]
    fn a_track_command_measures_and_writes_exactly_that_track() {
        let (library, canned) = fixture();
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::Track(TrackId("music/a1.flac".to_string())),
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(wired.analyzer.calls(), vec!["music/a1.flac".to_string()]);

        let writes = wired.writer.writes();
        assert_eq!(writes.len(), 1, "exactly one file write");
        let (path, tags) = &writes[0];
        assert_eq!(path, "music/a1.flac");
        assert_eq!(tags.track_gain, Some(-6.0));
        assert_eq!(tags.track_peak, Some(0.5));
        assert_eq!(
            tags.album_gain, None,
            "a Track command never writes the Album aggregate"
        );
        assert_eq!(tags.album_peak, None);

        let committed = wired.mutations.committed_flat();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].metadata.replaygain_track_gain, Some(-6.0));
        assert_eq!(
            committed[0].metadata.replaygain_album_gain, None,
            "the aggregate is untouched"
        );
    }

    /// A Track the library no longer carries is reported, not silently
    /// dropped.
    #[test]
    fn a_track_command_on_an_unknown_track_reports_the_failure() {
        let (library, canned) = fixture();
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::Track(TrackId("music/gone.flac".to_string())),
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 0);
        assert_eq!(report.failed, 1);
        assert_eq!(
            report.first_failure.as_deref(),
            Some("Track is no longer in the library")
        );
        assert_eq!(wired.writer.writes(), [] as [(String, ReplayGainTags); 0]);
    }

    // --- Targeted Album commands ----------------------------------------------------

    /// An Album aggregate command measures every member once and writes the
    /// album pair — the energy-weighted aggregate, the max peak — to every
    /// member. No track pairs are written.
    #[test]
    fn an_album_aggregate_command_writes_the_album_pair_to_every_member() {
        let (mut library, mut canned) = fixture();
        // The worked example: a1 and a3 at −9.03 LUFS for 2 s each, a2 at
        // −15.03 LUFS for 1 s (duration lives on the store tracks) — the
        // aggregate lands at −8.26 dB against the −18 reference, peak 0.5.
        canned.insert("music/a1.flac".to_string(), loudness(-8.9689, 0.5));
        canned.insert("music/a2.flac".to_string(), loudness(-2.9691, 0.25));
        canned.insert("music/a3.flac".to_string(), loudness(-8.9689, 0.5));
        library.tracks[1].duration = Some(Duration::from_secs(1));
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::AlbumAggregate {
                album_artist: "Artist A".to_string(),
                album_title: "Album A".to_string(),
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 3, "every member lands the album pair");
        assert_eq!(report.failed, 0);

        let writes = wired.writer.writes();
        assert_eq!(
            writes.len(),
            3,
            "every member's file carries the album pair"
        );
        for (path, tags) in &writes {
            assert_eq!(
                tags.track_gain, None,
                "an aggregate command writes no track values ({path})"
            );
            assert!(
                (tags.album_gain.unwrap() - (-8.26)).abs() < 0.01,
                "on {path}"
            );
            assert!((tags.album_peak.unwrap() - 0.5).abs() < 1e-6, "on {path}");
        }
        assert_eq!(wired.analyzer.calls().len(), 3, "one decode per member");
    }

    /// An Album tracks command fills in every member's own pair and writes
    /// no aggregate anywhere.
    #[test]
    fn an_album_tracks_command_fills_track_values_for_every_member() {
        let (library, canned) = fixture();
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::AlbumTracks {
                album_artist: "Artist A".to_string(),
                album_title: "Album A".to_string(),
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 3);
        for (path, tags) in wired.writer.writes() {
            assert_eq!(tags.track_gain, Some(-6.0), "on {path}");
            assert_eq!(tags.track_peak, Some(0.5), "on {path}");
            assert_eq!(
                tags.album_gain, None,
                "a tracks command writes no aggregate"
            );
        }
    }

    // --- Library-wide pass ------------------------------------------------------------

    /// The library-wide shape skips measured Tracks; Force re-measures them.
    #[test]
    fn library_wide_skips_measured_tracks_and_force_remeasures_them() {
        let (mut library, canned) = fixture();
        library.tracks[0].metadata.replaygain_track_gain = Some(-9.0);
        library.tracks[0].metadata.replaygain_track_peak = Some(0.4);
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: false,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );
        assert_eq!(report.measured, 3, "the three unmeasured tracks");
        assert_eq!(report.skipped, 1, "a1 is already measured");
        let measured_paths: Vec<String> = wired.analyzer.calls();
        assert!(!measured_paths.contains(&"music/a1.flac".to_string()));

        // The same pass under Force re-measures everything.
        let (library, canned) = {
            let mut library = FakeLibrary::default();
            library
                .tracks
                .push(track("music/a1.flac", Some(("Artist A", "Album A"))));
            library
                .tracks
                .push(track("music/a2.flac", Some(("Artist A", "Album A"))));
            library
                .tracks
                .push(track("music/a3.flac", Some(("Artist A", "Album A"))));
            library
                .tracks
                .push(track("music/b1.flac", Some(("Artist B", "Album B"))));
            library.tracks[0].metadata.replaygain_track_gain = Some(-9.0);
            library.tracks[0].metadata.replaygain_track_peak = Some(0.4);
            library.albums.insert(
                ("Artist A".to_string(), "Album A".to_string()),
                vec![
                    "music/a1.flac".to_string(),
                    "music/a2.flac".to_string(),
                    "music/a3.flac".to_string(),
                ],
            );
            library.albums.insert(
                ("Artist B".to_string(), "Album B".to_string()),
                vec!["music/b1.flac".to_string()],
            );
            (library, HashMap::new())
        };
        let mut wired = wire(library, FakeAnalyzer::new(canned));
        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: false,
                force: true,
            },
            &AtomicBool::new(false),
            no_progress(),
        );
        assert_eq!(
            report.measured, 4,
            "Force re-measures the measured track too"
        );
        assert_eq!(report.skipped, 0);
        assert!(
            wired
                .analyzer
                .calls()
                .contains(&"music/a1.flac".to_string())
        );
    }

    /// A library-wide pass that measured any member of an Album recomputes
    /// that Album's aggregate — from this pass's measurements plus
    /// already-measured members' stored facts, no re-decode — and rewrites
    /// it on all its Tracks, the previously-measured member included.
    #[test]
    fn a_library_wide_pass_recomputes_the_album_aggregate_on_all_its_tracks() {
        let (mut library, mut canned) = fixture();
        // a1 was measured long ago: its facts live in the store.
        library.tracks[0].metadata.replaygain_track_gain = Some(-8.9689);
        library.tracks[0].metadata.replaygain_track_peak = Some(0.5);
        // Durations: a1 2 s (fixture default), a2 1 s.
        library.tracks[1].duration = Some(Duration::from_secs(1));
        // The analyzer measures only the unmeasured members.
        canned.insert("music/a2.flac".to_string(), loudness(-2.9691, 0.25));
        canned.insert("music/a3.flac".to_string(), loudness(-8.9689, 0.5));
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: true,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(
            report.measured, 7,
            "two track pairs + Album A's three album pairs + b1's track pair and aggregate"
        );
        assert_eq!(report.skipped, 1);

        let writes = wired.writer.writes();
        // a2 and a3 carry both kinds; a1 — measured in an earlier pass — is
        // rewritten with the album pair only.
        let a1 = album_tags_of(&writes, "music/a1.flac").expect("a1's aggregate is rewritten");
        assert!((a1.album_gain.unwrap() - (-8.26)).abs() < 0.01);
        assert!((a1.album_peak.unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(a1.track_gain, None, "a1's track pair is not re-measured");
        let a2_writes: Vec<&ReplayGainTags> = writes
            .iter()
            .filter(|(p, _)| p == "music/a2.flac")
            .map(|(_, tags)| tags)
            .collect();
        assert_eq!(
            a2_writes.len(),
            2,
            "a2's track pair, then the album-pair rewrite"
        );
        assert_eq!(
            a2_writes[0].track_gain,
            Some(-2.9691),
            "the track pair first"
        );
        assert!(
            (a2_writes[1].album_gain.unwrap() - (-8.26)).abs() < 0.01,
            "then the album pair"
        );
        assert!(album_tags_of(&writes, "music/a3.flac").is_some());

        // The store carries the same aggregate on all three members.
        let committed = wired.mutations.committed_flat();
        for row in &committed {
            eprintln!(
                "DEBUG row {} track_gain {:?} album_gain {:?}",
                row.id.0, row.metadata.replaygain_track_gain, row.metadata.replaygain_album_gain
            );
        }
        let a1_row = committed
            .iter()
            .find(|t| t.id.0 == "music/a1.flac")
            .unwrap();
        assert!((a1_row.metadata.replaygain_album_gain.unwrap() - (-8.26)).abs() < 0.01);
    }

    /// An Album none of whose members this pass measured is left alone: no
    /// recompute, no rewrite. Album A's members all carry stored gains, so
    /// the pass skips them entirely; only Album B's member gets measured.
    #[test]
    fn a_library_wide_pass_leaves_untouched_albums_alone() {
        let mut library = FakeLibrary::default();
        for path in ["music/a1.flac", "music/a2.flac", "music/a3.flac"] {
            let mut t = track(path, Some(("Artist A", "Album A")));
            t.metadata.replaygain_track_gain = Some(-6.0);
            t.metadata.replaygain_track_peak = Some(0.5);
            library.tracks.push(t);
        }
        library
            .tracks
            .push(track("music/b1.flac", Some(("Artist B", "Album B"))));
        library.albums.insert(
            ("Artist A".to_string(), "Album A".to_string()),
            vec![
                "music/a1.flac".to_string(),
                "music/a2.flac".to_string(),
                "music/a3.flac".to_string(),
            ],
        );
        library.albums.insert(
            ("Artist B".to_string(), "Album B".to_string()),
            vec!["music/b1.flac".to_string()],
        );
        let mut wired = wire(library, FakeAnalyzer::new(HashMap::new()));

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: true,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 2, "b1's track pair + Album B's aggregate");
        assert_eq!(report.skipped, 3);
        let written_paths: Vec<String> =
            wired.writer.writes().into_iter().map(|(p, _)| p).collect();
        assert!(
            !written_paths.iter().any(|p| p.starts_with("music/a")),
            "Album A was not measured, so it is not rewritten"
        );
    }

    /// A member whose stored gain could not have come from this analyzer — a
    /// foreign `inf dB` tag read back — contributes neither energy nor duration
    /// weight to its Album's aggregate, so it cannot drag the shared pair toward
    /// silence with no sound behind it. The Album pair is still written on that
    /// member, and it is finite.
    #[test]
    fn a_non_finite_stored_gain_is_absent_from_the_album_aggregate() {
        let (mut library, mut canned) = fixture();
        library.tracks[0].metadata.replaygain_track_gain = Some(f32::INFINITY);
        library.tracks[0].metadata.replaygain_track_peak = Some(0.0);
        // b1 carries a normal measurement, so Album B is not this pass's work.
        library.tracks[3].metadata.replaygain_track_gain = Some(-6.0);
        library.tracks[3].metadata.replaygain_track_peak = Some(0.5);
        // The worked example's two members: a2 −8.97 dB over 2 s, a3 −2.97 dB
        // over 1 s. a1's 2 s of infinite gain weighs nothing.
        library.tracks[2].duration = Some(Duration::from_secs(1));
        canned.insert("music/a2.flac".to_string(), loudness(-8.9689, 0.5));
        canned.insert("music/a3.flac".to_string(), loudness(-2.9691, 0.25));
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: false,
                album_values: true,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(
            report.measured, 3,
            "Album A's three members land the shared pair"
        );
        assert_eq!(report.skipped, 2, "a1 and b1 already carry a value");
        assert_eq!(report.failed, 0);

        let a1 = album_tags_of(&wired.writer.writes(), "music/a1.flac")
            .expect("a1 carries the Album's pair like every other member");
        let album_gain = a1.album_gain.expect("the album pair is written");
        assert!(
            album_gain.is_finite(),
            "the pair written to a member's file is one a player can honor, got {album_gain}"
        );
        assert!(
            (album_gain - (-7.72)).abs() < 0.05,
            "the aggregate is the two weighted members alone, not diluted by a1, got {album_gain}"
        );
        assert!((a1.album_peak.unwrap() - 0.5).abs() < 1e-6);
    }

    /// Measurement always wins over manual edits: a hand-set value is simply
    /// overwritten by the next measurement, and Force is the way to get
    /// there.
    #[test]
    fn measurement_wins_over_manual_edits() {
        let (mut library, canned) = fixture();
        library.tracks[1].metadata.replaygain_track_gain = Some(-1.0);
        library.tracks[1].metadata.replaygain_track_peak = Some(0.1);
        library.tracks[1].metadata.replaygain_album_gain = Some(-2.0);
        library.tracks[1].metadata.replaygain_album_peak = Some(0.2);
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: true,
                force: true,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(
            report.measured, 8,
            "four track pairs + Album A's and Album B's aggregates"
        );
        let committed = wired.mutations.committed_flat();
        let a2_rows: Vec<&Track> = committed
            .iter()
            .filter(|t| t.id.0 == "music/a2.flac")
            .collect();
        assert_eq!(
            a2_rows.len(),
            2,
            "a2's track-pair commit, then the album-pair commit"
        );
        assert_eq!(
            a2_rows[0].metadata.replaygain_track_gain,
            Some(-6.0),
            "the measured value won over the manual edit"
        );
        assert!(
            (a2_rows[1].metadata.replaygain_album_gain.unwrap() - (-6.0)).abs() < 0.01,
            "the recomputed aggregate won"
        );
    }

    // --- Write ordering and failure isolation -----------------------------------------

    /// A file-write failure is reported and the Store is left untouched: the
    /// failed Track's facts are excluded from the commit, and the other
    /// members still land.
    #[test]
    fn a_file_write_failure_is_reported_and_leaves_the_store_untouched() {
        let (mut library, canned) = fixture();
        library.tracks[1].metadata.replaygain_track_gain = Some(-9.0);
        library.tracks[1].metadata.replaygain_track_peak = Some(0.4);
        let mut analyzer = FakeAnalyzer::new(canned);
        analyzer.fail.insert("music/a2.flac".to_string());
        let mut writer = FakeWriter::new();
        writer.fail.insert("music/a1.flac".to_string());
        let mut wired = wire_with(library, analyzer, writer);

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: false,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 2, "a3 and b1 landed");
        assert_eq!(report.failed, 1, "a1's write failed");
        assert_eq!(
            report.first_failure.as_deref(),
            Some("music/a1.flac: IO error: permission denied"),
            "the reason reaches the outcome report"
        );
        let committed = wired.mutations.committed_flat();
        assert_eq!(committed.len(), 2);
        assert!(
            committed.iter().any(|t| t.id.0 == "music/a3.flac")
                && committed.iter().all(|t| t.id.0 != "music/a1.flac"),
            "a3 landed, and there is no phantom value for a1"
        );
    }

    /// A measurement failure is reported the same way: reported, skipped,
    /// and nothing written for that Track.
    #[test]
    fn a_measurement_failure_is_reported_and_writes_nothing_for_that_track() {
        let (mut library, _canned) = fixture();
        library.tracks[2].metadata.replaygain_track_gain = Some(-9.0);
        library.tracks[2].metadata.replaygain_track_peak = Some(0.4);
        let mut analyzer = FakeAnalyzer::new(HashMap::new());
        analyzer.fail.insert("music/a2.flac".to_string());
        let mut wired = wire_with(library, analyzer, FakeWriter::new());

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: false,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 2, "a1 and b1 measured; only a2 failed");
        assert_eq!(report.failed, 1);
        assert!(report.first_failure.unwrap().contains("measurement failed"));
        assert!(
            !wired
                .writer
                .writes()
                .iter()
                .any(|(p, _)| p == "music/a2.flac")
        );
    }

    /// A store-commit failure is reported: the file tags carry the values,
    /// but the pass claims nothing landed.
    #[test]
    fn a_store_commit_failure_is_reported() {
        let (library, canned) = fixture();
        let mut wired = wire(library, FakeAnalyzer::new(canned));
        wired.mutations.fail.store(true, Ordering::Relaxed);

        let report = wired.pass.run(
            &PassCommand::Track(TrackId("music/a1.flac".to_string())),
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 0);
        assert_eq!(report.failed, 1);
        assert!(report.first_failure.unwrap().contains("store is dead"));
        assert_eq!(
            wired.writer.writes().len(),
            1,
            "the file write happened first"
        );
    }

    // --- Batching and interruption ------------------------------------------------------

    /// Commits are batched in tens, the scan's prior art.
    #[test]
    fn commits_are_batched_in_tens() {
        let mut library = FakeLibrary::default();
        for i in 0..25 {
            library
                .tracks
                .push(track(&format!("music/t{i:02}.flac"), None));
        }
        let mut wired = wire(library, FakeAnalyzer::new(HashMap::new()));

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: false,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 25);
        let batches = wired.mutations.committed();
        let sizes: Vec<usize> = batches.iter().map(std::vec::Vec::len).collect();
        assert_eq!(sizes, vec![10, 10, 5], "the scan's batch shape");
    }

    /// An interrupted pass keeps everything it already committed: the store
    /// holds the first batch, the report says cancelled, and nothing after
    /// the interruption is claimed.
    #[test]
    fn an_interrupted_pass_keeps_its_committed_batches() {
        let mut library = FakeLibrary::default();
        for i in 0..25 {
            library
                .tracks
                .push(track(&format!("music/t{i:02}.flac"), None));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let mut analyzer = FakeAnalyzer::new(HashMap::new());
        analyzer.cancel_after = Some((Arc::clone(&cancel), 10));
        let mut wired = wire(library, analyzer);

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: false,
                force: false,
            },
            &cancel,
            no_progress(),
        );

        assert!(report.cancelled, "the pass reports its interruption");
        assert_eq!(report.measured, 10, "exactly the first committed batch");
        let batches = wired.mutations.committed();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 10);
        assert_eq!(
            wired.writer.writes().len(),
            10,
            "no file writes after the interruption"
        );
    }

    /// Neither checkbox enabled: the pass is a no-op, not a full-library
    /// decode that writes nothing.
    #[test]
    fn a_library_wide_pass_with_neither_checkbox_enabled_does_nothing() {
        let (library, canned) = fixture();
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let report = wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: false,
                album_values: false,
                force: false,
            },
            &AtomicBool::new(false),
            no_progress(),
        );

        assert_eq!(report.measured, 0);
        assert_eq!(wired.analyzer.calls(), [] as [String; 0]);
        assert_eq!(wired.writer.writes(), [] as [(String, ReplayGainTags); 0]);
    }

    // --- progress -------------------------------------------------------------------

    /// Progress counts measurement completions against the pass's targets.
    #[test]
    fn progress_counts_measurement_completions() {
        let (library, canned) = fixture();
        let mut wired = wire(library, FakeAnalyzer::new(canned));

        let mut seen = Vec::new();
        wired.pass.run(
            &PassCommand::LibraryWide {
                track_values: true,
                album_values: false,
                force: false,
            },
            &AtomicBool::new(false),
            |done, total| seen.push((done, total)),
        );
        assert_eq!(seen.first(), Some(&(0, 4)));
        assert_eq!(seen.last(), Some(&(4, 4)));
    }

    /// Wire with pre-configured fakes (failures set before the pass runs).
    fn wire_with(library: FakeLibrary, analyzer: FakeAnalyzer, writer: FakeWriter) -> Wired {
        let analyzer = Arc::new(analyzer);
        let writer = Arc::new(writer);
        let mutations = Arc::new(FakeMutations::new());
        let pass = ReplayGainPass::new(
            Box::new(SharedAnalyzer(analyzer.clone())),
            Box::new(SharedWriter(writer.clone())),
            Box::new(library.stub()),
            Box::new(SharedMutations(mutations.clone())),
        );
        Wired {
            pass,
            analyzer,
            writer,
            mutations,
        }
    }
}
