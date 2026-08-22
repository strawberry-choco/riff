# Architecture

riff is a lightweight, offline-first desktop music player written in Rust on top of the egui immediate-mode UI framework. It ships as a single Cargo crate with no code-generation step and no plugin system; persistence runs through ordered, checksummed SQLite migrations (see [./persistence.md](./persistence.md)). The architecture is a conventional **layered desktop application**: pure business logic sits at the center, infrastructure adapters that wrap external crates sit at the edge, and a single composition root wires everything together at startup.

This document is the reference for how the crate is organized, how the layers are allowed to depend on one another, and what belongs where. For the runtime view of the system see [./threading-model.md](./threading-model.md) and [./data-flow.md](./data-flow.md); for the concrete types that flow between layers see [./data-model.md](./data-model.md).

## Overview

The crate is divided into four layers plus a composition root:

| Layer | Directory | Responsibility |
|-------|-----------|----------------|
| Domain | `src/domain/` | Core entities, value objects, and pure business logic. Zero external crate imports. |
| Application | `src/app/` | Use cases, orchestration, shared state, and the port traits that define what infrastructure must provide. |
| Infrastructure | `src/infra/` | Implementations of the application's port traits using external crates (symphonia, cpal, lofty, image, walkdir, notify). |
| Presentation | `src/ui/` | egui widgets, the main window, settings, fonts, and system tray integration. |
| Composition root | `src/main.rs` | The only file that imports from all four layers. It constructs the infrastructure implementations, creates the channels, spawns the threads, and starts the egui event loop. |

Dependencies point inward. The domain knows nothing about any other layer; the application layer knows only the domain and the standard library; infrastructure and presentation are outer layers that depend inward and are never referenced by the layers they depend on. `main.rs` is the single place where the concrete infrastructure types are named and handed to the application's trait-based ports.

## Layer Definitions

The actual source tree is shown below. Every filename here is real; older design notes that reference `playback_engine.rs`, `app_window.rs`, `library_panel.rs`, `control_bar.rs`, or `cover_display.rs` are out of date and do not correspond to files in the repository.

```
src/
├── main.rs                  # Composition root: wires channels, spawns threads, drives egui
├── domain/                  # Pure business logic. No external crate imports.
│   ├── mod.rs               # Re-exports Track, TrackId, PlaybackQueue, PlaybackState, ...
│   ├── track.rs             # Track, TrackId, TrackMetadata, Album, Artist, CoverSource
│   ├── playback.rs          # PlaybackState, RepeatMode, PlaybackPosition, PlaybackCommand, PlaybackUpdate
│   └── queue.rs             # PlaybackQueue (ordering, shuffle, repeat)
├── app/                     # Use cases, state, and port traits
│   ├── mod.rs               # Module root + MutexExt (poison-recovering lock helper)
│   ├── state.rs             # AppState, ViewMode, LibraryStatus, BrowseMode, WatchState
│   ├── traits.rs            # Port traits: AudioDecoder, AudioOutput, MetadataReader, CoverLoader
│   ├── store.rs             # Application Store ports: SettingsStore, PlaylistStore, LibraryQueryStore, ...
│   ├── projection.rs        # Session Projections over store query results (generation-invalidated)
│   ├── commands.rs          # LibraryCommand, LibraryUpdate message enums
│   ├── errors.rs            # AppError (thiserror)
│   ├── library_manager.rs   # LibraryManager: transitional in-memory mirror (never persisted)
│   ├── playlist_manager.rs  # Playlist entry validity helpers
│   ├── gapless.rs           # Gapless playback eligibility and frame math
│   ├── cover_resolver.rs    # CoverResolver: embedded > filesystem cover priority
│   └── watcher_manager.rs   # WatcherManager: folder-watch lifecycle, debounce, rescan trigger
├── infra/                   # Trait implementations using external crates
│   ├── mod.rs               # Re-exports the concrete adapters
│   ├── store.rs             # SqliteStore + mutex-guarded port views (rusqlite)
│   ├── decoder.rs           # SymphoniaDecoder (impl AudioDecoder)
│   ├── audio_output.rs      # CpalAudioOutput (impl AudioOutput)
│   ├── metadata_reader.rs   # LoftyMetadataReader (impl MetadataReader)
│   ├── metadata_writer.rs   # LoftyMetadataWriter (impl MetadataWriter)
│   ├── cover_loader.rs      # ImageCoverLoader (impl CoverLoader)
│   ├── scanner.rs           # AudioFileScanner (walkdir)
│   └── watcher.rs           # FilesystemWatcher (notify)
└── ui/                      # egui interface
    ├── mod.rs               # Re-exports RiffApp
    ├── app.rs               # Main window (RiffApp) — layout, views, cover LRU, cover thread
    ├── tray.rs              # System tray integration (non-Linux)
    ├── settings.rs          # Settings view: library path management, preferences, Clear Library
    └── fonts.rs             # Font configuration for the egui context
```

