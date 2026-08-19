# 09 — Land cache hardening and mock-based infra tests

**What to build:** The library cache degrades honestly instead of failing silently: it carries a schema version, a mismatch falls back to an empty library with a user-visible explanation, users can clear the cache from settings, and the infra layer is tested through its port traits with mocks rather than construction-only smoke tests. All three pieces already exist in the working tree; verify them, fix any gaps, and commit.

**Blocked by:** 01 — Baseline green gate.

**Status:** ready-for-agent

- [ ] The library cache file carries a schema version; loading checks it, and on mismatch the app logs a warning, falls back to an empty library, and surfaces a user-visible explanation
- [ ] Settings offer a "Clear Library Cache" action that deletes the cache, confirms the outcome in the UI, and the cache rebuilds on the next scan
- [ ] Infra-layer tests exercise real behavior through the port traits (decoder, audio output, metadata reader, cover loader) using mocks with controlled error injection — at least five meaningful behavior tests, not just construction
- [ ] Cache save/load round-trip with the version field is covered by a test
- [ ] Feature committed
