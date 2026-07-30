# Feature Catalog

This is the canonical reference for everything riff does today and everything it deliberately defers. Features are grouped into four epics — Audio Engine, Music Library, User Interface, and System Integration. Each table records the feature's implementation status (implemented, partial, or deferred), its priority (P0 is essential, P2 is polish), and its dependencies within the product. For the product framing behind these capabilities, see [./overview.md](./overview.md); for how they are built, see [../technical/architecture.md](../technical/architecture.md); for what is planned, see [./roadmap.md](./roadmap.md).

Status meanings: **implemented** means the feature meets its specification; **partial** means it works but has known gaps against the spec (noted in prose below).

## Audio Engine

The core playback system: decoding multiple audio formats, managing the audio output stream, and providing playback controls. All decoding is pure Rust; output goes through the operating system's standard audio stack.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|
| Multi-format Decoding | Decode MP3, AAC, Opus, FLAC, OGG, WAV via symphonia | implemented | P0 | — |
| Playback Control | Play, pause, stop, seek, volume with cpal output | implemented | P0 | Multi-format Decoding |
| Playback Queue | Queue management, next/previous track, shuffle, repeat | implemented | P1 | Playback Control |

**Multi-format Decoding.** riff plays MP3, AAC in M4A containers, Opus in OGG containers, FLAC, OGG Vorbis, and WAV. MP3, AAC, FLAC, Vorbis, and WAV use symphonia's native decoders; Opus uses the symphonia-adapter-libopus adapter because symphonia 0.5 ships no native Opus decoder. Decoding is streaming — even very large FLAC files are read packet by packet rather than loaded into memory — and failures (corrupt headers, truncated files, unsupported sub-codecs such as ALAC) surface as structured errors naming the file and the reason, never as crashes. Encoding, transcoding, DRM-protected files, and network streaming are out of scope.

**Playback Control.** Standard transport behavior: play, pause, resume, stop, seek to an arbitrary position, and volume from 0 to 100 percent. A dedicated audio engine thread owns the decoder and the cpal output stream; decoded samples flow through a shared ring buffer with backpressure at roughly two seconds of audio, so the decoder never races ahead of playback and memory stays bounded. Seeking beyond the end of a track clamps to the end, volume changes apply without audible clicks, and the current position is reported back to the UI several times per second. On Windows, where WASAPI shared mode commonly runs at 48 kHz, the output falls back to the device's default sample rate when a track's native rate is unsupported. Output-device selection, crossfade, and speed/pitch adjustment are intentionally not provided.

**Playback Queue.** Tracks play as an ordered queue with next and previous navigation, shuffle, and a repeat mode that cycles through no repeat, repeat all, and repeat one. Queue operations support replacing the queue, inserting a track immediately after the current one, and appending to the end — the primitives that the library explorer's context menus build on. Previous returns to the prior track, or restarts the current one if more than a few seconds have elapsed.

## Music Library

The system that discovers, indexes, and organizes local audio files: scanning directories, extracting tags, resolving cover art, and keeping the index fresh and persistent.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|
| Library Scanning | Recursively scan directories for audio files | implemented | P0 | — |
| Metadata Extraction | Extract and store tags from audio containers | implemented | P0 | Library Scanning |
| Cover Art Resolution | Resolve cover: embedded metadata, then filesystem fallback | implemented | P1 | Metadata Extraction |
| Library Search | Search by artist, album artist, album, title | implemented | P1 | Metadata Extraction |
| Music Library Management | Manage multiple library paths, add/delete, persist the list | implemented | P0 | Library Scanning |
| Library Cache Persistence | Persist scanned tracks so the library loads instantly on startup | implemented | P1 | Library Scanning, Music Library Management |
| Folder Watching | Auto-detect added and deleted files, per-path toggle, debounced rescan | implemented | P1 | Library Scanning |

**Library Scanning.** Each registered library path is walked recursively with walkdir, and every file with a supported audio extension is indexed. Scanning runs on a background thread so the UI never blocks, and rescans are incremental — already-indexed paths are skipped. A track's identity is its full file path, so the same file is never indexed twice.

**Metadata Extraction.** Tags are read with lofty: title, artist, album, album artist, genre, year, and track number. The album artist field drives album grouping (with a fallback to the track artist), which keeps compilations coherent. Files with missing tags still index and play; the UI shows only the fields that exist rather than littering the view with "Unknown" placeholders.

**Cover Art Resolution.** Cover resolution follows a deterministic priority: embedded artwork in the file's metadata first (ID3v2 APIC frames, FLAC/Vorbis picture blocks, M4A covr atoms), then a case-insensitive filesystem fallback in the track's directory — cover, folder, album, or front images in JPEG or PNG, in that name priority order. Images decode on a background thread, decoded textures are held in an LRU cache (capped at 50 entries), and oversized or corrupt images fall back to a placeholder instead of failing. Online artwork lookup is explicitly out of scope: riff is offline-only.

**Library Search.** A single search box filters the library by artist, album artist, album, and title. The same query also filters the folder view, pruning branches that contain no matches, so search behaves consistently whichever way you are browsing.

**Music Library Management.** A settings page lists every registered library path and lets you add, remove, and rescan them. On macOS and Windows, "Add Library" opens the native OS folder picker (via rfd); on Linux it is a plain text input, because the native-dialog dependency stack is not assumed there. The list survives restarts, adding an existing path is a no-op, and removing a path deletes only the index entries — never the files on disk. Paths that point to an ejected drive or unmounted share are shown as unavailable rather than silently dropped, so you can decide when to remove them. Each path gets its own Scan button, plus a Scan All for the whole collection.

