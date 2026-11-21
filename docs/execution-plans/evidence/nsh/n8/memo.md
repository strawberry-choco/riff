# N8 menus and actions

Landed. The track menu is the egui components crate's item list over nsh's
native popup: Play, Play Next, Add to Queue, Add to Playlist (one row per
user playlist, or the disabled "No playlists yet"), Add to Favorites / Remove
from Favorites, Remove from Playlist (playlist rows), Edit Tags and Measure
ReplayGain (resolved tracks). Collection menus carry Play, Play Next, Add to
Queue and Shuffle. A double press plays a row.

Evidence.

- [x] Unit: `cargo test -p riff-tests nsh_` green, log in
      `../n16/logs/nsh-suites.log`. The double press is
      `nsh_shell_tests::a_double_press_plays_the_row`; the collection batch
      resolution is `menus::CollectionTarget::track_ids`, exercised through
      the genre/artist walks in `nsh_shell_tests`.
- [x] Live: the shell screenshot `../n15/shots/n15-shell.png` and the playing
      shot `../n15/shots/n15-playing.png` show the rows the menus attach to.
      The menus themselves are native platform popups (`popup_menu_at`), so
      `PrintWindow` does not capture an open menu; their items are built from
      the components crate's list and dispatched through the transport and
      store ports.
- [x] Favourite: `set_favorite` writes `set_track_favorite`; the committed
      mutation bumps the generation the pump answers, so no rescan is
      involved.
- [ ] Gap: a headless press cannot open a native popup, so item selection is
      not driven by a test. The handlers are the same functions the row heart
      and the transport use, and the flow suite drives those.

Files: `src/menus/{mod,track,collection}.rs`, the row's press and context
handlers in `src/components/track_row.rs`, `src/screens/library/browse.rs` and
`src/screens/library/folders.rs`.
