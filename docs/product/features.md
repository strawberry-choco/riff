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
| Gapless Playback | Pre-decode the next track for seamless transitions at track boundaries | implemented | P2 | Playback Queue |
| ReplayGain Normalization | Track-gain leveling from ReplayGain tags, peak-capped, opt-in | implemented | P2 | Playback Control, Metadata Extraction |

**Multi-format Decoding.** riff plays MP3, AAC in M4A containers, Opus in OGG containers, FLAC, OGG Vorbis, and WAV. MP3, AAC, FLAC, Vorbis, and WAV use symphonia's native decoders; Opus uses the symphonia-adapter-libopus adapter because symphonia 0.5 ships no native Opus decoder. Decoding is streaming — even very large FLAC files are read packet by packet rather than loaded into memory — and failures (corrupt headers, truncated files, unsupported sub-codecs such as ALAC) surface as structured errors naming the file and the reason, never as crashes. Encoding, transcoding, DRM-protected files, and network streaming are out of scope.

**Playback Control.** Standard transport behavior: play, pause, resume, stop, seek to an arbitrary position, and volume from 0 to 100 percent. A dedicated audio engine thread owns the decoder and the cpal output stream; decoded samples flow through a shared ring buffer with backpressure at roughly two seconds of audio, so the decoder never races ahead of playback and memory stays bounded. Seeking beyond the end of a track clamps to the end, volume changes apply without audible clicks, and the current position is reported back to the UI several times per second. On Windows, where WASAPI shared mode commonly runs at 48 kHz, the output falls back to the device's default sample rate when a track's native rate is unsupported. Output-device selection, crossfade, and speed/pitch adjustment are intentionally not provided.

**Playback Queue.** Tracks play as an ordered queue with next and previous navigation, shuffle, and a repeat mode that cycles through no repeat, repeat all, and repeat one. Queue operations support replacing the queue, inserting a track immediately after the current one, and appending to the end — the primitives that the library explorer's context menus build on. Previous returns to the prior track, or restarts the current one if more than a few seconds have elapsed.

**Gapless Playback.** Consecutive tracks transition without a burst of silence whenever the engine can stage the handoff. About two seconds before the current track reaches its end, the engine begins decoding the next queue entry and builds a pre-buffer of up to four seconds, so the switch happens sample-continuously instead of stop-decode-start. Same-format handoffs are seamless, including repeat-one wrapping back onto the start of the same track; when the next track uses a different format, or shuffle replaces the expected successor mid-transition, the engine falls back to the ordinary gapped path rather than glitching. The pre-buffer is bounded, so memory stays flat no matter how long the queue runs.

**ReplayGain Normalization.** Tracks whose tags carry ReplayGain information play at consistent loudness. The scanner reads REPLAYGAIN_TRACK_GAIN and REPLAYGAIN_TRACK_PEAK alongside the rest of the metadata, and when the feature is enabled in settings the track gain is applied at the volume-scaling stage of the audio output callback, capped by the peak value so a boosting gain cannot push samples into clipping. The setting is opt-in and takes effect on the next track; untagged tracks play untouched at their native level. Analyzing untagged libraries and album-gain leveling remain deferred (see below).

## Music Library

The system that discovers, indexes, and organizes local audio files: scanning directories, extracting tags, resolving cover art, and keeping the index fresh and persistent.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|
| Library Scanning | Recursively scan directories for audio files | implemented | P0 | — |
| Metadata Extraction | Extract and store tags from audio containers | implemented | P0 | Library Scanning |
| Cover Art Resolution | Resolve cover: embedded metadata, then filesystem fallback | implemented | P1 | Metadata Extraction |
| Library Search | Search by artist, album artist, album, title | implemented | P1 | Metadata Extraction |
| Music Library Management | Manage multiple library paths, add/delete, persist the list | implemented | P0 | Library Scanning |
| Library Persistence | Persist scanned tracks in the Application Store so the library loads instantly on startup | implemented | P1 | Library Scanning, Music Library Management |
| Folder Watching | Auto-detect added and deleted files, per-path toggle, debounced rescan | implemented | P1 | Library Scanning |
| Tag Writing | Edit a track's tags from the UI; written via lofty on a background thread | implemented | P1 | Metadata Extraction |
| Smart Playlists | Locally generated discovery lists: Recently Added, Most Played, Never Played, Lost Gems | implemented | P1 | Library Persistence |
| Custom Playlists | Named playlists with ordered tracks; create, rename, delete, persist | implemented | P1 | Playback Queue, Library Scanning |
| Clear Library | Wipe the indexed collection while preserving playlists and settings | implemented | P2 | Library Persistence |

**Library Scanning.** Each registered library path is walked recursively with walkdir, and every file with a supported audio extension is indexed. Scanning runs on a background thread so the UI never blocks, and rescans are incremental — already-indexed paths are skipped. A track's identity is its full file path, so the same file is never indexed twice.

**Metadata Extraction.** Tags are read with lofty: title, artist, album, album artist, genre, year, and track number. ReplayGain track gain and peak tags are read too, feeding the normalization described under the Audio Engine. The album artist field drives album grouping (with a fallback to the track artist), which keeps compilations coherent. Files with missing tags still index and play; the UI shows only the fields that exist rather than littering the view with "Unknown" placeholders.

