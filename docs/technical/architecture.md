# Architecture

riff is a lightweight, offline-first desktop music player written in Rust on top of the egui immediate-mode UI framework. It ships as a Cargo **workspace** of seven crates with no code-generation step and no plugin system; persistence runs through ordered, checksummed SQLite migrations (see [./persistence.md](./persistence.md)). The architecture is a **vertical capability split**: the headless backend is divided into crates by capability with a strict, compiler-enforced dependency chain, infrastructure adapters that wrap external crates sit in one dedicated crate at the edge, and a single composition root wires everything together at startup.

This document is the reference for how the workspace is organized, how the crates are allowed to depend on one another, and what belongs where. For the runtime view of the system see [./threading-model.md](./threading-model.md) and [./data-flow.md](./data-flow.md); for the concrete types that flow between layers see [./data-model.md](./data-model.md). The split decision and its rationale are recorded in [ADR 0009](../adr/0009-vertical-crate-split-of-the-backend.md).

## Overview

The workspace members are five backend crates, the frontend, and the integration-test crate:

| Crate | Role | Responsibility |
|-------|------|----------------|
| `riff-persistence` | Persistence contract | The stored entities and the Application Store contract (store ports and DTOs). Pure `std` — no dependencies at all. |
| `riff-library` | Collection capability | Scanning, Session Projections, playlist management, cover resolution and service, the ports it consumes, its error type. |
| `riff-playback` | Playback capability | The Playback Queue, the audio engine, gapless logic, the playback coordinator, the Transport trait, the playback ports, the playback session, and the Up Next read model. |
| `riff-infra` | Adapter crate | Every port implementation and every native/external dependency, so toolchain requirements exist in exactly one place. |
| `riff-backend` | Application API | The Backend Facade, typed events and notices, the facade-adjacent application services, the library session state, and the Composition Root that owns the worker threads. |
| `riff-gui` | Frontend | egui UI, tray icon, native dialogs, fonts, and the `riff` binary entry point — a thin composition over `riff-backend`. |
| `tests` | Integration tests | The single workspace-root integration-test crate (cross-crate integration, UI, and golden-image suites). |

### Dependency chain

```
riff-gui (frontend, `riff` binary)
    | depends on
    v
riff-backend (application API + Composition Root)
    | depends on all four below
    |
    |--> riff-infra (adapters + native deps)
    |        | depends on all three below
    |        v
    |--> riff-library -------> riff-persistence
    |--> riff-playback ------> riff-persistence
    +-----------------------> riff-persistence
```

Read precisely, the edges are:

- `riff-persistence` depends on **nothing** (`std` only).
- `riff-library` and `riff-playback` each depend on `riff-persistence` (plus pure-Rust utilities such as `crossbeam-channel`, `thiserror`, `tracing`, `fastrand`). They are **true siblings with no edge between them** — neither can import a type from the other, and the compiler enforces it.
- `riff-infra` depends on all three (`riff-persistence`, `riff-library`, `riff-playback`) and implements their ports. This is where `rusqlite` (bundled SQLite, a C compiler), `cpal` (platform audio libraries), `symphonia`, `lofty`, `image`, `walkdir`, and `notify` live.
- `riff-backend` depends on all four. It is the only crate that names both the slice-defined ports and the concrete `riff-infra` adapters.
- `riff-gui` depends on `riff-backend` (plus UI/platform dependencies: `egui`/`eframe`, `resvg`, `tray-icon`/`muda`/`rfd` on non-Linux). It carries no adapter dependencies.
- `tests` depends on every crate by name (per-crate imports) so each suite reaches the type it needs directly.

The arrows point inward in the inversion-of-control sense: slices define ports and code against `Box<dyn Port>`; `riff-infra` depends on the slices and implements their traits; runtime control flows slices → adapters while the dependency arrow points adapters → slices.

## Crate Definitions and Membership Criteria

Each crate owns the types and ports it consumes; there is no shared dumping-ground crate.

### `riff-persistence` — types that cross the persistence boundary

**Membership criterion**: a type belongs here iff it crosses the persistence boundary.

It owns the stored entities (`Track`, `TrackId`, `TrackMetadata`, `Album`, `Artist`, `Playlist`, `PlaylistId`, `SmartPlaylistKind`, `CoverSource`), the Application Store ports (migrations, settings, playlists, library query, library mutation), and the store DTOs (`Settings`, `ScalarSettings`, `WatchState`, `PlaylistEntry`, `StoreGeneration`, `StoreChanged`, the Lost Gems threshold) plus the `StoreError` type. It is `std`-only and implements nothing — the SQLite adapter lives in `riff-infra`. The settings port and DTOs live here — not in a capability slice — because ports must sit below the adapter crate and the Application Store is one table family with one generation scheme; this is a contract placement, not a capability claim.

