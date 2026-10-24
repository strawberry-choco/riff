# Phase 1 — iTunes reference taxonomy (research)

> Shared context (status vocabulary, gap buckets, matrix columns): see [`index.md`](../index.md).

**Goal:** build a canonical, accurate iTunes feature list. Do not rely on memory alone.

## Sources
- Apple's iTunes 12.x feature pages / release notes
- Wikipedia "iTunes" version-history article
- Apple's "Music app on Mac" transition documentation (Catalina+)
- 1–2 secondary "iTunes 12 feature roundup" writeups to cross-check specifics

## Full taxonomy — 8 domains
1. **Library & ingestion** — add-folder / folder watch, manual add, auto-organize file structure, duplicate detection, import rules
2. **Metadata & tags** — ID3/MP4 tags, star rating, play count, last played, date added, grouping, comments, BPM, musical key, lyrics
3. **Playback** — Up Next / queue, repeat / shuffle, gapless, crossfade, Sound Check (volume normalization), EQ presets, per-track volume, AirPlay / Remote, visualizer, lyrics view, mini-player, scrobbling
4. **Organization** — playlists, **smart playlists**, playlist folders, Genius mixes
5. **UI / navigation** — sidebar / source list, column browser, **Cover Flow**, grid view, view options, search, sort, Now Playing, full-screen, mini-player
6. **Ecosystem** — iTunes Store (buy / rent), **Apple Music streaming**, iTunes Match, iCloud Music Library, **FairPlay DRM**, **device sync** (iPhone/iPad/iPod), backup / restore, Wi-Fi sync, **CD import / rip**, **CD burn**, format conversion, Home Sharing, **iTunes DJ**, **Podcasts**, **Audiobooks**, **TV / Movies**, iTunes U, ringtones, redeem / gift, Family Sharing
7. **Sharing / social** — Home Sharing, shared library over LAN, Genius recommendations, Ping (defunct)
8. **Interop / export** — export playlist (M3U / XML), iTunes Library XML, iPod sync protocol

## Music.app mapping
For each feature, note what changed post-Catalina:
- device sync → Finder
- Podcasts / Audiobooks / TV → separate apps
- Cover Flow already dropped in 11
- Store + Apple Music retained inside Music.app

Result: a `Music.app` column in the matrix rather than a second document.

## Output of this phase
A canonical feature list (the "iTunes 12.x + Music.app" reference rows) ready to feed Phase 3.
