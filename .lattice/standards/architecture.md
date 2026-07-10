---
mode: override
---

# Architecture Principles for riff

> These are the architecture principles for riff, following a **Layered Desktop Application** architecture. This document is the sole reference for the `architecture` atom — there are no embedded defaults.

**Table of contents:**

1. [Layer Definitions](#1-layer-definitions)
2. [Dependency Rules](#2-dependency-rules)
3. [Boundary Rules](#3-boundary-rules)
4. [Per-Layer Rules](#4-per-layer-rules)
5. [Key Flows](#5-key-flows)
6. [Validation Checklist](#6-validation-checklist)
7. [Anti-Patterns](#7-anti-patterns)
8. [Ambiguity Signals](#8-ambiguity-signals)
9. [Error Handling Patterns](#9-error-handling-patterns)
10. [Threading Model](#10-threading-model)

---

## 1. Layer Definitions

| Layer | Responsibility | Typical Directory |
|-------|---------------|-------------------|
| **Domain** | Core business entities, value objects, and domain logic. No dependencies on external libraries. | `src/domain/` |
| **Application** | Use cases, orchestration, state machines, and service interfaces. Defines ports/contracts for infrastructure. | `src/app/` |
| **Infrastructure** | Implementations of application-defined interfaces using external crates (symphonia, cpal, lofty, image). | `src/infra/` |
| **Presentation** | UI components, event handling, and system integration (egui widgets, tray icon). | `src/ui/` |
| **Main** | Application composition root, dependency wiring, and event loop setup. | `src/main.rs` |

### Directory Mapping

```
src/
├── main.rs              # Composition root, event loop
├── domain/              # Core entities and logic
│   ├── track.rs         # Track entity
│   ├── album.rs         # Album aggregate
│   ├── artist.rs        # Artist value object
│   ├── playback.rs      # Playback state machine
│   └── queue.rs         # Queue logic
├── app/                 # Use cases and orchestration
│   ├── library_manager.rs  # Library scanning, metadata, search
│   ├── playback_engine.rs # Playback control, audio pipeline
│   ├── cover_resolver.rs  # Cover art resolution strategy
│   └── mod.rs           # App state, message types
├── infra/               # External crate implementations
│   ├── decoder.rs       # symphonia audio decoder
│   ├── audio_output.rs  # cpal audio output
│   ├── metadata_reader.rs # lofty metadata extraction
│   └── cover_loader.rs  # image crate cover loading
└── ui/                  # egui interface
    ├── app_window.rs    # Main window, layout
    ├── library_panel.rs # Library explorer panel
    ├── control_bar.rs   # Player control bar
    ├── cover_display.rs # Cover art display widget
    └── tray.rs          # System tray integration
```

---

## 2. Dependency Rules

```
Presentation (ui/)
    ↑ depends on
Application (app/)
    ↑ depends on
Domain (domain/)

Infrastructure (infra/)
    ↑ implements interfaces defined in
Application (app/) and Domain (domain/)
```

**Core rule:** Dependencies point inward. Inner layers know nothing about outer layers.

**Specific rules:**
- `domain/` has zero external dependencies (no `use symphonia::`, no `use cpal::`, no `use egui::`)
- `app/` depends only on `domain/` and `std`
- `app/` defines traits (ports) that `infra/` implements
- `infra/` depends on `app/`, `domain/`, and external crates
- `ui/` depends on `app/`, `domain/`, and `egui`/`eframe`
- `main.rs` is the only file that imports from all layers — it wires dependencies

**Data crossing boundaries:**
- Domain → Application: domain entities and value objects (owned data)
- Application → Infrastructure: trait method calls with owned data or immutable references
- Infrastructure → Application: callbacks via channels or trait methods (never direct struct references)
- Application → Presentation: shared state wrapped in `Arc<RwLock<AppState>>` or message passing via channels

---

## 3. Boundary Rules

**Layer communication:**
- **Synchronous calls** for deterministic, fast operations (queue manipulation, state queries)
- **Channels (mpsc)** for cross-thread communication (audio thread → UI thread, decoder thread → audio thread)
- **Shared state** for read-heavy, concurrent access (current playback position, volume level)

**Dependency injection:**
- Manual constructor injection. No DI container.
- The `main.rs` composition root constructs infrastructure implementations and passes them to application services.
- Example: `PlaybackEngine::new(Box::new(SymphoniaDecoder), Box::new(CpalAudioOutput))`

**Thread boundaries:**
- Audio output runs on a dedicated thread (cpal callback thread)
- Library scanning runs on a background thread pool
- Cover art loading runs on a background thread
- UI runs on the main thread (egui requirement)
- Thread communication uses `std::sync::mpsc` or `crossbeam_channel`

**Error propagation:**
- Domain errors: propagate via `Result<T, DomainError>`
- Application errors: wrap domain errors and add context
- Infrastructure errors: map external crate errors to application error types
- UI errors: display user-friendly messages, never panic

---

## 4. Per-Layer Rules

### Domain

**What belongs here:**
- Entities: `Track`, `Album`, `Artist`, `Playlist`
- Value objects: `Duration`, `SampleRate`, `Bitrate`
- Domain logic: playback state transitions, queue ordering rules, shuffle algorithms
- Domain events: `TrackChanged`, `PlaybackStateChanged` (as plain enums, not event bus)
- Error types: `DecodeError`, `PlaybackError` (core variants only)

**What does not belong here:**
- File system operations
- Audio decoding logic (symphonia)
- Audio output logic (cpal)
- UI rendering code
- Thread management
- Configuration file I/O

**Common violations:**
- Putting `std::fs` calls in domain code
- Referencing `symphonia` types in domain structs
- Including `egui::ColorImage` in domain entities

### Application

**What belongs here:**
- Use case orchestration: `LibraryManager.scan_folder()`, `PlaybackEngine.play()`, `PlaybackEngine.seek()`
- Service interfaces (traits): `AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`
- State management: `AppState` struct that holds current playback, queue, library
- Message types for cross-thread communication: `AudioCommand`, `UiUpdate`
- Application-level error types that wrap domain errors

**What does not belong here:**
- Direct use of `symphonia`, `cpal`, `lofty`, `image` crates
- egui widget code
- Platform-specific system calls
- File format parsing details

**Common violations:**
- Calling `symphonia::default::get_codecs()` directly instead of through the `AudioDecoder` trait
- Constructing `egui::Image` in application services
- Putting UI state (selected tab, scroll position) in application services

### Infrastructure

**What belongs here:**
- `symphonia` decoder implementation of `AudioDecoder` trait
- `cpal` audio output implementation of `AudioOutput` trait
- `lofty` metadata reader implementation of `MetadataReader` trait
- `image` crate cover loader implementation of `CoverLoader` trait
- File system scanning with `walkdir`
- Audio thread management

**What does not belong here:**
- Business logic (what order to play tracks, when to shuffle)
- UI decisions (when to update the progress bar)
- Domain entity construction (should receive raw data from application layer)

**Common violations:**
- Implementing queue logic in the decoder
- Making UI update calls from the audio output thread
- Creating domain entities directly from file paths without going through the application layer

### Presentation

**What belongs here:**
- egui widget implementations
- Event handling (button clicks, slider drags)
- Window management (eframe integration)
- Tray icon setup and menu handling
- Converting application state to UI display format
- Keyboard shortcut handling

**What does not belong here:**
- Audio decoding or output logic
- File scanning logic
- Domain business rules
- Direct external crate calls (except egui/eframe/tray-icon)

**Common violations:**
- Calling `symphonia` from a UI event handler
- Performing file I/O in a UI update loop
- Putting domain logic in button click handlers (e.g., "if shuffle is on, pick random track")

---

## 5. Key Flows

### Flow 1: Play a Track

```
User clicks "Play" in UI
    → ui/ sends PlaybackCommand::Play(track_id) via crossbeam channel to audio engine thread
    → Audio engine thread (run_audio_engine in main.rs) receives command
    → Looks up Track in AppState.library, resolves file path
    → Calls SymphoniaDecoder::open() to open file and get format info (sample rate, channels)
    → Calls CpalAudioOutput::initialize() + start() to create cpal stream
    → CpalAudioOutput::clear_buffer() discards any leftover samples from previous track
    → Decode loop begins (push model, on same thread):
        a. Backpressure check: if shared buffer ≥ 2s of audio, sleep + poll commands
        b. decoder.next_frames(4096) decodes a batch of PCM samples
        c. audio_output.write_samples(&scaled) pushes samples into Arc<Mutex<VecDeque<f32>>>
        d. Position update sent via PlaybackUpdate::PositionChanged channel
        e. Command poll: try_recv on cmd channel for Pause/Stop/Seek/Volume
    → cpal callback (separate OS audio thread): try_lock on buffer, pop samples, write to device
    → On EOF (decoder returns None): send TrackEnded, wait for buffer to drain, then stop stream
    → UI update thread receives PlaybackUpdate messages, mutates AppState
    → UI reads AppState each frame for progress bar, state, and errors
```

**Key design decisions:**
- **Push model**: The audio engine pushes decoded samples into a shared buffer; the cpal callback pulls from it. The engine does not respond to cpal callback requests directly — this avoids blocking the real-time audio thread.
- **Backpressure**: Before each decode, the engine checks if the shared buffer already holds ≥ 2 seconds of audio (sample_rate × channels × 2). If full, it sleeps 10ms and polls for commands (Pause/Stop/Seek). This prevents the decoder from racing ahead of playback and consuming unbounded memory.
- **Drain on EOF**: When the decoder reaches end-of-file, the engine waits for the remaining buffer to drain through the cpal callback before stopping the audio stream. A Stop command during drain will discard remaining samples.
- **Buffer lifetime**: The shared buffer is not cleared on stop — it drains naturally through the callback. `clear_buffer()` is called explicitly only at the start of a new track, on Stop command, or on Seek.

### Flow 2: Scan Library

```
User selects a folder in UI
    → ui/library_panel.rs sends LibraryCommand::Scan(path) to app/library_manager.rs
    → app/library_manager.rs spawns thread calling infra/walkdir_scanner.rs
    → infra/walkdir_scanner.rs finds all audio files and returns paths
    → app/library_manager.rs calls infra/metadata_reader.rs (LoftyMetadataReader) for each file
    → infra/metadata_reader.rs reads tags and returns raw metadata
    → app/library_manager.rs constructs domain/Track and domain/Album entities
    → app/library_manager.rs inserts into in-memory library index
    → app/library_manager.rs sends LibraryUpdate::TracksAdded to UI thread
    → ui/library_panel.rs updates the display
```

### Flow 3: Resolve Cover Art

```
Track becomes current (play or select)
    → app/cover_resolver.rs receives request for cover art
    → app/cover_resolver.rs checks priority: embedded > filesystem fallback
    → app/cover_resolver.rs calls infra/metadata_reader.rs to check for embedded art
    → If found, infra/cover_loader.rs decodes image and returns RGBA bytes
    → If not found, app/cover_resolver.rs scans directory for cover.jpg/cover.png (case-insensitive)
    → If found on filesystem, infra/cover_loader.rs loads and decodes
    → app/cover_resolver.rs sends CoverUpdate::Loaded(image_data) to UI thread
    → ui/cover_display.rs creates egui texture and displays
```

---

## 6. Validation Checklist

STOP after generating each component. Verify ALL of the following before proceeding:

1. **LAYER PLACEMENT**: Is each module in the correct directory (domain/, app/, infra/, ui/) based on its responsibilities? Does it only import from layers it is allowed to depend on?

2. **DEPENDENCY DIRECTION**: Do all `use` statements point inward? Does `domain/` have zero imports from `app/`, `infra/`, or `ui/`? Does `app/` have zero imports from `infra/` or `ui/`?

3. **TRAIT ABSTRACTION**: Does `app/` define traits for all external dependencies (audio decoding, audio output, metadata reading, cover loading)? Does `infra/` implement these traits rather than exposing crate-specific types to `app/`?

4. **DOMAIN PURITY**: Does `domain/` contain only business logic? Are there no file I/O, network, audio, or UI imports in `domain/`?

5. **THREAD SAFETY**: Is shared state between threads properly synchronized? Are channels used for thread communication? Is `Arc<RwLock<T>>` or `Arc<Mutex<T>>` used for shared mutable state?

6. **ERROR HANDLING**: Are errors properly categorized by layer? Does `infra/` map external errors to application error types? Does `ui/` display user-friendly error messages rather than technical details?

7. **NO UI IN LOGIC**: Is there no `egui` code in `app/` or `domain/`? Is there no audio decoding in `ui/`?

8. **COMPOSITION ROOT**: Does only `main.rs` construct and wire all dependencies? Are no other files calling `::new()` on infrastructure implementations directly?

---

## 7. Anti-Patterns

After verifying the checklist above, scan output for these anti-patterns. If found, fix before presenting.

- [ ] **Symphonia types in domain**: Domain structs contain `symphonia::` types → Extract raw data into domain types. Domain must not depend on external audio crates.

- [ ] **egui in application layer**: `app/` files import `egui::` or construct UI widgets → Move UI code to `ui/` layer. Application layer should only hold state and logic.

- [ ] **Direct file I/O in domain**: `domain/` files use `std::fs` or `std::path` for operations → Move file operations to `infra/` or `app/`. Domain should be pure logic.

- [ ] **Application constructing infrastructure**: `app/` files call `SymphoniaDecoder::new()` directly → Infrastructure must be injected via traits from `main.rs`.

- [ ] **UI thread blocking**: `ui/` event handlers perform long operations (file scanning, image decoding) synchronously → Move long operations to background threads and use channels.

- [ ] **God state struct**: A single `AppState` struct contains every piece of state in the application → Split into focused structs (PlaybackState, LibraryState, UiState) and compose them.

- [ ] **Callback spaghetti**: Audio thread callbacks directly call UI update methods → Use channels for all thread-to-thread communication. Never call UI methods from non-UI threads.

- [ ] **Stringly typed errors**: Errors are returned as plain `String` rather than typed errors → Define error enums per layer with `thiserror` for error chaining.

---

## 8. Ambiguity Signals

These checks often have multiple valid outcomes. When you encounter one, present options rather than silently choosing.

- **Where does cover art caching belong?** Could be in `app/` (as part of cover resolution logic) or `infra/` (as an image cache). The decision depends on whether caching is a business rule (how long to keep covers) or an implementation detail (memory management).

- **Shared state vs. message passing for playback position:** Playback position could be updated via a shared `AtomicU64` (faster, simpler) or via channel messages (more explicit, easier to trace). For the MVP, shared atomic state is preferred for the audio thread → UI position updates.

- **Error recovery from corrupted files:** Should the decoder skip the corrupted frame and continue (permissive) or stop playback entirely (strict)? This is a product decision with valid arguments on both sides.

- **Library index data structure:** Could be a simple `Vec<Track>` with linear search, or a more complex indexed structure (`HashMap<Artist, Vec<Album>>`). The choice depends on expected library size and search performance requirements.

---

## 9. Error Handling Patterns

**Layer-specific error types:**
- `domain::errors::DomainError` — core business error variants (InvalidTrack, EmptyQueue, etc.)
- `app::errors::AppError` — wraps `DomainError` and adds application context (DecodeFailed, ScanFailed, etc.)
- `infra::errors::InfraError` — wraps external crate errors (SymphoniaError, CpalError, LoftyError, etc.)

**Error conversion:**
- `infra` maps external errors to `InfraError` using `thiserror` #[from] attributes
- `app` maps `InfraError` to `AppError` with context messages
- `ui` matches on `AppError` to display appropriate user messages

**Error display in UI:**
- Technical errors (decode failure) → brief toast notification + detailed log entry
- User errors (file not found) → inline message in the relevant panel
- Fatal errors (audio device unavailable) → modal dialog with retry/quit options

**Logging:**
- Use `tracing` crate for structured logging
- Log levels: ERROR for failures, WARN for recoverable issues, INFO for state changes, DEBUG for detailed operation tracing
- Audio thread uses span-based tracing for timing analysis

---

## 10. Threading Model

**Threads:**
1. **Main thread** — egui event loop, UI rendering, user input handling
2. **Audio engine thread** — spawned in `main.rs`, runs `run_audio_engine()`: owns the decoder, drives the decode loop, writes samples into the shared buffer, handles all `PlaybackCommand` messages, and sends `PlaybackUpdate` messages back to the update thread
3. **Update processor thread** — spawned in `main.rs`, receives `PlaybackUpdate` from the engine, mutates `Arc<Mutex<AppState>>`, and auto-advances the queue on `TrackEnded`
4. **cpal callback thread** — owned by cpal/OS audio subsystem, real-time thread that pulls samples from the shared `Arc<Mutex<VecDeque<f32>>>` buffer via `try_lock` and writes to the hardware
5. **Library scanner thread** — spawned when scanning, uses `walkdir`, sends `LibraryUpdate` messages
6. **Cover loader thread** — spawned per cover load request, terminates after image is decoded

**Communication:**
- Audio engine thread → Update processor thread: `crossbeam_channel<PlaybackUpdate>` (state changes, position updates, track ended, errors)
- UI thread → Audio engine thread: `crossbeam_channel<PlaybackCommand>` (play, pause, resume, stop, seek, volume, next, previous)
- UI thread → Library scanner thread: `crossbeam_channel<LibraryCommand>` (scan directory, cancel scan)
- Library scanner thread → UI thread: `crossbeam_channel<LibraryUpdate>` (scan progress, scan complete, scan error)
- Cover thread → UI thread: `crossbeam_channel<CoverUpdate>` (image loaded, load failed)

**Shared state:**
- `Arc<Mutex<AppState>>` — read by UI every frame, written by the update processor thread. Contains library, queue, playback state, current position, theme, and UI state.
- `Arc<Mutex<VecDeque<f32>>>` — shared audio buffer between the engine thread (producer) and the cpal callback thread (consumer). The engine pushes decoded samples; the callback pops them via `try_lock`. No hard upper bound; backpressure in the decode loop prevents unbounded growth.

**Constraints:**
- cpal callback must never block — uses `try_lock()` on the shared buffer; on lock failure, outputs silence
- Audio engine thread must not race ahead of playback — backpressure enforced by checking buffer fill level before each decode
- UI thread must never block (all heavy work on background threads)
- Background threads must not hold locks during long operations
- `AppState` is behind a single `Arc<Mutex<>>` — no nested locking, plan lock ordering carefully

---

*Generated for riff on 2025-07-09. Style: Layered Desktop Application.*
*Produced by the architecture-refiner skill.*
