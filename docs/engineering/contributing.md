# Contributing to riff

Thank you for considering a contribution to riff, a lightweight, offline-first desktop music player written in Rust and egui. This guide explains how to orient yourself in the codebase, the rules your change must respect, and the checklist to run through before opening a pull request. For the environment setup and the exact commands, see [development-setup.md](./development-setup.md); for the reasoning behind the conventions, see [coding-standards.md](./coding-standards.md).

## Getting Oriented

Before writing code, read these in order:

1. **`AGENTS.md`** at the repository root — a fast, high-level tour of the architecture, threading model, platform-specific code, and the implementation gotchas. It is the quickest way to build a mental model of the project.
2. **`docs/`** — these documents. Start with [coding-standards.md](./coding-standards.md) for the layering rules and lint configuration, then [development-setup.md](./development-setup.md) for the build workflow.
3. **The source tree itself** — the workspace crates (`riff-persistence/`, `riff-library/`, `riff-playback/`, `riff-infra/`, `riff-backend/`, `riff-gui/`) and the root test crate (`tests/`). The crate layout is the architecture; where a piece of code lives is a deliberate decision enforced by the dependency chain.

A note on source-of-truth: where older design documents name modules that do not exist in the tree (for example `playback_engine.rs` or `app_window.rs`), trust the actual source files over the prose.

## The Crate Architecture

Every contribution must respect the crate split and its dependency chain (see [ADR 0009](../adr/0009-vertical-crate-split-of-the-backend.md) and [../technical/architecture.md](../technical/architecture.md)). This is the single most reviewed aspect of a change, so internalize it before you start:

- **Dependencies follow the chain.** `riff-persistence` depends on nothing; `riff-library` and `riff-playback` depend only on `riff-persistence` (and never on each other); `riff-infra` depends on the three and implements their ports; `riff-backend` depends on all four; `riff-gui` depends only on `riff-backend`. Never add an edge that bypasses the chain — the compiler enforces it.
- **No external (native) dependencies in the slices.** `riff-persistence`, `riff-library`, and `riff-playback` stay pure Rust. If your change needs `symphonia`, `cpal`, `lofty`, `image`, `rusqlite`, `walkdir`, or `notify`, it belongs in `riff-infra`, behind a port trait defined in the slice that consumes it.
- **Use trait abstraction across the slice/infra boundary.** Each slice defines the port traits it consumes; `riff-infra` implements them. Adding a new external dependency means introducing or extending a port in the consuming slice and implementing it in `riff-infra` — never calling the crate from a slice directly.
- **One composition root.** Only `riff-backend/src/composition.rs` constructs concrete adapters and wires them into ports. Do not add a DI framework, and do not construct infrastructure elsewhere.

If you are unsure which crate a new piece of code belongs in, ask what it is: types crossing the persistence boundary go in `riff-persistence`, collection logic and its ports in `riff-library`, playback logic and its ports in `riff-playback`, anything touching an external crate in `riff-infra`, facade surface and wiring in `riff-backend`, and anything rendering pixels in `riff-gui`.

## Working on a Change

A typical contribution workflow looks like this:

1. Create a branch from the latest main branch.
2. Make your change, keeping each new module in the crate whose membership criterion it satisfies.
3. Run `cargo fmt` so formatting matches the project default (there is no `rustfmt.toml`).
4. Run `cargo clippy` and resolve any new warnings. Pedantic lints are enabled as warnings; a clean run is the expected standard.
5. Run `cargo test`. riff's suites live in `riff-infra/tests/` and the workspace-root `tests/` crate, and new logic should come with new tests. See [testing-strategy.md](./testing-strategy.md) for what each suite covers and where to add yours.
6. For UI changes, run `cargo run -p riff-gui` and manually verify the behavior, since much of the UI is exercised by hand rather than by automated tests.

### Adding tests

Tests exist and are expected to grow with the code. Adapter and store tests live in `riff-infra/tests/` (`store_tests.rs`, `adapter_tests.rs`); cross-crate integration, UI, and golden-image tests live in the workspace-root `tests/` crate (`domain_tests.rs`, `app_tests.rs`, `infra_tests.rs`, `ui_tests.rs`, `integration_tests.rs`, `golden_tests.rs`). Add new tests to the suite that matches the code you changed, and add a new test whenever you add or fix logic. The `tempfile` dev-dependency is available for tests that need a scratch directory on disk.

## Pull Request Checklist

Run through this before requesting review. Each item maps to a rule above:

- [ ] **Crate placement correct.** Every new or moved module sits in the crate whose membership criterion matches its responsibility.
- [ ] **Dependency chain respected.** No new workspace edge bypasses the chain; the slices remain free of native crates and of each other.
- [ ] **Trait abstraction used.** New external dependencies cross the slice/infra boundary through a port trait, not a direct call.
- [ ] **Composition root respected.** Only `riff-backend/src/composition.rs` constructs and wires infrastructure.
- [ ] **No panics in the UI.** Recoverable errors surface as user-facing messages, never as panics.
- [ ] **Clippy clean.** `cargo clippy` produces no new warnings under the pedantic configuration.
- [ ] **Formatted.** `cargo fmt` has been run and produces no diff.
- [ ] **Tests added.** New domain logic (and, where practical, app logic) has accompanying tests, and `cargo test` passes.

## What to Avoid

A few anti-patterns are specifically watched for in review:

- Putting `symphonia`/`cpal` types into domain structs, or `egui` types into the application layer.
- Doing file I/O or long-running work on the UI thread; heavy work belongs on a background thread with results returned over a channel.
- Returning errors as bare `String` values instead of the typed, per-owner error enums (`StoreError`, `LibraryError`, `PlaybackError`).
- Acquiring a second lock while holding a session mutex; each session (`PlaybackSession`, `LibrarySession`) is behind its own non-nested lock, and a lock must never be held across a long operation.

When in doubt, keep your change small and focused on one layer, and surface any architectural uncertainty in the pull request description rather than resolving it silently.
