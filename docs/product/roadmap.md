# Roadmap

This document describes where riff is headed. It has two parts: the items that were considered during requirements work and deliberately deferred, each with the reason it was set aside; and a review of the near-term improvements recommended at v0.1.0, all of which have now shipped in v0.2.0. The deferred list is a record of decisions; the delivery notes are a record of what landed. For what exists today, see [./features.md](./features.md); for the technical context, see [../technical/architecture.md](../technical/architecture.md).

**Where things stand at v0.2.0.** The product surface is complete: every feature in the catalog is implemented, including the surfaces that were partial at v0.1.0 — the main window with close-to-tray, the player control bar (now with mute, plus stop behind advanced mode), cover art display, the Now Playing view, and the system tray. Three items deferred at v0.1.0 shipped: playlist management (custom playlists plus locally generated smart playlists), gapless playback, and tag-based ReplayGain normalization. Behind the surface, the engineering foundations caught up with the feature list: the test suite grew from roughly twenty-five tests to about 165 across the domain, app, infra, UI, and integration layers, and continuous integration runs on GitHub Actions for Linux and Windows. What remains open is open on purpose: the macOS CI leg, the Linux system tray, ReplayGain loudness analysis, and the categories below.

## Deferred items

These capabilities were explicitly scoped out. None of them is accidental: each was weighed against the goal of shipping a solid offline player first, and each has a stated reason for waiting. Three items deferred at v0.1.0 — playlist management, gapless playback, and ReplayGain normalization — shipped in v0.2.0 and moved into the feature catalog; the list below is what is still out.

**Equalizer and audio effects.** Per-band EQ, reverb, and similar processing. A nice-to-have, not essential for the first release, and out of keeping with riff's deliberately bounded feature surface.

**ReplayGain loudness analysis and album-gain mode.** What shipped in v0.2.0 is the read-only half: track gain and peak values are taken from existing tags where present. Computing loudness for untagged libraries — an analysis pass over every file — and album-based leveling remain deferred; that is a project of its own.

**macOS continuous integration.** The CI matrix covers ubuntu-latest and windows-latest. A macOS runner is planned so the platform-conditional code (the tray, the native folder picker) gets the same automated scrutiny, but it is not part of the pipeline yet.

**Linux system tray.** Still intentionally absent: the libayatana-appindicator dependency stack is not reliably present across distributions, so Linux builds run window-only. See [./decisions/002-no-tray-on-linux.md](./decisions/002-no-tray-on-linux.md).

**Lyrics display.** Embedded or fetched lyrics. Not requested within the original scope, and fetching lyrics would conflict with the offline-first design unless limited to lyrics already embedded in file tags.

**Internet-based features.** Streaming, online metadata lookup, and scrobbling. Explicitly out of scope: riff is an offline-only player by design. This is the one deferred category that is closer to a non-goal than a future goal — see the positioning in [./overview.md](./overview.md). Online artwork lookup and scrobbling would each require rethinking the privacy guarantees that define the product.

## Delivered: the v0.1.0 recommendations

At v0.1.0 this document recommended seven improvements, sequenced infrastructure-first so the larger feature work could proceed safely. All seven shipped in v0.2.0.

| Recommendation | Was | Shipped in v0.2.0 as |
|---|---|---|
| Continuous integration pipeline | P1 | GitHub Actions on ubuntu-latest + windows-latest: fmt, clippy, tests |
| Expand test coverage | P1 | ~165 integration tests across domain, app, infra, ui, integration |
| Cache schema versioning | P2 | `schema_version` in library_cache.json, safe fallback with notice |
| A "Clear cache" control in settings | P2 | "Clear Library Cache" action with confirmation in Settings |
| Gapless playback | P2 | Pre-decode 2s before EOF, up to 4s pre-buffer, seamless handoff |
| Playlist management | P2 | Custom playlists plus smart/discovery playlists |
| ReplayGain normalization | P2 | Tag-based track gain/peak, peak-capped, opt-in |

**Continuous integration pipeline.** Delivered. `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy` with warnings denied, and the test suite on push and pull requests to main, on both ubuntu-latest and windows-latest. The macOS leg recommended here is the one piece still pending (see Deferred above).

**Expand test coverage.** Delivered. The suite grew from roughly twenty-five tests to about 165, organized as a single integration-test crate spanning domain, app, infra, UI, and integration suites with shared mocks and helpers. App-layer logic is exercised through the port traits via mock implementations, and filesystem-touching tests lean on the tempfile dev-dependency, exactly as suggested.

**Cache schema versioning.** Delivered. The library cache now carries a schema version. A version mismatch or a corrupt file falls back to an empty library with a warning log and a user-visible notice — the deliberate, explainable behavior this recommendation asked for, rather than a silent reset.

**A "Clear cache" control in settings.** Delivered. Settings has a "Clear Library Cache" action behind a UI confirmation. It deletes the cache file, which rebuilds on the next scan, giving the stale-library case a discoverable remedy.

**Gapless playback.** Delivered. The engine begins pre-decoding the next track about two seconds before the current one ends and stages up to four seconds of samples, so same-format transitions — including repeat-one — are seamless. Format mismatches and mid-transition shuffle changes fall back to the gapped path instead of glitching.

**Playlist management.** Delivered, twice over. Custom playlists support create, rename, delete, ordered tracks, and an add-to-playlist context menu with dedupe, persisted in `playlists.json` separate from the rebuildable library cache; entries whose files disappeared show struck-through as "(missing)" and are excluded from playback — the answer to the file-path identity question raised below at v0.1.0. Separately, four smart playlists (Recently Added, Most Played, Never Played, Lost Gems) generate discovery lists locally from play-count data that persists in the library cache.

**ReplayGain normalization.** Delivered in the form this document sketched: existing REPLAYGAIN_TRACK_GAIN and REPLAYGAIN_TRACK_PEAK tags are read during scanning, and the gain is applied — capped by peak so boosts cannot clip — in the same volume-scaling step the engine already uses. It is opt-in, takes effect on the next track, and leaves untagged tracks untouched. Computing loudness for untagged libraries remains deferred.

## How to read this

The deferred items define the product's edges and are unlikely to change soon — the internet-feature category especially, since offline-first is an identity, not a limitation. The v0.1.0 sequencing played out as designed: infrastructure first (CI, tests, cache robustness), then the larger feature work (gapless, playlists, ReplayGain) landing on top of it. The natural next candidates are the leftovers above — the macOS CI leg and loudness analysis chief among them — plus whatever post-v0.2.0 planning surfaces.
