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