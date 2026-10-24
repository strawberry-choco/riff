# Phase 2 — riff grounded inventory (source, not guessing)

> Shared context: see [`index.md`](../index.md).

**Goal:** mark every iTunes feature against riff's *actual* code & docs.

## Read first (seed sources already in repo)
- `docs/product/features.md` ← primary riff feature inventory
- `docs/product/roadmap.md` ← planned / not-yet-built signal
- `docs/product/decisions/*` ← "by design" rationale (offline-first, local-only discovery, track-identity-is-path…)
- `docs/technical/architecture.md`, `docs/adr/*` ← capability boundaries
- `AGENTS.md` ← crate responsibilities

## Scan crates for real capabilities
- `riff-library` — `ScanService`, `MetadataReader`/`MetadataWriter` (lofty), `CoverService`, playlist management, `FilesystemWatch`, `TagEditWorker`
- `riff-playback` — `PlaybackQueue`, gapless math, `AudioEngine` (symphonia + cpal), `Transport`/`ChannelTransport`, volume; verify presence/absence of EQ / crossfade / Sound Check
- `riff-persistence` — `Track` / `Album` / `Artist` / `Playlist`, `play_count` / `last_played` / `date_added`
- `riff-gui` — egui views, tray, native dialogs, search

## Method
Run an **Explore subagent** (very thorough) to return a per-domain inventory with file/line evidence, so scoring is defensible and not from memory.

## Proposed byproduct
Optionally also emit `docs/product/riff-features.md` (a cleaned inventory) if the existing `features.md` is incomplete — confirm with user before adding (default: no).

## Output of this phase
A draft `feature → riff status + evidence` map, ready to join with Phase 1 in Phase 3.
