# The UI Reads the Store Through Session Projections

**Status**: Accepted
**Date**: 2026-08-22

With SQLite as the authoritative Application Store, the UI must stop treating `AppState` as the owner of the full Library. Views read through application-layer store/query ports into small bounded in-memory results (**Session Projections**), invalidate them by a Store generation counter after writes, and render last-known data while reloading. Writes commit to the Store first, then refresh affected projections. A single SQLite connection guarded by a mutex is acceptable initially because egui frame work reads projections rather than issuing arbitrary SQL while holding it.

## Considered Options

- **Keep the full library resident in memory** and merely mirror it to SQLite: preserves today’s UI code but leaves two competing authorities.
- **Query SQLite synchronously from widgets every frame**: always-fresh data but exposes UI rendering to database and lock latency.
- **Session projections with generation-based invalidation (chosen)**: responsive frames, one persistent authority, and bounded reload cost.

## Consequences

- `AppState` shrinks to playback/session/UI concerns plus current projections; repository methods define the view shapes the UI needs.
- Stale reads are possible only between a committed write and the corresponding projection refresh, which generation invalidation makes explicit.
- Lock hold times and result-set sizes become deliberate API constraints rather than incidental implementation details.
- The generation counter is session-local and resets on launch; projections compare their loaded generation against the Store’s current in-memory generation.

## Amendment (2026-09-06)

The Session Projection read seam was deepened along the lines this record already
describes; the decision itself stands.

- **Per-projection modules**: what had been one large projection module holding every
  Session Projection type is now one small module per projection. Each projection owns
  its generation-keyed caches and its staleness contract, so a maintainer reads one
  file to understand one view's caching. The split is internal to the library
  capability: the module name and every import path are unchanged, and the UI still
  holds only `SessionViews`.
- **One canonical staleness contract**: the generation-keyed cache each Session
  Projection rides on exists exactly once, in the Application Store contract beside
  the generation counter it keys on. Its `peek` answers the stale-but-present value
  regardless of epoch — freshness is a caller-side check, explicit at every read that
  needs it. Stale-but-present reads, the bug class this contract prevents, now have a
  single audited implementation instead of one near-identical copy per crate.
- **Preserved façade**: `SessionViews` remains the UI's single read interface with the
  warn-and-default error policy — store failures render defaults (or last-good rows
  where a projection already holds them), and the UI never sees a `Result`. Each
  projection's staleness contract is proven through that façade by per-projection
  tests, independent of how the projections are laid out internally.

## Amendment (2026-09-22)

The decision above stands. One of its claims was factually wrong when it was written,
and becomes true only now, so it is corrected here rather than left as a description of
an intention.

- **"A single audited implementation" is now actual.** The 2026-09-06 amendment said
  the staleness contract had one implementation instead of one copy per crate. It did
  not: the procedure — observe the generation, serve the cached value while it is
  current, otherwise load and commit — was hand-copied at every cached level across the
  projection modules in six different spellings, and the eight paged reads above the
  projections stayed fresh by comparing two *separate* observations of the counter, a
  guard that can report "unchanged" across a write that did land. Both duplications are
  gone: `GenerationCache::level` (`riff-persistence`, `levels.rs`) is the one
  implementation of that procedure for a cached level, and a paged listing reads one
  **Listing Page** — its total and its visible window taken under a single connection
  acquisition, stamped with the generation captured inside it — so there is no gap for a
  torn count to fall through and no epoch value for a caller to mis-time.
- **Freshness is no longer a caller-side check.** `GenerationCache::peek` remains the
  epoch-agnostic read that the last-good error fallbacks use, but it is no longer
  documented as the trap every caller must guard: guarding is what `level` does, and the
  remaining direct `peek` reads are deliberate stale-but-present fallbacks, not
  half-written freshness checks.
- **Unchanged by this**: the generation counter stays session-local and in memory; the
  per-projection module layout stays; `SessionViews` stays the UI's single read seam with
  the same method signatures; and the two playback caches keep their hand-written bodies
  — they were designed to adopt `level` and are not migrated in this change.