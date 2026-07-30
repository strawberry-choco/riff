# Coding Standards

This document describes the conventions that keep riff's codebase consistent and maintainable. It covers the layered architecture rules, the lint and formatting configuration, the error-handling pattern, and the implementation gotchas that trip up contributors. Read this before writing any non-trivial change, and pair it with [development-setup.md](./development-setup.md) for the commands that enforce these standards.

riff is organized as a layered desktop application. The single most important rule is that dependencies point inward: outer layers may depend on inner layers, never the reverse. The compiler does not enforce this (it is one Cargo crate), so it is upheld by convention and code review.

## Architecture Layering

The source tree is divided into four layers plus a composition root:

| Layer | Directory | Responsibility |
|---|---|---|
| Domain | `src/domain/` | Pure business entities and logic. Zero external crate imports. |
| Application | `src/app/` | Use cases, shared state, and trait interfaces (ports). |
| Infrastructure | `src/infra/` | Implementations of the app-defined traits using external crates. |
| Presentation | `src/ui/` | egui widgets, tray icon, native file dialogs. |
| Composition root | `src/main.rs` | The only file that wires all layers together. |

### Dependency direction

Dependencies always point inward, and inner layers know nothing about outer layers:

- `domain/` has zero external dependencies and imports nothing from `app/`, `infra/`, or `ui/`. No `symphonia`, no `cpal`, no `egui`, and no `std::fs` file I/O.
- `app/` depends only on `domain/` and `std`. It defines the port traits (`AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`) that `infra/` implements.
- `infra/` depends on `app/`, `domain/`, and the external crates. It implements the port traits rather than exposing crate-specific types upward.
- `ui/` depends on `app/`, `domain/`, and `egui`/`eframe`.
- `main.rs` is the only file that imports from all four layers and constructs concrete infrastructure implementations.

### Trait abstraction and dependency injection

The application layer never touches `symphonia`, `cpal`, `lofty`, or `image` directly. Instead it declares traits, and the infrastructure layer provides the concrete implementations. Construction and wiring happen exclusively in `main.rs` via manual constructor injection; there is no DI framework. No file other than `main.rs` should call `::new()` on an infrastructure type and hand it around.

### Validation checklist

Before submitting a change, confirm:

1. Each module sits in the correct directory for its responsibility.
2. All `use` statements point inward; `domain/` and `app/` have no forbidden imports.
3. External dependencies cross the app/infra boundary only through traits.
4. `domain/` contains only business logic (no I/O, audio, or UI).
5. There is no `egui` code in `app/` or `domain/`, and no audio decoding in `ui/`.
6. Only `main.rs` constructs and wires infrastructure.

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
msrv = "1.92"
avoid-breaking-exported-api = false
upper-case-acronyms-aggressive = true
```

Run `cargo clippy` before committing. Pedantic lints are warnings, so the build will not fail on them, but a clean clippy run is the expected standard; see [contributing.md](./contributing.md) for the pull-request expectations.

## Formatting with rustfmt

There is no `rustfmt.toml` or `.rustfmt.toml` in the repository, so riff uses rustfmt's default style. Run `cargo fmt` before committing so that formatting is consistent and does not appear as noise in diffs. There are no pre-commit hooks to do this for you.

## Error Handling

Errors are typed per layer using the `thiserror` crate, and each layer maps the layer below it into its own error type. The layer-specific types are:

- `domain` errors — core business error variants (for example an invalid track or an empty queue).
- `app::errors::AppError` — wraps domain errors and adds application context such as decode-failed or scan-failed.
- `infra` errors — wrap external crate errors (symphonia, cpal, lofty) using `thiserror` `#[from]` conversions.

The conversion flow runs outward: infrastructure maps external crate errors into its own error type, the application layer maps those into `AppError` with added context, and the UI matches on `AppError` to show a user-appropriate message. Errors are never returned as bare `String` values, and the UI never panics on a recoverable error; it surfaces a message instead. Structured logging uses the `tracing` crate, with levels chosen by severity (ERROR for failures, WARN for recoverable issues, INFO for state changes, DEBUG for detailed tracing).

## Key Gotchas

These implementation details are easy to get wrong and worth internalizing before editing the relevant code.

- **`AppState` is one large struct behind a single `Arc<Mutex<>>`.** It holds the library, queue, playback state, theme, and UI state together. There is no nested locking today, and that must stay true; plan lock ordering carefully and never acquire a second lock while holding the `AppState` lock.
- **Cover art LRU is capped at 50 textures.** Cached textures live in a `cover_textures` map with manual LRU eviction tracked in a `cover_lru_keys` vector. Do not grow this unboundedly.
- **Audio buffer management.** `CpalAudioOutput` uses a `VecDeque<f32>` ring buffer shared between the decode loop (producer) and the cpal callback (consumer). The cpal callback must never block; it uses `try_lock` and outputs silence if the lock is unavailable.
- **Oversize decoded packets.** `SymphoniaDecoder` buffers decoded packets that are too large to emit in one call in an internal `pending_samples` buffer. Preserve this behavior when touching the decoder.
- **Sample-rate fallback.** If a track's sample rate is unsupported by the device, output falls back to the device default rate. This is common on Windows WASAPI shared mode at 48 kHz.
- **`TrackId` identity.** A track's identity is its full file path as a string (derived from `PathBuf::to_string_lossy()`). Renaming or moving a file therefore produces a new track identity.