## Dependency Rules

```
Presentation (ui/)
    | depends on
    v
Application (app/)  <---- implements ports ----  Infrastructure (infra/)
    | depends on                                       | depends on
    v                                                  v
Domain (domain/)  <------------------------------------+
```

The core rule is that dependencies point inward. Inner layers never know about outer layers.

Specific rules:

- `domain/` has zero external crate dependencies. No `symphonia`, no `cpal`, no `egui`, no `lofty`. It may use `std` and it may derive `serde` traits (serde is treated as a pure data-serialization concern), but it performs no I/O.
- `app/` depends only on `domain/` and `std` (plus `tracing` for logging and `crossbeam-channel` for message types). It defines the port traits — including the Application Store ports in `store.rs` — that `infra/` implements.
- `app/` defines traits — `AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader` — that `infra/` implements. The application layer never names a concrete adapter type.
- `infra/` depends on `app/` (for the traits and `AppError`), `domain/` (for the entity types), and the external crates it adapts.
- `ui/` depends on `app/`, `domain/`, and `egui`/`eframe` (plus the platform-conditional tray crates). It reads application state and sends commands; it does not decode audio or scan files itself.
- `main.rs` is the only file that imports from all layers. It is the only place that constructs `SymphoniaDecoder`, `CpalAudioOutput`, `LoftyMetadataReader`, `ImageCoverLoader`, `AudioFileScanner`, and `FilesystemWatcher` by name.

### Data crossing boundaries

- **Domain to Application**: owned domain entities and value objects (`Track`, `TrackId`, `PlaybackQueue`).
- **Application to Infrastructure**: trait method calls passing owned data or immutable references (for example `MetadataReader::read_all(&self, path: &PathBuf)`).
- **Infrastructure to Application**: results returned through trait methods, or asynchronous results delivered via channels. Infrastructure never holds a reference to an application struct.
- **Application to Presentation**: a shared `Arc<Mutex<AppState>>` that the UI reads each frame, plus channel messages (`LibraryUpdate`) polled from the UI loop.

## Boundary Rules

### Synchronous calls, channels, and shared state

Three communication mechanisms are used, each for a different kind of interaction:

- **Synchronous calls** for fast, deterministic operations: queue manipulation, library queries, state reads.
- **Channels** (`crossbeam_channel::unbounded`) for all cross-thread communication: UI to audio engine, audio engine to update processor, UI to scanner, scanner to UI, and the cover request/response pair.
- **Shared state** for read-heavy concurrent access: `Arc<Mutex<AppState>>` (read by the UI every frame, written by the update processor) and `Arc<Mutex<VecDeque<f32>>>` (the audio ring buffer shared between the decode loop and the cpal callback).

### Manual dependency injection

There is no DI container. `main.rs` performs manual constructor injection: it builds the concrete infrastructure adapters and passes them — boxed as trait objects where appropriate — into the application services that need them. The `CoverResolver`, for example, is constructed in the UI from a `Box<dyn MetadataReader>` and a `Box<dyn CoverLoader>`; the resolver never knows those are backed by lofty and the `image` crate.

### Thread boundaries

- The egui event loop runs on the main thread (an egui/eframe requirement).
- Audio decoding runs on a dedicated audio engine thread.
- The cpal callback runs on an OS-owned real-time audio thread.
- Library scanning and filesystem-watch event processing run on background threads.
- Cover decoding runs on a dedicated worker thread.

