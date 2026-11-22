# N16 tests and artifacts

Landed in code and CI. The egui suites are replaced by the nsh bridge, shell
and flow suites; the perf fixture exists and is ignored by default; the
release workflow installs the pinned nsh CLI and packages through
`nsh package` instead of the hand-rolled binaries and dmg.

Evidence.

- [x] Flow suite: `nsh_flow_tests` drives browse-adjacent play, the queue,
      playlist create/rename/delete, the tag editor, and the settings
      toggle. Log: `logs/nsh-suites.log` (39 passed, 5 ignored; the ignored
      five are the perf fixture and the pre-existing N4 ignition/geometry
      probes).
- [x] Browse, search and the drill-downs stay covered by
      `nsh_shell_tests` (`the_artists_walk_lands_on_albums_then_tracks`,
      `the_search_filters_the_flat_list_and_escape_restores`, and the rest).
- [x] Perf fixture: `logs/perf-fixture.log` prints a 50k fixture build of
      78 ms, a cold count+first row under 1 ms, 300 warm rows at
      0.001 ms/row, and 500 thumbnail covers at 6.0 MB RGBA in 3 ms. RSS is
      the live process's number; the in-process fixture does not read it.
- [x] README screenshots: `README.md` carries no image references, so there
      is nothing to regenerate. The live shots for this phase are under
      `shots/`.
- [x] Packaging shape: `.github/workflows/release.yml` installs
      `NSH_VERSION=v0.6.1` and runs `nsh package --target {windows,linux,macos}`
      from `crates/riff-gui-nsh`, then archives `.nsh/out/riff.zip` (Windows),
      `.nsh/out/riff.tar.gz` (Linux) and a zipped `.nsh/out/riff.app` (macOS).
      The install steps set `NSH_INSTALL_DIR` and append its `bin` to
      `$GITHUB_PATH` (the installers only patch the user PATH, so without this
      the packaging steps could not see `nsh`), and each step asserts
      `nsh --version` before moving on. The release notes' table and
      per-platform text describe the archive shape.
- [x] Packaging, Windows, run locally: the CLI was restored from the cached
      P0 zip (`.scratch/nsh-p0/nsh-windows-x86_64.zip`, SHA256 matches the P0
      memo) into `.scratch/nsh-cli/nsh/bin/nsh.exe` (`nsh 0.6.1`). Run from
      `crates/riff-gui-nsh`: `nsh package --target windows -v` built the crate
      (3.7 s) and the Windows runner (3 m 47 s), then wrote `.nsh/out/riff`
      and `.nsh/out/riff.zip` (2.7 s). Artifact: `riff.zip` 17,541,580 bytes
      (SHA256 `E01C4909…5A01A740`), `riff.exe` 41,293,312 bytes plus
      `nsh/assets/@riff-gui-nsh/{brand,fonts,icons}`. The packaged exe opened
      its window in 223 ms against the N3 fixture store and rendered the shell
      (`shots/n16-packaged-shell.png`, launched by `tools/packaged-drive.ps1`);
      log: `logs/package-windows.log`. Inno Setup 6 is not installed here, so
      `riff.iss` was written but no `Setup.exe` was compiled — the workflow
      archives the portable zip, not the installer.
- [ ] Gap: the release runner moved from ubuntu-22.04 to 24.04 for GTK 4.14.
      The glibc 2.39 claim behind that move is still unverified, as the plan's
      risk section records.
- [ ] Gap: `docs/engineering/golden-image-testing.md` is deleted; no pixel
      baselines exist.

Files: `tests/nsh_flow_tests.rs`, `tests/nsh_perf_fixture.rs`,
`tests/mod.rs`, `.github/workflows/release.yml`, `tools/packaged-drive.ps1`,
the deleted golden doc.
