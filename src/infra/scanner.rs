use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "opus", "ogg", "flac", "wav"];

pub struct AudioFileScanner {
    cancel_flag: Arc<AtomicBool>,
}

impl AudioFileScanner {
    pub fn new(cancel_flag: Arc<AtomicBool>) -> Self {
        Self { cancel_flag }
    }

    /// Recursively collect audio files under `path`. Directory entries that
    /// cannot be read (permissions etc.) are logged and skipped, so a scan
    /// never fails outright.
    pub fn scan(&self, path: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();

        for entry in WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
        {
            if self.cancel_flag.load(Ordering::Relaxed) {
                break;
            }

            match entry {
                Ok(entry) => {
                    if entry.file_type().is_file() {
                        if let Some(ext) = entry.path().extension() {
                            let ext_lower = ext.to_string_lossy().to_lowercase();
                            if AUDIO_EXTENSIONS.contains(&ext_lower.as_str()) {
                                files.push(entry.path().to_path_buf());
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Some(path) = e.path() {
                        tracing::warn!(
                            "Permission denied or error accessing {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }

        files
    }
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .is_some_and(|s| s.starts_with('.'))
}
