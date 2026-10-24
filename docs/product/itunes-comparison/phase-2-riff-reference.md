# Phase 2 output — riff grounded inventory (source, not guessing)

> Feeds: Phase 3 (scored matrix). Companion plan + vocabulary: [`index.md`](../index.md); iTunes reference rows: [`phase-1-itunes-reference.md`](./phase-1-itunes-reference.md).
> Status: **draft riff inventory produced** — `riff Status · Evidence · Provisional Gap Bucket · Notes` filled per Phase 1 `Domain + Feature` keys. `Priority` is intentionally left blank (Phase 3 scores).
> Method: **Explore subagent (very thorough)** scanned `crates/*` for real capabilities + cross-checked `docs/product/features.md`, `docs/product/roadmap.md`, `docs/product/decisions/*`. EQ / Sound Check / crossfade / per-track-volume were *verified absent*, not assumed (per index.md open flag #3).

Status vocabulary: ✅ Implemented · 🟡 Partial · 📋 Planned · ❌ Absent · 🚫 Won't-do (by design).
`Provisional Gap Bucket` is a Phase-3 aid only — final bucketing happens in Phase 3/4.

---

## Domain 1 — Library & ingestion

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| Add folder to library (watch) | ✅ Implemented | `riff-infra/src/filesystem/scanner.rs:21-64`; `riff-infra/src/filesystem/watcher.rs:12,45-63`; `riff-backend/src/app/watcher_manager.rs:32-87`; `settings.rs:1952,805` | Deliberate omission (partial) | Watches + incremental rescan via notify (2s debounce), per-path toggle. **No** copy-on-add / "iTunes Media folder" import behavior. |
| Manual add (drag-drop) | ❌ Absent | `sidebar.rs:640-703` (DnD is sidebar-row reorder only, not file import) | Core-parity gap (minor) | No drag-drop file import; add via folder picker / folder context. Sidebar DnD reorders rows only. |
| Auto-organize file structure | 🚫 Won't-do (by design) | `scanner.rs` (scans in place only); `features.md:48` | Deliberate omission | riff never moves user files; removing a path drops index entries, never files on disk. |
| Duplicate detection | ❌ Absent | grep `duplicate` → only playlist/cache dedup, no track-duplicate clustering | Core-parity gap (minor) | |
| CD import / rip | 🚫 Won't-do (by design) | grep `rip|cd_import` → none; `features.md:19` | Deliberate omission | No CD hardware access; offline scope. |
| CD burn | 🚫 Won't-do (by design) | grep `burn` → none | Deliberate omission | Out of scope. |
| Import rules / format conversion | 🚫 Won't-do (by design) | grep `transcode|encode|format_version` → none; `features.md:19` | Deliberate omission | Encoding/transcoding out of scope. |

---

## Domain 2 — Metadata & tags

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| Core tags (Name, Artist, Album, Genre, Year, Track #, Disc #, Composer) | ✅ Implemented | `riff-infra/src/media/metadata_reader.rs:85-138`; `metadata_writer.rs:47-108` | — | Read+write for title/artist/album/album_artist/track#/disc#/genre/year/composer/comment. |
| Grouping | ❌ Absent | `metadata_reader.rs:85-138` (not in extracted set) | Core-parity gap (minor) | Tag not read/stored/edited. |
| Comments | ✅ Implemented | `metadata_reader.rs` (comment); `metadata_writer.rs` (writes comment) | — | |
| BPM | ❌ Absent | not in extracted set | Core-parity gap (minor) | |
| Star rating (1–5) | ❌ Absent | `track.rs:40` (`favorite: bool` only); grep `rating|star_` → none | Core-parity gap | riff has a boolean **Favorites** flag instead of 1–5 stars. |
| Play count | ✅ Implemented | `track.rs:28-34`; `playback_coordinator.rs:137`; `sqlite.rs:979-988` | — | Auto-incremented on track end. |
| Last played | ✅ Implemented | `track.rs`; `sqlite.rs:979-988` (`last_played_nanos`) | — | |
| Date added | ✅ Implemented | `track.rs:31-34` (set at scan) | — | |
| Lyrics | ❌ Absent | grep `lyric` → none; `roadmap.md:19` (deferred) | Legacy/deferred | Embedded/fetched lyrics deferred. |
| Per-track EQ preset | ❌ Absent | no EQ at all (see Domain 3) | Deliberate omission | |
| Per-track volume adjustment | ❌ Absent | `TrackMetadata` lacks per-track volume (`track.rs:93-111`); only global volume | Deliberate omission | |
| Album artwork | ✅ Implemented | `riff-library` `cover_resolver.rs:30-95`; `cover_loader.rs`; `cover_service.rs:14` (LRU=50) | — | Embedded then filesystem fallback (`cover/folder/album/front.{jpg,png}`); LRU cache + placeholder. |
| Part of a Compilation flag | ❌ Absent | not stored; album grouping via `album_artist` (`features.md:50`) | Core-parity gap (minor) | |
| Sort fields (sort artist/album/name) | ❌ Absent | grep `sort_artist|sort_album|SortField` → none | Core-parity gap (minor) | |
| ID3 / MP4 tag read-write | ✅ Implemented | `metadata_reader.rs` / `metadata_writer.rs` (lofty) | — | |
| Batch tag edit ("Get Info" multi-select) | ❌ Absent | `app.rs:3371` (single-track modal); submit emits one `TagEditRequest` (`app.rs:3190-3217`) | Core-parity gap | Only single-track Edit Tags (advanced mode). |

---

## Domain 3 — Playback

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| Up Next / queue | ✅ Implemented | `riff-playback/src/domain/queue.rs:55-64` (append), `:81-87` (insert_next), `:140-226` (advance/previous); `transport.rs:173-184` | — | Insert-after-current = "Play Next"; shuffle/repeat are session flags. |
| Repeat / shuffle | ✅ Implemented | `queue.rs:124-131` (`toggle_repeat` None→All→One); `playback.rs:12-18` (`RepeatMode`) | — | |
| Gapless playback | ✅ Implemented | `riff-playback/src/app/gapless.rs` (pre-decode ~2s before EOF, pre-buffer ≤4s) | — | Same-format seamless; format mismatch/shuffle falls back to gapped path. |
| Crossfade | 🚫 Won't-do (by design) | `features.md:21` ("intentionally not provided") | Deliberate omission | |
| Sound Check (volume normalization) | ❌ Absent | grep `SoundCheck|iTunNORM` → none | Core-parity gap (mitigated) | riff uses **ReplayGain tag-based** normalization instead (row below). |
| ReplayGain (tag-based track gain/peak) | ✅ Implemented | `metadata_reader.rs:130-135`; `audio_output.rs:142-151`; `state.rs:9-23` (peak-capped); `settings.rs:223` | — | Opt-in, takes effect on next track, untagged untouched. Analog of loudness normalization. |
| Graphic equalizer (presets) | ❌ Absent | grep `equaliz|graphic_eq|biquad` → only animated bars (`sidebar.rs:53-91`); `roadmap.md:11` (deferred) | Core-parity gap | Deferred nice-to-have. |
| Per-track volume | ❌ Absent | only global volume + auto ReplayGain | Deliberate omission | |
| AirPlay / Remote | 🚫 Won't-do (by design) | grep `airplay|remote` → none; `features.md:128` (offline) | Deliberate omission | |
| Visualizer (full-screen) | ❌ Absent | grep `visualiz` → none (sidebar "equalizer bars" are non-FFT animation) | Legacy/deprecated | Music.app also dropped classic visualizers. |
| Lyrics view | ❌ Absent | see Domain 2 Lyrics | Legacy/deferred | |
| Mini-player | ❌ Absent | grep `mini.?player` → none | Core-parity gap (minor) | |
| Scrobbling (Last.fm) | 🚫 Won't-do (by design) | grep `scrobbl|lastfm` → none; `roadmap.md:21` | Deliberate omission | Offline-only by design. |

---

## Domain 4 — Organization

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| Standard playlists | ✅ Implemented | `playlist_manager.rs:100-130`; dedup `store.rs:257`; missing-track handling `playlist_manager.rs:19-35` | — | Create/rename/reorder/delete, persist, dedup, struck-through "(missing)" excluded from playback. |
| Smart playlists | 🟡 Partial | `track.rs:54-86` (6 fixed kinds); `sqlite.rs:2082-2231` | Core-parity gap | **6 fixed** discovery lists (Favorites, Recently Added, Most Played, Recently Played, Never Played, Lost Gems) computed as store queries — **not** a user rule engine. |
| Playlist folders | ❌ Absent | grep `PlaylistFolder|playlist_folder` → none | Core-parity gap (minor) | |
| Genius mixes | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy | Requires cloud analysis. |
| Genius (single-track) | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy | |

---

## Domain 5 — UI / navigation

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| Sidebar / source list | ✅ Implemented | `sidebar.rs` (LIBRARY/FOLDERS/PLAYLISTS + live counts `app.rs:2077`) | — | |
| Column browser | ✅ Implemented | `browser.rs` (BrowserColumn, genre chips `:78-87`); `detail.rs:18-49` (breadcrumb + inspector) | — | Elastic columns: Genres→Artists→Albums→Tracks. |
| Cover Flow | 🚫 Won't-do (by design) | grep `coverflow` → none; dropped in iTunes 11 | Legacy/deprecated | |
| Grid view | 🚫 Removed (by design) | grid retired end-to-end (titlebar-search spec): `browser.rs` is list-only; migration 012 drops the `browser_layout` column | Deliberate omission | Browser is permanently list-only; no toggle, no tile path. |
| View options (columns/sort) | 🟡 Partial | column-browser sort; no column-picker sheet | Core-parity gap (minor) | Sort present; no per-column visibility sheet. |
| Search (library + store) | ✅ Implemented | `chrome.rs` (titlebar search, global Ctrl+K) | — | Library search only (no Store scope — offline). |
| Sort | ✅ Implemented | column browser + clickable headers; `projection/counts.rs` | — | |
| Now Playing | ✅ Implemented | `now_playing.rs` (cover 240px, metadata, clickable up-next) | — | |
| Full-screen | 🟡 Partial | `playerbar.rs:302-354` (expand→Now Playing view); `chrome.rs:241-242` (window Maximize); no `set_fullscreen` | Core-parity gap (minor) | Enlarged Now Playing + window maximize, but no OS-level full-screen media mode. |
| Mini-player | ❌ Absent | grep `mini.?player` → none | Core-parity gap (minor) | |

---

## Domain 6 — Ecosystem (all offline-only by design)

> Rationale: `roadmap.md:21`, `features.md:128`, `docs/product/decisions/001-offline-first.md`. grep for `itunes|ipod|airplay|podcast|audiobook|ringtone|fairplay|drm|icloud|apple.?music|home.?shar|family.?shar` returned **zero** implementation hits.

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| iTunes Store (buy/rent music) | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| Apple Music streaming | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| iTunes Match | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| iCloud Music Library / Sync Library | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| FairPlay DRM | 🚫 Won't-do (by design) | grep `fairplay|drm` → none | Deliberate omission | |
| Device sync (iPhone/iPad/iPod) | 🚫 Won't-do (by design) | grep `ipod` → none | Deliberate omission | |
| Backup / restore | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| Wi-Fi sync | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| Home Sharing | 🚫 Won't-do (by design) | grep `home.?shar` → none | Deliberate omission | |
| iTunes DJ | 🚫 Won't-do (by design) | grep → none | Legacy/deprecated | Dropped in iTunes 11. |
| Podcasts | 🚫 Won't-do (by design) | grep `podcast` → none | Deliberate omission | |
| Audiobooks | 🚫 Won't-do (by design) | grep `audiobook` → none | Deliberate omission | |
| TV / Movies | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| iTunes U | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| Ringtones | 🚫 Won't-do (by design) | grep `ringtone` → none | Legacy/deprecated | Removed in iTunes 12.7. |
| Redeem / gift | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| Family Sharing | 🚫 Won't-do (by design) | grep `family.?shar` → none | Deliberate omission | |

---

## Domain 7 — Sharing / social

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| Home Sharing | 🚫 Won't-do (by design) | see Domain 6 | Deliberate omission | |
| Shared library over LAN | 🚫 Won't-do (by design) | grep → none | Deliberate omission | |
| Genius recommendations | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy | |
| Ping (social) | 🚫 Won't-do (by design) | grep → none | Legacy/defunct | Removed in iTunes 11. |

---

## Domain 8 — Interop / export

| Feature | riff Status | Evidence (file:line) | Provisional Gap Bucket | Notes |
|---|---|---|---|---|
| Export playlist (M3U) | ❌ Absent | grep `m3u` → none | Core-parity gap | No playlist export of any format. |
| iTunes Library XML | ❌ Absent | grep `itunes|library_xml|\.xml|export` → none (only asset/migration files) | Core-parity gap | No XML / DJ-app bridge. |
| iPod sync protocol | 🚫 Won't-do (by design) | see Domain 6 device sync | Deliberate omission | |

---

## Verification notes & discrepancies vs `features.md`

- **EQ / Sound Check / crossfade / per-track-volume** confirmed *genuinely absent* (not assumed), per index.md open flag #3. Only ReplayGain tag-based normalization exists.
- **Smart playlists**: `features.md:67` lists 5 (Recently Added, Most Played, Recently Played, Never Played, Lost Gems) and `roadmap.md:47` says "four"; source actually defines **6** (`track.rs:54-86`) — the extra is **Favorites**, derived from the boolean `favorite` flag. Map reflects 6.
- **Cover art clamp**: `features.md:97` claims 300px (Now Playing) / 200px (detail); source uses **240px** (`now_playing.rs:32`) / 200px (`selection.rs:72`). Fixed-size clamp present either way.
- **Drag-and-drop add**: only sidebar-row reordering DnD exists (`sidebar.rs:640-703`); no file-import DnD — scored as Absent.
- **Full-screen**: no OS `set_fullscreen` call; only expand-to-Now-Playing + window maximize — scored 🟡 Partial.

## What this feeds

Phase 3 assembles the full matrix (`Domain · Feature · iTunes 12.x · Music.app · riff Status · Evidence · Gap Bucket · Priority · Notes`) by joining [`phase-1-itunes-reference.md`](./phase-1-itunes-reference.md) to this file on `Domain + Feature` keys. `Priority` and final `Gap Bucket` are scored in Phase 3/4.
