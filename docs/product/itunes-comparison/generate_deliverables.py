#!/usr/bin/env python3
"""Generate Phase 5 deliverables for the riff x iTunes comparison.

Reads:
  docs/product/itunes-comparison/phase-3-scored-matrix.md  (75-row matrix)
  docs/product/itunes-comparison/phase-4-gap-narrative.md (buckets + backlog)

Writes:
  docs/product/itunes-comparison.md    (narrative report)
  docs/product/itunes-comparison.xlsx  (filterable, color-coded sheet + Backlog tab)
  docs/product/itunes-comparison.csv   (diff-able plain export)
"""
import re
import os
import csv
from collections import Counter

BASE = r"C:/projects/riff/docs/product/itunes-comparison"
OUT_DIR = r"C:/projects/riff/docs/product"
MATRIX_SRC = os.path.join(BASE, "phase-3-scored-matrix.md")
NARR_SRC = os.path.join(BASE, "phase-4-gap-narrative.md")
OUT_MD = os.path.join(OUT_DIR, "itunes-comparison.md")
OUT_XLSX = os.path.join(OUT_DIR, "itunes-comparison.xlsx")
OUT_CSV = os.path.join(OUT_DIR, "itunes-comparison.csv")

STATUS_ICONS = ["✅", "🟡", "❌", "🚫", "📋"]


def split_md_row(line):
    """Split a markdown table row on '|' but ignore pipes inside `code spans`."""
    s = line.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|"):
        s = s[:-1]
    cells = []
    buf = []
    in_code = False
    for ch in s:
        if ch == "`":
            in_code = not in_code
            buf.append(ch)
        elif ch == "|" and not in_code:
            cells.append("".join(buf).strip())
            buf = []
        else:
            buf.append(ch)
    cells.append("".join(buf).strip())
    return cells


def parse_matrix(path):
    rows = []
    domain_id = None
    domain_name = ""
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            m = re.match(r"^##\s*Domain\s+(\d+)\s*[\u2014\-]\s*(.+)$", line)
            if m:
                domain_id = int(m.group(1))
                domain_name = m.group(2).strip()
                continue
            if not line.startswith("|"):
                continue
            cells = split_md_row(line)
            if len(cells) != 9:
                continue
            if cells[0] == "Domain":
                continue
            if set("".join(cells)) <= set("-: "):
                continue
            rows.append({
                "domain_id": domain_id,
                "domain_name": domain_name,
                "domain": cells[0],
                "feature": cells[1],
                "itunes": cells[2],
                "music": cells[3],
                "status": cells[4],
                "evidence": cells[5],
                "bucket": cells[6],
                "priority": cells[7],
                "notes": cells[8],
            })
    return rows


def parse_backlog(path):
    items = []
    priority = None
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            m = re.match(r"^##\s*(P\d)\s*[\u2014\-]", line)
            if m:
                priority = m.group(1)
                continue
            if not line.startswith("|"):
                continue
            cells = split_md_row(line)
            if len(cells) != 5:
                continue
            if cells[0] == "#":
                continue
            if set("".join(cells)) <= set("-: "):
                continue
            items.append({
                "priority": priority,
                "num": cells[0],
                "feature": cells[1],
                "domain": cells[2],
                "crates": cells[3],
                "shape": cells[4],
            })
    return items


def esc(s):
    return s.replace("|", "\\|").replace("\n", " ")


def md_matrix_table(rows):
    head = "| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |"
    sep = "|---|---|---|---|---|---|---|---|---|"
    lines = [head, sep]
    for r in rows:
        lines.append("| {d} | {feature} | {it} | {mu} | {st} | {ev} | {bk} | {pr} | {no} |".format(
            d=r["domain"], feature=esc(r["feature"]), it=esc(r["itunes"]), mu=esc(r["music"]),
            st=esc(r["status"]), ev=esc(r["evidence"]), bk=esc(r["bucket"]),
            pr=esc(r["priority"]), no=esc(r["notes"])))
    return "\n".join(lines)


