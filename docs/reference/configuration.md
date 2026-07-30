# Configuration

riff has no traditional configuration file that you edit by hand. There is no `riff.toml`, no JSON settings file to author, and no command-line flags for configuration. Instead, state is persisted automatically at runtime in a small number of well-defined locations, and user-facing settings are driven through the UI and stored by the application framework. This page documents exactly where each piece of state and configuration lives. For the data flows that read and write this state, see [../technical/persistence.md](../technical/persistence.md); for problems involving the cache, see [./troubleshooting.md](./troubleshooting.md).

## Library Cache

The scanned library (tracks, artists, and albums) is serialized to a JSON file named `library_cache.json` so that the library loads instantly on startup without a rescan. It is loaded on startup and saved after each scan completes.

The file lives under the platform's per-user local data directory, resolved by the `directories` crate (`ProjectDirs` data-local dir):

| Platform | Library cache location |
|---|---|
| Linux | `~/.local/share/riff/library_cache.json` |
| macOS | `~/Library/Application Support/com.riff.riff/library_cache.json` |
| Windows | `%LOCALAPPDATA%\riff\riff\library_cache.json` |

If this file is missing or corrupted, riff recovers automatically by starting from an empty library; you then rescan your folders to rebuild it. See [./troubleshooting.md](./troubleshooting.md) for details.

## Settings Persistence

User-facing settings are UI-driven and persisted through `eframe::Storage`, the storage abstraction provided by the egui application framework. There is no settings file you are expected to edit; changing a setting in the UI writes it to this storage automatically.

The primary documented key is:

| Key | Type | Meaning |
|---|---|---|
| `library_paths` | JSON string array | The list of library folder paths the user has added. |

Additional UI state is persisted through the same mechanism — for example the playback volume and the per-path folder-watching enable/disable state — and the storage layer keeps a backup copy (for example a `library_paths_backup` key) that it can restore from if the primary value becomes corrupted. The exact on-disk location of `eframe::Storage` is managed by eframe per platform; treat it as framework-managed rather than as a file to edit directly.

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
- **CI** — There is no CI configuration file. Quality gates (format, lint, test) are run manually. See [../engineering/testing-strategy.md](../engineering/testing-strategy.md) for recommendations on adding CI.

## Summary

| Concern | Where it lives | Editable by hand? |
|---|---|---|
| Library cache | `library_cache.json` (per-OS data dir) | Not intended; rebuilt by scanning |
| Settings | `eframe::Storage` (framework-managed) | No — driven through the UI |
| Cover art cache | In-memory only (LRU, max 50) | No |
| Logging | `RUST_LOG` environment variable | Yes, at runtime |
| Lint config | `Cargo.toml` `[lints.clippy]` + `clippy.toml` | Yes, by developers |
| Formatting | Default rustfmt (no config file) | N/A |
| CI | None | N/A |
