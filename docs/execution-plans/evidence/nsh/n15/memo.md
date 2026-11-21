# N15 flip and delete

Landed. The binary is `riff` (`[[bin]] name = "riff"`). `crates/riff-gui`,
`crates/riff-gui-components` and `crates/riff-gui-theme` are deleted, and the
root manifest no longer names them or egui. `Cargo.lock` has no egui entry.
The test crate lost its egui-only suites and helpers and keeps the backend
suites plus the nsh bridge, shell and flow suites. Docs name the one
frontend; ADR 0004 gained the store-move amendment and ADR 0022's entries
now describe the landed flip.

Evidence.

- [x] `Cargo.lock` egui scan: `logs/lock-egui-check.txt` (no entries).
- [x] `cargo test --workspace`: `logs/workspace-tests.log` (every suite ok;
      24 result lines, no failures).
- [x] `cargo clippy --workspace --all-targets -- -D warnings`:
      `logs/clippy.log` (finished, no warnings).
- [x] `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
      --document-private-items`: `logs/rustdoc.log`.
- [x] `cargo build --release --workspace --bins`: `logs/release-build.log`;
      the binary is `target/release/riff.exe`, 38,288,896 bytes
      (`logs/release-binary.txt`).
- [x] Live: `../n15/shots/n15-playing.png` shows the shipped debug binary
      playing a fixture track through the new shell.
- [ ] Gap: the Linux and macOS binaries were not built on this machine; CI's
      matrix is the coverage, and N16's packaging leg needs the nsh CLI.

Files: `Cargo.toml`, `crates/riff-gui-nsh/Cargo.toml`, `tests/Cargo.toml`,
`tests/mod.rs`, the deleted crates and `tests/{ui,golden,docs_images,readme_screenshots}_tests.rs`.
