# 001: Offline-First Design

**Status**: Accepted
**Date**: 2026-07-31

## Context

Many music players are service clients with playback as an afterthought: they require an account, download metadata from the internet, scrobble listening history, or bundle advertising and subscription upsells. riff was designed to do one thing well — play local files — without any of those dependencies.

The three target personas (the collector, the minimalist, and the archivist) share a fundamental trait: they own their music as files on disk and prefer local control to cloud convenience.

## Decision

riff is offline-first by construction, not by configuration. The application shall never:

- Stream music from the internet.
- Fetch metadata (tags, artwork) from online sources.
- Scrobble or share listening history.
- Perform any network activity whatsoever.

This is an identity of the product, not a limitation that may be lifted later.

## Consequences

**Positive**:
- Zero network activity, ever — a complete privacy guarantee.
- Simpler architecture: no HTTP client, no authentication, no API integrations, no rate limits.
- No external dependencies that can break, be deprecated, or change terms of service.
- Smaller attack surface: nothing to exploit remotely.
- Serves the minimalist and archivist personas directly.

**Negative**:
- Users cannot discover or enrich metadata for untagged files via online lookup.
- No lyrics fetching, no online cover art lookup.
- The collector persona who wants cloud sync or cross-device listening is out of scope.
- Cannot offer streaming or online catalog browsing.

## Related Documents

- [Overview](./overview.md) — "What riff Is Not" section.
- [Personas](./personas.md) — all three personas.
- [Features](./features.md) — Deferred / Future section.
