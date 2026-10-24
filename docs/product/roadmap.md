# Roadmap

> Status: draft ranking, 2026-09-15. Prior art: [`features.md`](./features.md) (what ships today) and the iTunes comparison's [prioritized backlog](./itunes-comparison/phase-4-gap-narrative.md) (the 19 core-parity gaps this ranking draws from).
>
> Scope note: every P0 and P1 feature in `features.md` is implemented as of v0.2. What follows is therefore ranked *next* work: the parity gaps, in the order they should be built.

## Ranking criteria

Each candidate is scored on four axes, and the order below is the result:

- **Value** — how much it moves the product for the target listener (collector / minimalist / archivist per [`personas.md`](./personas.md)), and how often it is felt.
- **Effort** — schema + store query + app layer + UI + tests, per the vertical crate split in `AGENTS.md`.
- **Unblocking** — whether later items become cheaper once it lands.
- **Identity fit** — riff is *offline-first local library navigability*. Features that deepen navigability outrank features that add surface.

## Ranked backlog

### Tier 1 — Build next (high value, bounded effort)

| # | Feature | Rationale | Target |
|---|---|---|---|
| 1 | **Smart playlist rule engine** | The single defining parity gap and the one that most serves riff's navigability thesis: today's six fixed discovery lists are hard-coded, not user-authored. `RuleSet` (field / operator / value, match all/any, sort + limit) persisted in the store, evaluated as a live query through a `LibrarySession` projection, with the six existing lists pre-seeded as rules. Largest item here — scope it as its own spec. | `pers` · `lib` · `be` · `gui` |
| 2 | **Batch tag edit (multi-select Get Info)** | Generalize `TagEditRequest` to N track ids plus a field mask; the `TagEditWorker` already has the durable-commit machinery. Medium effort, outsized power-user value. | `be` · `gui` |
| 3 | **Export playlist (M3U)** | Smallest real win in the backlog: serialize to extended `.m3u`, high interop value, perfectly consistent with "your files, your library". Import is a later follow-up. | `lib` · `gui` |

### Tier 2 — Worth doing, pick after Tier 1

| # | Feature | Rationale | Target |
|---|---|---|---|
| 4 | **Drag-and-drop add** | Cheap; enqueue dropped paths to the scan service. Removes the "must open Settings to add music" friction. | `gui` · `lib` |
| 5 | **Duplicate detection** | Clustering query + a "Show Duplicates" view. Real value for collectors with multiple drives, but the matching heuristic deserves its own decision. | `lib` · `gui` |
| 6 | **Metadata wave — BPM, grouping, compilation, sort overrides** | Four P2 fields from the comparison. Build them as one migration + one `infra` read/write pass, never four separate features; each alone is not worth a release. | `pers` · `infra` · `gui` |

### Tier 3 — Only if a real user asks

| # | Feature | Note |
|---|---|---|
| 7 | **Graphic equalizer (presets)** | The notable playback parity gap, but it contradicts the "no EQ / no crossfade / no per-track volume" posture asserted in `overview.md` and `features.md`. Needs `decisions/007-dsp-scope.md` *first* — the decision, not the code, is the missing piece. |
| 8 | **Mini-player / full-screen media mode** | UI parity items; nice, not defining. |
| 9 | **Playlist folders** | Hierarchy in the sidebar; only matters once #1 makes playlist count grow. |
| 10 | **Lyrics pane** | Deferred twice already. Keep deferred. |
| 11 | **ReplayGain analysis pass + album gain** | Tag-based track gain ships; analysis for untagged libraries is a whole new background workload. |

## Deliberately not ranked

- **Sound Check / `iTunNORM`** — functionally mitigated by ReplayGain; a literal gap, not a real one.
- **iTunes Library XML export** — niche; revisit only if a DJ-app bridge is actually wanted.
- **Everything in the "Deferred / Future" section of `features.md`** — streaming, online lookup, scrobbling: offline-first is the moat, not a todo list.

## Doc debt to clear alongside Tier 0

- `roadmap.md` was referenced by `features.md` and `overview.md` but did not exist; this file closes that.
- `overview.md` is stale: it still says v0.1.0, claims riff "does not edit your tags" (tag writing shipped in 0.2.0), and lists Now Playing and the tray as partial. It needs a pass against `features.md`.
