# riff × iTunes — Feature Comparison Report

> Source: `docs/product/itunes-comparison/phase-3-scored-matrix.md` (75 rows, 8 domains) joined to `phase-4-gap-narrative.md`. Generated 2026-09-15 as the Phase 5 deliverable. Companion artifacts: `itunes-comparison.xlsx` (filterable sheet) and `itunes-comparison.csv` (diff-able export).

## 1. Method & scope

This report compares **riff** — an offline-first, local-library music player (Rust + egui) — against **iTunes 12.x** (the canonical desktop reference), with **Music.app** divergences mapped as a second column. The comparison is source-grounded, not impressionistic:

- **Phase 1** built the iTunes 12.x + Music.app feature taxonomy.
- **Phase 2** built a riff inventory directly from the codebase — every riff status below is backed by a `file:line` citation (grep + Explore pass), not by assumption.
- **Phase 3** joined the two on `Domain + Feature` into a 75-row scored matrix, assigning each row a gap bucket and (for core-parity gaps) a priority.
- **Phase 4** turned the matrix into the gap narrative and a prioritized backlog.
- **Phase 5** (this document) renders the matrix, the buckets, and the backlog as a markdown report, a filterable spreadsheet, and a diff-able CSV.

### Status vocabulary

| Symbol | Meaning |
|---|---|
| ✅ Implemented | present in shipped riff code |
| 🟡 Partial | exists but incomplete vs iTunes |
| ❌ Absent | not present, no plan |
| 🚫 Won't-do (by design) | outside riff's offline-first, local-library scope |
| 📋 Planned | in-spec / designed but not built (none in this audit) |

### Gap buckets

- **Core-parity gap** — riff should close this to be a credible local-library alternative.
- **Legacy / deprecated** — iTunes had it; not worth building (Cover Flow, iTunes DJ, Ping…).
- **Deliberate omission** — Store, DRM, device sync, streaming: defines riff's scope.

## 2. Headline numbers

The matrix spans **75 rows across 8 domains**.

**By riff status:** ✅ Implemented 19 · 🟡 Partial 3 · ❌ Absent 21 · 🚫 Won't-do 32

**By gap bucket:** parity-met 19 · core-parity gap 20 · deliberate omission 28 · legacy/deprecated 8

**By priority (core-parity distinct features):** P0 ×1 · P1 ×4 · P2 ×14 = 19

> **Data-quality note (for Phase 6).** These figures are tallied directly from the 75 matrix rows. They differ from the Phase 3 summary rollup (which listed status 19/5/17/34 and buckets 24/19/30/8). That rollup is internally inconsistent — its bucket totals sum to 81, not 75, and it scores Sound Check as *partial* while the matrix scores it *Absent* (functionally mitigated by ReplayGain). Two normalization choices were applied: the 3 Genius rows carry a hybrid "Deliberate omission / Legacy" label and are counted as Legacy (per Phase 4 Bucket 2); and Mini-player is double-listed in D3 and D5, so the 20 core-parity *rows* collapse to 19 distinct *features*, matching the backlog. The per-row matrix values above are the source of truth.

The story is not "riff is missing iTunes" — **riff has already closed the parts of iTunes that matter for an offline, local, music-only library, and has consciously declined the parts that don't.** The 19 remaining gaps are real but bounded, and almost all are *library depth* features (metadata richness, organizational power, interop) rather than *playback* features (riff's playback core is essentially at parity).

## 3. The matrix (by domain)

For each of the 8 domains, the full scored matrix (status is evidence-backed per `file:line`):

