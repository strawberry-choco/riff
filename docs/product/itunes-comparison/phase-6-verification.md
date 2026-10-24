# Phase 6 — verification

**Goal:** confirm the matrix is accurate and defensible before calling it done.

## Steps
1. Re-spot-check 3–4 riff statuses against actual code (trust-but-verify the subagent).
2. Confirm every 🚫 / 📋 row is justified.
3. No `cargo` needed — this is a docs artifact.

## Output of this phase
A verified, ready-to-share comparison (report + sheet + csv).

---

## Verification results (2026-09-15)

Executed trust-but-verify against the codebase and the cited decision docs. **Verdict: ✅ the comparison is accurate and ready to share.** The per-row matrix is sound; the only defect was the Phase 3 summary rollup, now corrected in place (see `phase-3-scored-matrix.md` · `## Rollup`).

### 1. Spot-checks against actual code (4 of 75 rows)

| Spot | Feature | Claimed status | Verified |
|---|---|---|---|
| D3 Up Next / queue | ✅ Implemented | `queue.rs:54-64,81-87,140-226`; `toggle_repeat:124-131` | ✅ `append` / `insert_next` (next-after-current = "Play Next") / `advance` / `previous` / `toggle_repeat` (None→All→One) all present exactly as cited |
| D4 Smart playlists | 🟡 Partial | `track.rs:54-86` (6 fixed kinds); `sqlite.rs:2082-2231` | ✅ `SmartPlaylistKind` enum = exactly 6 fixed kinds; `smart_playlist` + `smart_list_counts` implement them as fixed store queries — never a user rule engine |
| D2 Core tags | ✅ Implemented | `metadata_reader.rs:85-138`; `metadata_writer.rs:47-108` | ✅ reader + writer cover title/artist/album/album_artist/track#/disc#/genre/year/comment/composer (+ReplayGain); the absent tags (Grouping, BPM, sort fields) are correctly **not** read/written |
| D8 Export playlist (M3U) | ❌ Absent | grep `m3u` → none | ✅ zero hits across `crates/` — confirmed absent |

### 2. 🚫 / 📋 justification

- **No 📋 (Planned) rows exist** — confirmed by grep across the matrix; every deferred item is scored ❌ Absent with a `roadmap.md` citation. OK.
- **Every 🚫 row traces to a real, accepted ADR:** `decisions/001-offline-first.md` (no network / streaming / scrobble — backs Store, Apple Music, Match, iCloud, AirPlay, Scrobbling, Genius, Home Sharing, Podcasts, Audiobooks, TV/Movies, non-music, CD, device sync), `decisions/003-track-identity-is-path.md` (auto-organize — riff never moves files because identity *is* the path), `decisions/006-local-only-discovery.md` (AirPlay / LAN sharing). Legacy 🚫 (Cover Flow, iTunes DJ, Ringtones, Ping, Visualizer) cite iTunes' own deprecation, not a riff decision — no ADR required, correct.
- **Open flag #1 (DSP 🚫 lacking a first-class ADR):** the 3 deferred rows (per-track EQ preset, per-track volume ×2) are scored ❌ Absent and cite `features.md` / `roadmap.md` (`features.md:125` Equalizer "not essential"; `features.md:21` crossfade "intentionally not provided"; `roadmap.md:11/19/21`). Crossfade stays 🚫 (explicit in `features.md:21`). The recommendation to add `decisions/007-dsp-scope.md` remains open but is **non-blocking** — the statuses are defensible as-is.

### 3. Rollup arithmetic — CORRECTED

The Phase 3 summary rollup was internally inconsistent (bucket totals summed to 81, not 75; 🟡 mislabeled as 5; Sound Check mislabeled partial). Recomputed directly from the 75 per-row statuses:

| Cut | Before (wrong) | After (verified) |
|---|---|---|
| ✅ Implemented | 19 | 19 |
| 🟡 Partial | 5 | 3 |
| ❌ Absent | 17 | 21 |
| 🚫 Won't-do | 34 | 32 |
| — parity met | 24 | 19 |
| Core-parity gap | 19 | 20 |
| Deliberate omission | 30 | 28 |
| Legacy / deprecated | 8 | 8 |
| Priority (P0/P1/P2) | 1/4/14 | 1/4/14 (20 rows → 19 features; Mini-player double-listed D3+D5) |

Corrected figures match the Phase 5 deliverables (`itunes-comparison.md` §2). Phase 3 `## Rollup` block updated in place.

### Verdict
✅ **Verified, ready to share.** `itunes-comparison.md` (report), `itunes-comparison.xlsx` (sheet), `itunes-comparison.csv` (export) are accurate and mutually consistent. Remaining non-blocking item: consider `decisions/007-dsp-scope.md` for a first-class citation on the DSP-posture rows.
