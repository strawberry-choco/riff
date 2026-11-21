# N14 platform integration

Landed.

- **Folder picker.** `pick_paths(OpenDialog::folders().multiple(true))` via
  nsh, awaited in a task. A platform whose dialog errors lands on the typed
  path in Settings -> Library and says so.
- **Drop.** The shell root listens to `on_file_drop`; `file:` URIs are
  percent-decoded to local paths. The acceptance is the egui rule: the path
  must exist and be a directory, the canonical form registers, a re-drop
  never rescans, new roots are scanned at drop time, and refusals win the
  status line (`Added library path, scanning...`, then the last refusal).
- **Window state.** `close_on_press: false`; the close handler quits when
  "Quit on close" is set and hides to the tray otherwise. The tray's Show
  and a tray click show the window again.
- **Tray.** `set_tray` with the brand icon, the tooltip, Show riff,
  Play/Pause, Next, Previous and Quit. Handlers dispatch through the UI
  transport.
- **Notifications.** Every new typed notice raises one silent OS notification
  while "System notifications" is on; `show_notification` no-ops where the
  platform cannot show one.
- **Accessibility.** Every control carries a label and the interactive rows
  carry their selected/checked state.

Evidence.

- [x] Unit: `platform::library_paths::tests::a_file_uri_decodes_to_a_path`.
- [x] Live: the N7 evidence's folder tree and the N4 window-geometry tests
      cover the pieces that predate this unit; the picker and drop paths are
      wired but not driven on the live fixture.
- [ ] Gap: the tray and close-to-hide are the P0 gates that stayed partial
      (`on_click` INCONCLUSIVE). The code is installed and the attribute set
      is applied; a human pass is still the proof.
- [ ] Gap: OS notifications and the native picker need an interactive
      desktop; neither is asserted headlessly.

Files: `src/platform/{mod,library_paths,window_state,tray,notifications}.rs`,
the drop listener in `src/shell/layout.rs`, the Add folder and Browse
controls in `src/shell/sidebar.rs` and `src/screens/settings/library.rs`.