**Library Cache Persistence.** The full scanned library — tracks, artists, and albums — is serialized to `library_cache.json` in the platform data-local directory (via the directories crate). The cache loads on the first frame of the first launch, so a 50,000-track collection is browsable almost instantly instead of waiting for a disk walk. It is rewritten after every completed scan and whenever a library path is removed. A missing or corrupt cache silently falls back to an empty library — you just scan again — and a failed cache write logs a warning without disturbing the UI.

**Folder Watching.** Each library path has an opt-in "Watch" toggle. With watching enabled, the notify crate monitors the folder tree and, after a two-second quiet period that coalesces bursts (copying a whole album fires one rescan, not twelve), triggers an incremental rescan of the affected path. New files appear in the library; deleted files are evicted from the index so search results never point at ghosts. Watch state persists across restarts. Where the OS cannot watch a path — some network mounts, permission problems, or hitting the Linux inotify limit — the toggle shows a warning state with an explanation instead of failing silently.

## User Interface

The egui-based graphical interface: the main window, a dual-view library explorer, the player control bar, cover art display, and the Now Playing view. It runs on Linux, Windows, and macOS with minimal external dependencies.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|
| Main Application Window | egui window with cross-platform support | partial | P0 | — |
| Library Explorer Panel | Dual library/folder views with toggle, folder playback, context menus, search | implemented | P0 | Library Search |
| Player Control Bar | Transport controls, progress bar, volume slider | partial | P0 | Playback Control |
| Cover Art Display | Display resolved cover art in the UI | partial | P1 | Cover Art Resolution |
| Now Playing View | Full track info, large cover art, queue peek | partial | P2 | Playback Queue, Cover Art Display |

**Main Application Window.** The whole UI is an egui application hosted by eframe, with window size and position persisted between sessions. It is marked partial because some surrounding window-management behaviors (notably close-to-tray integration on supported platforms) are still being completed against the spec.

**Library Explorer Panel.** The left panel offers two ways into the same collection, switchable by a toggle that remembers each view's selection independently. The Library view browses by metadata — artist, then album (grouped by album artist, sorted by year), then track in track-number order. The Folders view mirrors the disk: each library path is a root node, only directories that actually contain audio appear, and children load lazily so huge trees stay responsive. Double-clicking a folder replaces the queue with everything under it and starts playing; right-click menus on folders and tracks offer Play, Play Next, and Append to Queue. The currently playing track is marked in both views.

**Player Control Bar.** The always-visible bottom bar carries previous / play-pause / next, a click-to-seek progress bar with elapsed and total time in MM:SS form, a volume slider with mute, shuffle and repeat toggles, and a queue position indicator such as "3 / 42". It is marked partial because the specified Stop button is not yet present; stop semantics themselves exist in the engine.

**Cover Art Display.** Resolved cover art renders alongside track information, backed by the same resolution and LRU-texture pipeline described under Cover Art Resolution, with a placeholder when no art exists. It is partial: the display works, but sizing and layout polish against the spec remain open.

**Now Playing View.** A togglable view focused on the current track: title, artist, and album, plus the upcoming queue. It is the most incomplete surface — currently partial, with large cover art, the full metadata field set (album artist, year, genre, track number), clickable up-next rows, and an in-view progress bar still to come. See [./roadmap.md](./roadmap.md).

## System Integration

Cross-platform integration with the desktop: the system tray and per-platform window and audio behavior.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|
| System Tray Icon | Minimize to tray, restore, quit (macOS/Windows only) | partial | P1 | Main Application Window |
| Cross-platform Support | Linux, Windows, macOS window and audio compatibility | implemented | P0 | Main Application Window |

**System Tray Icon.** On macOS and Windows, riff lives in the system tray (built on tray-icon and muda): the tooltip shows the current track, left-click toggles the window, and the right-click menu offers Play/Pause, Next, Previous, Show Window, and Quit, so playback continues with the window hidden. It is partial against the spec (some behaviors such as configurable close-to-tray are unfinished), and it deliberately does not exist on Linux — the tray dependency stack (libayatana-appindicator) is not reliably present across distributions, so Linux builds run as a normal window-only application. End-user details are in [./user-guide.md](./user-guide.md).

**Cross-platform Support.** The same binary pattern builds and runs on all three platforms: cpal selects ALSA/PipeWire/PulseAudio on Linux, WASAPI on Windows, and CoreAudio on macOS, with sample-rate fallback when a device does not accept a track's native rate. Platform differences are confined to integration points — tray and folder picker — behind conditional compilation, never in the core playback or library logic.

## Deferred / Future

These capabilities were considered and explicitly deferred. They are not bugs and not forgotten; each has a stated reason, and the most likely candidates for promotion are discussed in [./roadmap.md](./roadmap.md).

- **Playlist management** — creating, saving, and loading custom playlists. Deferred because core playback and library features must be solid first. Folder playback and queue operations cover ad-hoc sequencing in the meantime.
- **Equalizer / audio effects** — per-band EQ, reverb, and similar. A nice-to-have, not essential for the first release.
- **Gapless playback** — seamless transitions between consecutive album tracks. Requires complex cross-track buffering; deferred to post-1.0 work.
- **ReplayGain normalization** — automatic volume leveling across tracks. Requires a metadata analysis pass over the library; deferred.
- **Lyrics display** — embedded or fetched lyrics. Not requested within scope.
- **Internet-based features** — streaming, online metadata lookup, scrobbling. Explicitly out of scope: riff is an offline-only player by design, as described in [./overview.md](./overview.md).
