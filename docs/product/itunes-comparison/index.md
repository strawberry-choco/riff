# riff × iTunes — Feature Comparison: Execution Plan

- **Status:** Plan (not yet executed)
- **Date:** 2026-09-15
- **Scope decisions (locked):** reference = *both* iTunes 12.x (canonical) + Music.app divergences mapped · depth = full taxonomy across all domains · deliverable = markdown report + filterable spreadsheet
- **Goal:** a defensible, source-grounded comparison of riff against iTunes' feature set, with gaps classified into *core-parity gaps*, *legacy/deprecated*, and *deliberate omissions*.

This plan is split into one file per phase. The shared output contract, tooling, definition of done, and open flags live in this index; each phase file is self-contained for execution.

## Phases
- [Phase 1 — iTunes reference taxonomy](phase-1-itunes-taxonomy.md)
- [Phase 2 — riff grounded inventory](phase-2-riff-inventory.md)
- [Phase 3 — assemble the scored matrix](phase-3-matrix.md)
- [Phase 4 — gap narrative + prioritized backlog](phase-4-gap-narrative.md)
- [Phase 5 — generate deliverables](phase-5-deliverables.md)
- [Phase 6 — verification](phase-6-verification.md)

## 0. Output contract

| Artifact | Path | Purpose |
|---|---|---|
| Report | `docs/product/itunes-comparison.md` | Narrative: method, matrix by domain, gap buckets, prioritized backlog, sources |
| Sheet | `docs/product/itunes-comparison.xlsx` | Same matrix, color-coded + filterable / frozen header for slicing by domain & status |
| Raw export | `docs/product/itunes-comparison.csv` | Plain export so the matrix is diff-able in git |

### Status vocabulary (used everywhere)
- ✅ **Implemented** — present in shipped riff code
- 🟡 **Partial** — exists but incomplete vs iTunes
- 📋 **Planned** — in-spec / designed but not built (per docs/ADRs)
- ❌ **Absent** — not present, no plan
- 🚫 **Won't-do (by design)** — outside riff's offline-first, local-library scope

### Gap buckets (the analytical crux)
- **Core-parity gap** — riff should close this to be a credible local-library alternative
- **Legacy / deprecated** — iTunes had it; not worth building (Cover Flow, iTunes DJ, Ping…)
- **Deliberate omission** — Store, DRM, device sync, streaming: defines riff's scope

### Matrix columns
`Domain · Feature · iTunes 12.x · Music.app · riff Status · Evidence (file/line) · Gap Bucket · Priority · Notes`

## 7. Tooling summary
- **WebSearch / WebFetch** — iTunes taxonomy + Music.app deltas
- **Explore subagent** — riff source inventory (evidence-backed)
- **managed Python + openpyxl** — xlsx generation (isolated venv, no global pollution)
- **Write** — markdown + csv

## 8. Definition of done
Both files exist under `docs/product/`; the matrix covers all 8 iTunes domains with Music.app mapped; every status is evidence-backed; the three gap buckets + prioritized backlog are present.

## 9. Open flags / decisions needed
1. **riff feature inventory** — there is no single in-repo checklist; Phase 2 leans on the Explore pass + `docs/product/features.md`. OK to also draft `riff-features.md` as a byproduct? *(default: no, unless asked)*
2. **Music.app treatment** — mapped as a column, not a full second audit. Want Music.app as a fully separate comparison instead?
3. **EQ / DSP scope** — iTunes' EQ + Sound Check are notable parity gaps; confirm riff genuinely lacks them during Phase 2 before scoring (don't assume).
