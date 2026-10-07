# Development Setup

This guide gets a new contributor from a fresh machine to a running build of riff. riff is a lightweight, offline-first desktop music player written in Rust on top of the egui immediate-mode GUI framework. It is a Cargo workspace with no code-generation step and no feature flags, so the setup is deliberately minimal: install a recent Rust toolchain, clone the repository, and run `cargo`.

## Prerequisites

The only hard requirement is a Rust toolchain. Every crate declares `rust-version = "1.95"` in its manifest, so you want a toolchain at or above that version. Note that this MSRV is informational only; there is no CI job that enforces it, so a slightly older compiler may happen to work but is not supported.

Install Rust via [rustup](https://rustup.rs/), which manages the compiler, `cargo`, `rustfmt`, and `clippy`:

```bash
rustup update stable
rustc --version
```

### Platform-specific prerequisites

Audio output is provided by the `cpal` crate, which talks to each operating system's native audio API. Most of these dependencies are present by default, with one common exception on Linux:

| Platform | Native audio backend | Extra setup |
|---|---|---|
| Windows | WASAPI | None — ships with the OS |
| macOS | CoreAudio | None — ships with the OS |
| Linux | ALSA | ALSA development headers are required to compile `cpal` |

Note that ALSA is not the only Linux dependency. The windowing stack in `riff-gui` (`winit`) also needs X11/xcb and Wayland development headers, and `cpal` needs `libudev-dev`, so a first build of the GUI fails without them too — the ALSA headers are just the most commonly-missing piece. The full list CI installs on `ubuntu-latest`, which is authoritative, is in [../../CONTRIBUTING.md](../../CONTRIBUTING.md#prerequisites):

```bash
sudo apt-get install pkg-config libasound2-dev libudev-dev \
  libx11-dev libxcursor-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev libxi-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libssl-dev mesa-vulkan-drivers
```

The exact package names vary by distribution (for example `alsa-lib-devel` on Fedora). If the build fails inside `cpal` or `alsa-sys` on Linux, missing ALSA headers are almost always the cause; if it fails inside `winit` or `x11-dl`, it is the windowing headers. Note that the audio stack lives only in `riff-infra`: the pure-Rust crates (`riff-persistence`, `riff-library`, `riff-playback`) build and their tests run without any native audio libraries.

## Clone and Build

Clone the repository and build in debug mode:

```bash
git clone https://github.com/strawberry-choco/riff riff
cd riff
cargo build
```

The first build downloads and compiles all dependencies. Subsequent builds are incremental and much faster. To run the application during development, use `cargo run -p riff-gui`, which builds (if needed) and launches the player in one step.

Commit messages are validated locally too, so a malformed message is caught while the change is still fresh. One command wires the hook in:

```bash
git config core.hooksPath .githooks
```

The hook is a thin shim over `tools/validate-commit-msg.py` and needs `python3` on your `PATH`; without it the hook prints a notice and steps aside — CI (the `commit-lint` job in `ci.yml`) is the real gate.

## Commands

The full day-to-day command set is below. There is no project script runner, task runner, or Makefile — everything goes through `cargo`. The one project script is the commit-message validator (`tools/validate-commit-msg.py`); neither it nor its git hook is part of the build workflow. Run from the workspace root; add `-p <crate>` to scope a build to one member.

| Command | Purpose | Notes |
|---|---|---|
| `cargo run -p riff-gui` | Build and launch in dev mode | Uses the workspace `dev` profile (`opt-level = 1`) |
| `cargo check --workspace` | Fast type-check without codegen | Preferred for quick feedback while editing |
| `cargo build --release -p riff-gui` | Optimized release binary | LTO + strip; slow to compile, small binary |
| `cargo fmt` | Format all source files | No `rustfmt.toml`; uses default style |
| `cargo clippy --all-targets` | Lint the codebase | Pedantic lints enabled as warnings |
| `cargo test --all-targets` | Run the test suites | `riff-infra/tests/` plus the root `tests/` crate; see [testing-strategy.md](./testing-strategy.md) |
| `cargo test -p riff-infra` | Run the adapter/store suite | Real SQLite and real adapters, without building the UI |
| `cargo test -p riff-tests` | Run the workspace-root suite | Cross-crate integration, UI, and golden-image tests |

A typical inner loop is `cargo check` while editing, `cargo fmt` and `cargo clippy` before committing, and `cargo test` before opening a pull request. See [coding-standards.md](./coding-standards.md) for the lint and formatting conventions in detail, and [testing-strategy.md](./testing-strategy.md) for how the suites are organized.

One practical note: because the workspace splits pure logic from the adapter stack, you can type-check and test the pure crates (`cargo test -p riff-playback -p riff-library -p riff-persistence`) without a C compiler or platform audio libraries — only `riff-infra` (bundled SQLite) and `riff-gui` need the native toolchain.

The two build profiles and their trade-offs are described in [release-and-packaging.md](./release-and-packaging.md).

## Notes

A few facts about the project shape that simplify expectations:

- **No feature flags.** There are no Cargo features to enable or disable. The only conditional compilation is per-target-OS (`#[cfg(target_os = "linux")]` and its negation) for platform-specific system integration such as the tray icon and native file dialogs.
- **No codegen step.** There is no build script output, no schema generation, and no asset pipeline to run before compiling.
- **Embedded migrations.** State persists in the Application Store (`riff.sqlite3` via rusqlite, bundled — in `riff-infra`). Schema evolution runs through ordered, checksummed migrations applied automatically on open — there is no external migration tooling to run.
- **CI pipeline.** `.github/workflows/ci.yml` runs the quality gate (`cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`) on push and pull requests to master, on Linux and Windows runners. A commit-message hook is available locally (`git config core.hooksPath .githooks`) — optional, with the CI jobs as the real gate; see [commit-conventions.md](./commit-conventions.md). Pull requests are merged with rebase-and-merge only, and the `commit messages (pull request)` job (`commit-lint`) is the required branch-protection check behind it — every commit lands on `master` as written, so each must be well-formed. The full story is in [CONTRIBUTING.md](../../CONTRIBUTING.md#merge-strategy).
