//! Filesystem adapters: the walkdir scanner and the notify watcher, serving
//! riff-library's scanning and [`FilesystemWatch`](riff_library::infra::ports::FilesystemWatch)
//! needs.

pub mod scanner;
pub mod watcher;

pub use scanner::AudioFileScanner;
pub use watcher::FilesystemWatcher;
