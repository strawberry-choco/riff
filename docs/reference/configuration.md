# Configuration

riff has no traditional configuration file that you edit by hand. There is no `riff.toml`, no JSON settings file to author, and no command-line flags for configuration. Instead, state persists automatically at runtime in the **Application Store** (`riff.sqlite3`), and user-facing settings are driven through the UI. This page documents exactly where each piece of state lives. For how the store works, see [../technical/persistence.md](../technical/persistence.md); for problems involving the store, see [./troubleshooting.md](./troubleshooting.md).

## Application Store

The authoritative persistent state — the Library (tracks, artists, albums, play history), Playlists, and Settings — lives in one embedded SQLite database named `riff.sqlite3`, resolved by the `directories` crate (`ProjectDirs` data-local dir):

| Platform | Application Store location |
|---|---|
| Linux | `~/.local/share/riff/riff.sqlite3` |
| macOS | `~/Library/Application Support/com.riff.riff/riff.sqlite3` |
| Windows | `%LOCALAPPDATA%\riff\riff\riff.sqlite3` |

Every logical change commits as one small durable transaction; scan batches (~10 tracks) commit incrementally. If the file is missing, riff starts fresh; if it is corrupted, riff renames it aside (preserved beside a fresh copy) and starts over automatically. Schema evolution runs through ordered, checksummed migrations.

## Settings

User-facing settings are UI-driven and persisted in the Application Store's typed settings tables. There is nothing you are expected to edit by hand; changing a setting in the UI commits it immediately as its own small transaction.

| Table | Contents |
|---|---|
| `app_settings` | Single-row scalars: volume, advanced mode, high contrast, ReplayGain enabled |
| `library_paths` | The list of library folder paths the user has added |
| `watch_states` | Per-path watch choice: disabled, enabled, or warning with a diagnostic message |

## Cover Art Cache

Resolved cover-art textures are cached in memory only, with a fixed ceiling of 50 textures. Eviction is a manual least-recently-used (LRU) scheme. This cache is not written to disk and does not persist across restarts; cover art is re-resolved and re-decoded as needed after launch. The cap exists to bound memory usage, so it should not be raised without considering texture memory.

## Logging

riff uses the `tracing` crate for structured logging, with `tracing-subscriber` configured with its `env-filter` feature. This means log verbosity is controlled at runtime through the standard `RUST_LOG` environment variable rather than through any config file:

```bash
RUST_LOG=debug cargo run        # verbose logging for a dev run
RUST_LOG=riff=trace ./riff      # trace-level logging for the riff target
```

Without `RUST_LOG` set, logging stays at the default level. This is the only runtime "configuration knob" exposed via the environment.

## Build and Tool Configuration

Configuration for the development tooling lives in the repository, not in a user-facing settings file:

- **Clippy** — Lint levels are set in `Cargo.toml` under `[lints.clippy]` (pedantic as warnings, nursery allowed, plus a few individual allowances). Tool-level options live in `clippy.toml` (`msrv`, `avoid-breaking-exported-api`, `upper-case-acronyms-aggressive`). See [../engineering/coding-standards.md](../engineering/coding-standards.md) for the full listing.
- **rustfmt** — There is no `rustfmt.toml` or `.rustfmt.toml`; riff uses rustfmt's default style. Run `cargo fmt` before committing.
- **CI** — `.github/workflows/ci.yml` runs the quality gate (`cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`) on push and pull requests to main, on Linux and Windows runners.

## Summary

| Concern | Where it lives | Editable by hand? |
|---|---|---|
| Library, playlists, settings | `riff.sqlite3` (per-OS data dir) | Not intended; driven through the UI |
| Cover art cache | In-memory only (LRU, max 50) | No |
| Logging | `RUST_LOG` environment variable | Yes, at runtime |
| Lint config | `Cargo.toml` `[lints.clippy]` + `clippy.toml` | Yes, by developers |
| Formatting | Default rustfmt (no config file) | N/A |
| CI | `.github/workflows/ci.yml` | Yes, by developers |
