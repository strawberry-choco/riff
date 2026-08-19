use crossbeam_channel::Sender;
use notify::Watcher;
use std::path::{Path, PathBuf};

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "opus", "ogg", "flac", "wav"];

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

pub struct FilesystemWatcher {
    inner: notify::RecommendedWatcher,
}

impl FilesystemWatcher {
    pub fn new(event_tx: Sender<PathBuf>) -> Result<Self, notify::Error> {
        let inner = notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| match res {
                Ok(event) => {
                    for path in &event.paths {
                        if is_audio_file(path) {
                            let _ = event_tx.send(path.clone());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Filesystem watcher error: {}", e);
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
