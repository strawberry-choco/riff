# riff

riff is a lightweight, offline-first desktop music player written in Rust. It plays the audio files you already own — MP3, AAC, Opus, FLAC, OGG Vorbis, and WAV — directly from your local disks, with no account, no cloud, and no network access required. It ships as a single binary built from a Cargo workspace, runs on Linux, Windows, and macOS, and is designed to start fast, stay out of the way, and treat your file system as the source of truth.

## Overview

riff is a player for local music collections. You point it at one or more folders — your main music directory, an external SSD, a mounted NAS share — and it scans them, reads the tags, resolves the cover art, and gives you a fast, searchable library. Playback runs on a dedicated audio thread with standard transport controls, a queue, shuffle and repeat, and cover art display. A system tray icon (on macOS and Windows) lets it keep playing while the window is hidden.

Everything happens on your machine. The library, playlists, and settings live in one embedded SQLite database — the Application Store (`riff.sqlite3`) — in your local data directory. Nothing is uploaded, nothing is fetched, and nothing phones home. If you disconnect the network cable, riff behaves exactly the same as it did before.

## Design Philosophy

**Offline-first, by definition.** riff is not "cloud-capable but works offline" — it is offline, full stop. There is no streaming, no online metadata lookup, no telemetry, and no synchronization service. Your collection, your tags, and your cover art are the entire data model. This keeps the app simple, predictable, and private: there is no server to depend on, no API to break, and no account to leak.

**Your files are the library.** riff does not import, copy, or reorganize your music. It indexes what is on disk and remembers that index in the Application Store so the next launch is instant. Remove a library path and the index entries go away; your files are never touched. Edit your tags or drop a new album into a watched folder, and riff picks the change up on the next scan.

**Lightweight by construction.** One binary, immediate-mode UI, and a workspace that keeps the core logic in pure-Rust crates. The Application Store means the library is browsable on the first frame of a launch instead of re-walking the disk on every start. Decoding streams packet by packet rather than loading whole files into memory, and cover art is decoded on a background thread, with each display size kept as a Thumbnail on disk so a relaunch does not re-decode the album.

**Cross-platform without lowest-common-denominator.** The core experience — scanning, browsing, playing — is identical everywhere. Platform integration is adapted rather than faked: macOS and Windows get a native folder picker and a system tray icon; Linux gets a plain text path input and runs window-only.

## Who It's For

riff is for people who have a real music collection on disk and want a fast, quiet, respectful player for it. The [personas document](./personas.md) describes the target users in detail: the collector with music spread across several drives, the minimalist who wants a small fast player with no cloud entanglements, and the archivist who needs lossless formats and metadata to be treated with care.

## What riff Is Not

Setting expectations is as important as listing features. The complete list of what riff deliberately does not do — and why each item is deferred rather than forgotten — is in [./features.md](./features.md#deferred--future).

