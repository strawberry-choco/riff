# Release and Packaging

This document describes how riff is built for distribution. For the development build workflow, see [development-setup.md](./development-setup.md); for the feature catalog the release ships, see [../product/features.md](../product/features.md).

riff is a Cargo workspace that compiles to a single standalone binary (the `riff` binary in `riff-gui`). It has no feature flags, no runtime plugin system, and no external assets that must ship alongside the executable, which is what makes packaging a matter of building one binary per platform and wrapping it in the thinnest container each platform's users expect. There is no installer and no code signing, and those omissions are deliberate at this stage rather than oversights: they are the same limitations the release notes state to users. Windows ships the bare `riff.exe`; macOS ships a `.dmg` holding a minimal, ad-hoc-signed `.app` bundle with the brand icon.

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
| `macos-15` | `macos-arm64` | `dmg` | `riff.app` (brand icon) + `/Applications` symlink |
| `macos-15-intel` | `macos-x86_64` | `dmg` | `riff.app` (brand icon) + `/Applications` symlink |
| `ubuntu-22.04` | `linux-x86_64` | `tar.gz` | `riff` |
| `windows-latest` | `windows-x86_64` | `exe` | `riff.exe`, bare (no zip) |

The two macOS labels are pinned to currently-GA images, and keeping them current is a maintenance task rather than a preference. GitHub supports the latest two macOS releases and deprecates the rest, which means a deprecated image goes through announced brownouts and then disappears. Because `release` needs all four legs, one retired label does not degrade the release — it blocks it, with no partial publish. `macos-13` has already been removed from `actions/runner-images` and `macos-14` is badged deprecated, so an earlier version of this table named two runners that could not have run at all. When bumping, check the current table in `actions/runner-images`, and do not collapse both macOS legs to `macos-latest`: that label is arm64, so it would silently drop the Intel build while the artifact name still claimed `macos-x86_64`. Intel needs the explicit `-intel` label.

The published names are `riff-<tag>-<slug>.<ext>` where the extension follows the platform — `.dmg` on macOS, `.exe` on Windows, `.tar.gz` on Linux — for example `riff-v0.1.0-linux-x86_64.tar.gz` or `riff-v0.1.0-windows-x86_64.exe`.

**The executable carries its own fonts and icons; the containers carry nothing extra.** The Inter faces are `include_bytes!`-ed and the Lucide icons `include_str!`-ed by `crates/riff-gui/src/ui/fonts.rs` and `crates/riff-gui/src/ui/icons.rs`, so no assets folder is needed. The Linux leg uses `tar -czf` with a member name, which keeps the archive flat at one entry. The Windows leg copies `target/release/riff.exe` into `dist/` under the published name and uploads it directly — GitHub release assets must be single files, and the `.exe` already is one, so there is no zip wrapper; the `.exe` suffix matters, and a Unix path there would package nothing at all.

The macOS leg assembles a minimal `.app` bundle around the binary: `Contents/MacOS/riff`, an `Info.plist` (identifier `io.github.strawberry-choco.riff`, version taken from the tag), and the brand icon `crates/riff-gui/assets/brand/riff.icns`. The bundle is ad-hoc signed (`codesign -s -`), which is what an arm64 Mach-O needs to run at all; it is not identified with Apple, so Gatekeeper still quarantines the download — the release notes carry the instructions. The image is built with `hdiutil create -format UDZO` and holds the app plus an `/Applications` symlink for the drag-to-install gesture. The icon is the packaging copy of the mark the app generates at runtime: `mark_svg()` in `crates/riff-gui/src/ui/chrome.rs` derives four equalizer bars from `WORDMARK_BARS`, and `assets/brand/mark.svg` is the same geometry as a file (source for regenerating the `.icns` via `iconutil`). If the in-app mark changes, regenerate both — the two must not drift.

`if-no-files-found: error` on the upload step is the backstop: a packaging step that silently produced nothing must fail the leg rather than publish a zero-byte download.

### Why `build` and `release` are two jobs