def md_backlog_table(items):
    head = "| # | Feature | Domain | Target crate(s) | Rough shape |"
    sep = "|---|---|---|---|---|"
    lines = [head, sep]
    for it in items:
        lines.append("| {n} | {f} | {d} | {c} | {s} |".format(
            n=it["num"], f=esc(it["feature"]), d=esc(it["domain"]),
            c=esc(it["crates"]), s=esc(it["shape"])))
    return "\n".join(lines)


def main():
    matrix = parse_matrix(MATRIX_SRC)
    backlog = parse_backlog(NARR_SRC)

    assert len(matrix) == 75, "expected 75 matrix rows, got %d" % len(matrix)

    # ----- stats -----
    # Source of truth = the per-row matrix values. The Phase 3 summary rollup
    # (status 19/5/17/34, buckets 24/19/30/8) is internally inconsistent with the
    # 75-row matrix (its bucket totals sum to 81 != 75, and it labels Sound Check
    # partial while the matrix marks it 'Absent'). We derive from the matrix and
    # normalize the two known artifacts:
    #   * 3 Genius rows carry a hybrid "Deliberate omission / Legacy" label ->
    #     counted as Legacy (per Phase 4 Bucket 2).
    #   * Mini-player is double-listed (D3 + D5) -> 20 core-parity rows become
    #     19 distinct features, matching the backlog.
    status_counts = Counter()
    bucket_counts = Counter()
    for r in matrix:
        key = next((e for e in STATUS_ICONS if e in r["status"]), "?")
        status_counts[key] += 1
        b = r["bucket"]
        if b == "Deliberate omission / Legacy":
            bucket_counts["Legacy / deprecated"] += 1
        else:
            bucket_counts[b] += 1
    backlog_priority = Counter(it["priority"] for it in backlog)
    core_rows = [r for r in matrix if r["bucket"] == "Core-parity gap"]

    domains = []
    seen = set()
    for r in matrix:
        if r["domain_id"] not in seen:
            seen.add(r["domain_id"])
            domains.append((r["domain_id"], r["domain_name"]))
    domain_rows = {did: [r for r in matrix if r["domain_id"] == did] for did, _ in domains}

    # ============ MARKDOWN REPORT ============
    parts = []
    parts.append("# riff × iTunes — Feature Comparison Report")
    parts.append("")
    parts.append("> Source: `docs/product/itunes-comparison/phase-3-scored-matrix.md` (75 rows, 8 domains) joined to `phase-4-gap-narrative.md`. Generated 2026-09-15 as the Phase 5 deliverable. Companion artifacts: `itunes-comparison.xlsx` (filterable sheet) and `itunes-comparison.csv` (diff-able export).")
    parts.append("")

    parts.append("## 1. Method & scope")
    parts.append("")
    parts.append("This report compares **riff** — an offline-first, local-library music player (Rust + egui) — against **iTunes 12.x** (the canonical desktop reference), with **Music.app** divergences mapped as a second column. The comparison is source-grounded, not impressionistic:")
    parts.append("")
    parts.append("- **Phase 1** built the iTunes 12.x + Music.app feature taxonomy.")
    parts.append("- **Phase 2** built a riff inventory directly from the codebase — every riff status below is backed by a `file:line` citation (grep + Explore pass), not by assumption.")
    parts.append("- **Phase 3** joined the two on `Domain + Feature` into a 75-row scored matrix, assigning each row a gap bucket and (for core-parity gaps) a priority.")
    parts.append("- **Phase 4** turned the matrix into the gap narrative and a prioritized backlog.")
    parts.append("- **Phase 5** (this document) renders the matrix, the buckets, and the backlog as a markdown report, a filterable spreadsheet, and a diff-able CSV.")
    parts.append("")

    parts.append("### Status vocabulary")
    parts.append("")
    parts.append("| Symbol | Meaning |")
    parts.append("|---|---|")
    parts.append("| ✅ Implemented | present in shipped riff code |")
    parts.append("| 🟡 Partial | exists but incomplete vs iTunes |")
    parts.append("| ❌ Absent | not present, no plan |")
    parts.append("| 🚫 Won't-do (by design) | outside riff's offline-first, local-library scope |")
    parts.append("| 📋 Planned | in-spec / designed but not built (none in this audit) |")
    parts.append("")

    parts.append("### Gap buckets")
    parts.append("")
    parts.append("- **Core-parity gap** — riff should close this to be a credible local-library alternative.")
    parts.append("- **Legacy / deprecated** — iTunes had it; not worth building (Cover Flow, iTunes DJ, Ping…).")
    parts.append("- **Deliberate omission** — Store, DRM, device sync, streaming: defines riff's scope.")
    parts.append("")

    parts.append("## 2. Headline numbers")
    parts.append("")
    parts.append("The matrix spans **{n} rows across 8 domains**.".format(n=len(matrix)))
    parts.append("")
    parts.append("**By riff status:** ✅ Implemented {si} · 🟡 Partial {sp} · ❌ Absent {sa} · 🚫 Won't-do {sw}".format(
        si=status_counts.get("✅", 0), sp=status_counts.get("🟡", 0),
        sa=status_counts.get("❌", 0), sw=status_counts.get("🚫", 0)))
    parts.append("")
    parts.append("**By gap bucket:** parity-met {bm} · core-parity gap {bc} · deliberate omission {bd} · legacy/deprecated {bl}".format(
        bm=bucket_counts.get("—", 0), bc=bucket_counts.get("Core-parity gap", 0),
        bd=bucket_counts.get("Deliberate omission", 0), bl=bucket_counts.get("Legacy / deprecated", 0)))
    parts.append("")
    parts.append("**By priority (core-parity distinct features):** P0 ×{p0} · P1 ×{p1} · P2 ×{p2} = 19".format(
        p0=backlog_priority.get("P0", 0), p1=backlog_priority.get("P1", 0), p2=backlog_priority.get("P2", 0)))
    parts.append("")
    parts.append("> **Data-quality note (for Phase 6).** These figures are tallied directly from the 75 matrix rows. They differ from the Phase 3 summary rollup (which listed status 19/5/17/34 and buckets 24/19/30/8). That rollup is internally inconsistent — its bucket totals sum to 81, not 75, and it scores Sound Check as *partial* while the matrix scores it *Absent* (functionally mitigated by ReplayGain). Two normalization choices were applied: the 3 Genius rows carry a hybrid \"Deliberate omission / Legacy\" label and are counted as Legacy (per Phase 4 Bucket 2); and Mini-player is double-listed in D3 and D5, so the 20 core-parity *rows* collapse to 19 distinct *features*, matching the backlog. The per-row matrix values above are the source of truth.")
    parts.append("")
    parts.append("The story is not \"riff is missing iTunes\" — **riff has already closed the parts of iTunes that matter for an offline, local, music-only library, and has consciously declined the parts that don't.** The 19 remaining gaps are real but bounded, and almost all are *library depth* features (metadata richness, organizational power, interop) rather than *playback* features (riff's playback core is essentially at parity).")
    parts.append("")

    parts.append("## 3. The matrix (by domain)")
    parts.append("")
    parts.append("For each of the 8 domains, the full scored matrix (status is evidence-backed per `file:line`):")
    parts.append("")
    for did, dname in domains:
        parts.append("### Domain {d} — {n}".format(d=did, n=dname))
        parts.append("")
        parts.append(md_matrix_table(domain_rows[did]))
        parts.append("")

    parts.append("## 4. Gap analysis")
    parts.append("")

    parts.append("### 4.1 Core-parity gaps (19) — what riff should close")
    parts.append("")
    parts.append("These are the features a credible local-library alternative is expected to have, that riff lacks or only partially covers. They split cleanly into four themes:")
    parts.append("")
    parts.append("- **Organizational power (the biggest deficit).** Smart playlists in riff are *6 fixed discovery lists*, not a user-facing rule engine — this is the single most defining iTunes organizational feature and the only **P0**. Playlist folders (hierarchy) and duplicate detection are the next tier of library-management depth.")
    parts.append("- **Metadata richness.** riff reads/writes the core tags and artwork, but is missing the \"classical + DJ + power-user\" tag set: 1–5 star rating (riff has only a boolean Favorite), Grouping, BPM, Compilation flag, and per-field sort overrides. Batch (\"Get Info\" multi-select) tag editing is absent — only single-track edit exists today.")
    parts.append("- **DSP / listening UX.** A graphic equalizer (presets) is the notable playback parity gap. Sound Check is functionally *mitigated* by riff's ReplayGain implementation, so it is a gap only in the literal iTunes sense. Crossfade is a deliberate omission, not a gap.")
    parts.append("- **Interop / portability.** No playlist export (M3U) and no iTunes-Library-XML / DJ-app bridge — both high-value for a tool whose whole point is *your* local files. Mini-player and full-screen media mode are smaller UI-parity items.")
    parts.append("")

    parts.append("### 4.2 Legacy / deprecated (8) — skip, and it's not riff's call")
    parts.append("")
    parts.append("These are features iTunes itself removed (or Music.app dropped), so they are not a riff decision and not worth building:")
    parts.append("")
    parts.append("| Feature | Why deprecated | Where |")
    parts.append("|---|---|---|")
    parts.append("| Cover Flow | Removed as default in iTunes 11 (2012) | D5 |")
    parts.append("| iTunes DJ / Party Shuffle | Discontinued in iTunes 11, replaced by Up Next | D6 |")
    parts.append("| Ringtones | Removed in iTunes 12.7 (2017) | D6 |")
    parts.append("| Ping | Removed in iTunes 11 (2012); defunct social network | D7 |")
    parts.append("| Visualizer (full-screen) | Classic visualizers dropped in Music.app | D3 |")
    parts.append("| Genius / Genius Mixes / Genius recommendations | Cloud-analysis feature, deprecated in later macOS | D4 / D7 |")
    parts.append("")
    parts.append("riff's \"no\" here is simply \"don't rebuild a dead feature.\" No `decisions/` citation required — the rationale is iTunes' own deprecation history.")
    parts.append("")

    parts.append("### 4.3 Deliberate omissions (28) — what defines riff")
    parts.append("")
    parts.append("These are the features riff declines *by design*, and they are the clearest statement of product scope. Every one traces to the offline-first / local-library thesis:")
    parts.append("")
    parts.append("- **Commerce & streaming (never):** iTunes Store, Apple Music streaming, iTunes Match, iCloud/Sync Library, Redeem/gift, Family Sharing.")
    parts.append("- **Device & ecosystem (never):** iPhone/iPad/iPod sync, backup/restore, Wi-Fi sync, iPod sync protocol, AirPlay/Remote, Home Sharing, shared library over LAN.")
    parts.append("- **Non-music media (never):** Podcasts, Audiobooks, TV/Movies, iTunes U, Ringtones.")
    parts.append("- **DRM (never):** FairPlay — riff is local-only and DRM-free by construction.")
    parts.append("- **File-mutating ingestion (never):** auto-organize (\"Keep Media folder organized\") is declined because track identity *is* the file path (`decisions/003-track-identity-is-path.md`); CD import/rip, CD burn, and format conversion/transcoding are out of the offline scope (`decisions/001-offline-first.md`).")
    parts.append("- **DSP posture (by design, but under-cited):** crossfade and per-track volume/EQ are declined — riff uses global volume + ReplayGain instead. **Caveat:** these 🚫 rows currently cite only `features.md` / `roadmap.md`, not a first-class ADR — see Open Flag #1 below.")
    parts.append("")
    parts.append("**Net:** the deliberate-omission bucket is the product's moat, not a todo list. It should be read as \"scope defended,\" not as gaps.")
    parts.append("")

    parts.append("## 5. Prioritized backlog")
    parts.append("")
    parts.append("Only the 19 core-parity gaps are backlog items. Priorities: **P0** = defining · **P1** = meaningful/power-user · **P2** = nice-to-have. Each item names the target crate(s) and a rough implementation shape, mapped to the vertical crate split in `AGENTS.md`. Crate key: `pers` = riff-persistence · `lib` = riff-library · `play` = riff-playback · `infra` = riff-infra · `be` = riff-backend · `gui` = riff-gui.")
    parts.append("")
    for prio, label in [("P0", "P0 — defining parity gap (1)"),
                        ("P1", "P1 — meaningful, expected by power users (4)"),
                        ("P2", "P2 — nice-to-have (14)")]:
        parts.append("### {l}".format(l=label))
        parts.append("")
        items = [it for it in backlog if it["priority"] == prio]
        parts.append(md_backlog_table(items))
        parts.append("")

    parts.append("> **Backlog totals:** P0 ×1, P1 ×4, P2 ×14 = 19 core-parity gaps. Playback-domain gaps are only EQ (P1), Sound Check (P2, mitigated), and mini-player (P2) — confirming riff's playback core is at parity; the backlog is overwhelmingly *library depth + interop*.")
    parts.append("")

    parts.append("## 6. Sources & open flags")
    parts.append("")
    parts.append("### Source files")
    parts.append("")
    parts.append("- `docs/product/itunes-comparison/phase-1-itunes-taxonomy.md`, `phase-1-itunes-reference.md`")
    parts.append("- `phase-2-riff-inventory.md`, `phase-2-riff-reference.md`")
    parts.append("- `phase-3-matrix.md`, `phase-3-scored-matrix.md`")
    parts.append("- `phase-4-gap-narrative.md`")
    parts.append("- riff codebase (evidence cited per row), `AGENTS.md`, `docs/product/features.md`, `docs/product/roadmap.md`")
    parts.append("- Decisions cited: `decisions/001-offline-first.md`, `decisions/003-track-identity-is-path.md`, `decisions/006-local-only-discovery.md`")
    parts.append("")
    parts.append("### Open flags")
    parts.append("")
    parts.append("1. **DSP scope citation (carried from Phase 3/4).** The 🚫 rows for per-track EQ preset, per-track volume, and crossfade cite only `features.md` / `roadmap.md`, not a first-class `decisions/` ADR. Recommend adding `decisions/007-dsp-scope.md` stating riff's no-global-EQ / no-crossfade / no-per-track-volume posture so these rows have a first-class citation.")
    parts.append("2. **Mini-player double-listed** in the Phase 3 reference (D3 + D5); tracked once in the backlog (#14).")
    parts.append("3. **Sound Check** scored ❌ Absent but functionally mitigated by ReplayGain ✅ — reported as mitigated, not a build priority.")
    parts.append("4. **Phase 3 summary rollup is internally inconsistent** (see data-quality note in §2). Its status/bucket tallies (19/5/17/34 and 24/19/30/8) do not match the 75-row matrix (buckets sum to 81 ≠ 75; Sound Check mislabeled partial). This report uses the per-row matrix values as the source of truth. Recommend Phase 6 correct the Phase 3 rollup text to match.")
    parts.append("")

    with open(OUT_MD, "w", encoding="utf-8") as f:
        f.write("\n".join(parts))
    print("wrote", OUT_MD)

    # ============ CSV ============
    with open(OUT_CSV, "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f)
        w.writerow(["Domain", "Domain Name", "Feature", "iTunes 12.x", "Music.app",
                    "riff Status", "Evidence (file/line)", "Gap Bucket", "Priority", "Notes"])
        for r in matrix:
            w.writerow([r["domain"], r["domain_name"], r["feature"], r["itunes"], r["music"],
                        r["status"], r["evidence"], r["bucket"], r["priority"], r["notes"]])
    print("wrote", OUT_CSV)

    # ============ XLSX ============
    from openpyxl import Workbook
    from openpyxl.styles import Font, PatternFill, Alignment, Border, Side
    from openpyxl.utils import get_column_letter

    wb = Workbook()

    hdr_fill = PatternFill("solid", fgColor="1F4E78")
    hdr_font = Font(bold=True, color="FFFFFF")
    thin = Side(style="thin", color="D9D9D9")
    border = Border(left=thin, right=thin, top=thin, bottom=thin)

    STATUS_FILL = {"✅": "C6EFCE", "🟡": "FFEB9C", "❌": "FFC7CE", "🚫": "D9D9D9", "📋": "BDD7EE"}
    STATUS_FONT = {"✅": "006100", "🟡": "9C6500", "❌": "9C0006", "🚫": "3F3F3F", "📋": "1F4E78"}
    PRIORITY_FILL = {"P0": "FFC7CE", "P1": "FFEB9C", "P2": "D9D9D9"}
    PRIORITY_FONT = {"P0": "9C0006", "P1": "9C6500", "P2": "3F3F3F"}

    def style_header(ws, ncols):
        for c in range(1, ncols + 1):
            cell = ws.cell(row=1, column=c)
            cell.fill = hdr_fill
            cell.font = hdr_font
            cell.alignment = Alignment(vertical="center", horizontal="center", wrap_text=True)
            cell.border = border

    def apply_borders(ws, ncols):
        for r in range(1, ws.max_row + 1):
            for c in range(1, ncols + 1):
                ws.cell(row=r, column=c).border = border

    # --- Matrix sheet ---
    ws = wb.active
    ws.title = "Matrix"
    headers = ["Domain", "Feature", "iTunes 12.x", "Music.app", "riff Status",
               "Evidence (file/line)", "Gap Bucket", "Priority", "Notes"]
    ws.append(headers)
    for r in matrix:
        ws.append([r["domain"], r["feature"], r["itunes"], r["music"], r["status"],
                   r["evidence"], r["bucket"], r["priority"], r["notes"]])
    style_header(ws, len(headers))
    status_col = 5
    for ridx in range(2, ws.max_row + 1):
        val = ws.cell(row=ridx, column=status_col).value or ""
        key = next((e for e in STATUS_FILL if e in val), None)
        if key:
            cell = ws.cell(row=ridx, column=status_col)
            cell.fill = PatternFill("solid", fgColor=STATUS_FILL[key])
            cell.font = Font(color=STATUS_FONT[key], bold=True)
        ws.cell(row=ridx, column=6).alignment = Alignment(vertical="top", wrap_text=True)
        ws.cell(row=ridx, column=9).alignment = Alignment(vertical="top", wrap_text=True)
    apply_borders(ws, len(headers))
    ws.freeze_panes = "A2"
    ws.auto_filter.ref = "A1:I{0}".format(ws.max_row)
    for i, w in enumerate([8, 26, 34, 30, 16, 34, 18, 9, 52], start=1):
        ws.column_dimensions[get_column_letter(i)].width = w

    # --- Backlog sheet ---
    ws2 = wb.create_sheet("Backlog")
    bheaders = ["Priority", "#", "Feature", "Domain", "Target crate(s)", "Rough shape"]
    ws2.append(bheaders)
    for it in backlog:
        ws2.append([it["priority"], it["num"], it["feature"], it["domain"], it["crates"], it["shape"]])
    style_header(ws2, len(bheaders))
    for ridx in range(2, ws2.max_row + 1):
        val = ws2.cell(row=ridx, column=1).value or ""
        if val in PRIORITY_FILL:
            cell = ws2.cell(row=ridx, column=1)
            cell.fill = PatternFill("solid", fgColor=PRIORITY_FILL[val])
            cell.font = Font(color=PRIORITY_FONT[val], bold=True)
        ws2.cell(row=ridx, column=6).alignment = Alignment(vertical="top", wrap_text=True)
    apply_borders(ws2, len(bheaders))
    ws2.freeze_panes = "A2"
    ws2.auto_filter.ref = "A1:F{0}".format(ws2.max_row)
    for i, w in enumerate([9, 5, 34, 9, 26, 72], start=1):
        ws2.column_dimensions[get_column_letter(i)].width = w

    wb.save(OUT_XLSX)
    print("wrote", OUT_XLSX)
    print("matrix rows:", len(matrix), "| backlog rows:", len(backlog))


if __name__ == "__main__":
    main()
