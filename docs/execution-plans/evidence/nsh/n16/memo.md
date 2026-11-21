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
      The release notes' table and per-platform text describe the archive
      shape.
- [ ] Gap, environment: this machine could not download the nsh CLI.
      `nsh.dev/install.ps1` and the GitHub release zip both failed
      (connection reset, then a 120 s timeout). `nsh package` therefore was
      not run locally, and no installer/artifact was produced or installed.
      CI carries the pinned install and the packaging steps; the P0 memo
      proves the output paths on Windows (`nsh 0.6.1`, `.nsh/out/riff/riff.exe`,
      `riff.zip`, `riff.iss`).
- [ ] Gap: the release runner moved from ubuntu-22.04 to 24.04 for GTK 4.14.
      The glibc 2.39 claim behind that move is still unverified, as the plan's
      risk section records.
- [ ] Gap: `docs/engineering/golden-image-testing.md` is deleted; no pixel
      baselines exist.

Files: `tests/nsh_flow_tests.rs`, `tests/nsh_perf_fixture.rs`,
`tests/mod.rs`, `.github/workflows/release.yml`, the deleted golden doc.
