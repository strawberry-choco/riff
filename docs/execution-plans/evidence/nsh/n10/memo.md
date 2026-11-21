# N10 playlists

Landed. The sidebar has a PLAYLISTS create control with an inline name row,
and the playlist stage carries the name and count, Play, Shuffle, Rename
(inline field), Delete (confirmation band), the entry list, and missing
entries struck through with `(missing)` and skipped by playback. A drag on a
row moves it to a new index and commits one `reorder_playlist_entries`
transaction. The built-in name rule and the store's sidebar-wide name rule
both report their own wording.

Evidence.

- [x] Unit: `nsh_flow_tests::a_playlist_is_created_renamed_and_deleted` drives
      the sidebar create row, the rename band and the delete confirmation
      over a mutating store. Log: `../n16/logs/nsh-suites.log`.
- [x] Reorder: the row's `on_pan_end` computes `translation.y / ROW_H` and
      calls `screens::playlists::reorder`, which reads the current order from
      the seam and writes it in one transaction.
- [ ] Live: no screenshot of the playlist stage. The create control's click
      coordinate did not land in the frame the drive captured; the stage is
      covered by the flow test above. Missing rows are covered by the render
      path (`line_through` plus `(missing)`) and are not seeded in the live
      fixture.
- [ ] Restart survival: the store transaction is the same one the egui
      frontend used; a restart proof needs a live run, not a headless one.

Files: `src/screens/playlists.rs`, the create row and header in
`src/shell/sidebar.rs`, the detail build in `src/model.rs` and
`src/bridge/pump.rs`.
