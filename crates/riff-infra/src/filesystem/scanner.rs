use riff_persistence::store::ScanOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::WalkDir;

pub struct AudioFileScanner {
    cancel_flag: Arc<AtomicBool>,
}

impl AudioFileScanner {
    pub fn new(cancel_flag: Arc<AtomicBool>) -> Self {
        Self { cancel_flag }
    }

    /// Recursively collect audio files under `path`, honoring the Library
    /// Scan options (design-handoff issue 12): `skip_hidden_files` prunes
    /// dot-entries, `formats` decides which extensions are indexed.
    /// Directory entries that cannot be read (permissions etc.) are logged
    /// and skipped, so a scan never fails outright.
    pub fn scan(&self, path: &Path, options: &ScanOptions) -> Vec<PathBuf> {
        let mut files = Vec::new();

        for entry in WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            // The scan root is never pruned: the user picked it explicitly,
            // so only entries inside it answer to skip-hidden (tempdir-style
            // dot-prefixed roots would otherwise empty the walk).
            .filter_entry(|e| e.depth() == 0 || !options.skip_hidden_files || !is_hidden(e))
        {
            if self.cancel_flag.load(Ordering::Relaxed) {
                break;
            }

            match entry {
                Ok(entry) => {
                    if entry.file_type().is_file()
                        && let Some(ext) = entry.path().extension()
                    {
                        let ext_lower = ext.to_string_lossy().to_lowercase();
                        if options
                            .formats
                            .iter()
                            .any(|format| format.eq_ignore_ascii_case(&ext_lower))
                        {
                            files.push(entry.path().to_path_buf());
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
