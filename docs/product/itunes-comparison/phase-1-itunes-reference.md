# Phase 1 output — iTunes 12.x + Music.app reference taxonomy (canonical)

> Feeds: Phase 3 (scored matrix). Companion plan + vocabulary: [`index.md`](../index.md).
> Status: **canonical reference rows produced** — `riff Status / Evidence / Gap Bucket / Priority` are intentionally left blank here (Phase 2 inventories riff, Phase 3 scores).
> Method: web-grounded (Apple support docs, Wikipedia *History of iTunes*, PCMag/Macworld/MacPaw roundups, iLounge tagging guide, Apple Discussions). Version-origin anchors included so later phases can cite defensibly.

`Music.app` column uses short tokens:
- **Retained** — same feature, still in Music.app (Catalina+)
- **Retained (enhanced)** — present and improved
- **Moved → [app]** — functionality relocated to a sibling app
- **Moved → Finder** — device management moved to Finder
- **Dropped (legacy)** — removed because deprecated/legacy
- **Not carried over** — absent from Music.app, no replacement
- **N/A (scope)** — out of Music.app's local-library remit

---

## Domain 1 — Library & ingestion

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| Add folder to library (watch) | "Copy files to iTunes Media folder when adding" + "Keep iTunes Media folder organized" auto-imports watched folders | Retained (Preferences ▸ Files) |
| Manual add (drag-drop) | Drag file/folder onto library; copies per prefs | Retained |
| Auto-organize file structure | "Keep iTunes Media folder organized" rewrites `Artist/Album/Track` paths on import | Retained |
| Duplicate detection | "Show Duplicate Items" (file menu) + smart-playlist rules | Retained (File ▸ Library ▸ Show Duplicate Items) |
| CD import / rip | Insert CD → Gracenote lookup → Import CD; format choices AAC / MP3 / Apple Lossless / AIFF / WAV | Retained (CD import); see ⚠️ below on burn |
| CD burn | "Burn Playlist to Disc" (audio / MP3 / data) | **⚠️ Uncertain** — burn UI largely absent in Music.app; treat as Not carried over unless verified |
| Import rules / format conversion | "Create AAC/MP3/Apple Lossless/WAV version"; auto-downconvert to 128 kbps AAC for legacy iPods (iTunes 4.7.1) | Retained (Create [format] Version) |

*Version anchors:* auto-downconvert 4.7.1 (2005); Gracenote CD lookup since v3/v4.

---

