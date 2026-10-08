# Contributing to riff

Thanks for helping. riff is a lightweight, offline-first desktop music player written in Rust and egui, and a contribution can be any size — a bug report, a one-line fix, a test, or a feature. Bug reports and feature requests go to [the issue tracker](https://github.com/strawberry-choco/riff/issues).

This file is a front door, not a manual. The detail lives in [`docs/`](docs/README.md), which indexes every document in the repository. Two of them matter most before you write code: [docs/engineering/contributing.md](docs/engineering/contributing.md) (how to orient yourself, the crate architecture, and the rules a change must respect) and [docs/engineering/coding-standards.md](docs/engineering/coding-standards.md) (layering rules, lint configuration, and the implementation gotchas that trip contributors up).

## Code of Conduct

Be decent to each other. riff follows the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/): treat people here as colleagues, assume good faith, and accept that a maintainer may disagree with your approach. Harassment and exclusionary behavior are not tolerated. The full covenant now lives in [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) at the repository root, with the contact method and how enforcement works in a single-maintainer project filled in.

## Prerequisites

- **Rust stable**, installed via [rustup](https://rustup.rs/), which brings `cargo`, `rustfmt`, and `clippy` with it. Every crate manifest declares `rust-version = "1.95"` and edition 2024, so you want a toolchain at or above that; CI pins one exact toolchain (`toolchain:` in `.github/workflows/ci.yml`) rather than tracking `stable`. The MSRV is informational — no CI job enforces it, so an older compiler may happen to work but is not supported.
- **A C toolchain**, and on Linux the system libraries below. The pure-Rust crates build with no native dependencies at all; `riff-infra` (bundled SQLite, ALSA) and `riff-gui` (wgpu, winit) are the ones that need them.

On Linux, install the development headers. This is the exact list CI installs on its Linux runners, which is also the full list for any Debian/Ubuntu derivative:

```bash
sudo apt-get install pkg-config libasound2-dev libudev-dev \
  libx11-dev libxcursor-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev libxi-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libssl-dev mesa-vulkan-drivers
```

The families involved: ALSA development headers for `cpal`, X11/xcb and Wayland development headers for `winit`, plus `libxkbcommon-x11-dev` and `pkg-config`. `libudev-dev` covers cpal's device handling, and `mesa-vulkan-drivers` provides the software Vulkan adapter that renders the headless golden-image tests. **`libgtk-3-dev` is intentionally not needed** — the tray icon and native dialog stack (`tray-icon`, `muda`, `rfd`) are gated to non-Linux targets, and the Linux folder picker is a text input.

There are no feature flags, no code-generation step, and no migrations to run by hand.

## Build and verify

```bash
cargo run -p riff-gui                      # run in dev mode
cargo build --release -p riff-gui          # release build (LTO, stripped)
cargo fmt                                  # format
cargo check --all-targets                  # type/borrow checking across all targets
cargo clippy --all-targets -- -D warnings  # lint with warnings as errors
cargo test --all-targets                   # run all unit and integration tests
```

Run all of these before you open a pull request — it is the same gate CI runs on push and pull requests to `master`, on Linux and Windows runners. Two of them are stricter than they look:

- **Clippy fails the build on any warning** (`-D warnings`). The workspace enables clippy's pedantic group as warnings, so the tree is expected to stay completely warning-free; a clean run is the standard, not a nicety.
- **`cargo fmt` must produce no diff** (CI runs `cargo fmt --check`). There is no `rustfmt.toml` in the repository, so this is rustfmt's default style.

For UI changes, also run `cargo run -p riff-gui` and check the behavior by hand — much of the interface is exercised manually rather than by an automated test. New logic should come with new tests; [docs/engineering/testing-strategy.md](docs/engineering/testing-strategy.md) says which suite a given change belongs in.

Commit messages are validated at commit time, so you learn about a malformed message before CI does. One command wires the hook in:

```bash
git config core.hooksPath .githooks
```

The hook is a thin shim over `tools/validate-commit-msg.py` and needs `python3` on your `PATH`; without it the hook prints a notice and steps aside — CI (the `commit-lint` job in `ci.yml`) is the real gate.

## Golden-image tests

The committed golden snapshots are byte-exact baselines authored on a specific machine, with a **zero-pixel threshold** on macOS. wgpu picks a different adapter per machine, and GPU-rasterized shapes — 1px strokes, rounded-rect edges, dividers, image sampling — drift with the driver, so a golden that fails on your hardware is often a hardware difference rather than a change you made. Read the diff PNG before you assume either way, and never re-baseline to make an unexplained failure disappear: [docs/engineering/golden-image-testing.md](docs/engineering/golden-image-testing.md) documents the workflow, the determinism rules, and how to prove a re-baseline is what you think it is.

If the `riff-tests` binary dies mid-run with `STATUS_ACCESS_VIOLATION` and no failing test, that is a known open flake, already investigated and documented in [docs/engineering/access-violation-flake.md](docs/engineering/access-violation-flake.md) — read it before you start bisecting.

## Pull request checklist

- [ ] **Crate placement correct.** Every new or moved module sits in the crate whose membership criterion matches its responsibility.
- [ ] **Dependency chain respected.** No new workspace edge bypasses the chain; the slices remain free of native crates and of each other.
- [ ] **Trait abstraction used.** New external dependencies cross the slice/infra boundary through a port trait, not a direct call.
- [ ] **Composition root respected.** Only `riff-backend/src/composition.rs` constructs and wires infrastructure.
- [ ] **No panics in the UI.** Recoverable errors surface as user-facing messages, never as panics.
- [ ] **Clippy clean.** `cargo clippy --all-targets -- -D warnings` produces no warnings.
- [ ] **Formatted.** `cargo fmt` has been run and produces no diff.
- [ ] **Tests added.** New domain logic (and, where practical, app logic) has accompanying tests, and `cargo test --all-targets` passes.
- [ ] **Commit messages conventional.** Every commit on the branch is a well-formed Conventional Commit; see [Merge strategy](#merge-strategy) for why there is no rewrite step to fix one later.

This is the short version; [docs/engineering/contributing.md](docs/engineering/contributing.md#pull-request-checklist) has the same list with the reasoning behind each item, plus the anti-patterns that are specifically watched for in review.

## Merge strategy

Pull requests are merged with **rebase-and-merge only** — merge commits and squash merging are disabled in the repository settings, and history stays linear. What you write on the branch is what lands on `master`, verbatim: GitHub replays each commit onto the current tip of the merge target, preserving every message, author, and author date, and changing only the committer and the commit SHAs. There is no rewrite step between the branch and the default branch, so every commit on the branch must be a well-formed Conventional Commit (the [checklist item](#pull-request-checklist) above); the `commit messages (pull request)` CI job is the required branch-protection check that enforces it.

One cost of the replay is accepted knowingly: every commit receives a new SHA when it lands, so a commit SHA quoted in a review comment or an issue points at something that will not exist after the merge. The durable reference is the **pull request number**, never a commit SHA.

## Where things live

[`docs/README.md`](docs/README.md) is the index of all documentation — product specs, the architecture and threading references, the ADRs and product decision records, and the engineering conventions. [`AGENTS.md`](AGENTS.md) is the fastest single-page tour of the codebase: architecture, threading model, platform specifics, the command list, and the gotchas worth reading before you touch a given area.

## Scope

riff is offline-only, and that is a design decision rather than a limitation to be lifted later. Features that would require a server, an account, or any network call are out of scope — including online metadata or artwork lookup, streaming, and cloud sync. [docs/product/decisions/001-offline-first.md](docs/product/decisions/001-offline-first.md) records why. If you have an idea that needs a network, open an issue and argue the design first rather than a pull request.
