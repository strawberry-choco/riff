# Background Workers Live Behind App-Layer Service Seams

**Status**: Accepted
**Date**: 2026-08-25

The cover loader and tag-write worker threads are currently spawned inline by `RiffApp::new` (`src/ui/app.rs`), which holds their raw channel ends, their request/result protocols, the cover request-dedup and negative caches, and the post-write store-refresh orchestration. This makes the render layer own thread lifecycle and durability-sensitive persistence ordering, and leaves the Tag Edit save flow with no test surface — no adapter can stand in for the worker, so `app_tests`/`ui_tests` cannot exercise write-then-persist except through real Lofty disk I/O.

We will extract two modules into the application layer, spawned by the composition root exactly like the Audio Engine (ADR precedent: "extract audio engine"):

- **Tag Edit Service** (`TagEditService`, behind a `Box<dyn TagEdits>` handle): `submit(edit)` plus `poll() -> Option<TagEditOutcome>` where the outcome is `Saved` or `Failed { reason }`. The implementation owns the worker thread, the Lofty write, resolving the edited Track from the Application Store through the Library query port, applying the edit, committing through `LibraryMutationStore::apply_tag_refresh` (which bumps the session generation per ADR 0002), and surfacing one combined outcome. The UI maps the outcome onto modal state and status text only.
- **Cover Service** (`CoverService`, behind a `Box<dyn Covers>` handle): `request(track_id, path)` plus `poll() -> Vec<(TrackId, Option<CoverImage>)>`. The implementation owns the worker thread, the resolver chain, request deduplication, and the negative cache with eviction. The UI keeps egui-bound work only: rgba→texture conversion and the texture LRU.

## Considered Options

- **Keep both workers inline in `RiffApp`**: no new code, but the seam stays missing and the tag-edit → store-refresh flow stays untestable.
- **One combined "background jobs" module**: fewer moving parts, but a shallow grab-bag whose interface mentions two unrelated concerns (a read-side cache warmer vs a durability-sensitive write path).
- **Two service modules behind trait handles (chosen)**: each interface is small and honest; mocks become the second adapter at each seam, making the seams real rather than hypothetical.

## Consequences

- `RiffApp::new` loses two inline `thread::spawn`s, four channels, and the cover request/negative-cache fields; its constructor interface shrinks.
- Trait-handle style matches every existing port (`SettingsStore`, `PlaylistStore`, `LibraryMutationStore`); tests inject mock handles.
- The `TagWriteRequest`/`TagWriteResult` structs move out of the UI layer into the service implementation.
- Cover texture caching (GPU textures) deliberately stays on the main thread — the seam falls between decoded image and GPU texture, where egui forces it.
- Extraction order: Tag Edit Service first (durability-sensitive, currently untested path), Cover Service fast-follow on the same pattern.
- Domain language: CONTEXT.md gains **Tag Edit** for the user action this module serves.
