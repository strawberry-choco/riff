use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum LibraryCommand {
    ScanDirectory(PathBuf),
    CancelScan,
}

#[derive(Debug, Clone)]
pub enum LibraryUpdate {
    Progress {
        path: PathBuf,
        files_found: usize,
        current_dir: String,
    },
    Complete {
        path: PathBuf,
        total_files: usize,
    },
    Error {
        path: PathBuf,
        message: String,
    },
}
