# No Write-Side SessionStore Facade

**Status**: Accepted
**Date**: 2026-08-26

An architecture review proposed mirroring `SessionViews` on the write side: a single
`SessionStore` module absorbing the three store-mutation ports (`SettingsStore`,
`PlaylistStore`, `LibraryMutationStore`) behind one handle, "symmetric to reads".

Rejected. A facade only earns its place when there is behaviour to hide behind the
smaller interface — otherwise it fails the deletion test: deleting it just moves its
pass-through body back into callers, losing nothing.

At the time of this decision there is no such behaviour left to absorb:

- The SQLite store locks itself and bumps both session generations
  (`StoreGeneration` for Library, the dedicated playlist generation) **inside its own
  mutation impls** (landed in `f10c2e4`, "delete mutex store pass-through adapters").
  There is no mutation adapter layer for a facade to own.
- Each of the three ports is already a deep module: its interface carries real,
  documented semantics (one immediate durable transaction per mutation, dangling
  playlist references surviving by product behaviour per ADR 0001, play-history
  preservation rules). Merging deep interfaces under one name widens the interface
  callers must learn without hiding anything.
- Cross-port write flows that might someday justify a seam are already absorbed by
  the service that needs them — e.g. the Tag Edit flow composes file-tag writing with
  `apply_tag_refresh` inside `TagEditService` (ADR 0006), not in the UI.

## Considered Options

- **SessionStore facade over the three ports (rejected)**: symmetric-looking, but a
  pure pass-through today — shallow module, negative depth.
- **Keep the three ports as-is (chosen)**: each is deep, individually mockable, and
  generation invalidation is already local to the store implementation.

## Consequences

- Future architecture reviews should not re-propose a write-side facade unless a real
  cross-port invariant appears — a change that must span Settings + Playlists +
  Library mutations as one durable or ordered unit. That would be the behaviour worth
  hiding, and the facade question can be reopened then.
- UI code continues to hold the three boxed ports directly alongside `SessionViews`.
