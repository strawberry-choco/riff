# Release and Packaging

This document describes how riff is built for distribution. For the development build workflow, see [development-setup.md](./development-setup.md); for the feature catalog the release ships, see [../product/features.md](../product/features.md).

riff is a Cargo workspace that compiles to a single standalone binary (the `riff` binary in `riff-gui`). It has no feature flags, no runtime plugin system, and no external assets that must ship alongside the executable, which is what makes packaging a matter of building one binary per platform and putting it in an archive. There is no installer, no `.app` bundle, and no code signing, and those omissions are deliberate at this stage rather than oversights: they are the same limitations the release notes state to users.

## The release pipeline

Distribution is automated. `.github/workflows/release.yml` runs on a pushed tag and does everything from version check to published GitHub release, so nobody builds a distributable binary on a laptop.

### Trigger

The workflow triggers on `push` to a tag matching `v*`. The tag is the only input: the four artifact names, the release title, and the release notes header are all derived from `github.ref_name`. No version string is written literally anywhere in the workflow, so cutting `0.3.0` is a change to one line in `Cargo.toml` and one tag, not an edit to CI.

### The version guard

The first job, `verify`, compares the pushed tag against the workspace version and exits non-zero on a mismatch, before any build starts. The version is read with `cargo metadata --no-deps`, piped to a parser, **not** by grepping a manifest: the version is single-sourced in `[workspace.package]` and every member inherits it with `version.workspace = true`, so no member manifest contains a literal `version = "..."` any more and a grep-based guard would match nothing and pass vacuously.

The rule is that the tag is `v` plus the workspace version, optionally followed by a SemVer pre-release and/or build suffix (`v0.1.0`, `v0.1.0-rc.1`, `v0.1.0+20260928`). The numeric core must match exactly; anything that is not a release tag at all — `vfoo`, `v1.2`, `v1.2.3.4` — is rejected separately. The failure message names both the tag and the version cargo reported.

This guard lives in `release.yml` and not in `ci.yml` on purpose. CI triggers only on pushes to `master` and on pull requests, so a tag-versus-version check there would never execute: it would be a guard that always passes because it never runs.

### Targets and artifacts

`build` is a four-way matrix, each leg building natively on its own runner and packaging the result:

| Runner | Artifact slug | Format | Contents |
|---|---|---|---|
| `macos-15` | `macos-arm64` | `tar.gz` | `riff` |
| `macos-15-intel` | `macos-x86_64` | `tar.gz` | `riff` |
| `ubuntu-22.04` | `linux-x86_64` | `tar.gz` | `riff` |
| `windows-latest` | `windows-x86_64` | `zip` | `riff.exe` |

The two macOS labels are pinned to currently-GA images, and keeping them current is a maintenance task rather than a preference. GitHub supports the latest two macOS releases and deprecates the rest, which means a deprecated image goes through announced brownouts and then disappears. Because `release` needs all four legs, one retired label does not degrade the release — it blocks it, with no partial publish. `macos-13` has already been removed from `actions/runner-images` and `macos-14` is badged deprecated, so an earlier version of this table named two runners that could not have run at all. When bumping, check the current table in `actions/runner-images`, and do not collapse both macOS legs to `macos-latest`: that label is arm64, so it would silently drop the Intel build while the artifact name still claimed `macos-x86_64`. Intel needs the explicit `-intel` label.

The published names are `riff-<tag>-<slug>.tar.gz` (and `.zip` on Windows) — for example `riff-v0.1.0-linux-x86_64.tar.gz`.

**Each archive contains exactly one file, the executable.** There is no assets folder, and that is not a packaging oversight waiting to be fixed: the Inter faces are `include_bytes!`-ed and the Lucide icons `include_str!`-ed by `crates/riff-gui/src/ui/fonts.rs` and `crates/riff-gui/src/ui/icons.rs`, so the binary carries its own fonts and icons. The Unix legs use `tar -czf` with a member name, which keeps the archive flat; the Windows leg uses `Compress-Archive` on `target/release/riff.exe` — the `.exe` suffix matters, and a Unix path there would package nothing at all.

`if-no-files-found: error` on the upload step is the backstop: a packaging step that silently produced nothing must fail the leg rather than publish a zero-byte download.

### Why `build` and `release` are two jobs

`build` only ever produces artifacts. `release` is a separate job with `needs: [build]` that downloads them and creates the GitHub release. A single matrix job that also published would have a race built into it: the first leg to finish would create the release and the other three would fail with "release already exists", so the outcome would depend on which platform's runner happened to be fastest. Splitting them means the four legs cannot interfere with each other, and a build failure simply stops the publish.

