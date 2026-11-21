# N9 queue and Now Playing

Landed. The player bar gains a queue toggle and a Now Playing toggle. The Up
Next panel sits above the player bar with the current row marked, click to
promote, and Clear (stop plus an empty queue). The Now Playing screen carries
the hero cover, title, artist, the year/genre/duration line, a seek slider
gated on a known duration, and Up Next. Close returns to the Library view.

Evidence.

- [x] Unit: `nsh_shell_tests::the_queue_toggle_opens_and_closes_the_up_next_panel`,
      `the_now_playing_toggle_switches_the_stage_and_close_returns`, and
      `nsh_flow_tests::the_queue_promotes_and_clears` (promote and clear).
      Log: `../n16/logs/nsh-suites.log`.
- [x] Live: `../n15/shots/n15-now-playing.png` (playing a fixture track, the
      seek bar at 0:03 of 0:03 and two Up Next rows) and
      `../n15/shots/n15-queue.png` (the panel with six rows and Clear).
- [x] Reduce motion: the screen has no animated element; the playing marker
      is a settled glyph, so the preference's end state is the default.
- [ ] Gap: the live fixture has no artwork, so the hero cover shows the
      placeholder.

Files: `src/screens/queue.rs`, `src/screens/now_playing.rs`, the two toggles
in `src/shell/player_bar.rs`, the shell's queue slot in `src/shell/layout.rs`.
