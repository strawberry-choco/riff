# N11 tag editor

Landed. The inspector's TAGS section gains an edit control; the track menu's
Edit Tags opens the same editor. The form carries the eleven values the egui
editor edited (title, artist, album, album artist, genre, year, track number,
the four ReplayGain facts). Save track submits one `TagEditRequest`; Save
album submits one per track whose values the draft changes. The album
aggregate blanks a field the tracks disagree on and marks it `(different)`;
an unset field shows `(none)`. Status is polled and a failure reads as
`<path>: <reason>`, so the file is named.

Evidence.

- [x] Unit: `model::tests::the_album_draft_marks_what_the_tracks_disagree_on`,
      `the_draft_writes_only_what_it_carries`, and
      `a_draft_is_dirty_only_when_a_field_moves`.
- [x] Flow: `nsh_flow_tests::a_tag_edit_submits_one_request` opens the editor,
      types a title, saves, and reads one recorded request back from
      `FakeTagEdits`. Log: `../n16/logs/nsh-suites.log`.
- [x] Album save keeps its count in the status line and reports each failure
      with the file path.
- [ ] Read-only files: the refusal is the writer's own error text; no
      read-only fixture exists in the live run.

Files: `src/screens/inspector/tag_editor.rs`, the TAGS section in
`src/screens/inspector.rs`, the draft in `src/model.rs`, the signals in
`src/bridge/state.rs`.