**Cover Art Resolution.** Cover resolution follows a deterministic priority: embedded artwork in the file's metadata first (ID3v2 APIC frames, FLAC/Vorbis picture blocks, M4A covr atoms), then a case-insensitive filesystem fallback in the track's directory — cover, folder, album, or front images in JPEG or PNG, in that name priority order. Images decode on a background thread, decoded textures are held in an LRU cache (capped at 50 entries), and oversized or corrupt images fall back to a placeholder instead of failing. Online artwork lookup is explicitly out of scope: riff is offline-only.

**Library Search.** A single search box filters the library by artist, album artist, album, and title. The same query also filters the folder view, pruning branches that contain no matches, so search behaves consistently whichever way you are browsing.

**Music Library Management.** A settings page lists every registered library path and lets you add, remove, and rescan them. On macOS and Windows, "Add Library" opens the native OS folder picker (via rfd); on Linux it is a validated text input with autocomplete — nonexistent paths and non-directories get clear error messages, and the settings page carries platform notes documenting what the text-input picker can and cannot do. The list survives restarts, adding an existing path is a no-op, and removing a path deletes only the index entries — never the files on disk. Paths that point to an ejected drive or unmounted share are shown as unavailable rather than silently dropped, so you can decide when to remove them. Each path gets its own Scan button, plus a Scan All for the whole collection.

**Library Persistence.** The full scanned library — tracks, artists, albums, and per-track play history — lives in the Application Store (`riff.sqlite3`) in the platform data-local directory (via the directories crate). The store is the single authority: every logical change commits as one small durable transaction, and scan batches (~10 tracks) commit incrementally so an interrupted scan keeps everything already saved. The library is available on the first frame of a launch, read through Session Projections over store queries, so a 50,000-track collection is browsable almost instantly instead of waiting for a disk walk. A missing store starts fresh; a corrupt one is set aside automatically (preserved beside a fresh copy) and the app continues. Schema evolution runs through ordered, checksummed migrations.

**Folder Watching.** Each library path has an opt-in "Watch" toggle. With watching enabled, the notify crate monitors the folder tree and, after a two-second quiet period that coalesces bursts (copying a whole album fires one rescan, not twelve), triggers an incremental rescan of the affected path. New files appear in the library; deleted files are evicted from the index so search results never point at ghosts. Watch state persists across restarts. Where the OS cannot watch a path — some network mounts, permission problems, or hitting the Linux inotify limit — the toggle shows a warning state with an explanation instead of failing silently.

**Tag Writing.** Right-clicking a track offers Edit Tags, which opens a modal with the editable fields. Writes run on a background thread through lofty so the UI never blocks, and a successful write updates the in-memory library immediately — no rescan needed. Failures, such as read-only files or unsupported tag formats, surface as graceful errors naming the file rather than losing the edit silently.

**Smart Playlists.** Four discovery playlists are generated locally from data riff already has: Recently Added, Most Played, Never Played, and Lost Gems (tracks unheard for more than ninety days). Nothing leaves the machine — the lists are computed as store queries, and the play counts and timestamps they rely on persist in the Application Store across restarts. They behave like any other playable list: selecting one fills the queue.

**Custom Playlists.** You can create named playlists, add tracks to them from the library's context menus (with deduplication, so a track appears once per playlist), reorder their contents, rename them, and delete them. Playlists persist in the Application Store — every mutation commits as one immediate durable transaction, because playlists are user data. Entries whose files have vanished stay visible, rendered struck-through as "(missing)", and are excluded from playback rather than breaking the list.

**Clear Library.** Settings provides a "Clear Library" action, guarded by a confirmation dialog, for the cases where you want to force a clean rebuild. One transaction wipes the indexed collection — tracks with their play history, albums, and artists — while playlists and settings are untouched; entries pointing at wiped tracks stay listed and recover when the files return via a rescan.

## User Interface

The egui-based graphical interface: the main window, a dual-view library explorer, the player control bar, cover art display, and the Now Playing view. It runs on Linux, Windows, and macOS with minimal external dependencies.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|
| Main Application Window | egui window with cross-platform support, close-to-tray on macOS/Windows | implemented | P0 | — |
| Library Explorer Panel | Dual library/folder views with toggle, folder playback, context menus, search | implemented | P0 | Library Search |
| Player Control Bar | Transport controls, progress bar, volume with mute, stop behind advanced mode | implemented | P0 | Playback Control |
| Cover Art Display | Display resolved cover art in the UI | implemented | P1 | Cover Art Resolution |
| Now Playing View | Full track info, large cover art, clickable up-next queue | implemented | P2 | Playback Queue, Cover Art Display |
| Progressive Disclosure | Advanced mode reveals tag editing, smart playlists, and the stop control | implemented | P1 | Player Control Bar, Tag Writing, Smart Playlists |
| Keyboard Accessibility & High Contrast | Full keyboard navigation, visible focus indicator, persistent high-contrast theme | implemented | P1 | Main Application Window |

