use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum LibraryCommand {
    ScanDirectory(PathBuf),
    CancelScan,
}

#[derive(Debug, Clone)]
pub enum LibraryUpdate {
    ScanProgress { files_found: usize, current_dir: String },
    ScanComplete { total_files: usize },
    ScanError(String),
}