### `riff-library` — the collection capability

**Membership criterion**: collection use cases and the ports they consume.

It owns the scan-side Track construction and the Library Scan Service (`scan.rs`, `scan_service.rs`), playlist entry validity (`playlist_manager.rs`), cover resolution and the cover service (`cover_resolver.rs`, `cover_service.rs`), the Library Session Projections (`projection.rs`), the ports it consumes (`traits.rs`: `MetadataReader`, `MetadataWriter` with the `TagEdit` DTO, `CoverLoader` with the decoded-image DTO, `FilesystemWatch`), its own `LibraryError` type, and re-exports of the store contract from `riff-persistence`. It has no edge to `riff-playback` and no native dependencies.

### `riff-playback` — the playback capability

**Membership criterion**: playback use cases and the ports they consume.

It owns the Playback Queue and repeat mode (`domain/queue.rs`), the playback command/update/state/position types, the playback session (`PlaybackSession` — the half of the former `AppState` the engine, coordinator, and transport touch), its ports (`infra/ports.rs`: `AudioDecoder`, `DecoderFactory`, `AudioOutput`, `AudioFormatInfo`), its own `PlaybackError` type, the pure-Rust audio engine (`infra/audio_engine.rs` — decode scheduling and gapless handoff over the port traits, no codec code), gapless eligibility and frame/duration math (`gapless.rs`), the playback coordinator (`playback_coordinator.rs` — the decider of queue continuation), the `Transport` trait with `ChannelTransport` and `FacadeTransport` (`transport.rs`), and the Up Next / playback read model (`projection.rs` — a Session Projection that reads the Playback Queue, placed here so the library slice never imports a playback type).

### `riff-infra` — every port implementation and every native dependency

**Membership rule**: an item belongs here iff it implements a port defined in another crate or wraps a native/external dependency — nothing else.

Concretely: the SQLite Application Store (`store/`: `SqliteStore`, migrations, corruption recovery), the symphonia decoder and cpal output (`audio/`), the lofty metadata reader/writer and image cover loader (`media/`), and the walkdir scanner and notify watcher (`filesystem/`). The rationale is quarantine: bundled SQLite needs a C compiler and cpal needs platform audio libraries (ALSA on Linux), so keeping adapters out of the slices is what makes the slices pure Rust and testable anywhere. The crate preserves clean internal module seams (store / audio / media / filesystem) so it can be split further later without redesign if compile times ever demand it.

### `riff-backend` — the application API

**Membership criterion**: the facade surface, the facade-adjacent application services, and the one place that knows both ports and adapters.

It owns the Backend Facade (`facade.rs` — typed `BackendEvent`s, notices with source and severity, the command correlation record), the facade-adjacent application services that orchestrate across the facade (the Session Views read facade `views.rs`, the Tag Edit service `tag_edit_service.rs`, the Watcher Manager `watcher_manager.rs`), the library half of the session state (`state.rs`: `LibrarySession`, `ViewMode`, `BrowseMode`, `LibraryStatus`, `UiFlags` — the playback half lives in `riff-playback`), the Composition Root (`composition.rs`: `AppRuntime::spawn` opens the Application Store, constructs every real adapter, wires them into the slice-defined ports, and spawns the worker threads), and the re-export surface that keeps historical `riff_backend::…` import paths resolving for the frontend and the test suite. The crate carries no native dependencies of its own and no UI crate dependencies.

### `riff-gui` — the frontend

**Membership criterion**: rendering, input, and platform integration.

It owns egui widget code, the main window and its views, fonts and icon rasterization, the system tray (non-Linux), native dialogs, and the `riff` binary — a thin composition over `riff_backend::composition::AppRuntime::spawn` that opens the store at its default location and hands the returned `AppRuntime` handles to the UI and tray. The cover-texture LRU lives here because it is an egui-specific concern (`egui::TextureHandle`).

## Dependency Rules

The core rule is that **dependencies follow the chain above and nothing bypasses it**. Specific rules:

- `riff-persistence` has zero dependencies — not even `serde`. It performs no I/O.
- `riff-library` and `riff-playback` depend only on `riff-persistence` and pure-Rust utilities. They never name a concrete adapter, never import from each other, and never import `egui`, `rusqlite`, `symphonia`, `cpal`, `lofty`, or `image`.
- Each slice defines the port traits for every external dependency it has, and `riff-infra` implements them. A slice never names a concrete adapter type.
- `riff-infra` implements slice-defined ports; it contains no business logic (no play-order or shuffle decisions, no scan policy).
- `riff-backend` is the only place that names both a port and its concrete implementation (`composition.rs`). Concrete adapters are not re-exported to the frontend.
- `riff-gui` depends on `riff-backend` only (plus UI/platform crates). It reads the backend's re-exported read-side surface — entities, Session Views, projections, Transport — and never touches an adapter.

