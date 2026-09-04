# Plan: Integrate `color-eyre` for Panic & Error Reporting

## Problem

Panics (main thread or worker threads) rely on Rust's default handler, which prints to stderr.
When the app is launched without a terminal (e.g. double-clicked `.exe` on Windows), panic output
is invisible — the window just vanishes. Worker thread panics also silently kill the thread with
no recovery path.

## Decision

Add `color-eyre` as a dependency of `riff-gui`. It replaces the default panic hook with a
formatted report (message + backtrace + tracing span trace), giving us:

- **Terminal launches**: rich panic output on stderr (already visible).
- **Non-terminal launches**: same output, still on stderr — but now structured enough to
  redirect to a file if we want later (not in scope for this change; defer to a follow-up).
- **Span traces**: `tracing` spans that were active at the panic site are captured and printed,
  making it possible to diagnose which worker path / scan / playback operation was in flight.

This is the minimum-viable diagnostic improvement. Worker-thread `catch_unwind` wrappers are
a separate follow-up (see Future Work below).

## Changes

### 1. Add dependency — `riff-gui/Cargo.toml`

```toml
[dependencies]
# Error reporting: pretty panic reports with backtrace + span trace.
color-eyre = "0.6"
```

Place it under the existing `# Logging` comment group, after `tracing-subscriber`.

### 2. Install before tracing — `riff-gui/src/main.rs`

```rust
fn main() {
    color_eyre::install().expect("failed to install color_eyre");
    tracing_subscriber::fmt::init();
    // ... rest unchanged
}
```

Order matters: `color_eyre::install()` must run before any other code that might panic,
including tracing initialization. It installs the panic hook; `tracing_subscriber::fmt::init()`
installs the log subscriber — they are independent and compose cleanly.

No other code changes required. `color_eyre::install()` returns `Result<(), Report>`; a failure
here means something is catastrophically wrong (e.g. hook already installed), so `.expect()` is
appropriate.

### 3. No other crates change

`color-eyre` is a binary-only concern. The backend crates do not depend on it and continue to
use `anyhow` / typed errors as before. The `tracing` spans they already emit will automatically
appear in panic reports via `SpanTrace`.

## Verification

1. **Build**: `cargo check --workspace` — no new warnings.
2. **Lint**: `cargo clippy --all-targets -- -D warnings` — clean.
3. **Smoke test (terminal)**: `cargo run -p riff-gui` — app launches, no panic report visible
   (no panic happened). Trigger a panic in a test scenario if desired.
4. **Panic report test**: Temporarily add `panic!("test")` at the top of `main()`, run with
   `cargo run -p riff-gui` — verify the formatted report with backtrace appears on stderr.
   Remove the test panic afterward.
5. **Release build**: `cargo build --release -p riff-gui` — confirm it compiles and the binary
   size increase is acceptable (~100–200 KB for the extra dependency).

## Risk

- **Binary size**: `color-eyre` pulls in `backtrace` crate (~100–200 KB). Acceptable for a
  desktop app.
- **Hook conflicts**: `color_eyre::install()` panics if called twice. Only called once in
  `main()`, before any spawning. No risk.
- **`RUST_BACKTRACE`**: `color-eyre` respects this env var. When set, backtraces are printed.
  When unset, only the panic message + span trace appear. Document this in AGENTS.md dev workflow.

## Future Work (not in this change)

- **Worker thread `catch_unwind`**: Wrap each worker's entry point in `catch_unwind` to convert
  panics into logged `Result` errors instead of thread death. Separate plan.
- **Log file redirect**: Layer a custom hook on top of `color-eyre` that also writes the panic
  report to a `.panic.log` file in the app data directory, for non-terminal launches. Separate
  plan.

## Files touched

| File | Change |
|------|--------|
| `riff-gui/Cargo.toml` | Add `color-eyre = "0.6"` dependency |
| `riff-gui/src/main.rs` | Add `color_eyre::install()` call before tracing init |
