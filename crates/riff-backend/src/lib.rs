//! riff Backend — the application API.
//!
//! Owns the Backend Facade, the typed event and notice surface, the facade
//! transport, and the Composition Root (`composition::AppRuntime::spawn`),
//! which is the only place that knows both the slice-defined ports and the
//! concrete `riff-infra` adapters — and the worker threads that run them.
//! Re-exports the read-side surface the frontend renders (entities, Session
//! Views, projections, Transport) so the frontend keeps one dependency.
//! Contains no UI crate dependency and no native dependencies of its own.

pub mod app;
pub mod composition;
pub mod domain;
