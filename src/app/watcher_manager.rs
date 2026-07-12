use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use crossbeam_channel::Sender;
use crate::app::commands::LibraryCommand;
use crate::infra::watcher::FilesystemWatcher;

pub struct WatcherManager {
    watcher: Option<FilesystemWatcher>,
    lib_cmd_tx: Sender<LibraryCommand>,
    debounce_timers: HashMap<PathBuf, Instant>,
    scan_in_progress: HashMap<PathBuf, bool>,
    pending_rescan: HashMap<PathBuf, bool>,
}

impl WatcherManager {
    pub fn new(
        watcher: Option<FilesystemWatcher>,
        lib_cmd_tx: Sender<LibraryCommand>,
    ) -> Self {
        Self {
            watcher,
            lib_cmd_tx,
            debounce_timers: HashMap::new(),
            scan_in_progress: HashMap::new(),
            pending_rescan: HashMap::new(),
        }
    }

    pub fn start_watching(&mut self, path: &Path) -> Result<(), String> {
        let Some(ref mut watcher) = self.watcher else {
            return Err("Watcher not initialized".to_string());
        };
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        watcher.watch(&canonical).map_err(|e| format!("Watch failed: {}", e))?;
        self.scan_in_progress.insert(canonical.clone(), false);
        self.pending_rescan.insert(canonical, false);
        Ok(())
    }

    pub fn stop_watching(&mut self, path: &Path) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(ref mut watcher) = self.watcher {
            let _ = watcher.unwatch(&canonical);
        }
        self.debounce_timers.remove(&canonical);
        self.scan_in_progress.remove(&canonical);
        self.pending_rescan.remove(&canonical);
    }

    pub fn stop_all(&mut self) {
        let paths: Vec<PathBuf> = self.scan_in_progress.keys().cloned().collect();
        for path in paths {
            self.stop_watching(&path);
        }
        self.watcher = None;
    }

    pub fn on_fs_event(&mut self, changed_path: &Path) {
        let lib_path = self.find_library_path(changed_path);
        let Some(lib_path) = lib_path else {
            return;
        };

        if self.scan_in_progress.get(&lib_path).copied().unwrap_or(false) {
            self.pending_rescan.insert(lib_path, true);
            return;
        }

        self.debounce_timers
            .entry(lib_path.clone())
            .and_modify(|t| *t = Instant::now())
            .or_insert_with(Instant::now);

        if self.scan_in_progress.get(&lib_path).copied().unwrap_or(false) {
            self.pending_rescan.insert(lib_path, true);
        }
    }

    pub fn poll(&mut self) {
        let now = Instant::now();
        let mut expired = Vec::new();
        for (path, timer) in &self.debounce_timers {
            if now.duration_since(*timer) >= std::time::Duration::from_secs(2) {
                expired.push(path.clone());
            }
        }
        for path in expired {
            self.debounce_timers.remove(&path);
            if !self.scan_in_progress.get(&path).copied().unwrap_or(false) {
                self.scan_in_progress.insert(path.clone(), true);
                let _ = self.lib_cmd_tx.send(LibraryCommand::ScanDirectory(path));
            }
        }
    }

    pub fn mark_scan_complete(&mut self, path: &Path) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.scan_in_progress.insert(canonical.clone(), false);

        if self.pending_rescan.get(&canonical).copied().unwrap_or(false) {
            self.pending_rescan.insert(canonical.clone(), false);
            let _ = self.lib_cmd_tx.send(LibraryCommand::ScanDirectory(canonical));
        }
    }

    fn find_library_path(&self, changed_path: &Path) -> Option<PathBuf> {
        let canonical = std::fs::canonicalize(changed_path).unwrap_or_else(|_| changed_path.to_path_buf());
        self.scan_in_progress
            .keys()
            .find(|lib_path| canonical.starts_with(lib_path))
            .cloned()
    }
}
