//! The `ReplayGain` Pass service: the library app layer's Pass use case behind
//! a backend app-layer service seam (ADR 0006), on its own worker thread with
//! its own stop flag — spawned and joined by the Composition Root.
//!
//! The front end ([`Passes`]) is the shape every sibling service shares:
//! `submit` a command, poll the outcome, poll the progress. The worker drains
//! one command at a time to completion — a pass is long, and a second pass
//! queued behind a running one is the Settings surface's job to prevent, not
//! this worker's to interleave.

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use riff_library::app::replaygain_pass::{PassCommand, PassReport, ReplayGainPass};
use riff_library::app::traits::{LoudnessAnalyzer, ReplayGainWriter};
use riff_persistence::store::{LibraryMutationStore, LibraryQueryStore};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app::MutexExt;

/// How often the idle worker wakes to check its stop lever.
const WORKER_POLL: Duration = Duration::from_millis(10);

/// The `ReplayGain` Pass front end: submit a pass command, poll its progress
/// and its outcome.
pub trait Passes: Send {
    /// Queue one pass. A command submitted while another pass runs waits its
    /// turn: the worker is serial.
    fn submit(&self, command: PassCommand);

    /// Ask the running pass to stop at its next measurement boundary. Every
    /// batch it already committed stays committed; resumption is just
    /// running it again.
    fn cancel(&self);

    /// Whether a pass is currently running.
    fn is_running(&self) -> bool;

    /// Drain the settled outcome of the last pass, if one has finished.
    fn poll(&self) -> Option<PassReport>;

    /// The running pass's `(done, total)` measurement progress. `(0, 0)`
    /// while idle.
    fn poll_progress(&self) -> (usize, usize);
}

/// The [`Passes`] front end over the channels the worker reads.
pub struct PassService {
    request_tx: Sender<PassCommand>,
    outcome_rx: Receiver<PassReport>,
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    progress: Arc<Mutex<(usize, usize)>>,
}

