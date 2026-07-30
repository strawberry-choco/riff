# Roadmap

This document describes where riff is headed. It has two parts: the items that were considered during requirements work and deliberately deferred, each with the reason it was set aside; and a set of recommended near-term improvements, grounded in the current state of the codebase. The deferred list is a record of decisions; the recommendations are suggestions, not commitments. For what exists today, see [./features.md](./features.md); for the technical context behind the suggestions, see [../technical/architecture.md](../technical/architecture.md).

**Where things stand at v0.1.0.** The product surface is largely complete: every Music Library and Audio Engine feature is implemented, and the library explorer and cross-platform support are done. Five UI and integration features remain partial — the main window, the player control bar (missing only its Stop button), cover art display, the Now Playing view, and the system tray. Behind that surface, the engineering foundations are thinner than the feature list suggests: there is a real but small test suite (roughly twenty-five tests spanning the domain, app, infra, UI, and integration layers) and no continuous integration at all. That gap between feature completeness and engineering infrastructure shapes the recommendations below.

## Deferred items

These capabilities were explicitly scoped out of the initial release. None of them is accidental: each was weighed against the goal of shipping a solid offline player first, and each has a stated reason for waiting.

**Playlist management.** Creating, saving, and loading custom playlists. Deferred because core playback and library features must be solid first. In the meantime, folder playback (double-click a folder to queue everything under it) and the queue operations — Play Next and Append to Queue — cover ad-hoc sequencing for most listening sessions.

**Equalizer and audio effects.** Per-band EQ, reverb, and similar processing. A nice-to-have, not essential for the first release, and out of keeping with riff's deliberately bounded feature surface.

**Gapless playback.** Seamless transitions between consecutive album tracks, with no silence at track boundaries. Deferred because it requires complex cross-track buffering — the next track must be decoded and staged before the current one ends — which was judged too risky for the initial audio engine. This is the most-requested-feeling of the deferred items for continuous listening and is a strong candidate for future work (see below).

**ReplayGain normalization.** Automatic volume leveling across tracks so albums and shuffled queues play at consistent loudness. Deferred because it requires a metadata analysis pass over the library — computing or reading loudness information for every track — which is a project of its own.

**Lyrics display.** Embedded or fetched lyrics. Not requested within the original scope, and fetching lyrics would conflict with the offline-first design unless limited to lyrics already embedded in file tags.

**Internet-based features.** Streaming, online metadata lookup, and scrobbling. Explicitly out of scope: riff is an offline-only player by design. This is the one deferred category that is closer to a non-goal than a future goal — see the positioning in [./overview.md](./overview.md). Online artwork lookup and scrobbling would each require rethinking the privacy guarantees that define the product.

## Recommended near-term improvements

The following are **suggestions**, prioritized by the value they would add relative to their cost. Priorities use the same scale as the feature catalog: P1 is high value and should come soon, P2 is worthwhile but can wait. Effort is a rough estimate (small, medium, large).

| Recommendation | Priority | Effort | Theme |
|---|---|---|---|
| Continuous integration pipeline | P1 | small | engineering infrastructure |
| Expand test coverage | P1 | medium | engineering infrastructure |
| Cache schema versioning | P2 | small | robustness |
| A "Clear cache" control in settings | P2 | small | robustness / UX |
| Gapless playback | P2 | large | listening experience |
| Playlist management | P2 | medium–large | library / playback |
| ReplayGain normalization | P2 | medium | listening experience |

The ordering is intentional: the two infrastructure items protect everything else, the two cache items harden a subsystem users already depend on, and the three feature items are the larger efforts that benefit most from that safety net.

**Continuous integration pipeline.** Priority P1, effort small. The repository currently has no CI at all — no automated build, lint, or test runs on any platform. Given that riff ships on three operating systems with platform-conditional code (the tray and folder picker are compiled only on macOS and Windows), a pipeline that runs `cargo fmt --check`, `cargo clippy`, and `cargo test` on Linux, Windows, and macOS would catch platform regressions that no single developer machine can. This is the highest-leverage improvement available: it protects every other item on this list.

**Expand test coverage.** Priority P1, effort medium. A test suite exists — roughly twenty-five tests across domain, app, infra, UI, and integration modules — which is a real foundation, but it is thin relative to the surface area. The pure domain layer (tracks, queue, playback state) and the app-layer logic (library indexing, cover resolution priority, search) are the cheapest places to add dense coverage, because they have no hardware dependencies. Infrastructure behavior that touches the file system can lean on the existing tempfile dev-dependency. Growing this suite alongside CI turns the test count from a fact into a safety net.

**Cache schema versioning.** Priority P2, effort small. The library cache is a JSON file with no schema version field. Today that is harmless, but the first time the track or library data model changes shape, older caches will fail to deserialize and fall back to an empty library — correct behavior, but a forced full rescan that a version field could avoid or at least explain. Adding a version marker now, before the format has ever changed, is cheap insurance and makes future migrations deliberate rather than accidental.

**A "Clear cache" control in settings.** Priority P2, effort small. This was raised during the cache feature's design and deferred on the grounds that deleting the cache file manually, or simply running a scan, both work. That is true, but neither is discoverable. A button in the settings page would give users a visible remedy when the library looks stale, and it pairs naturally with cache schema versioning as part of making the cache a first-class, user-legible part of the product.

**Gapless playback.** Priority P2, effort large. Of the deferred features, this is the one that most directly improves the core act of listening to an album. The audio engine already drains its shared buffer at track end before stopping the stream, which is a useful starting point, but true gaplessness means pre-buffering the next track's decoded samples before the current track finishes — a substantial change to the decode loop and track-transition logic. Worth doing, but it should land on top of a mature, well-tested engine, which is why CI and test coverage rank ahead of it.

**Playlist management.** Priority P2, effort medium to large. The queue already supports the primitives playlists are made of — replace, insert-after, append — so persistent playlists are mostly a persistence and UI story: named, ordered track lists saved to disk and loadable back into the queue. The main design question is how playlists relate to the file-path-based track identity when files move or are deleted. A natural follow-on once the library features have settled.

**ReplayGain normalization.** Priority P2, effort medium. Valuable for shuffled listening and unevenly-mastered collections. The practical path is reading existing ReplayGain tags where present (lofty can surface them) and applying the gain in the same volume-scaling step the engine already uses, deferring the much larger job of computing loudness for untagged libraries. Even the read-only version would be a meaningful improvement for the archivist audience described in [./personas.md](./personas.md).

## How to read this

The deferred items define the product's edges and are unlikely to change soon — the internet-feature category especially, since offline-first is an identity, not a limitation. The recommendations are where the project should invest next, and they are deliberately sequenced: infrastructure first (CI, tests, cache robustness), so that the larger feature work (gapless, playlists, ReplayGain) can proceed without risking the stability the first release earned.
