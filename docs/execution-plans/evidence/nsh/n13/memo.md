# N13 settings

Landed. The sidebar's Settings control opens the five pages. Library: the
registered paths with readiness and watch state, typed add, Browse, per-path
Rescan and Remove, Rescan all, and Clear Library with its confirmation.
Playback: ReplayGain and its Track/Album mode. Appearance: Dark, Light,
System, both high-contrast variants, and Reduce Motion. Advanced: Advanced
mode, system notifications, quit on close, thumbnail cache clear with its
polled outcome, and the library-wide ReplayGain pass with its two value
toggles, force, progress and report. About: version, offline statement,
licence and third-party notices.

Evidence.

- [x] Flow: `nsh_flow_tests::the_settings_toggle_writes_the_session` opens
      Settings, Advanced, flips Advanced mode and reads the session flag back.
      Log: `../n16/logs/nsh-suites.log`.
- [x] Live: `../n16/shots/n16-settings-{library,playback,appearance,advanced,about}.png`
      (library and advanced are also in `../n15/shots/`).
- [x] Persistence: the pump's frame-end `commit_if_changed` is the one writer;
      the N4 shell test pins it.
- [ ] Appearance mode is session state. The persisted scalar row has no
      light/dark/system column (the egui store did not either); the
      high-contrast flag is persisted and restored at launch.
- [ ] Gap: Clear Library, cache clear and the pass are wired to their ports
      but were not exercised on the live fixture (a library wipe would destroy
      the evidence store).

Files: `src/screens/settings/{mod,library,playback,appearance,advanced,about}.rs`,
the settings entry in `src/shell/sidebar.rs`, the pump's path views and
service polls in `src/bridge/pump.rs`.