impl Passes for PassService {
    fn submit(&self, command: PassCommand) {
        let _ = self.request_tx.send(command);
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn poll(&self) -> Option<PassReport> {
        self.outcome_rx.try_recv().ok()
    }

    fn poll_progress(&self) -> (usize, usize) {
        *self.progress.lock_or_recover()
    }
}

impl PassService {
    /// Wire one service/worker pair over the given ports. `stop` ends the
    /// worker thread; `cancel` aborts the pass in flight; `progress` is the
    /// shared `(done, total)` cell the worker's pass writes and the front
    /// end's `poll_progress` reads — caller-supplied so whoever wires the
    /// runtime can hold the same cell the pass fills. The worker runs
    /// `pass` — the app-layer use case with its real adapters already
    /// injected.
    pub fn new(
        analyzer: Box<dyn LoudnessAnalyzer + Send>,
        writer: Box<dyn ReplayGainWriter + Send>,
        queries: Box<dyn LibraryQueryStore + Send>,
        mutations: Box<dyn LibraryMutationStore + Send>,
        cancel: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        progress: Arc<Mutex<(usize, usize)>>,
    ) -> (Self, PassWorker) {
        let (request_tx, request_rx) = unbounded();
        let (outcome_tx, outcome_rx) = unbounded();
        let running = Arc::new(AtomicBool::new(false));
        (
            Self {
                request_tx,
                outcome_rx,
                cancel: Arc::clone(&cancel),
                running: Arc::clone(&running),
                progress: Arc::clone(&progress),
            },
            PassWorker {
                request_rx,
                outcome_tx,
                cancel,
                running,
                progress,
                stop,
                pass: ReplayGainPass::new(analyzer, writer, queries, mutations),
            },
        )
    }
}

/// The blocking half: drains one pass command at a time to completion.
pub struct PassWorker {
    request_rx: Receiver<PassCommand>,
    outcome_tx: Sender<PassReport>,
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    progress: Arc<Mutex<(usize, usize)>>,
    stop: Arc<AtomicBool>,
    pass: ReplayGainPass,
}

impl PassWorker {
    /// Serve pass commands until the stop lever is raised or the front end
    /// disconnects.
    pub fn run(mut self) {
        while !self.stop.load(Ordering::Relaxed) {
            match self.request_rx.recv_timeout(WORKER_POLL) {
                Ok(command) => {
                    self.running.store(true, Ordering::Relaxed);
                    *self.progress.lock_or_recover() = (0, 0);
                    // A fresh pass starts uncancelled — a cancel that landed
                    // between submit and start must not eat it.
                    self.cancel.store(false, Ordering::Relaxed);
                    let report = self.pass.run(&command, &self.cancel, |done, total| {
                        *self.progress.lock_or_recover() = (done, total);
                    });
                    self.running.store(false, Ordering::Relaxed);
                    let _ = self.outcome_tx.send(report);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::store::FullScanSummary;
    use riff_library::app::errors::LibraryError;
    use riff_library::app::traits::{ReplayGainTags, TrackLoudness};
    use riff_persistence::errors::StoreError;
    use riff_persistence::test_support::StubLibraryQueryStore;
    use riff_persistence::track::{Track, TrackId, TrackMetadata};
    use std::path::Path;
    use std::thread;
    use std::time::Instant;

    // --- Fakes (the pass use case's own suite asserts its rule set; these
    // only need to make the service seam observable) -------------------------

    /// The fake analyzer: instant canned measurements, plus an optional hook
    /// fired on every call so a test can sample the seam mid-pass
    /// deterministically (progress on call N, cancel on call N).
    struct FakeAnalyzer {
        on_call: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    }

    impl FakeAnalyzer {
        fn new() -> Self {
            Self {
                on_call: Mutex::new(None),
            }
        }

        fn set_hook(&self, hook: Box<dyn Fn() + Send + Sync>) {
            *self.on_call.lock().unwrap() = Some(hook);
        }
    }

    impl LoudnessAnalyzer for FakeAnalyzer {
        fn measure_track(&self, _path: &Path) -> Result<TrackLoudness, LibraryError> {
            if let Some(hook) = self.on_call.lock().unwrap().as_ref() {
                hook();
            }
            Ok(TrackLoudness {
                track_gain_db: -6.0,
                track_peak: 0.5,
            })
        }
    }

    /// The analyzer sits behind an `Arc` so the test can arm its hook after
    /// the service owns the port object.
    struct SharedAnalyzer(Arc<FakeAnalyzer>);

    impl LoudnessAnalyzer for SharedAnalyzer {
        fn measure_track(&self, path: &Path) -> Result<TrackLoudness, LibraryError> {
            self.0.measure_track(path)
        }
    }

    struct FakeWriter;

    impl ReplayGainWriter for FakeWriter {
        fn write_replaygain(
            &self,
            _path: &Path,
            _tags: &ReplayGainTags,
        ) -> Result<(), LibraryError> {
            Ok(())
        }
    }

    struct FakeMutations;

    impl LibraryMutationStore for FakeMutations {
        fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, StoreError> {
            Ok(0)
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
        fn remove_library_path(&mut self, _root: &Path) -> Result<usize, StoreError> {
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

    /// A library of `count` unmeasured tracks for the library-wide command.
    fn library_of(count: usize) -> StubLibraryQueryStore {
        let tracks: Arc<Vec<String>> =
            Arc::new((0..count).map(|i| format!("music/t{i:02}.flac")).collect());
        let ids = Arc::clone(&tracks);
        StubLibraryQueryStore::new(move |id| {
            tracks.contains(&id.0).then(|| Track {
                id: TrackId(id.0.clone()),
                file_path: std::path::PathBuf::from(&id.0),
                metadata: TrackMetadata::default(),
                duration: None,
                sample_rate: None,
                channels: None,
                play_count: 0,
                last_played: None,
                date_added: None,
                favorite: false,
                search_text: String::new(),
            })
        })
        .with_all_track_ids(move || ids.iter().cloned().map(TrackId).collect())
    }

    /// The wired service over a real worker thread, with the analyzer's
    /// `Arc` and the shared `cancel`/`progress` cells kept back for hook
    /// arming.
    #[allow(clippy::type_complexity)]
    fn service_with(
        tracks: usize,
    ) -> (
        PassService,
        Arc<FakeAnalyzer>,
        Arc<AtomicBool>,
        Arc<Mutex<(usize, usize)>>,
        thread::JoinHandle<()>,
        Arc<AtomicBool>,
    ) {
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new((0, 0)));
        let analyzer = Arc::new(FakeAnalyzer::new());
        let (service, worker) = PassService::new(
            Box::new(SharedAnalyzer(Arc::clone(&analyzer))),
            Box::new(FakeWriter),
            Box::new(library_of(tracks)),
            Box::new(FakeMutations),
            Arc::clone(&cancel),
            Arc::clone(&stop),
            Arc::clone(&progress),
        );
        let handle = thread::spawn(move || worker.run());
        (service, analyzer, cancel, progress, handle, stop)
    }

    /// Poll until the worker settles the outcome (bounded, so a broken seam
    /// fails the test instead of hanging it).
    fn wait_outcome(service: &PassService) -> PassReport {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(report) = service.poll() {
                return report;
            }
            assert!(Instant::now() < deadline, "the pass never settled");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_submitted_pass_runs_and_its_outcome_is_polled() {
        let (service, _analyzer, _cancel, _progress, handle, stop) = service_with(25);
        service.submit(PassCommand::LibraryWide {
            track_values: true,
            album_values: false,
            force: false,
        });

        let report = wait_outcome(&service);
        assert_eq!(report.measured, 25);
        assert!(!service.is_running(), "the pass is over");

        stop.store(true, Ordering::Relaxed);
        handle.join().expect("the worker ends cleanly");
    }

    /// The running pass's progress is polled: call *n* of a 25-track pass
    /// samples `n − 1` completed measurements against the target.
    #[test]
    fn progress_counts_up_while_the_pass_runs() {
        let (service, analyzer, _cancel, progress, handle, stop) = service_with(25);
        // The hook owns a clone of the same shared progress cell the worker's
        // pass writes, so it samples the seam mid-pass without borrowing the
        // front end.
        let probe = Arc::clone(&progress);
        analyzer.set_hook(Box::new(move || {
            // Sampling the cell while the pass is between locks: read and
            // record whatever is current.
            let _ = probe;
        }));
        let sampled: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(Vec::new()));
        let probe = Arc::clone(&sampled);
        let cell = Arc::clone(&progress);
        analyzer.set_hook(Box::new(move || {
            probe.lock().unwrap().push(*cell.lock().unwrap());
        }));
        service.submit(PassCommand::LibraryWide {
            track_values: true,
            album_values: false,
            force: false,
        });

        let report = wait_outcome(&service);
        assert_eq!(report.measured, 25);

        let sampled = sampled.lock().unwrap();
        assert_eq!(sampled.len(), 25, "one sample per measurement");
        assert_eq!(sampled[1], (1, 25), "call two sees one landed measurement");
        assert_eq!(*sampled.last().unwrap(), (24, 25));

        stop.store(true, Ordering::Relaxed);
        handle.join().expect("the worker ends cleanly");
    }

    /// Cancelling stops the pass at its next measurement boundary and keeps
    /// everything it already committed — the outcome says so.
    #[test]
    fn cancel_stops_a_running_pass_which_keeps_its_committed_batches() {
        let (service, analyzer, cancel, _progress, handle, stop) = service_with(25);
        // Arm the pass's own cancel flag on the tenth measurement: the pass
        // stops at the next boundary, its first ten-track batch committed.
        let flag = Arc::clone(&cancel);
        analyzer.set_hook(Box::new(move || {
            static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            if CALLS.fetch_add(1, Ordering::Relaxed) + 1 == 10 {
                flag.store(true, Ordering::Relaxed);
            }
        }));
        service.submit(PassCommand::LibraryWide {
            track_values: true,
            album_values: false,
            force: false,
        });

        let report = wait_outcome(&service);
        assert!(report.cancelled, "the pass reports its interruption");
        assert_eq!(report.measured, 10, "exactly the committed first batch");

        stop.store(true, Ordering::Relaxed);
        handle.join().expect("the worker ends cleanly");
    }

    /// The stop lever ends an idle worker cleanly — the shutdown contract.
    #[test]
    fn the_stop_lever_ends_an_idle_worker() {
        let (service, _analyzer, _cancel, _progress, handle, stop) = service_with(3);
        assert!(!service.is_running());
        stop.store(true, Ordering::Relaxed);
        handle.join().expect("an idle worker ends on the lever");
        let _ = service;
    }
}
