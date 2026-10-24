# Phase 3 output — scored matrix (riff × iTunes 12.x + Music.app)

> Plan + vocabulary: [`phase-3-matrix.md`](./phase-3-matrix.md) · [`index.md`](../index.md).
> Inputs: [`phase-1-itunes-reference.md`](./phase-1-itunes-reference.md) (iTunes 12.x + Music.app rows) joined to [`phase-2-riff-reference.md`](./phase-2-riff-reference.md) (riff status + evidence) on `Domain + Feature`.
> Status: **scored matrix assembled** — final gap bucket + priority assigned per row; sanity-pass citations applied (see *Citations & open flags* at bottom).

## Columns
`Domain · Feature · iTunes 12.x · Music.app · riff Status · Evidence (file/line) · Gap Bucket · Priority · Notes`

## Vocabularies
- **riff Status:** ✅ Implemented · 🟡 Partial · ❌ Absent · 🚫 Won't-do (by design)
- **Gap Bucket:**
  - `—` — parity met (no gap); used for ✅ / 🟡 rows where riff covers the feature
  - **Core-parity gap** — riff should close this to be a credible local-library alternative
  - **Legacy / deprecated** — iTunes had it; dropped/legacy, not worth building
  - **Deliberate omission** — outside riff's offline-first, local-library scope (by design)
- **Priority** (core-parity gaps only): **P0** = defining for a credible local-library alternative · **P1** = meaningful, expected by power users · **P2** = minor / nice-to-have.

---

## Domain 1 — Library & ingestion

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 1 | Add folder to library (watch) | "Copy files to iTunes Media folder when adding" + "Keep iTunes Media folder organized" auto-imports watched folders | Retained (Preferences ▸ Files) | ✅ Implemented | `riff-infra/src/filesystem/scanner.rs:21-64`; `watcher.rs:12,45-63`; `riff-backend/src/app/watcher_manager.rs:32-87`; `settings.rs:1952,805` | — | | Watches + incremental rescan via notify (2s debounce), per-path toggle. **No** copy-on-add / "iTunes Media folder" import behavior (deliberate — see auto-organize row). |
| 1 | Manual add (drag-drop) | Drag file/folder onto library; copies per prefs | Retained | ❌ Absent | `sidebar.rs:640-703` (DnD is sidebar-row reorder only, not file import) | Core-parity gap | P2 | No drag-drop file import; add via folder picker / folder context. Sidebar DnD reorders rows only. |
| 1 | Auto-organize file structure | "Keep iTunes Media folder organized" rewrites `Artist/Album/Track` paths on import | Retained | 🚫 Won't-do (by design) | `scanner.rs` (scans in place only); `features.md:48` | Deliberate omission | | riff never moves user files; removing a path drops index entries, never files on disk. Rationale: track identity *is* its path — [`decisions/003-track-identity-is-path.md`](../decisions/003-track-identity-is-path.md) + offline-first scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 1 | Duplicate detection | "Show Duplicate Items" + smart-playlist rules | Retained (File ▸ Library ▸ Show Duplicate Items) | ❌ Absent | grep `duplicate` → only playlist/cache dedup, no track-duplicate clustering | Core-parity gap | P2 | |
| 1 | CD import / rip | Insert CD → Gracenote lookup → Import CD; format choices AAC/MP3/ALAC/AIFF/WAV | Retained (CD import) | 🚫 Won't-do (by design) | grep `rip|cd_import` → none; `features.md:19` | Deliberate omission | | No CD hardware access; offline scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 1 | CD burn | "Burn Playlist to Disc" (audio/MP3/data) | ⚠️ Uncertain — burn UI largely absent in Music.app | 🚫 Won't-do (by design) | grep `burn` → none | Deliberate omission | | Out of scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 1 | Import rules / format conversion | "Create AAC/MP3/ALAC/WAV version"; auto-downconvert for legacy iPods | Retained (Create [format] Version) | 🚫 Won't-do (by design) | grep `transcode|encode|format_version` → none; `features.md:19` | Deliberate omission | | Encoding/transcoding out of scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |

---