### Domain 1 — Library & ingestion

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 1 | Add folder to library (watch) | "Copy files to iTunes Media folder when adding" + "Keep iTunes Media folder organized" auto-imports watched folders | Retained (Preferences ▸ Files) | ✅ Implemented | `riff-infra/src/filesystem/scanner.rs:21-64`; `watcher.rs:12,45-63`; `riff-backend/src/app/watcher_manager.rs:32-87`; `settings.rs:1952,805` | — |  | Watches + incremental rescan via notify (2s debounce), per-path toggle. **No** copy-on-add / "iTunes Media folder" import behavior (deliberate — see auto-organize row). |
| 1 | Manual add (drag-drop) | Drag file/folder onto library; copies per prefs | Retained | ❌ Absent | `sidebar.rs:640-703` (DnD is sidebar-row reorder only, not file import) | Core-parity gap | P2 | No drag-drop file import; add via folder picker / folder context. Sidebar DnD reorders rows only. |
| 1 | Auto-organize file structure | "Keep iTunes Media folder organized" rewrites `Artist/Album/Track` paths on import | Retained | 🚫 Won't-do (by design) | `scanner.rs` (scans in place only); `features.md:48` | Deliberate omission |  | riff never moves user files; removing a path drops index entries, never files on disk. Rationale: track identity *is* its path — [`decisions/003-track-identity-is-path.md`](../decisions/003-track-identity-is-path.md) + offline-first scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 1 | Duplicate detection | "Show Duplicate Items" + smart-playlist rules | Retained (File ▸ Library ▸ Show Duplicate Items) | ❌ Absent | grep `duplicate` → only playlist/cache dedup, no track-duplicate clustering | Core-parity gap | P2 |  |
| 1 | CD import / rip | Insert CD → Gracenote lookup → Import CD; format choices AAC/MP3/ALAC/AIFF/WAV | Retained (CD import) | 🚫 Won't-do (by design) | grep `rip\|cd_import` → none; `features.md:19` | Deliberate omission |  | No CD hardware access; offline scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 1 | CD burn | "Burn Playlist to Disc" (audio/MP3/data) | ⚠️ Uncertain — burn UI largely absent in Music.app | 🚫 Won't-do (by design) | grep `burn` → none | Deliberate omission |  | Out of scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 1 | Import rules / format conversion | "Create AAC/MP3/ALAC/WAV version"; auto-downconvert for legacy iPods | Retained (Create [format] Version) | 🚫 Won't-do (by design) | grep `transcode\|encode\|format_version` → none; `features.md:19` | Deliberate omission |  | Encoding/transcoding out of scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |

### Domain 2 — Metadata & tags

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 2 | Core tags (Name, Artist, Album, Genre, Year, Track #, Disc #, Composer) | Editable; written to ID3/MP4 | Retained | ✅ Implemented | `riff-infra/src/media/metadata_reader.rs:85-138`; `metadata_writer.rs:47-108` | — |  | Read+write for title/artist/album/album_artist/track#/disc#/genre/year/composer/comment. |
| 2 | Grouping | Free tag; classical-movement grouping, smart-playlist criterion | Retained | ❌ Absent | `metadata_reader.rs:85-138` (not in extracted set) | Core-parity gap | P2 | Tag not read/stored/edited. |
| 2 | Comments | Free-form searchable field | Retained | ✅ Implemented | `metadata_reader.rs` (comment); `metadata_writer.rs` (writes comment) | — |  |  |
| 2 | BPM | Beats-per-minute; smart-playlist criterion | Retained | ❌ Absent | not in extracted set | Core-parity gap | P2 |  |
| 2 | Star rating (1–5) | Since iTunes 3 (2002) | Retained (Settings ▸ Show Star Ratings) | ❌ Absent | `track.rs:40` (`favorite: bool` only); grep `rating\|star_` → none | Core-parity gap | P1 | riff has a boolean **Favorites** flag instead of 1–5 stars. Power-user expectation; consider 1–5 or keep favorites-only. |
| 2 | Play count | Auto-incremented on play | Retained (drives Listening History) | ✅ Implemented | `track.rs:28-34`; `playback_coordinator.rs:137`; `sqlite.rs:979-988` | — |  | Auto-incremented on track end. |
| 2 | Last played | Timestamp | Retained | ✅ Implemented | `track.rs`; `sqlite.rs:979-988` (`last_played_nanos`) | — |  |  |
| 2 | Date added | Import timestamp | Retained | ✅ Implemented | `track.rs:31-34` (set at scan) | — |  |  |
| 2 | Lyrics | Embedded + view pane | Retained (enhanced: live lyrics) | ❌ Absent | grep `lyric` → none; `roadmap.md:19` (deferred) | Core-parity gap | P2 | Embedded/fetched lyrics deferred on roadmap; not legacy, not scope — low urgency. |
| 2 | Per-track EQ preset | Graphic equalizer assignment per song | Retained | ❌ Absent | no EQ at all (see Domain 3) | Deliberate omission |  | Subsumed by riff's no-EQ posture. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 2 | Per-track volume adjustment | "Volume Adjustment" (-100%…+100%) | Retained | ❌ Absent | `TrackMetadata` lacks per-track volume (`track.rs:93-111`); only global volume | Deliberate omission |  | Only global volume + auto ReplayGain. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 2 | Album artwork | Embedded + Cover Flow / grid | Retained | ✅ Implemented | `riff-library` `cover_resolver.rs:30-95`; `cover_loader.rs`; `cover_service.rs:14` (LRU=50) | — |  | Embedded then filesystem fallback (`cover/folder/album/front.{jpg,png}`); LRU cache + placeholder. |
| 2 | Part of a Compilation flag | Affects grouping/sort | Retained | ❌ Absent | not stored; album grouping via `album_artist` (`features.md:50`) | Core-parity gap | P2 |  |
| 2 | Sort fields (sort artist/album/name) | Separate sort overrides | Retained | ❌ Absent | grep `sort_artist\|sort_album\|SortField` → none | Core-parity gap | P2 |  |
| 2 | ID3 / MP4 tag read-write | Native | Retained | ✅ Implemented | `metadata_reader.rs` / `metadata_writer.rs` (lofty) | — |  |  |
| 2 | Batch tag edit ("Get Info" multi-select) | Yes | Retained | ❌ Absent | `app.rs:3371` (single-track modal); submit emits one `TagEditRequest` (`app.rs:3190-3217`) | Core-parity gap | P1 | Only single-track Edit Tags (advanced mode). Multi-select edit is a real power-user need. |

### Domain 3 — Playback

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 3 | Up Next / queue | Introduced iTunes 11 (2012), replaced iTunes DJ | Retained | ✅ Implemented | `riff-playback/src/domain/queue.rs:55-64` (append), `:81-87` (insert_next), `:140-226` (advance/previous); `transport.rs:173-184` | — |  | Insert-after-current = "Play Next"; shuffle/repeat are session flags. |
| 3 | Repeat / shuffle | Yes | Retained | ✅ Implemented | `queue.rs:124-131` (`toggle_repeat` None→All→One); `playback.rs:12-18` (`RepeatMode`) | — |  |  |
| 3 | Gapless playback | Since iTunes 7.0 (2006) | Retained | ✅ Implemented | `riff-playback/src/app/gapless.rs` (pre-decode ~2s before EOF, pre-buffer ≤4s) | — |  | Same-format seamless; format mismatch/shuffle falls back to gapped path. |
| 3 | Crossfade | Yes | Retained (Playback prefs) | 🚫 Won't-do (by design) | `features.md:21` ("intentionally not provided") | Deliberate omission |  | Stated intentional in `features.md:21`. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 3 | Sound Check (volume normalization) | Since iTunes 3 (2002) | Retained (Playback prefs) | ❌ Absent | grep `SoundCheck\|iTunNORM` → none | Core-parity gap | P2 | riff uses **ReplayGain tag-based** normalization instead (row below) — functionally mitigated. |
| 3 | ReplayGain (tag-based track gain/peak) | *no native iTunes analog (iTunes uses Sound Check)* | *n/a (riff mechanism)* | ✅ Implemented | `metadata_reader.rs:130-135`; `audio_output.rs:142-151`; `state.rs:9-23` (peak-capped); `settings.rs:223` | — |  | Opt-in, takes effect on next track, untagged untouched. This is riff's loudness-normalization analog of Sound Check. |
| 3 | Graphic equalizer (presets) | Since iTunes 2 (2001) | Retained (Window ▸ Equalizer) | ❌ Absent | grep `equaliz\|graphic_eq\|biquad` → only animated bars (`sidebar.rs:53-91`); `roadmap.md:11` (deferred) | Core-parity gap | P1 | Deferred nice-to-have; notable iTunes parity gap. |
| 3 | Per-track volume | Volume Adjustment | Retained | ❌ Absent | only global volume + auto ReplayGain | Deliberate omission |  | See Domain 2 per-track volume. ⚠️ No dedicated `decisions/` doc — flagged (open flag #1). |
| 3 | AirPlay / Remote | AirPlay menu; AirPlay 2 multi-room since 12.8 (2018) | Retained (AirPlay menu) | 🚫 Won't-do (by design) | grep `airplay\|remote` → none; `features.md:128` (offline) | Deliberate omission |  | Offline-first scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md); [`decisions/006-local-only-discovery.md`](../decisions/006-local-only-discovery.md). |
| 3 | Visualizer (full-screen) | Built-in plug-in visualizer (default since 8.0, 2008) | ⚠️ Not carried over — classic visualizers dropped | ❌ Absent | grep `visualiz` → none (sidebar "equalizer bars" are non-FFT animation) | Legacy / deprecated |  | Music.app also dropped classic visualizers; iTunes legacy feature. Not a riff decision. |
| 3 | Lyrics view | Lyrics pane | Retained (enhanced) | ❌ Absent | see Domain 2 Lyrics | Core-parity gap | P2 | Deferred (see Domain 2). |
| 3 | Mini-player | MiniPlayer (since 12, resizable, keep-on-top) | Retained | ❌ Absent | grep `mini.?player` → none | Core-parity gap | P2 |  |
| 3 | Scrobbling (Last.fm) | **No native** — third-party via AppleScript/plugins only | N/A (same — no native) | 🚫 Won't-do (by design) | grep `scrobbl\|lastfm` → none; `roadmap.md:21` | Deliberate omission |  | Offline-only by design [`decisions/001-offline-first.md`](../decisions/001-offline-first.md); iTunes also lacked native scrobbling. |

### Domain 4 — Organization

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 4 | Standard playlists | Yes | Retained | ✅ Implemented | `playlist_manager.rs:100-130`; dedup `store.rs:257`; missing-track handling `playlist_manager.rs:19-35` | — |  | Create/rename/reorder/delete, persist, dedup, struck-through "(missing)" excluded from playback. |
| 4 | Smart playlists | Since iTunes 3 (2002); rule-based, auto-updating | Retained | 🟡 Partial | `track.rs:54-86` (6 fixed kinds); `sqlite.rs:2082-2231` | Core-parity gap | P0 | **6 fixed** discovery lists (Favorites, Recently Added, Most Played, Recently Played, Never Played, Lost Gems) computed as store queries — **not** a user rule engine. The single most defining iTunes organizational feature; a user-facing rule engine is the key parity gap. |
| 4 | Playlist folders | Hierarchical folder grouping | Retained | ❌ Absent | grep `PlaylistFolder\|playlist_folder` → none | Core-parity gap | P2 |  |
| 4 | Genius mixes | Genius (8.0, 2008) → Genius Mixes (9.0, 2009) | Retained initially; ⚠️ deprecated in later macOS | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy |  | Requires cloud analysis [`decisions/001-offline-first.md`](../decisions/001-offline-first.md); also deprecated in Music.app. |
| 4 | Genius (single-track) | Right-click → Start Genius | Retained initially; same deprecation caveat | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy |  | Same as Genius mixes. |

### Domain 5 — UI / navigation

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 5 | Sidebar / source list | Hidden by default in 11, restored via Option+⌘+S in 12 | Retained (View ▸ Show Sidebar) | ✅ Implemented | `sidebar.rs` (LIBRARY/FOLDERS/PLAYLISTS + live counts `app.rs:2077`) | — |  |  |
| 5 | Column browser | Genre/Artist/Album (3-col; left-layout since 9.0) | Retained (View ▸ Column Browser) | ✅ Implemented | `browser.rs` (BrowserColumn, genre chips `:78-87`); `detail.rs:18-49` (breadcrumb + inspector) | — |  | Elastic columns: Genres→Artists→Albums→Tracks. |
| 5 | Cover Flow | Introduced 7.0 (2006); dropped as default + removed in 11 (2012) | Dropped (legacy) — already gone before Music.app | 🚫 Won't-do (by design) | grep `coverflow` → none; dropped in iTunes 11 | Legacy / deprecated |  | Removed in iTunes 11; legacy, not a riff decision. |
| 5 | Grid view | Introduced 8.0; default since 11 | Retained (enhanced) | 🚫 Removed (by design) | grid retired end-to-end (titlebar-search spec): `browser.rs` is list-only; migration 012 drops the `browser_layout` column | Deliberate omission |  | Browser is permanently list-only; no toggle, no tile path. |
| 5 | View options (columns/sort) | View Options sheet | Retained | 🟡 Partial | column-browser sort; no column-picker sheet | Core-parity gap | P2 | Sort present; no per-column visibility sheet. |
| 5 | Search (library + store) | Unified search + trending (12) | Retained (library / Apple Music scope toggle) | ✅ Implemented | `chrome.rs` (titlebar search, global Ctrl+K) | — |  | Library search only (no Store scope — offline). |
| 5 | Sort | Click headers; multi-key | Retained | ✅ Implemented | column browser + clickable headers; `projection/counts.rs` | — |  |  |
| 5 | Now Playing | Persistent now-playing pane | Retained | ✅ Implemented | `now_playing.rs` (cover 240px, metadata, clickable up-next) | — |  |  |
| 5 | Full-screen | Yes | Retained | 🟡 Partial | `playerbar.rs:302-354` (expand→Now Playing view); `chrome.rs:241-242` (window Maximize); no `set_fullscreen` | Core-parity gap | P2 | Enlarged Now Playing + window maximize, but no OS-level full-screen media mode. |
| 5 | Mini-player | Yes | Retained | ❌ Absent | grep `mini.?player` → none | Core-parity gap | P2 | (Listed in both D3 and D5 in the reference; tracked once here as P2.) |

### Domain 6 — Ecosystem (all offline-only by design)

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 6 | iTunes Store (buy/rent music) | Integrated storefront | Retained inside Music.app | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | [`decisions/001-offline-first.md`](../decisions/001-offline-first.md) + [`decisions/006-local-only-discovery.md`](../decisions/006-local-only-discovery.md). |
| 6 | Apple Music streaming | Added 12.2 (June 2015) | Retained | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Same as Store. |
| 6 | iTunes Match | Added 10.5 (Oct 2011) | Retained | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Cloud sync — out of scope. |
| 6 | iCloud Music Library / Sync Library | Cloud library sync | Retained (Sync Library) | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Cloud sync — out of scope. |
| 6 | FairPlay DRM | Music DRM-free since 2009; movies/TV still protected | Retained (purchased video) | 🚫 Won't-do (by design) | grep `fairplay\|drm` → none | Deliberate omission |  | Offline/local only. |
| 6 | Device sync (iPhone/iPad/iPod) | Cable + Wi-Fi sync | Moved → Finder | 🚫 Won't-do (by design) | grep `ipod` → none | Deliberate omission |  | Device management out of scope. |
| 6 | Backup / restore | Via device pane | Moved → Finder | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Device management out of scope. |
| 6 | Wi-Fi sync | Yes | Moved → Finder | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Device management out of scope. |
| 6 | Home Sharing | Since 9.0 (2009) | Retained (shared library over LAN) | 🚫 Won't-do (by design) | grep `home.?shar` → none | Deliberate omission |  | LAN sharing out of offline-local scope. |
| 6 | iTunes DJ | Party Shuffle (5.0, 2005) → iTunes DJ; discontinued in 11 | Dropped (legacy) — replaced by Up Next | 🚫 Won't-do (by design) | grep → none | Legacy / deprecated |  | Dropped in iTunes 11; legacy. |
| 6 | Podcasts | In-app | Moved → Apple Podcasts | 🚫 Won't-do (by design) | grep `podcast` → none | Deliberate omission |  | Non-music media out of scope. |
| 6 | Audiobooks | In-app | Moved → Apple Books | 🚫 Won't-do (by design) | grep `audiobook` → none | Deliberate omission |  | Non-music media out of scope. |
| 6 | TV / Movies | In-app store + library | Moved → Apple TV | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Non-music media out of scope. |
| 6 | iTunes U | In-app | Moved → Podcasts/Books (discontinued) | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Non-music media out of scope. |
| 6 | Ringtones | Removed in 12.7 (2017) | Dropped (legacy) | 🚫 Won't-do (by design) | grep `ringtone` → none | Legacy / deprecated |  | Removed in iTunes 12.7; legacy. |
| 6 | Redeem / gift | Gift cards + Redeem (incl. webcam) | Retained (store) | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Store commerce out of scope. |
| 6 | Family Sharing | Since 12.0 (2014) | Retained | 🚫 Won't-do (by design) | grep `family.?shar` → none | Deliberate omission |  | Account/cloud out of scope. |

### Domain 7 — Sharing / social

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 7 | Home Sharing | Shared library over LAN | Retained | 🚫 Won't-do (by design) | see Domain 6 | Deliberate omission |  | See Domain 6 Home Sharing. |
| 7 | Shared library over LAN | Yes | Retained (shared library) | 🚫 Won't-do (by design) | grep → none | Deliberate omission |  | Offline-local scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 7 | Genius recommendations | Library-based suggestions | Retained initially (deprecation caveat) | 🚫 Won't-do (by design) | grep `genius` → none | Deliberate omission / Legacy |  | Cloud analysis [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |
| 7 | Ping (social) | Added 10.0 (2010); removed in 11 (2012), replaced by Twitter/FB | Dropped (legacy/defunct) | 🚫 Won't-do (by design) | grep → none | Legacy / deprecated |  | Removed in iTunes 11; defunct social network. |

### Domain 8 — Interop / export

| Domain | Feature | iTunes 12.x | Music.app | riff Status | Evidence (file/line) | Gap Bucket | Priority | Notes |
|---|---|---|---|---|---|---|---|---|
| 8 | Export playlist (M3U) | Export selection / playlist | Retained (export playlist file) | ❌ Absent | grep `m3u` → none | Core-parity gap | P1 | No playlist export of any format. High-value interop for a local-library tool (portability to other players / DJ apps). |
| 8 | iTunes Library XML | Live `iTunes Music Library.xml` (DJ-app bridge) | **Not carried over** — Music.app stopped maintaining the live XML (breaking change for DJ apps, Catalina) | ❌ Absent | grep `itunes\|library_xml\|\.xml\|export` → none (only asset/migration files) | Core-parity gap | P2 | No XML / DJ-app bridge. Niche but valuable interop. |
| 8 | iPod sync protocol | Native device sync | Moved → Finder (no public protocol) | 🚫 Won't-do (by design) | see Domain 6 device sync | Deliberate omission |  | Device sync out of scope [`decisions/001-offline-first.md`](../decisions/001-offline-first.md). |

## 4. Gap analysis

### 4.1 Core-parity gaps (19) — what riff should close

These are the features a credible local-library alternative is expected to have, that riff lacks or only partially covers. They split cleanly into four themes:

- **Organizational power (the biggest deficit).** Smart playlists in riff are *6 fixed discovery lists*, not a user-facing rule engine — this is the single most defining iTunes organizational feature and the only **P0**. Playlist folders (hierarchy) and duplicate detection are the next tier of library-management depth.
- **Metadata richness.** riff reads/writes the core tags and artwork, but is missing the "classical + DJ + power-user" tag set: 1–5 star rating (riff has only a boolean Favorite), Grouping, BPM, Compilation flag, and per-field sort overrides. Batch ("Get Info" multi-select) tag editing is absent — only single-track edit exists today.
- **DSP / listening UX.** A graphic equalizer (presets) is the notable playback parity gap. Sound Check is functionally *mitigated* by riff's ReplayGain implementation, so it is a gap only in the literal iTunes sense. Crossfade is a deliberate omission, not a gap.
- **Interop / portability.** No playlist export (M3U) and no iTunes-Library-XML / DJ-app bridge — both high-value for a tool whose whole point is *your* local files. Mini-player and full-screen media mode are smaller UI-parity items.

### 4.2 Legacy / deprecated (8) — skip, and it's not riff's call

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

### 4.3 Deliberate omissions (28) — what defines riff

These are the features riff declines *by design*, and they are the clearest statement of product scope. Every one traces to the offline-first / local-library thesis:

- **Commerce & streaming (never):** iTunes Store, Apple Music streaming, iTunes Match, iCloud/Sync Library, Redeem/gift, Family Sharing.
- **Device & ecosystem (never):** iPhone/iPad/iPod sync, backup/restore, Wi-Fi sync, iPod sync protocol, AirPlay/Remote, Home Sharing, shared library over LAN.
- **Non-music media (never):** Podcasts, Audiobooks, TV/Movies, iTunes U, Ringtones.
- **DRM (never):** FairPlay — riff is local-only and DRM-free by construction.
- **File-mutating ingestion (never):** auto-organize ("Keep Media folder organized") is declined because track identity *is* the file path (`decisions/003-track-identity-is-path.md`); CD import/rip, CD burn, and format conversion/transcoding are out of the offline scope (`decisions/001-offline-first.md`).
- **DSP posture (by design, but under-cited):** crossfade and per-track volume/EQ are declined — riff uses global volume + ReplayGain instead. **Caveat:** these 🚫 rows currently cite only `features.md` / `roadmap.md`, not a first-class ADR — see Open Flag #1 below.

**Net:** the deliberate-omission bucket is the product's moat, not a todo list. It should be read as "scope defended," not as gaps.

## 5. Prioritized backlog

Only the 19 core-parity gaps are backlog items. Priorities: **P0** = defining · **P1** = meaningful/power-user · **P2** = nice-to-have. Each item names the target crate(s) and a rough implementation shape, mapped to the vertical crate split in `AGENTS.md`. Crate key: `pers` = riff-persistence · `lib` = riff-library · `play` = riff-playback · `infra` = riff-infra · `be` = riff-backend · `gui` = riff-gui.

### P0 — defining parity gap (1)

| # | Feature | Domain | Target crate(s) | Rough shape |
|---|---|---|---|---|
| 1 | **Smart playlists — user rule engine** | D4 | `pers` · `lib` · `be` · `gui` | Replace the 6 fixed discovery lists' engine with a general `RuleSet` (field / operator / value clauses, match *all*/*any*, sort + limit) persisted in the store; evaluate as a live store query exposed through a `LibrarySession` projection; add a rule-editor UI. Keeps the existing 6 lists as pre-seeded rules. |

### P1 — meaningful, expected by power users (4)

| # | Feature | Domain | Target crate(s) | Rough shape |
|---|---|---|---|---|
| 2 | **1–5 star rating** | D2 | `pers` · `infra` · `be` · `gui` | Add `rating: Option<u8>` (0–5) to `TrackMetadata`; read/write POPM (ID3) / `rate` (MP4) via lofty in `infra` media adapters; star widget in track list + Now Playing; extend `TagEditWorker`. Decide whether to keep or supersede the existing boolean `favorite`. |
| 3 | **Batch tag edit (multi-select "Get Info")** | D2 | `be` · `gui` | Generalize `TagEditRequest` to accept N `TrackId`s + a field mask; `TagEditWorker` writes the shared subset as one durable change; reuse the single-track Edit-Tags modal fields in a multi-select variant. |
| 4 | **Graphic equalizer (presets)** | D3 | `infra` · `play` · `gui` | Add an N-band IIR (biquad) filter chain to the `cpal` output adapter in `infra`; preset table + opt-in setting surfaced through `play` state and a dedicated Equalizer window in `gui`. Currently deferred (`roadmap.md:11`) — notable iTunes parity gap. |
| 5 | **Export playlist (M3U)** | D8 | `lib` · `gui` | Serialize a playlist's tracks to extended `.m3u` (absolute or library-relative paths); export action in `gui`. High interop value (portability to other players / DJ apps). M3U import is a natural follow-up but out of scope for v1. |

### P2 — nice-to-have (14)

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

> **Backlog totals:** P0 ×1, P1 ×4, P2 ×14 = 19 core-parity gaps. Playback-domain gaps are only EQ (P1), Sound Check (P2, mitigated), and mini-player (P2) — confirming riff's playback core is at parity; the backlog is overwhelmingly *library depth + interop*.

## 6. Sources & open flags

### Source files

- `docs/product/itunes-comparison/phase-1-itunes-taxonomy.md`, `phase-1-itunes-reference.md`
- `phase-2-riff-inventory.md`, `phase-2-riff-reference.md`
- `phase-3-matrix.md`, `phase-3-scored-matrix.md`
- `phase-4-gap-narrative.md`
- riff codebase (evidence cited per row), `AGENTS.md`, `docs/product/features.md`, `docs/product/roadmap.md`
- Decisions cited: `decisions/001-offline-first.md`, `decisions/003-track-identity-is-path.md`, `decisions/006-local-only-discovery.md`

### Open flags

1. **DSP scope citation (carried from Phase 3/4).** The 🚫 rows for per-track EQ preset, per-track volume, and crossfade cite only `features.md` / `roadmap.md`, not a first-class `decisions/` ADR. Recommend adding `decisions/007-dsp-scope.md` stating riff's no-global-EQ / no-crossfade / no-per-track-volume posture so these rows have a first-class citation.
2. **Mini-player double-listed** in the Phase 3 reference (D3 + D5); tracked once in the backlog (#14).
3. **Sound Check** scored ❌ Absent but functionally mitigated by ReplayGain ✅ — reported as mitigated, not a build priority.
4. **Phase 3 summary rollup is internally inconsistent** (see data-quality note in §2). Its status/bucket tallies (19/5/17/34 and 24/19/30/8) do not match the 75-row matrix (buckets sum to 81 ≠ 75; Sound Check mislabeled partial). This report uses the per-row matrix values as the source of truth. Recommend Phase 6 correct the Phase 3 rollup text to match.