`build` only ever produces artifacts. `release` is a separate job with `needs: [build]` that downloads them and creates the GitHub release. A single matrix job that also published would have a race built into it: the first leg to finish would create the release and the other three would fail with "release already exists", so the outcome would depend on which platform's runner happened to be fastest. Splitting them means the four legs cannot interfere with each other, and a build failure simply stops the publish.

`release` runs on `ubuntu-latest` and downloads the four artifacts into one flat staging directory with `merge-multiple: true` — correct precisely because the four names are distinct, so flattening cannot collide. It then checks that all four expected files are present (naming the directory listing on failure) before calling `gh release create` with the four paths. That check is what turns a stale list of targets into a clear error rather than a release quietly missing a platform.

The job never checks the repository out, so `GH_REPO` is set explicitly: `gh` cannot infer the target repository from a git remote that is not there.

### Token scope

The workflow-level default is `permissions: contents: read`. Exactly one job — `release` — widens it to `contents: write`, because `gh release create` is the only write in the whole pipeline. The build matrix compiles a large dependency graph and runs its build scripts; it has no write access at all, so a compromised dependency cannot push to the repository or cut a release.

### Release notes and the pre-release policy

The "what changed" part of the release notes is generated in a dedicated `notes` job by git-cliff, from the tag range and `cliff.toml` — the commit log is the only source of the change list, and there is no separate changelog file to keep in sync with it. The rest of the notes stay a literal template in the workflow, and the reason is that the hand-written parts have a job the commit log does not: they must state, per platform, what the artifact actually is. The Windows caveat (a bare unsigned `riff.exe`, SmartScreen, and the note that an NSIS installer would hit the same wall), the macOS caveat (an ad-hoc-signed but unidentified `riff.app` in a `.dmg`, so Gatekeeper blocks the first launch until quarantine is cleared — `xattr -dr` or System Settings → Privacy & Security → Open Anyway, since the right-click → Open bypass is gone on Sequoia and later), and the Linux caveat (dynamically linked against `libasound2`) are written into the workflow so they cannot drift from what the pipeline produces — no commit message can answer what the artifact actually is on each platform.

**Every release is published as a GitHub pre-release**, unconditionally, whatever the tag says. riff is pre-1.0 and the binaries are unsigned — no identified Apple signature, no Windows Authenticode certificate; promoting a release out of pre-release is a deliberate manual act, not a side effect of a tag. The notes also state that this is the first public release and that riff is offline-only by design.

## Cutting a release

1. Set the version in the root `Cargo.toml` `[workspace.package]` block. The seven member manifests inherit it and must not be edited.
2. Merge to `master` and let CI pass. The release workflow does not re-run the quality gate, so a red `master` will not be caught by tagging it.
3. Push the tag: `git push origin v<version>`. The `verify` job fails in under a minute if the tag and the workspace version disagree, which is the intended moment to notice a version bump that was forgotten. The `notes` job then renders the release notes' change section from the commit range `v0.1.0..<tag>` with `cliff.toml`; an empty section — no conventional commits in the range — fails the release before the build matrix, because a release with no change entries is a tagging mistake rather than notes to publish.
4. When the four artifacts are published, add the download badges to `README.md`. This is deliberately **not** automated: the badges are a claim that a real release exists, and a workflow that rewrote the README on a failed run would leave a claim behind with nothing to back it.

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
- **No companion assets.** Fonts and icons are compiled into the executable. The one deliberate addition is the macOS app bundle's brand icon (`assets/brand/riff.icns`), which is packaging, not a runtime asset.
- **Stripped.** Debug symbols are removed, reducing binary size.

### Not automated

Signing (an identified Apple Developer ID plus notarization, or a Windows Authenticode certificate), installers, checksums, and per-platform asset hosting are all absent, and each absence is a documented limitation rather than a gap in the pipeline. `.github/workflows/ci.yml` still owns the quality gate (`fmt`, `clippy`, `test` on Linux and Windows); the release workflow builds and publishes, and assumes the tag points at a commit that already passed CI.