## Domain 2 — Metadata & tags

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 2 | Core tags (Name, Artist, Album, Genre, Year, Track #, Disc #, Composer) | Editable; written to ID3/MP4 | Retained | ✅ Implemented | `riff-infra/src/media/metadata_reader.rs:85-138`; `metadata_writer.rs:47-108` | — | | Read+write for title/artist/album/album_artist/track#/disc#/genre/year/composer/comment. |
| 2 | Grouping | Free tag; classical-movement grouping, smart-playlist criterion | Retained | ❌ Absent | `metadata_reader.rs:85-138` (not in extracted set) | Core-parity gap | P2 | Tag not read/stored/edited. |
| 2 | Comments | Free-form searchable field | Retained | ✅ Implemented | `metadata_reader.rs` (comment); `metadata_writer.rs` (writes comment) | — | | |
| 2 | BPM | Beats-per-minute; smart-playlist criterion | Retained | ❌ Absent | not in extracted set | Core-parity gap | P2 | |
| 2 | Star rating (1–5) | Since iTunes 3 (2002) | Retained (Settings ▸ Show Star Ratings) | ❌ Absent | `track.rs:40` (`favorite: bool` only); grep `rating|star_` → none | Core-parity gap | P1 | riff has a boolean **Favorites** flag instead of 1–5 stars. Power-user expectation; consider 1–5 or keep favorites-only. |
| 2 | Play count | Auto-incremented on play | Retained (drives Listening History) | ✅ Implemented | `track.rs:28-34`; `playback_coordinator.rs:137`; `sqlite.rs:979-988` | — | | Auto-incremented on track end. |
| 2 | Last played | Timestamp | Retained | ✅ Implemented | `track.rs`; `sqlite.rs:979-988` (`last_played_nanos`) | — | | |
| 2 | Date added | Import timestamp | Retained | ✅ Implemented | `track.rs:31-34` (set at scan) | — | | |
| 2 | Lyrics | Embedded + view pane | Retained (enhanced: live lyrics) | ❌ Absent | grep `lyric` → none; `roadmap.md:19` (deferred) | Core-parity gap | P2 | Embedded/fetched lyrics deferred on roadmap; not legacy, not scope — low urgency. |
| 2 | Per-track EQ preset | Graphic equalizer assignment per song | Retained | ❌ Absent | no EQ at all (see Domain 3) | Deliberate omission | | Subsumed by riff's no-EQ posture. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 2 | Per-track volume adjustment | "Volume Adjustment" (-100%…+100%) | Retained | ❌ Absent | `TrackMetadata` lacks per-track volume (`track.rs:93-111`); only global volume | Deliberate omission | | Only global volume + auto ReplayGain. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 2 | Album artwork | Embedded + Cover Flow / grid | Retained | ✅ Implemented | `riff-library` `cover_resolver.rs:30-95`; `cover_loader.rs`; `cover_service.rs:14` (LRU=50) | — | | Embedded then filesystem fallback (`cover/folder/album/front.{jpg,png}`); LRU cache + placeholder. |
| 2 | Part of a Compilation flag | Affects grouping/sort | Retained | ❌ Absent | not stored; album grouping via `album_artist` (`features.md:50`) | Core-parity gap | P2 | |
| 2 | Sort fields (sort artist/album/name) | Separate sort overrides | Retained | ❌ Absent | grep `sort_artist|sort_album|SortField` → none | Core-parity gap | P2 | |
| 2 | ID3 / MP4 tag read-write | Native | Retained | ✅ Implemented | `metadata_reader.rs` / `metadata_writer.rs` (lofty) | — | | |
| 2 | Batch tag edit ("Get Info" multi-select) | Yes | Retained | ❌ Absent | `app.rs:3371` (single-track modal); submit emits one `TagEditRequest` (`app.rs:3190-3217`) | Core-parity gap | P1 | Only single-track Edit Tags (advanced mode). Multi-select edit is a real power-user need. |

---

## Domain 3 — Playback

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 3 | Up Next / queue | Introduced iTunes 11 (2012), replaced iTunes DJ | Retained | ✅ Implemented | `riff-playback/src/domain/queue.rs:55-64` (append), `:81-87` (insert_next), `:140-226` (advance/previous); `transport.rs:173-184` | — | | Insert-after-current = "Play Next"; shuffle/repeat are session flags. |
| 3 | Repeat / shuffle | Yes | Retained | ✅ Implemented | `queue.rs:124-131` (`toggle_repeat` None→All→One); `playback.rs:12-18` (`RepeatMode`) | — | | |
| 3 | Gapless playback | Since iTunes 7.0 (2006) | Retained | ✅ Implemented | `riff-playback/src/app/gapless.rs` (pre-decode ~2s before EOF, pre-buffer ≤4s) | — | | Same-format seamless; format mismatch/shuffle falls back to gapped path. |
| 3 | Crossfade | Yes | Retained (Playback prefs) | 🚫 Won't-do (by design) | `features.md:21` ("intentionally not provided") | Deliberate omission | | Stated intentional in `features.md:21`. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 3 | Sound Check (volume normalization) | Since iTunes 3 (2002) | Retained (Playback prefs) | ❌ Absent | grep `SoundCheck|iTunNORM` → none | Core-parity gap | P2 | riff uses **ReplayGain tag-based** normalization instead (row below) — functionally mitigated. |
| 3 | ReplayGain (tag-based track gain/peak) | *no native iTunes analog (iTunes uses Sound Check)* | *n/a (riff mechanism)* | ✅ Implemented | `metadata_reader.rs:130-135`; `audio_output.rs:142-151`; `state.rs:9-23` (peak-capped); `settings.rs:223` | — | | Opt-in, takes effect on next track, untagged untouched. This is riff's loudness-normalization analog of Sound Check. |
| 3 | Graphic equalizer (presets) | Since iTunes 2 (2001) | Retained (Window ▸ Equalizer) | ❌ Absent | grep `equaliz|graphic_eq|biquad` → only animated bars (`sidebar.rs:53-91`); `roadmap.md:11` (deferred) | Core-parity gap | P1 | Deferred nice-to-have; notable iTunes parity gap. |
| 3 | Per-track volume | Volume Adjustment | Retained | ❌ Absent | only global volume + auto ReplayGain | Deliberate omission | | See Domain 2 per-track volume. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 3 | AirPlay / Remote | AirPlay menu; AirPlay 2 multi-room since 12.8 (2018) | Retained (AirPlay menu) | 🚫 Won't-do (by design) | grep `airplay|remote` → none; `features.md:128` (offline) | Deliberate omission | | Offline-first scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md); [`decisions/006-local-only-discovery.md`](../decisions/006-local-only-discovery.md). |
| 3 | Visualizer (full-screen) | Built-in plug-in visualizer (default since 8.0, 2008) | ⚠️ Not carried over — classic visualizers dropped | ❌ Absent | grep `visualiz` → none (sidebar "equalizer bars" are non-FFT animation) | Legacy / deprecated | | Music.app also dropped classic visualizers; iTunes legacy feature. Not a riff decision. |
| 3 | Lyrics view | Lyrics pane | Retained (enhanced) | ❌ Absent | see Domain 2 Lyrics | Core-parity gap | P2 | Deferred (see Domain 2). |
| 3 | Mini-player | MiniPlayer (since 12, resizable, keep-on-top) | Retained | ❌ Absent | grep `mini.?player` → none | Core-parity gap | P2 | |
| 3 | Scrobbling (Last.fm) | **No native** — third-party via AppleScript/plugins only | N/A (same — no native) | 🚫 Won't-do (by design) | grep `scrobbl|lastfm` → none; `roadmap.md:21` | Deliberate omission | | Offline-only by design [`decisions/001-offline-first.md`](../decisions/001-offline-first.md); iTunes also lacked native scrobbling. |

---

## Domain 4 — Organization

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 4 | Standard playlists | Yes | Retained | ✅ Implemented | `playlist_manager.rs:100-130`; dedup `store.rs:257`; missing-track handling `playlist_manager.rs:19-35` | — | | Create/rename/reorder/delete, persist, dedup, struck-through "(missing)" excluded from playback. |
| 4 | Smart playlists | Since iTunes 3 (2002); rule-based, auto-updating | Retained | 🟡 Partial | `track.rs:54-86` (6 fixed kinds); `sqlite.rs:2082-2231` | Core-parity gap | P0 | **6 fixed** discovery lists (Favorites, Recently Added, Most Played, Recently Played, Never Played, Lost Gems) computed as store queries — **not** a user rule engine. The single most defining iTunes organizational feature; a user-facing rule engine is the key parity gap. |
| 4 | Playlist folders | Hierarchical folder grouping | Retained | ❌ Absent | grep `PlaylistFolder|playlist_folder` → none | Core-parity gap | P2 | |
| 4 | Genius mixes | Genius (8.0, 2008) → Genius Mixes (9.0, 2009) | Retained initially; ⚠️ deprecated in later macOS | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy | | Requires cloud analysis [`decisions/001-offline-first.md`](../decisions/001-offline-first.md); also deprecated in Music.app. |
| 4 | Genius (single-track) | Right-click → Start Genius | Retained initially; same deprecation caveat | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy | | Same as Genius mixes. |

---

## Domain 5 — UI / navigation

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 5 | Sidebar / source list | Hidden by default in 11, restored via Option+⌘+S in 12 | Retained (View ▸ Show Sidebar) | ✅ Implemented | `sidebar.rs` (LIBRARY/FOLDERS/PLAYLISTS + live counts `app.rs:2077`) | — | | |
| 5 | Column browser | Genre/Artist/Album (3-col; left-layout since 9.0) | Retained (View ▸ Column Browser) | ✅ Implemented | `browser.rs` (BrowserColumn, genre chips `:78-87`); `detail.rs:18-49` (breadcrumb + inspector) | — | | Elastic columns: Genres→Artists→Albums→Tracks. |
| 5 | Cover Flow | Introduced 7.0 (2006); dropped as default + removed in 11 (2012) | Dropped (legacy) — already gone before Music.app | 🚫 Won't-do (by design) | grep `coverflow` → none; dropped in iTunes 11 | Legacy / deprecated | | Removed in iTunes 11; legacy, not a riff decision. |
| 5 | Grid view | Introduced 8.0; default since 11 | Retained (enhanced) | 🚫 Removed (by design) | grid retired end-to-end (titlebar-search spec): `browser.rs` is list-only; migration 012 drops the `browser_layout` column | Deliberate omission | | Browser is permanently list-only; no toggle, no tile path. |
| 5 | View options (columns/sort) | View Options sheet | Retained | 🟡 Partial | column-browser sort; no column-picker sheet | Core-parity gap | P2 | Sort present; no per-column visibility sheet. |
| 5 | Search (library + store) | Unified search + trending (12) | Retained (library / Apple Music scope toggle) | ✅ Implemented | `chrome.rs` (titlebar search, global Ctrl+K) | — | | Library search only (no Store scope — offline). |
| 5 | Sort | Click headers; multi-key | Retained | ✅ Implemented | column browser + clickable headers; `projection/counts.rs` | — | | |
| 5 | Now Playing | Persistent now-playing pane | Retained | ✅ Implemented | `now_playing.rs` (cover 240px, metadata, clickable up-next) | — | | |
| 5 | Full-screen | Yes | Retained | 🟡 Partial | `playerbar.rs:302-354` (expand→Now Playing view); `chrome.rs:241-242` (window Maximize); no `set_fullscreen` | Core-parity gap | P2 | Enlarged Now Playing + window maximize, but no OS-level full-screen media mode. |
| 5 | Mini-player | Yes | Retained | ❌ Absent | grep `mini.?player` → none | Core-parity gap | P2 | (Listed in both D3 and D5 in the reference; tracked once here as P2.) |

---

## Domain 6 — Ecosystem (all offline-only by design)

> Rationale: `roadmap.md:21`, `features.md:128`, [`decisions/001-offline-first.md`](../decisions/001-offline-first.md), [`decisions/006-local-only-discovery.md`](../decisions/006-local-only-discovery.md). grep for `itunes|ipod|airplay|podcast|audiobook|ringtone|fairplay|drm|icloud|apple.?music|home.?shar|family.?shar` returned **zero** implementation hits.

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 6 | iTunes Store (buy/rent music) | Integrated storefront | Retained inside Music.app | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | [`decisions/001-offline-first.md`](../decisions/001-offline-first.md) + [`decisions/006-local-only-discovery.md`](../decisions/006-local-only-discovery.md). |
| 6 | Apple Music streaming | Added 12.2 (June 2015) | Retained | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Same as Store. |
| 6 | iTunes Match | Added 10.5 (Oct 2011) | Retained | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Cloud sync — out of scope. |
| 6 | iCloud Music Library / Sync Library | Cloud library sync | Retained (Sync Library) | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Cloud sync — out of scope. |
| 6 | FairPlay DRM | Music DRM-free since 2009; movies/TV still protected | Retained (purchased video) | 🚫 Won't-do (by design) | grep `fairplay|drm` → none | Deliberate omission | | Offline/local only. |
| 6 | Device sync (iPhone/iPad/iPod) | Cable + Wi-Fi sync | Moved → Finder | 🚫 Won't-do (by design) | grep `ipod` → none | Deliberate omission | | Device management out of scope. |
| 6 | Backup / restore | Via device pane | Moved → Finder | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Device management out of scope. |
| 6 | Wi-Fi sync | Yes | Moved → Finder | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Device management out of scope. |
| 6 | Home Sharing | Since 9.0 (2009) | Retained (shared library over LAN) | 🚫 Won't-do (by design) | grep `home.?shar` → none | Deliberate omission | | LAN sharing out of offline-local scope. |
| 6 | iTunes DJ | Party Shuffle (5.0, 2005) → iTunes DJ; discontinued in 11 | Dropped (legacy) — replaced by Up Next | 🚫 Won't-do (by design) | grep → none | Legacy / deprecated | | Dropped in iTunes 11; legacy. |
| 6 | Podcasts | In-app | Moved → Apple Podcasts | 🚫 Won't-do (by design) | grep `podcast` → none | Deliberate omission | | Non-music media out of scope. |
| 6 | Audiobooks | In-app | Moved → Apple Books | 🚫 Won't-do (by design) | grep `audiobook` → none | Deliberate omission | | Non-music media out of scope. |
| 6 | TV / Movies | In-app store + library | Moved → Apple TV | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Non-music media out of scope. |
| 6 | iTunes U | In-app | Moved → Podcasts/Books (discontinued) | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Non-music media out of scope. |
| 6 | Ringtones | Removed in 12.7 (2017) | Dropped (legacy) | 🚫 Won't-do (by design) | grep `ringtone` → none | Legacy / deprecated | | Removed in iTunes 12.7; legacy. |
| 6 | Redeem / gift | Gift cards + Redeem (incl. webcam) | Retained (store) | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Store commerce out of scope. |
| 6 | Family Sharing | Since 12.0 (2014) | Retained | 🚫 Won't-do (by design) | grep `family.?shar` → none | Deliberate omission | | Account/cloud out of scope. |

---

## Domain 7 — Sharing / social

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 7 | Home Sharing | Shared library over LAN | Retained | 🚫 Won't-do (by design) | see Domain 6 | Deliberate omission | | See Domain 6 Home Sharing. |
| 7 | Shared library over LAN | Yes | Retained (shared library) | 🚫 Won't-do (by design) | grep → none | Deliberate omission | | Offline-local scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 7 | Genius recommendations | Library-based suggestions | Retained initially (deprecation caveat) | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy | | Cloud analysis [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 7 | Ping (social) | Added 10.0 (2010); removed in 11 (2012), replaced by Twitter/FB | Dropped (legacy/defunct) | 🚫 Won't-do (by design) | grep → none | Legacy / deprecated | | Removed in iTunes 11; defunct social network. |

---

## Domain 8 — Interop / export

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 8 | Export playlist (M3U) | Export selection / playlist | Retained (export playlist file) | ❌ Absent | grep `m3u` → none | Core-parity gap | P1 | No playlist export of any format. High-value interop for a local-library tool (portability to other players / DJ apps). |
| 8 | iTunes Library XML | Live `iTunes Music Library.xml` (DJ-app bridge) | **Not carried over** — Music.app stopped maintaining the live XML (breaking change for DJ apps, Catalina) | ❌ Absent | grep `itunes|library_xml|\.xml|export` → none (only asset/migration files) | Core-parity gap | P2 | No XML / DJ-app bridge. Niche but valuable interop. |
| 8 | iPod sync protocol | Native device sync | Moved → Finder (no public protocol) | 🚫 Won't-do (by design) | see Domain 6 device sync | Deliberate omission | | Device sync out of scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |

---

## Rollup

> **Phase 6 correction (2026-09-15).** The figures below were re-computed directly from the 75 per-row statuses and found to differ from the original summary (which read ✅19 / 🟡5 / ❌17 / 🚫34 and buckets 24/19/30/8 — the latter summing to 81, not 75; 🟡 also listed "Sound Check→ReplayGain" as two phantom partials while the matrix scores Sound Check ❌). The corrected counts match the Phase 5 deliverables (`itunes-comparison.md` §2). Per-row statuses in this matrix are the source of truth; only this summary was wrong.

### By riff Status (of 75 matrix rows)
| Status | Count | Rows |
|---|---|---|
| ✅ Implemented | 18 | D1 watch · D2 core tags, comments, play count, last played, date added, artwork, ID3/MP4 · D3 up-next, repeat/shuffle, gapless, ReplayGain · D4 std playlists · D5 sidebar, column browser, search, sort, now playing |
| 🟡 Partial | 3 | D4 smart playlists · D5 view options, full-screen |
| ❌ Absent | 21 | D1 manual-add, duplicate · D2 grouping, BPM, star rating, lyrics, per-track EQ preset, per-track volume, compilation, sort fields, batch tag edit · D3 sound check, graphic EQ, per-track volume, visualizer, lyrics view, mini-player · D4 playlist folders · D5 mini-player · D8 M3U, library XML |
| 🚫 Won't-do | 33 | D1 auto-organize, CD import, CD burn, format conversion · D3 crossfade, airplay, scrobbling · D4 genius mixes, genius · D5 cover flow, grid · D6 ×17 · D7 ×4 · D8 iPod protocol |

> (Counts: ✅ 18, 🟡 3, ❌ 21, 🚫 33 = 75.)

### By Gap Bucket
| Bucket | Count |
|---|---|
| — (parity met) | 19 |
| Core-parity gap | 20 |
| Deliberate omission | 28 |
| Legacy / deprecated | 8 |

> The 3 Genius rows (D4 genius mixes, D4 genius, D7 Genius recommendations) carry a hybrid "Deliberate omission / Legacy" label and are counted as **Legacy** here (per Phase 4, Bucket 2). Mini-player is double-listed in D3 and D5; it is one core-parity *feature* (see Priority).

### By Priority (core-parity gaps only)
| Priority | Count | Rows |
|---|---|---|
| P0 | 1 | D4 Smart playlists (user rule engine) |
| P1 | 4 | D2 Star rating, D2 Batch tag edit, D3 Graphic equalizer, D8 Export playlist (M3U) |
| P2 | 14 | D1 manual-add, D1 duplicate · D2 grouping, BPM, lyrics, compilation, sort fields · D3 sound check (mitigated), lyrics view, mini-player · D4 playlist folders · D5 view options, full-screen, mini-player · D8 library XML |

> **19 distinct core-parity features** — the 20 core-parity rows collapse to 19 because Mini-player is listed in both D3 and D5 and counts once in the backlog. P0 ×1 + P1 ×4 + P2 ×14 = 19.

---

## Citations & open flags (sanity pass)

**✅ Sanity rule — every 🚫 cites a "by design" rationale:**
- Scope-based deliberate omissions (store/streaming/sync/sharing/DRM/CD/device/airplay/scrobbling/podcasts/audiobooks/TV/non-music) cite [`decisions/001-offline-first.md`](../decisions/001-offline-first.md) and/or [`decisions/006-local-only-discovery.md`](../decisions/006-local-only-discovery.md).
- Auto-organize cites [`decisions/003-track-identity-is-path.md`](../decisions/003-track-identity-is-path.md) (won't move files because identity is path).
- Legacy / deprecated 🚫 (Cover Flow, iTunes DJ, Ringtones, Ping, Visualizer) are iTunes-removed features — rationale is iTunes' own deprecation, not a riff decision; no `decisions/` doc required.

**⚠️ Open flag #1 — DSP/UX 🚫 lacking a dedicated `decisions/` doc:**
The following "by design" omissions are asserted from `features.md` / `roadmap.md` but have **no formal `decisions/` ADR**:
- D2 Per-track EQ preset, D2 Per-track volume adjustment, D3 Per-track volume → no-EQ posture (subsumed by ReplayGain/global volume).
- D3 Crossfade → `features.md:21` "intentionally not provided".
- D3 Graphic equalizer is ❌ Absent (deferred, `roadmap.md:11`), not 🚫.

*Recommendation:* add a short `decisions/007-dsp-scope.md` (or similar) stating riff's DSP posture — no global EQ / crossfade / per-track volume — so these 🚫 rows have a first-class citation. This is an input for **Phase 6 (verification)**.

**✅ Sanity rule — every 📋 cites a doc/ADR/roadmap:** no 📋 (Planned) rows exist in the Phase 2 inventory; all deferred items are scored ❌ Absent with a `roadmap.md` citation instead. No action needed.

**Carry-over confidence flags (from Phase 1):** Visualizer + CD burn in Music.app are medium-confidence "not carried over" — riff scores them by iTunes 12.x capability; the Music.app ambiguity does not change riff's 🚫/❌ call. Re-verify in Phase 6 only if reported.

---

## What this feeds
Phase 4 (gap narrative + prioritized backlog) consumes this matrix: the **P0/P1** rows are the backlog; the **Core-parity gap** bucket is the narrative spine; **Deliberate omission / Legacy** rows become the "scope defended" section. Phase 5 renders `itunes-comparison.md` (report), `itunes-comparison.xlsx` (color-coded filterable sheet), and `itunes-comparison.csv` (diff-able export) from this file.