**Main Application Window.** The whole UI is an egui application hosted by eframe, with window size and position persisted between sessions. On macOS and Windows, closing the window minimizes to the system tray with playback continuing rather than quitting; on Linux, closing the window quits, by design (see [./decisions/002-no-tray-on-linux.md](./decisions/002-no-tray-on-linux.md)).

**Library Explorer Panel.** The left panel offers two ways into the same collection, switchable by a toggle that remembers each view's selection independently. The Library view browses by metadata — artist, then album (grouped by album artist, sorted by year), then track in track-number order. The Folders view mirrors the disk: each library path is a root node, only directories that actually contain audio appear, and children load lazily so huge trees stay responsive. Double-clicking a folder replaces the queue with everything under it and starts playing; right-click menus on folders and tracks offer Play, Play Next, and Append to Queue. The currently playing track is marked in both views.

**Player Control Bar.** The always-visible bottom bar carries previous / play-pause / next, a click-to-seek progress bar with elapsed and total time in MM:SS form, a volume slider with a mute toggle that restores the exact previous volume when unmuted, shuffle and repeat toggles, and a queue position indicator such as "3 / 42". A Stop button is available in advanced mode, keeping the default surface minimal (see Progressive Disclosure below); stop semantics themselves live in the engine.

**Cover Art Display.** Resolved cover art renders alongside track information, backed by the same resolution and LRU-texture pipeline described under Cover Art Resolution, with a placeholder glyph when no art exists. Rendering is fixed-size and clamped in both places art appears — the library detail pane (200 px) and the Now Playing view (300 px) — so odd-sized images stay contained.

**Now Playing View.** A togglable view focused on the current track: large cover art (300 px), the full metadata field set (title, artist, album, album artist, year, genre, track number), and the upcoming queue with clickable rows that promote a track via Play Next. An in-view seek slider mirrors the control bar's progress and clamps the same way, and with nothing playing the view shows a calm empty state instead of dangling controls.

**Progressive Disclosure.** The default interface stays minimal; an advanced-mode toggle in settings reveals the power features — tag editing, the smart playlists, and the transport's stop control. The intent is that a first-time user sees a player, not a database frontend, while the collector-grade tooling stays one toggle away.

**Keyboard Accessibility & High Contrast.** The entire UI is operable from the keyboard, with a clearly visible focus indicator showing where you are. A high-contrast theme is available from settings; the choice persists across restarts, and switching it off fully restores the normal theme.

## System Integration

Cross-platform integration with the desktop: the system tray and per-platform window and audio behavior.

| Feature | Summary | Status | Priority | Depends On |
|---|---|---|---|---|
| System Tray Icon | Close-to-tray, restore, playback controls, quit (macOS/Windows only) | implemented | P1 | Main Application Window |
| Linux Folder Picker | Validated text-input folder picker with autocomplete and clear errors | implemented | P2 | Music Library Management |
| Cross-platform Support | Linux, Windows, macOS window and audio compatibility | implemented | P0 | Main Application Window |

**System Tray Icon.** On macOS and Windows, riff lives in the system tray (built on tray-icon and muda). Closing the window minimizes to the tray with playback continuing; the tooltip shows the current track as "Artist - Title"; left-click toggles the window; and the right-click menu offers Play/Pause, Next Track, Previous Track, Show Window, and Quit (which stops playback, then exits). It deliberately does not exist on Linux — the tray dependency stack (libayatana-appindicator) is not reliably present across distributions, so Linux builds run as a normal window-only application where closing the window quits. End-user details are in [./user-guide.md](./user-guide.md).

**Linux Folder Picker.** On Linux, where the native-dialog dependency stack is not assumed, "Add Library" is a text input field rather than a native dialog. Input is validated with autocomplete over existing directories; entering a path that does not exist, or points at a file rather than a directory, produces a clear error instead of a silent failure. The settings page documents the platform's limitations in-app, so the difference from macOS and Windows is explained where you configure it.

**Cross-platform Support.** The same binary pattern builds and runs on all three platforms: cpal selects ALSA/PipeWire/PulseAudio on Linux, WASAPI on Windows, and CoreAudio on macOS, with sample-rate fallback when a device does not accept a track's native rate. Platform differences are confined to integration points — tray and folder picker — behind conditional compilation, never in the core playback or library logic.

## Deferred / Future

These capabilities were considered and explicitly deferred. They are not bugs and not forgotten; each has a stated reason, and the most likely candidates for promotion are discussed in [./roadmap.md](./roadmap.md).

- **Equalizer / audio effects** — per-band EQ, reverb, and similar. A nice-to-have, not essential for the first release.
- **ReplayGain analysis and album-gain mode** — computing loudness data for untagged libraries and album-based leveling. Tag-based track ReplayGain ships today (see Audio Engine); the analysis pass remains deferred.
- **Lyrics display** — embedded or fetched lyrics. Not requested within scope.
- **Internet-based features** — streaming, online metadata lookup, scrobbling. Explicitly out of scope: riff is an offline-only player by design, as described in [./overview.md](./overview.md).
