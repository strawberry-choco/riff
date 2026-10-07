# Contributing to riff

Thank you for considering a contribution to riff, a lightweight, offline-first desktop music player written in Rust and egui. This guide explains how to orient yourself in the codebase, the rules your change must respect, and the checklist to run through before opening a pull request. For the environment setup and the exact commands, see [development-setup.md](./development-setup.md); for the reasoning behind the conventions, see [coding-standards.md](./coding-standards.md).

## Getting Oriented

Before writing code, read these in order:

1. **`AGENTS.md`** at the repository root — a fast, high-level tour of the architecture, threading model, platform-specific code, and the implementation gotchas. It is the quickest way to build a mental model of the project.
2. **`docs/`** — these documents. Start with [coding-standards.md](./coding-standards.md) for the layering rules and lint configuration, then [development-setup.md](./development-setup.md) for the build workflow.
3. **The source tree itself** — the crate layout *is* the architecture; where a piece of code lives is a deliberate decision enforced by the dependency chain.

## The Crate Architecture

Every contribution must respect the crate split and its dependency chain (see [ADR 0009](../adr/0009-vertical-crate-split-of-the-backend.md) and [../technical/architecture.md](../technical/architecture.md)). This is the single most reviewed aspect of a change, so internalize it before you start. The short form: dependencies follow the chain with no bypasses, the slices stay pure Rust and reach external crates only through port traits they define, and only `riff-backend/src/composition.rs` constructs and wires concrete adapters. Which crate a new piece of code belongs in is decided by the membership criteria in [../technical/architecture.md](../technical/architecture.md#crate-definitions-and-membership-criteria).

## Working on a Change

A typical contribution workflow looks like this:

1. Create a branch from the latest master branch.
2. Make your change, keeping each new module in the crate whose membership criterion it satisfies.
3. Run `cargo fmt` so formatting matches the project default (there is no `rustfmt.toml`).
4. Run `cargo clippy` and resolve any new warnings. Pedantic lints are enabled as warnings; a clean run is the expected standard.
5. Run `cargo test`. New logic should come with new tests; see [testing-strategy.md](./testing-strategy.md) for where each suite lives and which one matches the code you changed.
6. For UI changes, run `cargo run -p riff-gui` and manually verify the behavior, since much of the UI is exercised by hand rather than by automated tests.

### Adding tests

Tests exist and are expected to grow with the code: adapter and store tests live in `riff-infra/tests/`, while cross-crate integration, UI, and golden-image tests live in the workspace-root `tests/` crate. Add new tests to the suite that matches the code you changed, and add a new test whenever you add or fix logic. The `tempfile` dev-dependency is available for tests that need a scratch directory on disk.

## Pull Request Checklist

Run through this before requesting review. Each item maps to a rule above:

- [ ] **Crate placement correct.** Every new or moved module sits in the crate whose membership criterion matches its responsibility.
- [ ] **Dependency chain respected.** No new workspace edge bypasses the chain; the slices remain free of native crates and of each other.
- [ ] **Trait abstraction used.** New external dependencies cross the slice/infra boundary through a port trait, not a direct call.
- [ ] **Composition root respected.** Only `riff-backend/src/composition.rs` constructs and wires infrastructure.
- [ ] **No panics in the UI.** Recoverable errors surface as user-facing messages, never as panics.
- [ ] **Clippy clean.** `cargo clippy` produces no new warnings under the pedantic configuration.
- [ ] **Formatted.** `cargo fmt` has been run and produces no diff.
- [ ] **Commit messages conventional.** Every commit on the branch passes the commit-message rules (type, scope, header length); rebase rather than squash-fix, since commits land as written — see [commit-conventions.md](./commit-conventions.md).
- [ ] **Tests added.** New domain logic (and, where practical, app logic) has accompanying tests, and `cargo test` passes.

## What to Avoid

A few anti-patterns are specifically watched for in review:

- Putting `symphonia`/`cpal` types into domain structs, or `egui` types into the application layer.
- Doing file I/O or long-running work on the UI thread; heavy work belongs on a background thread with results returned over a channel.
- Returning errors as bare `String` values instead of the typed, per-owner error enums (`StoreError`, `LibraryError`, `PlaybackError`).
- Acquiring a second lock while holding a session mutex; each session (`PlaybackSession`, `LibrarySession`) is behind its own non-nested lock, and a lock must never be held across a long operation.

When in doubt, keep your change small and focused on one layer, and surface any architectural uncertainty in the pull request description rather than resolving it silently.
