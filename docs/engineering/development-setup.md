# Development Setup

This guide gets a new contributor from a fresh machine to a running build of riff. riff is a lightweight, offline-first desktop music player written in Rust on top of the egui immediate-mode GUI framework. It is a single Cargo crate with no code-generation step, no database migrations, and no feature flags, so the setup is deliberately minimal: install a recent Rust toolchain, clone the repository, and run `cargo`.

Everything on this page reflects the current state of the repository. Where this document offers guidance beyond what is wired up today, it is labeled as a recommendation.

## Prerequisites

The only hard requirement is a Rust toolchain. riff declares `rust-version = "1.92"` in `Cargo.toml`, so you want a toolchain at or above that version. Note that this MSRV is informational only; there is no CI job that enforces it, so a slightly older compiler may happen to work but is not supported.

Install Rust via [rustup](https://rustup.rs/), which manages the compiler, `cargo`, `rustfmt`, and `clippy`:

```bash
rustup update stable
rustc --version   # confirm 1.92 or newer
```

### Platform-specific prerequisites

Audio output is provided by the `cpal` crate, which talks to each operating system's native audio API. Most of these dependencies are present by default, with one common exception on Linux:

| Platform | Native audio backend | Extra setup |
|---|---|---|
| Windows | WASAPI | None — ships with the OS |
| macOS | CoreAudio | None — ships with the OS |
| Linux | ALSA | ALSA development headers are often required to compile `cpal` |

On Debian/Ubuntu you typically need the ALSA development package before the first build will succeed:

```bash
sudo apt-get install libasound2-dev pkg-config
```

The exact package name varies by distribution (for example `alsa-lib-devel` on Fedora). If the build fails inside `cpal` or `alsa-sys` on Linux, missing ALSA headers are almost always the cause.

## Clone and Build

Clone the repository and build in debug mode:

```bash
git clone <repository-url> riff
cd riff
cargo build
```

The first build downloads and compiles all dependencies, which takes several minutes. Subsequent builds are incremental and much faster. To run the application during development, use `cargo run`, which builds (if needed) and launches the player in one step.

## Commands

The full day-to-day command set is below. There are no project-specific scripts, task runners, or Makefiles; everything goes through `cargo`.

| Command | Purpose | Notes |
|---|---|---|
| `cargo run` | Build and launch in dev mode | Uses the `dev` profile (`opt-level = 1`) |
| `cargo check` | Fast type-check without codegen | Preferred for quick feedback while editing |
| `cargo build --release` | Optimized release binary | LTO + strip; slow to compile, small binary |
| `cargo fmt` | Format all source files | No `rustfmt.toml`; uses default style |
| `cargo clippy` | Lint the codebase | Pedantic lints enabled as warnings |
| `cargo test` | Run the test suite | Tests under `tests/`; see caveat below |

A typical inner loop is `cargo check` while editing, `cargo fmt` and `cargo clippy` before committing, and `cargo test` before opening a pull request. See [coding-standards.md](./coding-standards.md) for the lint and formatting conventions in detail, and [testing-strategy.md](./testing-strategy.md) for how the test suite is organized.

One current caveat: the test files under `tests/` do not compile as of this writing (they reference crate-internal paths that do not resolve from the integration-test location of a binary-only crate), so `cargo test` fails at build time. This is a known issue tracked in [testing-strategy.md](./testing-strategy.md), where making the suite compile is the top recommendation. The other commands above are unaffected.

## Build Profiles

riff tunes its two build profiles for their respective roles. The relevant settings live in `Cargo.toml`:

- The `dev` profile sets `opt-level = 1`. This keeps debug builds reasonably fast to compile while still producing a binary responsive enough for interactive UI work.
- The `release` profile sets `opt-level = 3`, `lto = true`, `codegen-units = 1`, and `strip = true`. Link-time optimization and single-codegen-unit compilation enable whole-program optimization, and `strip` removes debug symbols. The result is a small, fully optimized standalone binary.

The trade-off is compile time: a release build is significantly slower than a debug build because LTO and `codegen-units = 1` defeat incremental parallel codegen. This is expected and normal. Use `cargo run` (dev) for iteration and reserve `cargo build --release` for producing distributable binaries. See [release-and-packaging.md](./release-and-packaging.md) for the release workflow.

## Notes

A few facts about the project shape that simplify expectations:

- **No feature flags.** There are no Cargo features to enable or disable. The only conditional compilation is per-target-OS (`#[cfg(target_os = "linux")]` and its negation) for platform-specific system integration such as the tray icon and native file dialogs.
- **No codegen step.** There is no build script output, no schema generation, and no asset pipeline to run before compiling.
- **No migrations.** State is persisted as plain JSON files at runtime; there is no database and no migration tooling.
- **No CI pipeline or pre-commit hooks.** Quality gates (format, lint, test) are currently run manually by the developer. See [testing-strategy.md](./testing-strategy.md) for recommendations on adding CI.
- **MSRV is informational.** `rust-version = "1.92"` documents the intended minimum compiler but is not enforced by automation.

For where the application stores its state and how logging is configured, see [../reference/configuration.md](../reference/configuration.md).