All thread-to-thread communication uses `crossbeam_channel`. See [./threading-model.md](./threading-model.md) for the full thread inventory and constraints.

### Error propagation

- The application layer defines a single `AppError` enum (via `thiserror`) with variants such as `Decode`, `AudioOutput`, `MetadataRead`, `CoverLoad`, `LibraryScan`, `Io`, `TrackNotFound`, and `InvalidOperation`.
- Infrastructure maps external crate errors into `AppError` at the adapter boundary, so crate-specific error types never leak into `app/` or `ui/`.
- The UI displays user-friendly messages and never panics on a recoverable error.
- Mutex access uses the `MutexExt::lock_or_recover` helper, which recovers a poisoned lock instead of panicking, so a panic on one thread does not cascade into every other thread that shares `AppState`.

## Per-Layer Rules

### Domain (`src/domain/`)

**Belongs here**: entities (`Track`, `Album`, `Artist`), identifiers (`TrackId`), value objects (`TrackMetadata`, `PlaybackPosition`), the playback state machine (`PlaybackState`, `RepeatMode`), queue ordering/shuffle/repeat logic (`PlaybackQueue`), and the command/update message enums (`PlaybackCommand`, `PlaybackUpdate`, `CoverSource`).

**Does not belong here**: file I/O, audio decoding, audio output, UI rendering, thread management, or configuration file access.

**Common violations**: calling `std::fs` from domain code; referencing `symphonia` or `cpal` types in a domain struct; embedding an `egui::ColorImage` in a domain entity.

### Application (`src/app/`)

**Belongs here**: use-case orchestration (`LibraryManager::scan_and_add_tracks`, search), the port traits (`AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`, and the Application Store ports in `store.rs`), Session Projections over store query results, shared state (`AppState` and its supporting enums), cross-thread message types (`LibraryCommand`, `LibraryUpdate`), cover resolution policy (`CoverResolver`), watcher orchestration (`WatcherManager`), and the application error type (`AppError`).

**Does not belong here**: direct use of `symphonia`, `cpal`, `lofty`, or `image`; egui widget code; platform-specific system calls.

**Common violations**: calling `symphonia` directly instead of through the `AudioDecoder` trait; constructing an `egui::Image` in an application service; placing purely visual state (scroll position, focus) in `AppState` rather than in the UI struct.

### Infrastructure (`src/infra/`)

**Belongs here**: the `symphonia` decoder (`SymphoniaDecoder`), the `cpal` output (`CpalAudioOutput`), the `lofty` metadata reader and writer, the `image`-based cover loader (`ImageCoverLoader`), filesystem scanning with `walkdir` (`AudioFileScanner`), filesystem watching with `notify` (`FilesystemWatcher`), and the Application Store implementation over `rusqlite` (`SqliteStore` in `store.rs`, including its migrations and corruption recovery).

**Does not belong here**: business logic (play order, shuffle decisions), UI decisions, or domain entity construction policy.

**Common violations**: implementing queue logic inside the decoder; calling UI update methods from the audio output thread; building domain entities directly from file paths without going through the application layer.

### Presentation (`src/ui/`)

**Belongs here**: egui widget code, the main window (`RiffApp` in `app.rs`), the settings view (`settings.rs`), font configuration (`fonts.rs`), system tray setup (`tray.rs`), event handling, and conversion of application state into display form. The cover texture LRU cache lives here because it is an egui-specific concern (`egui::TextureHandle`).

**Does not belong here**: audio decoding or output, file scanning, domain business rules, or direct calls to external crates other than egui/eframe and the platform-conditional tray/dialog crates.

**Common violations**: calling `symphonia` from a UI event handler; doing file I/O inside the frame update loop; putting domain logic (for example, shuffle selection) inside a button click handler.

## Validation Checklist

Use this checklist when adding or reviewing a component:

