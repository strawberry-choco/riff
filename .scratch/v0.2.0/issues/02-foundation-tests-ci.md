# 02 — Land Phase 0: test harness, CI, and coverage

**What to build:** Verified proof that the foundation holds: the integration test crate can exercise every layer of the app, the CI pipeline gates pushes on format, lint, and tests on both Linux and Windows, and the domain/app test coverage reaches the depth the plan called for. Anything missing is added; everything is committed as the v0.2.0 foundation.

**Blocked by:** 01 — Baseline green gate.

**Status:** done

- [ ] The integration test crate exercises domain, app, infra, and ui layers and compiles cleanly from a fresh checkout
- [ ] Domain tests cover queue edge cases (empty queue, single track, wrap-around, repeat-one, repeat-all, shuffle preserves the track multiset), playback state transitions, TrackId derivation/equality, and repeat-mode cycling — roughly twenty or more domain tests
- [ ] Library manager tests cover add/remove indexing, multi-path deduplication, unavailable-path handling, search across all metadata fields, cache save/load round-trip, and corrupt-cache fallback using temporary directories
- [ ] The CI workflow runs fmt + clippy + test on both Linux and Windows runners, on push to main and on pull requests, and is green
- [ ] Foundation committed as a focused commit (harness, CI workflow, test additions)

