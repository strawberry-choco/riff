# Phase 5 — generate deliverables

**Goal:** produce the final artifacts from the Phase 3 matrix + Phase 4 narrative.

## Steps
1. **Markdown** via `Write` → `docs/product/itunes-comparison.md`:
   intro/method → matrix by domain → gap buckets → prioritized backlog → sources appendix.
2. **Spreadsheet** via managed Python (`openpyxl` in the isolated venv) → `itunes-comparison.xlsx`:
   - frozen header row, autofilter on, status column color-coded, grouped by domain
   - a second "Backlog" sheet tab
3. **CSV** export alongside for git-diffing → `itunes-comparison.csv`.

## Output of this phase
The three deliverable files under `docs/product/`.