`release` runs on `ubuntu-latest` and downloads the four artifacts into one flat staging directory with `merge-multiple: true` — correct precisely because the four names are distinct, so flattening cannot collide. It then checks that all four expected files are present (naming the directory listing on failure) before calling `gh release create` with the four paths. That check is what turns a stale list of targets into a clear error rather than a release quietly missing a platform.

The job never checks the repository out, so `GH_REPO` is set explicitly: `gh` cannot infer the target repository from a git remote that is not there.

### Token scope

The workflow-level default is `permissions: contents: read`. Exactly one job — `release` — widens it to `contents: write`, because `gh release create` is the only write in the whole pipeline. The build matrix compiles a large dependency graph and runs its build scripts; it has no write access at all, so a compromised dependency cannot push to the repository or cut a release.

### Release notes and the pre-release policy

The release notes are generated in the workflow from a literal template rather than from `CHANGELOG.md`, and the reason is that the notes have a job the changelog does not: they must state, per platform, what the artifact actually is. The Windows caveat (a zipped bare `riff.exe`, unsigned, SmartScreen, and the note that an NSIS installer would hit the same wall), the macOS caveat (a `tar.gz` and not a `.app` bundle, so no Dock icon and unreliable menu-bar focus, plus the Gatekeeper quarantine and the `xattr` command), and the Linux caveat (dynamically linked against `libasound2`) are written into the workflow so they cannot drift from what the pipeline produces.

**Every release is published as a GitHub pre-release**, unconditionally, whatever the tag says. riff is pre-1.0 and the binaries are unsigned and un-packaged per platform; promoting a release out of pre-release is a deliberate manual act, not a side effect of a tag. The notes also state that this is the first public release and that riff is offline-only by design.

## Cutting a release

1. Set the version in the root `Cargo.toml` `[workspace.package]` block. The seven member manifests inherit it and must not be edited.
2. Update `CHANGELOG.md`: fill in the date on the version section, move anything under `## [Unreleased]` into it, and repoint the link at the bottom of the file.
3. Merge to `master` and let CI pass. The release workflow does not re-run the quality gate, so a red `master` will not be caught by tagging it.
4. Push the tag: `git push origin v<version>`. The `verify` job fails in under a minute if the tag and the workspace version disagree, which is the intended moment to notice a version bump that was forgotten.
5. When the four artifacts are published, add the download badges to `README.md`. This is deliberately **not** automated: the badges are a claim that a real release exists, and a workflow that rewrote the README on a failed run would leave a claim behind with nothing to back it.

The pipeline is not idempotent, and it does not pretend to be: re-running the `release` job after a successful publish fails, because the release for that tag exists. Edit or delete the existing release and re-run.

## Current State

### Release profile

The release build is configured in the root `Cargo.toml` (workspace-level, so it applies to every member) for maximum optimization and a small binary:

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
strip = true
```

- `opt-level = 3` — full optimizations.
- `lto = true` — link-time optimization across all crates for whole-program optimization.
- `codegen-units = 1` — a single codegen unit, which gives the optimizer the most visibility at the cost of parallelism.
- `strip = true` — debug symbols are stripped from the final binary.

The `dev` profile is the mirror image for iteration speed: it sets
`opt-level = 1`, which keeps debug builds reasonably fast to compile while still
producing a binary responsive enough for interactive UI work. The trade-off is
compile time — a release build is significantly slower than a debug build because
LTO and `codegen-units = 1` defeat incremental parallel codegen. This is expected
and normal: use `cargo run -p riff-gui` while iterating and reserve
`cargo build --release -p riff-gui` for producing distributable binaries, which is
exactly the command the release workflow's four legs run.

### Build command

Produce a release binary with:

```bash
cargo build --release -p riff-gui
```

The resulting binary is written to `target/release/` (for example `target/release/riff` on Linux/macOS or `target/release/riff.exe` on Windows).

### Binary characteristics

- **Single standalone binary.** riff links its dependencies statically into one executable; there is no separate runtime or library to install. The Linux build is the one exception, and it is dynamic against `libasound2` only, which the release notes name.
- **One workspace, one shipped artifact.** The workspace splits the backend into capability crates, but only `riff-gui` produces a binary (the `riff` package target); the other members are libraries, so there is no multi-artifact assembly.
- **No feature flags.** Every build includes the same set of capabilities; the only conditional compilation is per-target-OS for system integration (tray icon and native file dialogs on non-Linux platforms).
- **No companion assets.** Fonts and icons are compiled into the executable, so the archives are one file each.
- **Stripped.** Debug symbols are removed, reducing binary size.

### Not automated

Signing, notarization, installers, checksums, and per-platform asset hosting are all absent, and each absence is a documented limitation rather than a gap in the pipeline. `.github/workflows/ci.yml` still owns the quality gate (`fmt`, `clippy`, `test` on Linux and Windows); the release workflow builds and publishes, and assumes the tag points at a commit that already passed CI.