Inside each slice, the historical layering is preserved as module convention: `domain/` (pure types and logic), `app/` (use cases, session state, projections), `infra/` (port traits the adapter crate implements). What the split added is compiler enforcement of the boundaries *between* capabilities.

### Data crossing boundaries

- **Persistence contract to slices**: owned stored entities and DTOs (`Track`, `TrackId`, `Settings`, `PlaylistEntry`).
- **Slices to infrastructure**: trait method calls passing owned data or immutable references (for example `MetadataReader::read_all(&self, path: &Path)`).
- **Infrastructure to slices**: results returned through trait methods. Infrastructure never holds a reference to application state.
- **Backend to frontend**: the `AppRuntime` handles — the two session mutexes, the facade, the transports, the service front ends, and the store port views — plus channel messages drained by the UI each frame.

## Boundary Rules

### Synchronous calls, channels, and shared state

Three communication mechanisms are used, each for a different kind of interaction:

- **Synchronous calls** for fast, deterministic operations: queue manipulation, library queries, state reads.
- **Channels** (`crossbeam_channel::unbounded`) for all cross-thread communication: UI/tray to audio engine, engine to playback coordinator, scan service to worker, watcher events to the manager, tag-edit and cover request/response pairs, store change notifications to the facade.
- **Shared state** for read-heavy concurrent access: `Arc<Mutex<PlaybackSession>>` and `Arc<Mutex<LibrarySession>>` (the split of the former single `AppState`, each behind its own mutex), plus `Arc<Mutex<BackendFacade>>`. The audio ring buffer between the decode loop and the cpal callback lives inside `riff-infra`'s output adapter and is not part of the application surface.

### Manual dependency injection

There is no DI container. `composition.rs` performs manual constructor injection: it builds the concrete adapters and passes them — boxed as trait objects where appropriate — into the services and engine that need them. The `CoverResolver`, for example, is constructed over a `MetadataReader` and a `CoverLoader` port; it never knows those are backed by lofty and the `image` crate. No file outside the Composition Root constructs an adapter.

### Thread boundaries

- The egui event loop runs on the main thread (an egui/eframe requirement).
- Audio decoding runs on a dedicated audio engine thread; the Playback Coordinator runs on its own thread.
- Library scanning, filesystem-watch event processing, tag editing, and cover decoding each run on dedicated worker threads — all spawned by the Composition Root in `riff-backend`, not by the frontend.
- The cpal callback runs on an OS-owned real-time audio thread.

All thread-to-thread communication uses `crossbeam_channel`. See [./threading-model.md](./threading-model.md) for the full thread inventory and constraints.

### Error propagation

- Errors are typed per owner: `StoreError` in `riff-persistence`, `LibraryError` in `riff-library`, `PlaybackError` in `riff-playback` — each defined with `thiserror`, string-based so adapters can map into whichever port's error they answer.
- Infrastructure maps external crate errors into the owning port's error at the adapter boundary, so crate-specific error types never leak above `riff-infra`.
- Playback failures surface to the session as typed notices through the facade's notice channel (source + severity), not as a cross-slice state write.
- The UI displays user-friendly messages and never panics on a recoverable error.
- Mutex access uses the `MutexExt::lock_or_recover` helper (defined in `riff-backend`), which recovers a poisoned lock instead of panicking, so a panic on one thread does not cascade into every other thread that shares a session mutex.

## Validation Checklist

Use this checklist when adding or reviewing a component:

- [ ] **Crate placement**: each module is in the crate whose membership criterion it satisfies (see above), and in the conventional `domain/`/`app/`/`infra/` layer for its responsibility.
- [ ] **Dependency direction**: the crate's `Cargo.toml` gains no edge that violates the chain. The slices never gain a dependency on `riff-infra`, on each other, or on any native crate.
- [ ] **Trait abstraction**: every external dependency of a slice crosses a port trait defined in that slice and is implemented in `riff-infra`.
- [ ] **Purity**: `riff-persistence` builds with no dependencies at all; the slices stay pure Rust. If a change needs a C compiler or platform audio libraries, it belongs in `riff-infra`.
- [ ] **Thread safety**: shared state between threads is synchronized with `Arc<Mutex<_>>` (one mutex per session — never nested), and cross-thread communication uses channels.
- [ ] **Error handling**: errors are owned by the crate that raises them; `riff-infra` maps external errors into the owning port's error; the UI shows user-friendly messages.
- [ ] **No UI in logic**: there is no `egui` code outside `riff-gui`, and no audio decoding in the UI.
- [ ] **Composition root**: only `riff-backend/src/composition.rs` constructs and wires infrastructure; no other file calls `::new()` on a concrete adapter and injects it across a boundary.