- [ ] **Layer placement**: each module is in the correct directory for its responsibility and imports only from layers it may depend on.
- [ ] **Dependency direction**: all `use` statements point inward. `domain/` imports nothing from `app/`, `infra/`, or `ui/`. `app/` imports nothing from `infra/` or `ui/`.
- [ ] **Trait abstraction**: `app/` defines traits for every external dependency (decoding, output, metadata, cover loading), and `infra/` implements them rather than exposing crate-specific types upward.
- [ ] **Domain purity**: `domain/` contains only business logic, with no file, network, audio, or UI imports.
- [ ] **Thread safety**: shared state between threads is synchronized with `Arc<Mutex<_>>`, and cross-thread communication uses channels.
- [ ] **Error handling**: errors are categorized by layer; `infra/` maps external errors into `AppError`; `ui/` shows user-friendly messages.
- [ ] **No UI in logic**: there is no `egui` code in `app/` or `domain/`, and no audio decoding in `ui/`.
- [ ] **Composition root**: only `main.rs` constructs and wires infrastructure; no other file calls `::new()` on a concrete adapter and injects it across layers.

## Anti-Patterns

- **Symphonia types in domain**: a domain struct contains `symphonia::` types. Extract the raw data into domain types; the domain must not depend on external audio crates.
- **egui in the application layer**: an `app/` file imports `egui::` or builds a widget. Move it to `ui/`.
- **Direct file I/O in domain**: a `domain/` file uses `std::fs`. Move the operation to `app/` or `infra/`.
- **Application constructing infrastructure**: an `app/` file calls `SymphoniaDecoder::new()` directly. Inject the adapter from `main.rs` through a trait.
- **UI thread blocking**: a `ui/` handler performs scanning or image decoding synchronously. Move it to a background thread and use a channel.
- **Callback spaghetti**: an audio callback calls UI methods directly. Use channels for all thread-to-thread communication; never call UI code from a non-UI thread.
- **Stringly typed errors**: errors passed as bare `String`. Use the typed `AppError` enum so callers can match on variants.

Note that a single large `AppState` behind one `Arc<Mutex<_>>` is a deliberate trade-off in this codebase, not an accident. It simplifies lock ordering (there is exactly one application-state lock to reason about) at the cost of a coarse-grained state struct. Keep all `AppState` access short and never hold the lock across a long operation.

## Ambiguity Signals

These are decisions with more than one defensible answer. Surface them explicitly rather than choosing silently:

- **Where cover caching belongs.** The in-memory cover texture LRU currently lives in `ui/app.rs` because it stores egui-specific `TextureHandle`s. If caching policy (how long to keep covers) ever becomes a business rule, it may warrant an application-layer home.
- **Shared state versus message passing for playback position.** Position currently flows as a `PlaybackUpdate::PositionChanged` channel message rather than a shared atomic. The channel approach is more explicit and easier to trace; an atomic would be marginally faster.
- **Recovery from corrupted files.** Whether the decoder should skip a bad frame and continue or stop playback is a product decision with valid arguments on both sides.
- **Library index structure.** The transitional in-memory mirror uses `HashMap`s keyed by `TrackId`, artist name, and album key for O(1) lookup; the authoritative collection lives in the Application Store, and views read through Session Projections over store queries.

## Error Handling Patterns

- **Application errors**: `app::errors::AppError` is the single typed error enum, defined with `thiserror` and `Clone`. Variants carry a `String` message and cover decoding, audio output, metadata reading, cover loading, scanning, I/O, missing tracks, and invalid operations.
- **Infrastructure mapping**: each adapter converts its crate's error type into the appropriate `AppError` variant at the boundary, typically with `map_err`, so external error types never cross into `app/` or `ui/`.
- **UI display**: the UI matches on results and shows brief, user-facing messages. Technical detail goes to the log, not the screen.
- **Logging**: the `tracing` crate provides structured logging, initialized in `main.rs` via `tracing_subscriber::fmt::init()` with env-filter support. Use ERROR for failures, WARN for recoverable issues (for example, a failed store write or a failed cover decode), INFO for notable state changes, and DEBUG for detailed tracing.
- **Lock poisoning**: `MutexExt::lock_or_recover` recovers from a poisoned mutex instead of panicking, so an isolated thread panic does not take down the whole application.

## See also

- [./threading-model.md](./threading-model.md) — threads, channels, shared state, and real-time constraints.
- [./data-flow.md](./data-flow.md) — step-by-step sequences for playback, scanning, and cover resolution.
- [./data-model.md](./data-model.md) — the domain entities and the `AppState` struct.
