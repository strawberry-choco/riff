//! Re-export of the Application Store contract from riff-persistence.
//!
//! All store ports, DTOs, and error types are re-exported so existing
//! qualified paths (`riff_backend::app::store::Settings`, etc.) continue to compile.

pub use riff_persistence::store::*;
