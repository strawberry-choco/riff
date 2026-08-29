# Coding Standards

This document describes the conventions that keep riff's codebase consistent and maintainable. It covers the layered architecture rules, the lint and formatting configuration, the error-handling pattern, and the implementation gotchas that trip up contributors. Read this before writing any non-trivial change, and pair it with [development-setup.md](./development-setup.md) for the commands that enforce these standards.

riff is organized as a Cargo workspace of capability crates. The single most important rule is that dependencies follow the crate chain: `riff-persistence` ← `riff-library`/`riff-playback` ← `riff-infra` ← `riff-backend` ← `riff-gui`, with the two capability slices as siblings and no edge between them. The compiler enforces this chain, so a misplaced dependency fails the build rather than rotting silently.

## Architecture Layering

The workspace is divided into five backend crates plus the frontend:

| Crate | Role | Responsibility |
|---|---|---|
| `riff-persistence` | Persistence contract | Stored entities and the Application Store ports and DTOs. `std` only. |
| `riff-library` | Collection capability | Scanning, projections, playlists, covers; its own ports and error type. |
| `riff-playback` | Playback capability | Queue, engine, gapless, coordinator, Transport; its own ports and error type. |
| `riff-infra` | Adapters | Implements the slices' ports using external crates; owns every native dependency. |
| `riff-backend` | Application API | Backend Facade, facade-adjacent services, `LibrarySession`, Composition Root. |
| `riff-gui` | Frontend | egui widgets, tray icon, native file dialogs, the `riff` binary. |

### Dependency direction

Dependencies always follow the chain, and lower crates know nothing about higher ones:

- `riff-persistence` has zero dependencies — no `serde`, no I/O, nothing but `std`.
- `riff-library` and `riff-playback` depend only on `riff-persistence` plus pure-Rust utilities. They never import each other, never name a concrete adapter, and never touch `symphonia`, `cpal`, `lofty`, `image`, `rusqlite`, or `egui`.
- Each slice defines the port traits for its external dependencies; `riff-infra` implements them and depends on the slices — never the reverse.
- `riff-backend` is the only crate that names both ports and concrete adapters; that happens exclusively in `composition.rs`.
- `riff-gui` depends only on `riff-backend` (plus UI/platform crates).

### Trait abstraction and dependency injection

The slices never touch `symphonia`, `cpal`, `lofty`, `image`, or `rusqlite` directly. Instead they declare port traits, and `riff-infra` provides the concrete implementations. Construction and wiring happen exclusively in `riff-backend/src/composition.rs` via manual constructor injection; there is no DI framework. No file other than the Composition Root should call `::new()` on an adapter and hand it across a boundary.

### Validation checklist

Before submitting a change, confirm:

1. Each module sits in the crate whose membership criterion matches its responsibility.
2. The workspace dependency chain has no new bypasses; the slices have no native imports.
3. External dependencies cross the slice/infra boundary only through port traits.
4. The persistence contract stays dependency-free, and the slices stay pure Rust.
5. There is no `egui` code outside `riff-gui`, and no audio decoding in the frontend.
6. Only `riff-backend/src/composition.rs` constructs and wires infrastructure.

For the full treatment, including key flows and the threading model, see [../technical/architecture.md](../technical/architecture.md).

## Linting with Clippy

Lint levels are configured in `Cargo.toml` under `[lints.clippy]`, and tool-level options live in `clippy.toml`. The configuration enables the pedantic group as warnings, explicitly allows the nursery group, and carves out a small set of additional allowances:

```toml
[lints.clippy]
pedantic = { level = "warn", priority = -1 }
nursery = { level = "allow", priority = -1 }
needless_pass_by_value = "allow"
module_name_repetitions = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
must_use_candidate = "allow"
```

The five individually allowed lints are worth understanding, because they reflect deliberate project choices:

- `needless_pass_by_value` — passing small values by value is accepted where it reads more clearly.
- `module_name_repetitions` — type names may repeat their module name (for example `app::state::AppState`).
- `missing_errors_doc` — `Result`-returning functions are not required to document every error case.
- `missing_panics_doc` — functions that may panic are not required to document it.
- `must_use_candidate` — the project does not annotate `#[must_use]` aggressively.

`clippy.toml` sets the tool MSRV and a couple of behavioral options:

```toml
msrv = "1.95"
avoid-breaking-exported-api = false
upper-case-acronyms-aggressive = true
```

Run `cargo clippy` before committing. Pedantic lints are warnings, so the build will not fail on them, but a clean clippy run is the expected standard; see [contributing.md](./contributing.md) for the pull-request expectations.

## Formatting with rustfmt

There is no `rustfmt.toml` or `.rustfmt.toml` in the repository, so riff uses rustfmt's default style. Run `cargo fmt` before committing so that formatting is consistent and does not appear as noise in diffs. There are no pre-commit hooks to do this for you.

## Error Handling

Errors are typed per owner using the `thiserror` crate, and each owner's type is the one its ports answer with:

- `riff_persistence::errors::StoreError` — the persistence boundary: store failures and invalid operations.
- `riff_library::app::errors::LibraryError` — the collection capability: metadata read/write, cover load, scan, and I/O failures.
- `riff_playback::app::errors::PlaybackError` — the playback capability: decode and audio-output failures.

The conversion flow runs outward: `riff-infra` maps external crate errors into the owning port's error at the adapter boundary, the slices and services match on those typed errors, and the UI surfaces a user-appropriate message. Playback failures reach the session as typed notices through the facade's notice channel (source + severity) rather than as a cross-slice state write. Errors are never returned as bare `String` values across a port, and the UI never panics on a recoverable error; it surfaces a message instead. Structured logging uses the `tracing` crate, with levels chosen by severity (ERROR for failures, WARN for recoverable issues, INFO for state changes, DEBUG for detailed tracing).

## Key Gotchas

These implementation details are easy to get wrong and worth internalizing before editing the relevant code.

- **Session state is two structs, each behind its own `Arc<Mutex<>>`.** `PlaybackSession` (in `riff-playback`) holds the queue, playback state, position, volume, and mute; `LibrarySession` (in `riff-backend`) holds selection, views, search, library paths and statuses, scan status, and watch states. There is no nested locking, and that must stay true: never acquire a second lock while holding a session lock.
- **Cover art LRU is capped at 50 textures.** Cached textures live in a `cover_textures` map in `riff-gui` with manual LRU eviction tracked in a `cover_lru_keys` vector; the decoded-cover cache in `riff-library`'s `CoverService` is bounded by `COVER_CACHE_CAP` (50). Do not grow either unboundedly.
- **Audio buffer management.** `CpalAudioOutput` (`riff-infra`) uses a lock-free SPSC ring buffer (`ringbuf`) shared between the decode loop (producer) and the cpal callback (consumer). The cpal callback must never block; it outputs silence when the buffer cannot keep up.
- **Oversize decoded packets.** `SymphoniaDecoder` buffers decoded packets that are too large to emit in one call in an internal `pending_samples` buffer. Preserve this behavior when touching the decoder.
- **Sample-rate fallback.** If a track's sample rate is unsupported by the device, output falls back to the device default rate. This is common on Windows WASAPI shared mode at 48 kHz; the effective rate is reported through the `AudioOutput::effective_sample_rate` port method.
- **`TrackId` identity.** A track's identity is its full file path as a string (derived from `PathBuf::to_string_lossy()`). Renaming or moving a file therefore produces a new track identity.
