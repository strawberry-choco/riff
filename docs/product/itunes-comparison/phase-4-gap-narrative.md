# Phase 4 — gap narrative + prioritized backlog

> Gap-bucket definitions: see [`index.md`](../index.md).
> Consumes: [`phase-3-scored-matrix.md`](../phase-3-scored-matrix.md) (75 rows, scored & bucketed).

**Goal:** turn the matrix into an actionable story, not just a table.

## Steps
1. Synthesize **three buckets** with rationale:
   - *Core-parity gaps* riff should close
   - *Legacy / deprecated* iTunes features to skip
   - *Deliberate omissions* that define riff
2. Build the **prioritized backlog** (core-parity gaps only):
   - **P0** — closes a real parity gap, low effort / high value
   - **P1** — meaningful but larger
   - **P2** — nice-to-have
   - Each item = feature + target crate + rough shape

## Output of this phase
The narrative section + backlog list that populate the report (Phase 5) and the "Backlog" spreadsheet tab.

---

# Part A — The gap narrative

The scored matrix resolves into **75 rows across 8 domains**: 24 parity-met (`—`), 19 core-parity gaps, 30 deliberate omissions, and 8 legacy/deprecated. The story of riff-vs-iTunes is not "riff is missing iTunes" — it is that **riff has already closed the parts of iTunes that matter for an offline, local, music-only library, and has consciously declined the parts that don't.** The remaining 19 gaps are real but bounded, and almost all of them are *library depth* features (metadata richness, organizational power, interop) rather than *playback* features (riff's playback core is essentially at parity).

## Bucket 1 — Core-parity gaps (19) riff should close

These are the features a credible local-library alternative is expected to have, that riff lacks or only partially covers. They split cleanly into four themes:

- **Organizational power (the biggest deficit).** Smart playlists in riff are *6 fixed discovery lists*, not a user-facing rule engine — this is the single most defining iTunes organizational feature and the only **P0**. Playlist folders (hierarchy) and duplicate detection are the next tier of library-management depth.
- **Metadata richness.** riff reads/writes the core tags and artwork, but is missing the "classical + DJ + power-user" tag set: 1–5 star rating (riff has only a boolean Favorite), Grouping, BPM, Compilation flag, and per-field sort overrides. Batch ("Get Info" multi-select) tag editing is absent — only single-track edit exists today.
- **DSP / listening UX.** A graphic equalizer (presets) is the notable playback parity gap. Sound Check is functionally *mitigated* by riff's ReplayGain implementation, so it is a gap only in the literal iTunes sense. Crossfade is a deliberate omission, not a gap.
- **Interop / portability.** No playlist export (M3U) and no iTunes-Library-XML / DJ-app bridge — both high-value for a tool whose whole point is *your* local files. Mini-player and full-screen media mode are smaller UI-parity items.

## Bucket 2 — Legacy / deprecated (8) — skip, and it's not riff's call

These are features iTunes itself removed (or Music.app dropped), so they are not a riff decision and not worth building:

| Feature | Why deprecated | Where |
|---|---|---|
| Cover Flow | Removed as default in iTunes 11 (2012) | D5 |
| iTunes DJ / Party Shuffle | Discontinued in iTunes 11, replaced by Up Next | D6 |
| Ringtones | Removed in iTunes 12.7 (2017) | D6 |
| Ping | Removed in iTunes 11 (2012); defunct social network | D7 |
| Visualizer (full-screen) | Classic visualizers dropped in Music.app | D3 |
| Genius / Genius Mixes / Genius recommendations | Cloud-analysis feature, deprecated in later macOS | D4 / D7 |

riff's "no" here is simply "don't rebuild a dead feature." No `decisions/` citation required — the rationale is iTunes' own deprecation history.

## Bucket 3 — Deliberate omissions (30) — this is what defines riff

These are the features riff declines *by design*, and they are the clearest statement of product scope. Every one traces to the offline-first / local-library thesis:

- **Commerce & streaming (never):** iTunes Store, Apple Music streaming, iTunes Match, iCloud/Sync Library, Redeem/gift, Family Sharing.
- **Device & ecosystem (never):** iPhone/iPad/iPod sync, backup/restore, Wi-Fi sync, iPod sync protocol, AirPlay/Remote, Home Sharing, shared library over LAN.
- **Non-music media (never):** Podcasts, Audiobooks, TV/Movies, iTunes U, Ringtones.
- **DRM (never):** FairPlay — riff is local-only and DRM-free by construction.
- **File-mutating ingestion (never):** auto-organize ("Keep Media folder organized") is declined because track identity *is* the file path ([`decisions/003-track-identity-is-path.md`](../decisions/003-track-identity-is-path.md)); CD import/rip, CD burn, and format conversion/transcoding are out of the offline scope ([`decisions/001-offline-first.md`](../decisions/001-offline-first.md)).
- **DSP posture (by design, but under-cited):** crossfade and per-track volume/EQ are declined — riff uses global volume + ReplayGain instead. **Caveat:** these 🚫 rows currently cite only `features.md` / `roadmap.md`, not a first-class ADR — see Open Flag #1 below and Phase 6.

**Net:** the deliberate-omission bucket is the product's moat, not a todo list. It should be presented in the report as "scope defended," not as gaps.

---

# Part B — Prioritized backlog (core-parity gaps only)

19 gaps. Priority mirrors index.md: **P0** = defining, **P1** = meaningful/power-user, **P2** = minor. Each item names the target crate(s) and a rough implementation shape, mapped to the vertical crate split in `AGENTS.md`.

