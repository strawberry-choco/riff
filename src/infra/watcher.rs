use std::path::{Path, PathBuf};
use crossbeam_channel::Sender;
use notify::Watcher;

pub struct FilesystemWatcher {
    inner: notify::RecommendedWatcher,
}

impl FilesystemWatcher {
    pub fn new(event_tx: Sender<PathBuf>) -> Result<Self, notify::Error> {
        let inner = notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        for path in &event.paths {
                            let _ = event_tx.send(path.clone());
                        }
                    }
                    Err(e) => {
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
