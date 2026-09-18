# Inline Tag Editor in the Detail Panel

**Status**: Proposed
**Date**: 2026-09-16

Tag editing today is a modal reachable only from a single context-menu item: "Edit Tags" on a Track row, behind the advanced toggle (REQ-UI-006). The modal ([`crate::ui::prompts::tag_edit_modal`](../../crates/riff-gui/src/ui/prompts.rs)) edits one Track at a time across seven fields (Title, Artist, Album, Album Artist, Genre, Year, Track Number), submits one [`TagEditRequest`](../../crates/riff-backend/src/app/tag_edit_service.rs) per save, and reports one [`TagEditOutcome`](../../crates/riff-backend/src/app/tag_edit_service.rs) — file tags and Store facts as one durable change (ADR 0006). The readout panels already summarize some tag data (Artist / Album / Genre rows) but are strictly display-only, and the Album readout aggregates nothing: it shows only the album-level year and genre plus aggregate plays and path.

## Decision

Replace the Edit Tags modal with an **Inline Tag Editor** inside the Detail Panel (the right-column selection readout). The panel gains a tag section of exactly the seven modal fields; on the Album readout each field is **aggregated** across every Track of the Album. Editing happens in place with a Save bar underneath, and saving on the Album readout applies the edit to **every Track of the Album**.

Agreed rules:

- **Tag section field set**: exactly the fields the modal could edit — Title, Artist, Album, Album Artist, Genre, Year, Track Number — in that order. Duration ("length") is derived data, not an editable tag, and is not included.
- **Track readout**: the tag section replaces the redundant Artist / Album / Genre detail rows. Plays, Last played, and Path stay as detail rows below.
- **Album readout**: the tag section replaces the Released / Genre detail rows. Tracks · total time, Plays, Last played, and Path stay.
- **Aggregation (Tag Aggregation)**: one row per field. All Tracks share the value → the value as-is. Values differ → orange `(different)` text. No Track carries the value → grey `(none)` text (an em-dash was also considered; the grey `(none)` label was chosen for scannability). The rule applies per field; a field like Year that is missing on some Tracks and present on others renders `(different)`, never a partial value.
- **Album editing (Batch Tag Edit)**: editing an aggregated row and saving submits one `TagEditRequest` per album Track through the existing Tag Edit service (N durable changes, serialized by the worker). Partial failure is reported, never silent.
- **Commit model**: inline fields with a Save bar; **Enter** saves, **Esc** cancels, Tab moves between fields; Save is disabled while a write is in flight; switching the selection mid-edit discards the draft (the modal's Cancel semantics, kept).
- **Album save payload (dirty-only)**: only fields whose typed text differs from the row's original displayed value are submitted (the `TagEdit` contract "only `Some` fields are written" applies directly). A `(different)` row left with an empty input is treated as untouched and skipped — a shared field cannot be blanked across the Album by accident; clearing a shared tag stays a single-Track action. Save is disabled when nothing is dirty, so an empty batch submits nothing and writes no files.
- **Batch outcome surfacing**: the Save bar shows a spinner while any request is in flight; when all outcomes land, a line under the bar reports "Saved N of M tracks", or "Saved N of M tracks — k failed: <reason>" (first failure reason, orange). The titlebar status line keeps its per-request outcome reporting.
- **Background completion**: requests already submitted keep completing even if the selection moves away; each outcome lands in the titlebar status line as today, whether or not the Detail Panel still shows that Track or Album. The editor simply stops rendering when its selection leaves; it is never resurrected.
- **Modal retirement**: the Edit Tags modal is deleted — the modal state struct, the `prompts.rs` modal seam, and its golden image. The context-menu item becomes an entry point into the inline editor (select the Track and focus the editor).
- **Editing is un-gated**: tag editing is available to every user, not behind the advanced toggle. The advanced-only gating of the "Edit Tags" context-menu item (REQ-UI-006) is removed; the tag section in the Detail Panel is editable for all.

The single durable change per `TagEditRequest` (ADR 0006) is unchanged; a batch save is deliberately N durable changes, not one atomic transaction, so a mid-batch failure leaves earlier Tracks saved and is reported as a partial failure.

## Considered Options

- **Keep the modal, add a read-only tag display (rejected)**: two tag-editing surfaces for one feature, and no path from seeing a `(different)` Album field to fixing it. The modal stays the single entry point only in name; the display and the edit surface drift apart.
- **Album readout display-only; edit Track by Track (rejected)**: the whole point of surfacing aggregation is to fix a wrong Year or Genre in one gesture. Making `(different)` a dead end forces the user to enter each Track's readout.
- **Album save writes every field to every Track (rejected)**: rewrites untouched shared values, and an empty `(different)` input would silently blank a field on every Track. Edits instead submit only what the user actually changed (the `TagEdit` contract "only `Some` fields are written" supports this directly).
- **Chosen**: inline seven-field editor on both readouts, aggregation rules above, batch save through one request per Track.
- **Duration as an aggregated tag row (rejected)**: it is derived, not editable; a multi-Track Album would render `(different)` nearly always, which is noise.

## Consequences

- `TagEditState` and the `prompts.rs::tag_edit_modal` widget are deleted; the tag editor moves into the Detail Panel and is owned by the rendered selection (TrackId / Album identity), with a per-selection draft.
- The Detail Panel widget seam ([`crate::ui::selection`](../../crates/riff-gui/src/ui/selection.rs)) gains tag-row rendering modes (value / edit field / `(different)` / `(none)`) and a Save bar; `app.rs` resolves per-field aggregation from the Session Views and owns the batch submission loop.
- Golden images re-baselined: `selection_panel_*`, `elastic_*_inspector_*` change; `tag_edit_modal_dark` is deleted. The tag-edit outcome already surfaces through the titlebar status line (`apply_tag_edit_outcome`), so outcome plumbing reuses an existing surface.
- Advanced-product policy changes: the advanced-only gating of tag editing (REQ-UI-006) is removed; editing and its display are available to all users. New user-facing copy is introduced for partial batch failure ("Saved N of M tracks — k failed: <reason>") and for missing values (grey `(none)`).