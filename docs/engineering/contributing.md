# Contributing to riff

Thank you for considering a contribution to riff, a lightweight, offline-first desktop music player written in Rust and egui. This guide explains how to orient yourself in the codebase, the rules your change must respect, and the checklist to run through before opening a pull request. For the environment setup and the exact commands, see [development-setup.md](./development-setup.md); for the reasoning behind the conventions, see [coding-standards.md](./coding-standards.md).

## Getting Oriented

Before writing code, read these in order:

1. **`AGENTS.md`** at the repository root — a fast, high-level tour of the architecture, threading model, platform-specific code, and the implementation gotchas. It is the quickest way to build a mental model of the project.
2. **`docs/`** — these documents. Start with [coding-standards.md](./coding-standards.md) for the layering rules and lint configuration, then [development-setup.md](./development-setup.md) for the build workflow.
3. **The source tree itself** — `src/domain/`, `src/app/`, `src/infra/`, `src/ui/`, and `src/main.rs`. The directory layout is the architecture; where a piece of code lives is a deliberate decision.

A note on source-of-truth: where older design documents name modules that do not exist in the tree (for example `playback_engine.rs` or `app_window.rs`), trust the actual source files over the prose.

## The Four-Layer Architecture

Every contribution must respect the layered architecture. This is the single most reviewed aspect of a change, so internalize it before you start:

- **Dependencies point inward.** `ui/` may depend on `app/` and `domain/`; `app/` may depend on `domain/`; `domain/` depends on nothing outside `std`. Never add an import that points outward.
- **No external imports in `domain/`.** The domain layer is pure business logic. It must not import `symphonia`, `cpal`, `lofty`, `image`, `egui`, or perform file I/O. If your change needs an external crate, it does not belong in `domain/`.
- **Use trait abstraction across the app/infra boundary.** The application layer defines port traits (`AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`); the infrastructure layer implements them. If you add a new external dependency, introduce or extend a trait in `app/` and implement it in `infra/` rather than calling the crate from `app/` directly.
- **Manual dependency injection only in `main.rs`.** `main.rs` is the composition root and the only file that constructs concrete infrastructure implementations and wires them together. Do not add a DI framework, and do not construct infrastructure elsewhere.

If you are unsure which layer a new piece of code belongs in, ask yourself what it depends on: pure logic goes in `domain/`, orchestration and state in `app/`, anything touching an external crate in `infra/`, and anything rendering pixels in `ui/`.

## Working on a Change

A typical contribution workflow looks like this:

1. Create a branch from the latest main branch.
2. Make your change, keeping each new module in the correct layer.
3. Run `cargo fmt` so formatting matches the project default (there is no `rustfmt.toml`).
4. Run `cargo clippy` and resolve any new warnings. Pedantic lints are enabled as warnings; a clean run is the expected standard.
5. Run `cargo test`. riff has an existing test suite under `tests/`, and new logic should come with new tests. See [testing-strategy.md](./testing-strategy.md) for what each suite covers and where to add yours.
6. For UI changes, run `cargo run` and manually verify the behavior, since much of the UI is exercised by hand rather than by automated tests.

### Adding tests

Tests exist and are expected to grow with the code. The `tests/` directory contains per-layer suites (`domain_tests.rs`, `app_tests.rs`, `infra_tests.rs`, `ui_tests.rs`, and `integration_tests.rs`). Add new tests to the suite that matches the layer you changed, and add a new test whenever you add or fix domain logic. The `tempfile` dev-dependency is available for tests that need a scratch directory on disk.

## Pull Request Checklist

Run through this before requesting review. Each item maps to a rule above:

- [ ] **Layer placement correct.** Every new or moved module sits in the directory that matches its responsibility.
- [ ] **Dependencies point inward.** No new outward imports; `domain/` and `app/` remain free of forbidden crates.
- [ ] **Trait abstraction used.** New external dependencies cross the app/infra boundary through a port trait, not a direct call.
- [ ] **Composition root respected.** Only `main.rs` constructs and wires infrastructure.
- [ ] **No panics in the UI.** Recoverable errors surface as user-facing messages, never as panics.
- [ ] **Clippy clean.** `cargo clippy` produces no new warnings under the pedantic configuration.
- [ ] **Formatted.** `cargo fmt` has been run and produces no diff.
- [ ] **Tests added.** New domain logic (and, where practical, app logic) has accompanying tests, and `cargo test` passes.

## What to Avoid

A few anti-patterns are specifically watched for in review:

- Putting `symphonia`/`cpal` types into domain structs, or `egui` types into the application layer.
- Doing file I/O or long-running work on the UI thread; heavy work belongs on a background thread with results returned over a channel.
- Returning errors as bare `String` values instead of the typed, per-layer error enums.
- Acquiring a second lock while holding the `AppState` mutex; the codebase relies on a single, non-nested lock.

When in doubt, keep your change small and focused on one layer, and surface any architectural uncertainty in the pull request description rather than resolving it silently.
