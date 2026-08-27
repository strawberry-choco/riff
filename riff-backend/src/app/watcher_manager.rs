use crate::app::scan_service::{ScanService, Scans};
use crate::infra::watcher::FilesystemWatcher;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct WatcherManager {
    watcher: Option<FilesystemWatcher>,
    /// Shared clone of the Library Scan Service front end: rescans are
    /// initiated through [`ScanService::request`] and "is a scan running"
    /// is answered by [`ScanService::is_scanning`] — the service's per-path
    /// state is the single source of truth, so this manager keeps no
    /// parallel copy of it (and needs no Complete relay from the UI).
    scans: ScanService,
    /// Roots whose changes arrived while a scan was running and want exactly
    /// one follow-up rescan once it ends.
    pending_rescan: HashSet<PathBuf>,
    /// Roots currently watched (`start_watching` minus `stop_watching`);
    /// `on_fs_events` maps a changed path back to its root through this set.
    watched: HashSet<PathBuf>,
}

impl WatcherManager {
    pub fn new(watcher: Option<FilesystemWatcher>, scans: ScanService) -> Self {
        Self {
            watcher,
            scans,
            pending_rescan: HashSet::new(),
            watched: HashSet::new(),
        }
    }

    pub fn start_watching(&mut self, path: &Path) -> Result<(), String> {
        let Some(ref mut watcher) = self.watcher else {
            return Err("Watcher not initialized".to_string());
        };
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        watcher
            .watch(&canonical)
            .map_err(|e| format!("Watch failed: {e}"))?;
        self.watched.insert(canonical);
        Ok(())
    }

    pub fn stop_watching(&mut self, path: &Path) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(ref mut watcher) = self.watcher {
            let _ = watcher.unwatch(&canonical);
        }
        self.pending_rescan.remove(&canonical);
        self.watched.remove(&canonical);
    }

    pub fn stop_all(&mut self) {
        let paths: Vec<PathBuf> = self.watched.iter().cloned().collect();
        for path in paths {
            self.stop_watching(&path);
        }
        self.watcher = None;
    }

    /// Consume one already-debounced batch of changed audio-file paths (the
    /// coalescing window lives upstream in [`FilesystemWatcher`], so no
    /// timing state is kept here). Each distinct watched root gets exactly
    /// one decision per batch:
    ///
    /// - changes landing during a running scan cannot be scanned right now:
    ///   remember the root for exactly one follow-up rescan once the service
    ///   reports the path idle again (checked in [`Self::poll`]);
    /// - otherwise request the rescan immediately.
    pub fn on_fs_events(&mut self, changed_paths: &[PathBuf]) {
        let mut roots: Vec<PathBuf> = Vec::new();
        for changed_path in changed_paths {
            if let Some(lib_path) = self.find_library_path(changed_path)
                && !roots.contains(&lib_path)
            {
                roots.push(lib_path);
            }
        }

        for lib_path in roots {
            if self.scans.is_scanning(&lib_path) {
                self.pending_rescan.insert(lib_path);
            } else {
                self.scans.request(lib_path);
            }
        }
    }

    /// Fire deferred follow-ups: a root whose changes landed mid-scan gets
    /// one rescan as soon as the running scan ends. The service's
    /// `is_scanning` is queried directly — no Complete relay needed. Called
    /// once per UI frame, exactly like before.
    pub fn poll(&mut self) {
        let ready: Vec<PathBuf> = self
            .pending_rescan
            .iter()
            .filter(|path| !self.scans.is_scanning(path))
            .cloned()
            .collect();
        for path in ready {
            self.pending_rescan.remove(&path);
            self.scans.request(path);
        }
    }

    fn find_library_path(&self, changed_path: &Path) -> Option<PathBuf> {
        let canonical =
            std::fs::canonicalize(changed_path).unwrap_or_else(|_| changed_path.to_path_buf());
        self.watched
            .iter()
            .find(|lib_path| canonical.starts_with(*lib_path))
            .cloned()
    }
}
