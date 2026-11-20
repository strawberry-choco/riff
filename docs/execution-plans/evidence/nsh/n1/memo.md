# N1 scaffold and CI memo

Branch `n1/scaffold-ci`, stacked on `p0/nsh-feasibility` at `a435a03`. Started 2026-11-20.
Machine: Windows 11 22631, x64, 12 logical cores, rustc 1.98.1 MSVC. CI pins 1.99.0.

Status: complete on Windows, pending the first CI run. Every Windows verdict below carries an
artifact under `logs/`, `shots/` or `tools/`. The Linux leg is wired but unexercised: this
machine cannot compile `nsh-platform-linux`, so the first Linux CI run is the measurement for
the GTK 4 floor, the GStreamer superset and `clang`.

## Verdicts

| N1 box | Verdict | Evidence |
| --- | --- | --- |
| Create the crate, manifest, identity, theme, brand asset | VERIFIED | `crates/riff-gui-nsh/`; PNG sha256 `71cee681…c9300`, 28702 bytes, 1024x1024, extracted from `riff.icns` ic10 by `tools/extract_brand_png.py` |
| Root `App` renders a token-styled view with name and version | VERIFIED | `shots/n1-window.png` shows `riff` and `v0.2.0` centred; `src/app.rs` holds no design literal |
| `cargo run -p riff-gui-nsh` opens a window on Windows | VERIFIED | `logs/n1-debug-run.log`, `shots/n1-window.png` |
| Opens a window on Linux | NOT EXERCISED | no Linux toolchain here; the first CI Linux leg is the check |
| Both binaries build in CI | WIRED, NOT RUN | `ci.yml` `Link the binaries` step; local `cargo build --locked --workspace --bins` green in `logs/n1-parent-gates.log`; no CI run exists until the branch is pushed |
| MSRV 1.97 in all six declarations | VERIFIED | root `Cargo.toml`, `clippy.toml`, `AGENTS.md`, `CONTRIBUTING.md`, `development-setup.md`, `ci.yml` comment |
| Resolver stays 2 | VERIFIED | root manifest comment; ADR 0022 |
| Seven exact pins at 0.6.1, defaults off on nsh-base and nsh-engine | VERIFIED | root `[workspace.dependencies]`; `nsh-engine` inherited by the member so its pin binds |
| Four scaffold tests | VERIFIED | `logs/n1-test-final.log`, `cargo test -p riff-gui-nsh` 4 passed |
| Full suite incl. 87 goldens and the apt guard | VERIFIED | `logs/n1-test-final.log`, 866 passed, 0 failed, `--test-threads=1` |
| clippy, fmt, rustdoc clean at the final head | VERIFIED | `logs/n1-parent-gates.log` |
| Release-only `windows_subsystem` well-formed | VERIFIED | `logs/n1-release-build.log`; PE subsystem read directly: release 2 (GUI), debug 3 (console); `logs/n1-release-run.log` |
| Lock churn gate | VERIFIED with six flagged moves | `lock-diff.txt`, `feature-tree-diff.txt` |
| ADR 0022 and the ADR 0004 coexistence amendment | VERIFIED | `docs/adr/0022-…md`, amendment appended to `0004` |
| Plan corrections applied | VERIFIED | `docs/execution-plans/nsh-ui-rewrite.md` |

## Measurements

Lock: 658 packages before, 798 after. 115 added, 24 duplicate coexistences where the old version
survives beside a new one, 6 replacements, 0 removals. None of the six is in the golden render
path. The four captured feature trees (`riff-gui`, `riff-gui-components`, `riff-gui-theme`,
`riff-tests`) show zero feature-edge changes; the only tree differences are the `cc` and
`find-msvc-tools` build-tool versions.

The six replacements are `cc` 1.4.4 to 1.11.3, `find-msvc-tools` 0.1.11 to 0.1.15, `objc2` 0.6.4
to 0.6.5, `proc-macro-crate` 2.0.2 to 2.0.0, `toml` 0.8.2 to 0.8.23, `toml_datetime` 0.6.3 to
0.6.11. Their cause is UNATTRIBUTED. A pristine HEAD worktree re-resolves with zero changes, so
the moves come from adding the nsh graph; `toml_datetime` 0.6.3 and `find-msvc-tools` 0.1.11 are
not yanked, and `cc`'s dependents are unchanged riff crates. All six are build-script,
proc-macro or macOS-only crates outside the render path. Remedy if a golden ever fails:
`cargo update -p <crate> --precise <old version>`, a reviewed hold, not a re-baseline.

Build and test wall times, debug unless stated: clippy 2m36s, full suite 141.78s, rustdoc 19.96s,
release link of `riff-nsh` 8m22s. The release time is LTO plus codegen-units 1 and is paid once,
locally, not per CI run.

## Method

Cargo ran serially throughout; three owners held disjoint file sets and only the code owner ran
cargo. The design came from a two-candidate arena with a readonly cross-judge; the synthesis and
its graft provenance are in `.scratch/nsh-n1/synthesis.md` (gitignored working artifact). The
decision trail is `decisions.tsv` beside this memo.

Two gates ran before any fan-out: the lock-diff lever (`tools/lock_diff.py`) and the feature-tree
capture. The lever's first version conflated duplicate coexistence with replacement and stopped
on a false positive; it was refined to fail only on a golden-path replacement or a removal, and
re-verified against the pristine pair.

The brand PNG is extracted, not re-rasterized, so it is byte-identical to the macOS icon copy.
`release-and-packaging.md` now names three icon copies instead of two.

## Open, with named owners

- The first CI run, both legs. Nothing here proves the Linux compile. `clang` and the GStreamer
  packages are a deliberate superset until that run measures them. Owner: the operator's push.
- macOS is never compiled by CI at N1. `nsh-platform-macos`'s feature names are copied from the
  generated template. Owner: N15, with one operator run before cutover.
- The packaged graph is not this graph: the CLI's generated runner re-adds `highlight` on
  `nsh-base`. Owner: N16, which measures it.
- `nsh run` and `nsh package` remain unexercised. Owner: N16.
- Whether the packaged runner applies `theme.json` without the `Root::tokens` wiring. The cargo
  path is wired and tested; the packaged path is untraced. Owner: N16.
- Dropping `shader` and its visual cost on the Linux renderer, whose source is not on this
  machine. Owner: N2, with the `dev_tokens` fixture as the before/after.
- P0's carried gates: tray `on_click` and close-to-hide, caption-button suppression (N14), and
  the N6 scroll budget with its prefetch window.