## Anti-Patterns

- **Native crates in a slice**: a `Cargo.toml` of `riff-library` or `riff-playback` gains `rusqlite`, `cpal`, `symphonia`, `lofty`, or `image`. Move the code to `riff-infra` behind a port.
- **A slice imports the other slice**: `riff-library` code touches a `riff-playback` type (or vice versa). Move the shared type down into `riff-persistence`, or move the code into `riff-backend`.
- **egui outside the frontend**: any crate other than `riff-gui` imports `egui::`. UI concerns belong in the frontend.
- **Second composition root**: a file other than `composition.rs` calls `SymphoniaDecoder::new()` (or any concrete adapter constructor) and injects it across a boundary. Route the wiring through the Composition Root.
- **UI thread blocking**: a `riff-gui` handler performs scanning or image decoding synchronously. Move it to a worker thread and use a channel.
- **Callback spaghetti**: an audio callback calls UI methods directly. Use channels for all thread-to-thread communication; never call UI code from a non-UI thread.
- **Stringly typed errors**: errors passed as bare `String` across a port. Use the owning crate's typed error enum so callers can match on variants.

Note that the two-session split (`PlaybackSession` / `LibrarySession` behind their own mutexes) is a deliberate design, not an accident: each session is owned and mutated by the capability that cares about it, and the only cross-slice interaction (a playback failure setting a scan-status message) is a typed notice through the facade. Keep all session access short and never hold one session's lock while acquiring the other's.

## Ambiguity Signals

These are decisions with more than one defensible answer. Surface them explicitly rather than choosing silently:

- **Where the facade-adjacent services belong.** The Session Views facade, Tag Edit service, and Watcher Manager live in `riff-backend` because they orchestrate the facade surface the frontend renders; the use cases and ports beneath them live in `riff-library`. If a service ever needs to serve a non-frontend consumer, moving it into its slice is the considered step.
- **Where cover caching belongs.** The in-memory cover texture LRU lives in `riff-gui` because it stores egui-specific `TextureHandle`s. If caching policy (how long to keep covers) ever becomes a business rule, it may warrant a home in a slice.
- **Shared state versus message passing for playback position.** Position flows as a `PlaybackUpdate::PositionChanged` channel message rather than a shared atomic. The channel approach is more explicit and easier to trace; an atomic would be marginally faster.
- **Recovery from corrupted files.** Whether the decoder should skip a bad frame and continue or stop playback is a product decision with valid arguments on both sides.
- **Library index structure.** The Application Store is the single implementation of collection semantics: tracks, albums, and artists live in SQLite, and every view reads them through Session Projections over store queries or direct port calls. There is no second in-memory copy to keep consistent; if a future feature needs a different index shape, the decision is which store query (and projection) serves it.

## Error Handling Patterns

- **Per-owner error types**: `riff_persistence::errors::StoreError` (store/invalid-operation failures), `riff_library::app::errors::LibraryError` (metadata read/write, cover load, scan, I/O, track lookup), and `riff_playback::app::errors::PlaybackError` (decode, audio output) — all `thiserror`-derived and `Clone`, variants carrying a `String` message.
- **Infrastructure mapping**: each adapter in `riff-infra` converts its crate's error type into the appropriate owning port's error at the boundary, typically with `map_err`, so external error types never cross above the adapter crate.
- **Typed notices for playback failures**: the Playback Coordinator sends pre-formatted failure messages over the notice channel; the facade stamps them with playback source and error severity. No slice ever writes into another slice's session state.
- **UI display**: the frontend matches on results and shows brief, user-facing messages. Technical detail goes to the log, not the screen.
- **Logging**: the `tracing` crate provides structured logging, initialized in `riff-gui/src/main.rs` via `tracing_subscriber::fmt::init()` with env-filter support. Use ERROR for failures, WARN for recoverable issues (for example, a failed store write or a failed cover decode), INFO for notable state changes, and DEBUG for detailed tracing.
- **Lock poisoning**: `MutexExt::lock_or_recover` recovers from a poisoned mutex instead of panicking, so an isolated thread panic does not take down the whole application.

## See also

- [./threading-model.md](./threading-model.md) — threads, channels, shared state, and real-time constraints.
- [./data-flow.md](./data-flow.md) — step-by-step sequences for playback, scanning, and cover resolution.
- [./data-model.md](./data-model.md) — the entities, the two session structs, the store ports, and the port traits.
- [./dependencies.md](./dependencies.md) — every dependency, grouped by owning crate.
- [ADR 0009](../adr/0009-vertical-crate-split-of-the-backend.md) — the crate-split decision and its consequences.
