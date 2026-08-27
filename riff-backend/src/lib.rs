//! riff Backend — headless domain, application, and infrastructure.
//!
//! Owns the domain, use cases, infrastructure adapters, worker threads, and
//! the Application Store. Contains no UI crate dependency. Publicly re-exports
//! every module under `domain/`, `app/`, `infra/` so the root `riff` package
//! can re-export them verbatim.

pub mod app;
pub mod domain;
pub mod infra;
