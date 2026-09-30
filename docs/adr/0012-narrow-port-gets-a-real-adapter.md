# A Narrow Port Gets a Real Adapter, Not a Blanket Impl

**Status**: Accepted
**Date**: 2026-09-29

`PlaybackLibrary` (`crates/riff-playback/src/infra/ports.rs`) is a two-method port — `get_track` and `library_track_ids` — that the Audio Engine reads and nothing else. It has no adapter of its own. Instead `ports.rs:47` carries `impl<T: LibraryQueryStore> PlaybackLibrary for T`, so every `LibraryQueryStore` in the workspace *is* a `PlaybackLibrary`, and the narrowing is expressed only by which methods the engine is allowed to call.

That is a hypothetical seam, and it has a measured cost. Because the blanket impl is the only way to satisfy the port, any fake of the engine's dependency must implement the whole 35-method `LibraryQueryStore`. The workspace test suite does exactly that: `tests/app_tests.rs:4359-4603` is 245 lines and 35 methods to exercise a two-method port, and it re-derives the store's canonical path-ascending ordering (`:4387-4396`) that `riff-infra/tests/store_tests.rs` already pins against real SQLite. Meanwhile the two-method fake that already exists *inside* the crate (`crates/riff-playback/src/infra/audio_engine.rs:683`) cannot be reused from the suite, because a `FakeLibrary` that is not a `LibraryQueryStore` is not a `PlaybackLibrary`.

We drop the blanket impl and give the port a real adapter. `StorePlaybackLibrary` lives in `riff-infra` — an item belongs there iff it implements a port defined in another crate, per [ADR 0009](0009-vertical-crate-split-of-the-backend.md) — and the Composition Root constructs it around the store. The seam then has two adapters, one on each side: the store in production, a two-method fake in tests. That is what makes the seam real rather than notional.

## Considered Options

- **Keep the blanket impl (rejected)**: it is genuinely convenient and costs nothing at the Composition Root, which is its only production user. But it leaves the port unfakeable, so every test of anything sitting above it inherits 35 methods. The narrowing the port exists to express cannot be observed from outside the crate, which is the one thing it was written to do.
- **Move the adapter into `riff-playback` as a newtype over `&dyn LibraryQueryStore` (rejected)**: it would sit closer to the port it implements, but `riff-playback` would name a store port it cannot implement — the inverse of ADR 0009's membership rule — and a production adapter would live in the capability slice rather than the adapter crate.
- **Widen `PlaybackLibrary` to mirror `LibraryQueryStore` (rejected)**: the two methods are exactly the two reads the engine makes. Widening would delete the narrowing this port was written to make, and return the dependency to the 35-method shape.

## Consequences

- The Composition Root constructs one more value. It is the only production site that relied on the blanket impl.
- `tests/app_tests.rs:4359-4603` collapses from 245 lines and 35 methods to two, and the re-derived ordering it carried is deleted with it.
- `tests/app_tests.rs:4670-4687` hand-rolls the PlaybackCoordinator's post-`TrackEnded` step because `main.rs`'s update loop is not a callable module. That copy is a liability — it is the same logic the coordinator tests already cover — and it goes when the coordinator becomes drivable without a thread.
- **The other seven `LibraryQueryStore` fakes in the suite are not affected.** They back SessionViews tests that need real rows and counts to answer a paged listing. Do not assume this decision removes them; they are load-bearing and are a separate piece of work.
- **Future reviews should not re-add a blanket impl as an ergonomic convenience.** It will look like a tidy simplification, and it silently converts a real seam back into a hypothetical one — which is precisely the cost every one of the eight fakes has been paying.
