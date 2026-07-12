---
project: riff
last_updated: 2026-07-12
---

# Requirements Index — riff

## Definitions

**Epic:** A high-level product capability that groups related user-facing behaviors.
**Feature:** A user-facing functionality that can be independently designed and implemented.

---

## Epics

### Audio Engine
The core audio playback system responsible for decoding multiple audio formats, managing the audio output stream, and providing playback controls (play, pause, stop, seek, volume). It must support MP3, AAC (M4A), Opus, FLAC, OGG Vorbis, and WAV formats using pure Rust libraries.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|---|
| [Multi-format Decoding](features/multi-format-decoding.md) | Decode MP3, AAC, Opus, FLAC, OGG, WAV via symphonia | implemented | P0 | — |
| [Playback Control](features/playback-control.md) | Play, pause, stop, seek, volume with cpal output | implemented | P0 | Multi-format Decoding |
| [Playback Queue](features/playback-queue.md) | Queue management, next/previous track, shuffle, repeat | implemented | P1 | Playback Control |

### Music Library
The system that discovers, indexes, and organizes local audio files. It scans directories for supported audio files, extracts metadata (artist, album, title, album artist, genre, year, track number), resolves cover art (embedded metadata first, then filesystem fallback), and provides fast search capabilities.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|---|---|
| [Library Scanning](features/library-scanning.md) | Recursively scan directories for audio files | implemented | P0 | — |
| [Metadata Extraction](features/metadata-extraction.md) | Extract and store tags from audio containers | implemented | P0 | Library Scanning |
| [Cover Art Resolution](features/cover-art-resolution.md) | Resolve cover: embedded metadata > cover.jpg/cover.png | implemented | P1 | Metadata Extraction |
| [Library Search](features/library-search.md) | Search by artist, album artist, album, title | implemented | P1 | Metadata Extraction |
| [Music Library Management](features/music-library-management.md) | Manage multiple library paths, OS file picker for add, delete, persist list | implemented | P0 | Library Scanning |
| [Library Cache Persistence](features/library-cache-persistence.md) | Persist scanned tracks to disk so library loads instantly on startup without re-scan | implemented | P1 | Library Scanning, Music Library Management |
| [Folder Watching](features/folder-watching.md) | Auto-detect new and deleted files in library folders with per-path toggle and debounced rescan | implemented | P1 | Library Scanning |

### User Interface
The egui-based graphical interface that provides a main application window, a library explorer with dual views (file tree and searchable list), a player control bar with transport controls, a cover art display panel, and a now playing view. Must work on Linux, Windows, and macOS with minimal external dependencies.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|---|
| [Main Application Window](features/main-application-window.md) | egui window with cross-platform support | partial | P0 | — |
| [Library Explorer Panel](features/library-explorer-panel.md) | Dual library/folder views with toggle, folder playback, context menus, search coexistence | implemented | P0 | Library Search |
| [Player Control Bar](features/player-control-bar.md) | Transport controls, progress bar, volume slider | partial | P0 | Playback Control |
| [Cover Art Display](features/cover-art-display.md) | Display resolved cover art in UI | partial | P1 | Cover Art Resolution |
| [Now Playing View](features/now-playing-view.md) | Full track info, large cover art, queue peek | partial | P2 | Playback Queue, Cover Art Display |

### System Integration
Cross-platform system integration including a system tray icon that allows the application to continue running minimized, platform-appropriate window management, and optional media key support.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|---|
| [System Tray Icon](features/system-tray-icon.md) | Minimize to tray, restore from tray, quit from tray (macOS/Windows only) | partial | P1 | Main Application Window |
| [Cross-platform Support](features/cross-platform-support.md) | Linux, Windows, macOS window and audio compatibility | implemented | P0 | Main Application Window |

## Glossary

| Term | Definition |
|---|---|
| **Codec** | Software component that encodes or decodes audio data in a specific format (MP3, AAC, Opus, etc.) |
| **Container** | File format that wraps encoded audio data along with metadata tags and optionally cover art (M4A, OGG, FLAC, etc.) |
| **Cover Art** | Image associated with an album or track, typically embedded in audio file metadata or stored as cover.jpg/cover.png in the same directory |
| **Library** | The complete set of audio files discovered and indexed by the application |
| **Library Cache** | Persistent on-disk JSON copy of scanned tracks, artists, and albums that avoids re-scanning on startup |
| **Playback Queue** | Ordered list of tracks scheduled for sequential playback |
| **Metadata** | Descriptive information embedded in audio files (artist, album, title, genre, year, track number, etc.) |
| **Album Artist** | The primary artist credited for an album, distinct from track-specific artists (e.g., compilations) |
| **Symphonia** | Pure Rust audio media library used for format parsing and decoding |
| **egui** | Immediate mode GUI library written in pure Rust |
| **cpal** | Cross-platform audio I/O library for Rust |
| **lofty** | Pure Rust audio metadata reading/writing library |

## Source Materials

| Document | Type | Features Derived |
|---|---|---|
| User requirements (this conversation) | Stakeholder specification | All features |

## Deferred Items

- **Playlist management** — Creating, saving, and loading custom playlists. Reason: Core playback and library features must be solid first.
- **Equalizer / audio effects** — Per-band EQ, reverb, etc. Reason: Nice-to-have, not essential for MVP.
- **Gapless playback** — Seamless transition between consecutive album tracks. Reason: Complex cross-track buffering, defer to post-MVP.
- **ReplayGain normalization** — Automatic volume leveling across tracks. Reason: Requires metadata analysis pass, defer.
- **Lyrics display** — Embedded or fetched lyrics. Reason: Not requested in scope.
- **Internet-based features** — Streaming, online metadata lookup, scrobbling. Reason: Explicitly out of scope (offline-only player).
