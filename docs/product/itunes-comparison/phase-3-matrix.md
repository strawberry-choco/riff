# Phase 3 — assemble the scored matrix

> Status vocabulary, gap buckets, and matrix columns: see [`index.md`](../index.md).

**Goal:** combine the Phase 1 reference and Phase 2 inventory into one scored matrix.

## Steps
1. Join the two inputs on the feature key into the matrix schema:
   `Domain · Feature · iTunes 12.x · Music.app · riff Status · Evidence (file/line) · Gap Bucket · Priority · Notes`
2. Assign each row a **gap bucket**:
   - Core-parity gap
   - Legacy / deprecated
   - Deliberate omission
3. For core-parity gaps only, assign a **priority** (P0 / P1 / P2).
4. Sanity pass:
   - every 🚫 needs a one-line "by design" rationale (cite a `decisions/` doc)
   - every 📋 must cite a doc / ADR / roadmap item

## Output of this phase
The complete scored matrix (the data backbone for both deliverables in Phase 5).
