use crossbeam_channel::Sender;
use notify_debouncer_full::{DebouncedEvent, Debouncer, RecommendedCache, new_debouncer};
use std::path::{Path, PathBuf};
use std::time::Duration;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "opus", "ogg", "flac", "wav"];

/// How long the filesystem must stay quiet before a burst of changes is
/// flushed downstream as one debounced batch (the coalescing window).
const DEBOUNCE_TIMEOUT: Duration = Duration::from_secs(2);

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

/// The audio-file paths carried by one debounced event batch: non-audio
/// paths dropped, duplicates collapsed, first-seen order preserved. This is
/// the "which paths changed" half of the batch contract forwarded to the
/// watcher manager.
pub fn debounced_audio_paths(events: &[DebouncedEvent]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for event in events {
        for path in event.paths.iter().filter(|path| is_audio_file(path)) {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }
    paths
}

pub struct FilesystemWatcher {
    inner: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
}

impl FilesystemWatcher {
    /// Watch adapter over `notify-debouncer-full`: raw filesystem events are
    /// coalesced per [`DEBOUNCE_TIMEOUT`] and each flush is forwarded as ONE
    /// batch of changed audio-file paths. The callback only sends on the
    /// channel — it never blocks and never touches shared app state.
    pub fn new(event_tx: Sender<Vec<PathBuf>>) -> Result<Self, notify::Error> {
        let inner = new_debouncer(
            DEBOUNCE_TIMEOUT,
            None,
            move |res: Result<Vec<DebouncedEvent>, Vec<notify::Error>>| match res {
                Ok(events) => {
                    let paths = debounced_audio_paths(&events);
                    if !paths.is_empty() {
                        let _ = event_tx.send(paths);
                    }
                }
                Err(errors) => {
                    for e in errors {
                        tracing::warn!("Filesystem watcher error: {}", e);
                    }
                }
            },
        )?;
        Ok(Self { inner })
    }

    pub fn watch(&mut self, path: &Path) -> Result<(), notify::Error> {
        self.inner.watch(path, notify::RecursiveMode::Recursive)
    }

    pub fn unwatch(&mut self, path: &Path) -> Result<(), notify::Error> {
        self.inner.unwatch(path)
    }
}
