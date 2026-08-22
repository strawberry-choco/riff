# riff

riff is a lightweight, offline-first desktop music player written in Rust. It plays the audio files you already own — MP3, AAC, Opus, FLAC, OGG Vorbis, and WAV — directly from your local disks, with no account, no cloud, and no network access required. It ships as a single binary from a single Cargo crate, runs on Linux, Windows, and macOS, and is designed to start fast, stay out of the way, and treat your file system as the source of truth.

## Overview

riff is a player for local music collections. You point it at one or more folders — your main music directory, an external SSD, a mounted NAS share — and it scans them, reads the tags, resolves the cover art, and gives you a fast, searchable library. Playback runs on a dedicated audio thread with standard transport controls, a queue, shuffle and repeat, and cover art display. A system tray icon (on macOS and Windows) lets it keep playing while the window is hidden.

Everything happens on your machine. The library, playlists, and settings live in one embedded SQLite database — the Application Store (iff.sqlite3) — in your local data directory. Nothing is uploaded, nothing is fetched, and nothing phones home. If you disconnect the network cable, riff behaves exactly the same as it did before.

The project is a single Rust crate with a four-layer architecture: a pure domain layer (tracks, queue, playback state), an application layer that defines the use cases and port traits, an infrastructure layer that implements those traits with real crates (symphonia for decoding, cpal for output, lofty for tags), and an egui-based UI layer. `src/main.rs` is the composition root that wires the layers together with channels and threads. See [../technical/architecture.md](../technical/architecture.md) for the full technical picture, and [./features.md](./features.md) for the complete feature catalog.

## Design Philosophy

**Offline-first, by definition.** riff is not "cloud-capable but works offline" — it is offline, full stop. There is no streaming, no online metadata lookup, no telemetry, and no synchronization service. Your collection, your tags, and your cover art are the entire data model. This keeps the app simple, predictable, and private: there is no server to depend on, no API to break, and no account to leak.

**Your files are the library.** riff does not import, copy, or reorganize your music. It indexes what is on disk and remembers that index in the Application Store so the next launch is instant. Remove a library path and the index entries go away; your files are never touched. Edit your tags or drop a new album into a watched folder, and riff picks the change up on the next scan.

**Lightweight by construction.** One crate, one binary, immediate-mode UI. The Application Store means large collections (tens of thousands of tracks) load in well under a second instead of re-walking the disk on every start. Decoding streams packet by packet rather than loading whole files into memory, and cover art is decoded on a background thread and held in a small LRU cache.

**Cross-platform without lowest-common-denominator.** The core experience — scanning, browsing, playing — is identical everywhere. Platform integration is adapted rather than faked: macOS and Windows get a native folder picker and a system tray icon; Linux gets a plain text path input and runs window-only, because the tray dependency stack is not reliably available there.

## Who It's For

riff is for people who have a real music collection on disk and want a fast, quiet, respectful player for it. If your music is a folder of files rather than a subscription, if you care about tags and cover art, and if you would rather not create an account just to play a FLAC file, riff is built for you.

The [personas document](./personas.md) describes the target users in detail: the collector with music spread across several drives, the minimalist who wants a small fast player with no cloud entanglements, and the archivist who needs lossless formats and metadata to be treated with care.

## What riff Is Not

Setting expectations is as important as listing features. riff deliberately does **not**:

- **Stream music.** There is no support for HTTP streams, HLS, Spotify, or any online catalog. Playback is from local files only.
- **Sync or back up your library.** There is no cloud storage integration and no device sync. Network mounts (NAS shares) are supported simply because the operating system presents them as local folders.
- **Scrobble or share.** No Last.fm, no listening history in the cloud, no social features of any kind.
- **Fetch metadata or artwork from the internet.** Tags and cover art are read from your files and their directories, never looked up online.
- **Edit your tags.** riff reads metadata; it does not write it. Use a dedicated tagger for that.
- **Play DRM-protected files, DSD/SACD, or legacy formats** such as WMA or MIDI. The supported set is MP3, AAC (M4A), Opus, FLAC, OGG Vorbis, and WAV.
- **Replace a full studio player.** There is no equalizer, no crossfade, no output-device selection, and no visualization. See [./features.md](./features.md) for the complete deferred list.

## Technology at a Glance

riff is built from well-established Rust crates, chosen so that the core audio path is pure Rust and the whole product builds from a single crate:

| Concern | Technology |
|---|---|
| Language | Rust (edition 2021, MSRV 1.92) |
| UI | egui / eframe 0.34 (immediate-mode GUI, window persistence) with egui-elegance styling |
| Audio decoding | symphonia 0.5 (MP3, AAC, FLAC, OGG Vorbis, WAV) plus symphonia-adapter-libopus for Opus |
| Audio output | cpal 0.18 (ALSA/PipeWire/PulseAudio on Linux, WASAPI on Windows, CoreAudio on macOS) |
| Metadata | lofty 0.19 (pure-Rust tag and embedded-picture reading) |
| Cover art images | image 0.25 (JPEG and PNG decoding) |
| Library scanning | walkdir 2, with notify 7 for folder watching |
| Concurrency | std threads, crossbeam-channel, parking_lot |
| System integration | tray-icon + muda (tray menu) and rfd (native folder picker) on macOS/Windows only |
| Persistence | rusqlite (embedded SQLite), directories 5 |

The architecture keeps these crates out of the domain layer entirely: business logic (tracks, queues, playback state) has zero external imports, and the UI talks to hardware only through trait boundaries defined in the app layer. Details are in [../technical/architecture.md](../technical/architecture.md).

## Status

riff is at **v0.1.0** and is dual-licensed under **MIT OR Apache-2.0**.

The core is in place: multi-format decoding, playback control, the playback queue, library scanning, metadata extraction, cover art resolution, search, multi-library management, library persistence in the Application Store, folder watching, the dual-view library explorer, and cross-platform audio all work today and meet their specifications.

Several UI surfaces are functional but still marked **partial** against their full specifications. The known gaps are specific and modest:

- **Player control bar** — transport, seeking, volume, shuffle, repeat, and the queue indicator all work; the specified Stop button is not yet present (stop semantics already exist in the engine).
- **Now Playing view** — basic track info and the upcoming-queue list work; large cover art, the full metadata field set, clickable up-next rows, and an in-view progress bar remain to be built.
- **System tray** — minimize/restore and the playback context menu work on macOS and Windows; some specified behaviors such as configurable close-to-tray are unfinished.
- **Main window and cover art display** — working, with layout and window-management polish outstanding.

These surfaces are the focus of ongoing work. See [./features.md](./features.md) for per-feature status and [./roadmap.md](./roadmap.md) for what comes next, including the engineering infrastructure — CI and broader test coverage — that should accompany it.