Crate key: `pers` = riff-persistence · `lib` = riff-library · `play` = riff-playback · `infra` = riff-infra · `be` = riff-backend · `gui` = riff-gui.

## P0 — defining parity gap (1)

| # | Feature | Domain | Target crate(s) | Rough shape |
|---|---|---|---|---|
| 1 | **Smart playlists — user rule engine** | D4 | `pers` · `lib` · `be` · `gui` | Replace the 6 fixed discovery lists' engine with a general `RuleSet` (field / operator / value clauses, match *all*/*any*, sort + limit) persisted in the store; evaluate as a live store query exposed through a `LibrarySession` projection; add a rule-editor UI. Keeps the existing 6 lists as pre-seeded rules. |

## P1 — meaningful, expected by power users (4)

| # | Feature | Domain | Target crate(s) | Rough shape |
|---|---|---|---|---|
| 2 | **1–5 star rating** | D2 | `pers` · `infra` · `be` · `gui` | Add `rating: Option<u8>` (0–5) to `TrackMetadata`; read/write POPM (ID3) / `rate` (MP4) via lofty in `infra` media adapters; star widget in track list + Now Playing; extend `TagEditWorker`. Decide whether to keep or supersede the existing boolean `favorite`. |
| 3 | **Batch tag edit (multi-select "Get Info")** | D2 | `be` · `gui` | Generalize `TagEditRequest` to accept N `TrackId`s + a field mask; `TagEditWorker` writes the shared subset as one durable change; reuse the single-track Edit-Tags modal fields in a multi-select variant. |
| 4 | **Graphic equalizer (presets)** | D3 | `infra` · `play` · `gui` | Add an N-band IIR (biquad) filter chain to the `cpal` output adapter in `infra`; preset table + opt-in setting surfaced through `play` state and a dedicated Equalizer window in `gui`. Currently deferred (`roadmap.md:11`) — notable iTunes parity gap. |
| 5 | **Export playlist (M3U)** | D8 | `lib` · `gui` | Serialize a playlist's tracks to extended `.m3u` (absolute or library-relative paths); export action in `gui`. High interop value (portability to other players / DJ apps). M3U import is a natural follow-up but out of scope for v1. |

## P2 — nice-to-have (14)

| # | Feature | Domain | Target crate(s) | Rough shape |
|---|---|---|---|---|
| 6 | Manual drag-drop add | D1 | `gui` · `lib` | Handle OS file drag into the window; enqueue dropped path(s) to the scan service as a quick-add (per-path), separate from the folder picker. |
| 7 | Duplicate detection | D1 | `lib` · `gui` | Store query / projection clustering candidates (same artist+album+title+≈length, or acoustic hash); a "Show Duplicates" smart view. |
| 8 | Grouping tag | D2 | `pers` · `infra` · `gui` | Add `grouping` to `TrackMetadata`; read/write in `infra`; show in detail/edit UI. |
| 9 | BPM | D2 | `pers` · `infra` · `gui` | Add `bpm` field; read/write via lofty; optional sort/filter. |
| 10 | Lyrics pane | D2 | `infra` · `gui` | Embedded tag read/write for lyrics; a view pane. Deferred on `roadmap.md:19`. |
| 11 | Compilation flag | D2 | `pers` · `infra` · `gui` | Add `compilation: bool`; affects album grouping/sort. |
| 12 | Sort fields (sort artist/album/name) | D2 | `pers` · `infra` | Add `sort_*` overrides; use as sort keys in store. |
| 13 | Sound Check (volume normalization) | D3 | `infra` | **Mitigated** by ReplayGain (✅). Low priority; only a literal gap. Optionally write iTunes-compatible `iTunNORM` for cross-player parity. |
| 14 | Mini-player | D3/D5 | `gui` | Resizable, always-on-top mini window (`tray-icon`/`muda` + `egui`); listed once (consolidated from D3 & D5). |
| 15 | Playlist folders | D4 | `pers` · `lib` · `gui` | Add `PlaylistFolder` entity; hierarchical grouping in the store; sidebar tree. |
| 16 | View options sheet | D5 | `gui` | "View Options" sheet for per-column visibility + sort (beyond the column-browser sort that exists today). |
| 17 | Full-screen media mode | D5 | `gui` | OS-level full-screen toggle for the Now Playing view (today only enlarged + window maximize). |
| 18 | Library XML (iTunes Library XML) | D8 | `lib` · `gui` | Serialize library to `iTunes Music Library.xml` for DJ-app bridges. Niche but valuable interop. |
| 19 | *(tracked)* Mini-player duplicate | — | — | Already covered by #14; no separate work item. |

> **Backlog totals:** P0 ×1, P1 ×4, P2 ×14 = 19 core-parity gaps. Playback-domain gaps are only EQ (#4), Sound Check (#13, mitigated), and mini-player (#14) — confirming riff's playback core is at parity; the backlog is overwhelmingly *library depth + interop*.

---

# Open flags carried into Phase 5/6

- **Open Flag #1 (DSP scope citation):** The 🚫 rows for per-track EQ preset, per-track volume, and crossfade cite only `features.md`/`roadmap.md`, not a first-class `decisions/` ADR. Recommend adding `decisions/007-dsp-scope.md` stating riff's no-global-EQ / no-crossfade / no-per-track-volume posture so these rows have a first-class citation. (Action item for Phase 6.)
- **Mini-player is double-listed** in the Phase 3 reference (D3 + D5); the backlog tracks it once (#14).
- **Sound Check** is scored ❌ Absent but functionally mitigated by ReplayGain ✅ — reported as mitigated, not a build priority.
