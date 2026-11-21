# N12 smart playlists

Landed. The SMART LISTS section carries the six built-ins (the last two behind
Advanced mode), the saved custom lists, and a create control. A built-in list
opens as a read-only track listing with Play and Shuffle. A custom list opens
as its listing with Play, Shuffle, Edit rules and Delete. The editor carries
the name, the nested rule tree (all/any per box, add rule, add group to seven
levels, remove), field and comparison cycling from the catalog, a value field
typed by the field's value type, negation, the live match count, unreadable
rows with their delete control, and Save/Back. Save refuses an unreadable
row, an empty tree or a half-filled row with the egui editor's sentences.

Evidence.

- [x] Unit: `rules::tests` round-trips a draft through its tree, blocks a
      half-filled row and an unreadable row, and round-trips a date value;
      `date_picker::tests` pins the date, duration and Today rules.
- [x] The built-in listing is `builtin_stage`; the custom listing reads
      `custom_list_count` and `custom_list_row` through the seam, and the
      pump re-reads the total on either generation.
- [ ] Gap: the editor's live click-through is not captured. The debug
      keybindings that open it are unreachable from the automated drive
      (Windows folds Ctrl+Alt into AltGr), and the headless suite does not
      yet drive the tree's buttons. The tree's correctness is pinned by the
      unit tests; a live screenshot is the follow-up.

Files: `src/screens/smart_playlists/{mod,editor,rules,date_picker}.rs`, the
sidebar rows in `src/shell/sidebar.rs`, the custom-list total in
`src/bridge/pump.rs`.
