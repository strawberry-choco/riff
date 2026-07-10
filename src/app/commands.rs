use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum LibraryCommand {
    ScanDirectory(PathBuf),
    CancelScan,
}

#[derive(Debug, Clone)]
pub enum LibraryUpdate {
    ScanProgress { path: PathBuf, files_found: usize, current_dir: String },
    ScanComplete { path: PathBuf, total_files: usize },
    ScanError { path: PathBuf, message: String },
}
