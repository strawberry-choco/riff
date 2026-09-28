# What this PR changes

<!-- One or two sentences. What a reviewer needs to know before reading the diff. -->

## Related issue

Closes #

## Checklist

- [ ] **Crate placement correct.** Every new or moved module sits in the crate whose membership criterion matches its responsibility.
- [ ] **Dependency chain respected.** No new workspace edge bypasses the chain; the slices remain free of native crates and of each other.
- [ ] **Trait abstraction used.** New external dependencies cross the slice/infra boundary through a port trait, not a direct call.
- [ ] **Composition root respected.** Only `riff-backend/src/composition.rs` constructs and wires infrastructure.
- [ ] **No panics in the UI.** Recoverable errors surface as user-facing messages, never as panics.
- [ ] **Clippy clean.** `cargo clippy --all-targets -- -D warnings` produces no warnings.
- [ ] **Formatted.** `cargo fmt` has been run and produces no diff.
- [ ] **Tests added.** New domain logic (and, where practical, app logic) has accompanying tests, and `cargo test --all-targets` passes.

The same list, with the reasoning behind each item, is in [docs/engineering/contributing.md](docs/engineering/contributing.md#pull-request-checklist).

## Before you request review

CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all-targets` on Linux and Windows — that is the gate, and clippy treats any warning as a failure. Run all three locally first.

For UI changes, also run `cargo run -p riff-gui` and check the behavior by hand; much of the interface is exercised manually rather than by an automated test. If a golden-image test fails, read [docs/engineering/golden-image-testing.md](docs/engineering/golden-image-testing.md) before re-baselining — the committed snapshots are byte-exact and legitimately differ across hardware.