## Domain 2 — Metadata & tags

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| Core tags (Name, Artist, Album, Genre, Year, Track #, Disc #, Composer) | Editable; written to ID3/MP4 | Retained |
| Grouping | Free tag; classical-movement grouping, smart-playlist criterion | Retained |
| Comments | Free-form searchable field | Retained |
| BPM | Beats-per-minute; smart-playlist criterion | Retained |
| Star rating (1–5) | Since iTunes 3 (2002) | Retained (Settings ▸ Show Star Ratings) |
| Play count | Auto-incremented on play | Retained (drives Listening History) |
| Last played | Timestamp | Retained |
| Date added | Import timestamp | Retained |
| Lyrics | Embedded + view pane | Retained (enhanced: real-time/"live" lyrics) |
| Per-track EQ preset | Graphic equalizer assignment per song | Retained |
| Per-track volume adjustment | "Volume Adjustment" (-100%…+100% | Retained |
| Album artwork | Embedded + Cover Flow / grid | Retained |
| Part of a Compilation flag | Affects grouping/sort | Retained |
| Sort fields (sort artist/album/name) | Separate sort overrides | Retained |
| ID3 / MP4 tag read-write | Native | Retained |
| Batch tag edit ("Get Info" multi-select) | Yes | Retained |

*Note:* Play Count / Last Played / Date Added / Rating / EQ / Volume live in the **library DB** (iTunes Library.itl), not the file — confirmed via Apple Discussions. Music.app preserves ratings + play counts on migration (Catalina).

---

## Domain 3 — Playback

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| Up Next / queue | Introduced iTunes 11 (2012), replaced iTunes DJ | Retained |
| Repeat / shuffle | Yes | Retained |
| Gapless playback | Since iTunes 7.0 (2006) | Retained |
| Crossfade | Yes | Retained (Playback prefs) |
| Sound Check (volume normalization) | Since iTunes 3 (2002) | Retained (Playback prefs) |
| Graphic equalizer (presets) | Since iTunes 2 (2001) | Retained (Window ▸ Equalizer) |
| Per-track volume | Volume Adjustment | Retained |
| AirPlay / Remote | AirPlay menu; AirPlay 2 multi-room since 12.8 (2018) | Retained (AirPlay menu) |
| Visualizer (full-screen) | Built-in plug-in visualizer (default since 8.0, 2008) | **⚠️ Not carried over** — classic visualizers dropped (lower confidence; verify) |
| Lyrics view | Lyrics pane | Retained (enhanced) |
| Mini-player | MiniPlayer (since 12, resizable, keep-on-top) | Retained |
| Scrobbling (Last.fm) | **No native** — third-party via AppleScript/plugins only | N/A (same — no native) |

*Note:* "scrobbling" is listed in the plan's playback bucket but iTunes never shipped native Last.fm scrobbling — flag for Phase 3 as a riff parity question, not an iTunes gap.

---

## Domain 4 — Organization

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| Standard playlists | Yes | Retained |
| **Smart playlists** | Since iTunes 3 (2002); rule-based, auto-updating | Retained |
| Playlist folders | Hierarchical folder grouping | Retained |
| Genius mixes | Genius (8.0, 2008) → Genius Mixes (9.0, 2009) | Retained initially; **⚠️ deprecated in later macOS** — verify, likely Deliberate/Legacy later |
| Genius (single-track) | Right-click → Start Genius | Retained initially; same deprecation caveat |

*Note:* Genius requires Apple-ID analysis; on-device "Up Next" + smart playlists are the durable local mechanism.

---

## Domain 5 — UI / navigation

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| Sidebar / source list | Hidden by default in 11, restored via Option+⌘+S in 12 | Retained (View ▸ Show Sidebar) |
| **Column browser** | Genre/Artist/Album (3-col; left-layout since 9.0) | Retained (View ▸ Column Browser) |
| **Cover Flow** | Introduced 7.0 (2006); dropped as default + removed in 11 (2012) | Dropped (legacy) — already gone before Music.app |
| Grid view | Introduced 8.0; default since 11 | Retained (enhanced) |
| View options (columns/sort) | View Options sheet | Retained |
| Search (library + store) | Unified search + trending (12) | Retained (library / Apple Music scope toggle) |
| Sort | Click headers; multi-key | Retained |
| Now Playing | Persistent now-playing pane | Retained |
| Full-screen | Yes | Retained |
| Mini-player | Yes | Retained |

---

## Domain 6 — Ecosystem

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| iTunes Store (buy/rent music) | Integrated storefront | Retained inside Music.app |
| **Apple Music streaming** | Added 12.2 (June 2015) | Retained |
| iTunes Match | Added 10.5 (Oct 2011) | Retained |
| iCloud Music Library / Sync Library | Cloud library sync | Retained (Sync Library) |
| **FairPlay DRM** | Music DRM-free since 2009; movies/TV still protected | Retained (applies to purchased video) |
| **Device sync** (iPhone/iPad/iPod) | Cable + Wi-Fi sync | Moved → Finder |
| Backup / restore | Via device pane | Moved → Finder |
| Wi-Fi sync | Yes | Moved → Finder |
| **CD import / rip** | Yes | Retained |
| **CD burn** | Yes | **⚠️ Uncertain** — see Domain 1 |
| Format conversion | Create-[format] Version | Retained |
| **Home Sharing** | Since 9.0 (2009) | Retained (shared library over LAN) |
| **iTunes DJ** | Party Shuffle (5.0, 2005) → iTunes DJ; discontinued in 11 | Dropped (legacy) — replaced by Up Next |
| **Podcasts** | In-app | Moved → Apple Podcasts |
| **Audiobooks** | In-app | Moved → Apple Books |
| **TV / Movies** | In-app store + library | Moved → Apple TV |
| iTunes U | In-app | Moved → Podcasts/Books (discontinued) |
| Ringtones | Removed in 12.7 (2017) | Dropped (legacy) |
| Redeem / gift | Gift cards + Redeem (incl. webcam) | Retained (store) |
| Family Sharing | Since 12.0 (2014) | Retained |

---

## Domain 7 — Sharing / social

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| Home Sharing | Shared library over LAN | Retained |
| Shared library over LAN | Yes | Retained (shared library) |
| Genius recommendations | Library-based suggestions | Retained initially (deprecation caveat) |
| Ping (social) | Added 10.0 (2010); removed in 11 (2012), replaced by Twitter/FB | Dropped (legacy/defunct) |

---

## Domain 8 — Interop / export

| Feature | iTunes 12.x | Music.app |
|---|---|---|
| Export playlist (M3U) | Export selection / playlist | Retained (export playlist file) |
| iTunes Library XML | Live `iTunes Music Library.xml` (DJ-app bridge) | **Not carried over** — Music.app stopped maintaining the live XML (breaking change for DJ apps, Catalina) |
| iPod sync protocol | Native device sync | Moved → Finder (no public protocol) |

---

## Confidence flags / open questions for Phase 3

1. **⚠️ Visualizer** — listed as Not carried over, but confidence is medium. Verify against current Music.app before scoring riff parity.
2. **⚠️ CD burn** — Music.app's CD-burning support is ambiguous. Treat as Not carried over pending verification; iTunes 12.x definitely had it.
3. **⚠️ Genius / Genius Mixes** — retained in early Music.app, later deprecated in macOS. Classify as **Legacy/deprecated** for riff purposes (local-library alternative doesn't need cloud Genius).
4. **Cover Flow** already dropped in iTunes 11 — so it is a *legacy* item, not a Music.app regression.
5. **Scrobbling** is not a native iTunes feature — do not score it as an iTunes gap; it's a riff parity question only.

## Sources

- Apple Support — "Use the Music app…" / "Where is my iTunes content on Mac" (Catalina split) `support.apple.com`
- Wikipedia — *History of iTunes* (version-origin anchors: smart playlists/rating/Sound Check v3; gapless+Cover Flow v7; Genius v8; Genius Mixes+Home Sharing v9; Ping v10; iTunes Match v10.5; Family Sharing v12; Apple Music v12.2; AirPlay 2 v12.8; Cover Flow removal v11)
- PCMag — "Apple iTunes 12" review (interface, Family Sharing, playlists)
- Macworld — "The iTunes break up" (smart playlists / Genius / metadata power-user value)
- MacPaw — "What happened to iTunes in Catalina" (Finder sync, XML removal breaking DJ apps)
- iLounge — "Album Tagging, Art and Playlists" (tag fields, column browser)
- Apple Discussions — metadata stored in file vs library DB
- Cult of Mac / MacRumors — Catalina feature-mapping confirmations

## What this feeds

Phase 2 inventories **riff** against these exact reference rows (Domain + Feature keys). Phase 3 assembles the full matrix (`Domain · Feature · iTunes 12.x · Music.app · riff Status · Evidence · Gap Bucket · Priority · Notes`) by joining this file to the riff inventory. No riff columns are asserted here.
