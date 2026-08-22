# Acceptance Requirements

This document defines the atomic, testable acceptance criteria for every riff feature. It converts the prose descriptions in [features.md](./features.md) into discrete statements that an implementation PR can verify against.

**How to use this document**
- Each criterion is independent: one feature, one behavior, one verification.
- Positive criteria describe expected behavior. Negative criteria describe what must *not* happen.
- Edge cases are called out explicitly; if a criterion is not listed, the behavior is unspecified.
- For deferred features, the criteria are written as if the feature were being implemented today — they define what "done" looks like.

**Status mapping**
- **P0** = essential; must be satisfied for the feature to be considered functional.
- **P1** = important; should be satisfied before the feature is considered complete.
- **P2** = polish; nice-to-have, can be deferred within a feature.

---

## Audio Engine

### REQ-AE-001: Multi-format Decoding

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-AE-001-01 | riff shall decode MP3 files with symphonia's native MP3 decoder. | P0 |
| REQ-AE-001-02 | riff shall decode AAC files in M4A containers with symphonia's native AAC decoder. | P0 |
| REQ-AE-003 | riff shall decode Opus files in OGG containers using the symphonia-adapter-libopus adapter. | P0 |
| REQ-AE-001-04 | riff shall decode FLAC files with symphonia's native FLAC decoder. | P0 |
| REQ-AE-001-05 | riff shall decode OGG Vorbis files with symphonia's native Vorbis decoder. | P0 |
| REQ-AE-001-06 | riff shall decode WAV files with symphonia's native WAV decoder. | P0 |
| REQ-AE-001-07 | Decoding shall be streaming — even very large FLAC files shall be read packet by packet, not loaded entirely into memory. | P0 |
| REQ-AE-001-08 | When a file cannot be decoded (corrupt header, truncated data, unsupported sub-codec), riff shall report a structured error naming the file and the reason, not crash. | P0 |
| REQ-AE-001-09 | riff shall not encode audio files. | P0 |
| REQ-AE-001-10 | riff shall not transcode audio between formats. | P0 |
| REQ-AE-001-11 | riff shall not play DRM-protected files (WMA, etc.). | P0 |
| REQ-AE-001-12 | riff shall not stream audio over the network. | P0 |

### REQ-AE-002: Playback Control

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-AE-002-01 | riff shall start playback of the current track on play. | P0 |
| REQ-AE-002-02 | riff shall pause playback of the current track on pause. | P0 |
| REQ-AE-002-03 | riff shall stop playback of the current track on stop. | P0 |
| REQ-AE-002-04 | riff shall seek to an arbitrary position within the current track on seek. | P0 |
| REQ-AE-002-05 | Seeking past the end of a track shall clamp to the end, not wrap or error. | P0 |
| REQ-AE-002-06 | Volume shall be adjustable from 0 to 100 percent. | P0 |
| REQ-AE-002-07 | Volume changes shall apply without audible clicks. | P0 |
| REQ-AE-002-08 | Playback position shall be reported to the UI several times per second during playback. | P0 |
| REQ-AE-002-09 | A dedicated audio engine thread shall own the decoder and audio output stream. | P0 |
| REQ-AE-002-10 | Decoded samples shall flow through a shared ring buffer with backpressure at approximately two seconds of audio. | P0 |
| REQ-AE-002-11 | The decoder shall never race ahead of playback; memory shall remain bounded. | P0 |
| REQ-AE-002-12 | On Windows WASAPI shared mode (commonly 48 kHz), riff shall fall back to the device's default sample rate when a track's native rate is unsupported. | P0 |
| REQ-AE-002-13 | If the audio output device disconnects mid-track, playback shall pause gracefully without crashing. | P0 |
| REQ-AE-002-14 | When an audio output device becomes available again after disconnect, the user shall be able to resume playback without restarting the app. | P0 |
| REQ-AE-002-15 | riff shall not provide output-device selection. | P0 |
| REQ-AE-002-16 | riff shall not provide crossfade between tracks. | P0 |
| REQ-AE-002-17 | riff shall not provide speed/pitch adjustment. | P0 |

### REQ-AE-003: Playback Queue

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-AE-003-01 | Tracks shall play in an ordered queue. | P1 |
| REQ-AE-003-02 | Next shall advance to the subsequent track in the queue. | P1 |
| REQ-AE-003-03 | Previous shall return to the prior track in the queue. | P1 |
| REQ-AE-003-04 | Previous shall restart the current track if more than a few seconds have already elapsed. | P1 |
| REQ-AE-003-05 | Shuffle shall randomize the queue order. | P1 |
| REQ-AE-003-06 | Repeat shall cycle through: no repeat → repeat all → repeat one. | P1 |
| REQ-AE-003-07 | The queue shall support replacing the entire queue with a new list of tracks. | P1 |
| REQ-AE-003-08 | The queue shall support inserting a track immediately after the currently playing track. | P1 |
| REQ-AE-003-09 | The queue shall support appending a track to the end of the queue. | P1 |

---

## Music Library

### REQ-ML-001: Library Scanning

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-ML-001-01 | riff shall recursively scan each registered library path for audio files with supported extensions. | P0 |
| REQ-ML-001-02 | Scanning shall run on a background thread; the UI shall never block. | P0 |
| REQ-ML-001-03 | Rescans shall be incremental: already-indexed file paths shall be skipped. | P0 |
| REQ-ML-001-04 | A track's identity shall be its full file path. | P0 |
| REQ-ML-001-05 | The same file shall never be indexed twice, regardless of which library path it was scanned through. | P0 |

### REQ-ML-002: Metadata Extraction

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-ML-002-01 | riff shall extract and store the title tag from audio containers. | P0 |
| REQ-ML-002-02 | riff shall extract and store the artist tag. | P0 |
| REQ-ML-002-03 | riff shall extract and store the album tag. | P0 |
| REQ-ML-002-04 | riff shall extract and store the album artist tag. | P0 |
| REQ-ML-002-05 | riff shall extract and store the genre tag. | P0 |
| REQ-ML-002-06 | riff shall extract and store the year tag. | P0 |
| REQ-ML-002-07 | riff shall extract and store the track number tag. | P0 |
| REQ-ML-002-08 | Album grouping in the library view shall be driven by the album artist field. | P0 |
| REQ-ML-002-09 | When album artist is missing, album grouping shall fall back to the track artist. | P0 |
| REQ-ML-002-10 | Files with missing tags shall still be indexed and playable. | P0 |
| REQ-ML-002-11 | The UI shall display only the metadata fields that exist for a track, rather than showing "Unknown" placeholders for missing fields. | P0 |

### REQ-ML-003: Cover Art Resolution

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-ML-003-01 | riff shall resolve cover art by first checking embedded artwork in the file's metadata. | P1 |
| REQ-ML-003-02 | Embedded artwork shall include ID3v2 APIC frames, FLAC/Vorbis picture blocks, and M4A covr atoms. | P1 |
| REQ-ML-003-03 | If no embedded artwork exists, riff shall check the track's directory for a filesystem cover image. | P1 |
| REQ-ML-003-04 | Filesystem cover lookup shall match filenames case-insensitively. | P1 |
| REQ-ML-003-05 | Filesystem cover names shall follow this priority order: cover → folder → album → front. | P1 |
| REQ-ML-003-06 | Filesystem covers shall support JPEG and PNG formats. | P1 |
| REQ-ML-003-07 | Cover images shall be decoded on a background thread. | P1 |
| REQ-ML-003-08 | Decoded cover textures shall be held in an LRU cache. | P1 |
| REQ-ML-003-09 | The cover texture cache shall be capped at 50 entries. | P1 |
| REQ-ML-003-10 | Oversized cover images shall fall back to a placeholder instead of failing. | P1 |
| REQ-ML-003-11 | Corrupt cover images shall fall back to a placeholder instead of failing. | P1 |
| REQ-ML-003-12 | If neither embedded nor filesystem artwork exists, riff shall show a placeholder. | P1 |
| REQ-ML-003-13 | riff shall never download cover art from the internet. | P0 |

### REQ-ML-004: Library Search

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-ML-004-01 | riff shall provide a single search box that filters the library by artist name. | P1 |
| REQ-ML-004-02 | The search box shall filter by album artist name. | P1 |
| REQ-ML-004-03 | The search box shall filter by album name. | P1 |
| REQ-ML-004-04 | The search box shall filter by track title. | P1 |
| REQ-ML-004-05 | In Library view, the search shall filter the metadata tree (artists → albums → tracks). | P1 |
| REQ-ML-004-06 | In Folders view, the search shall prune the tree to show only folders that contain matching tracks. | P1 |
| REQ-ML-004-07 | The search filter shall behave consistently between Library and Folders views for the same query. | P1 |

### REQ-ML-005: Music Library Management

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-ML-005-01 | riff shall provide a settings page that lists all registered library paths. | P0 |
| REQ-ML-005-02 | The user shall be able to add a new library path. | P0 |
| REQ-ML-005-03 | The user shall be able to remove a registered library path. | P0 |
| REQ-ML-005-04 | On macOS and Windows, adding a library path shall open the native OS folder picker (via rfd). | P0 |
| REQ-ML-005-05 | On Linux, adding a library path shall present a plain text input for the path. | P0 |
| REQ-ML-005-06 | The list of library paths shall be persisted across restarts. | P0 |
| REQ-ML-005-07 | Adding a path that is already in the list shall be a no-op (no duplicate entry). | P0 |
| REQ-ML-005-08 | Removing a path shall delete only the index entries for that path, never the files on disk. | P0 |
| REQ-ML-005-09 | A path pointing to an ejected drive or unmounted share shall be shown as unavailable rather than silently dropped. | P0 |
| REQ-ML-005-10 | The user shall be able to trigger a scan of a single library path. | P0 |
| REQ-ML-005-11 | The user shall be able to trigger a scan of all registered library paths. | P0 |

### REQ-ML-006: Library Persistence (Application Store)

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-ML-006-01 | The full scanned library (tracks, artists, albums, play history) shall persist in the Application Store (`riff.sqlite3`), the single authoritative store. | P1 |
| REQ-ML-006-02 | The store file shall live in the platform's data-local directory via the directories crate. | P1 |
| REQ-ML-006-03 | The library shall be available on the first frame of a launch, read from the store through Session Projections and startup hydration. | P1 |
| REQ-ML-006-04 | A large collection (50,000+ tracks) shall be browsable almost instantly after startup, without waiting for a disk walk. | P1 |
| REQ-ML-006-05 | Each scan batch (~10 tracks) shall commit as one durable transaction, so an interrupted scan keeps every committed batch. | P1 |
| REQ-ML-006-06 | Removing a library path shall remove its index entries in one durable transaction. | P1 |
| REQ-ML-006-07 | If the store file is missing, riff shall start with an empty library and create a fresh store. | P1 |
| REQ-ML-006-08 | If the store is corrupt, riff shall set the broken file aside automatically (preserved beside a fresh copy) and start fresh. | P1 |
| REQ-ML-006-09 | A failed store write shall log an error but shall not disturb the UI. | P1 |
| REQ-ML-006-10 | Schema evolution shall use ordered, checksummed migrations; a tampered migration is a fatal startup error. | P1 |

### REQ-ML-007: Folder Watching

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-ML-007-01 | Each library path shall have an opt-in "Watch" toggle for automatic change detection. | P1 |
| REQ-ML-007-02 | With watching enabled, riff shall monitor the folder tree for added and deleted files. | P1 |
| REQ-ML-007-03 | After a change is detected, riff shall wait for a two-second quiet period before triggering a rescan. | P1 |
| REQ-ML-007-04 | The debounce shall coalesce bursts: copying a whole album of a dozen tracks shall trigger a single rescan, not twelve. | P1 |
| REQ-ML-007-05 | New files detected by the watcher shall appear in the library after the rescan. | P1 |
| REQ-ML-007-06 | Deleted files detected by the watcher shall be evicted from the index. | P1 |
| REQ-ML-007-07 | Watch state shall persist across restarts. | P1 |
| REQ-ML-007-08 | If the OS cannot watch a path (network mount, permission problem, Linux inotify limit), the toggle shall show a warning state with an explanation. | P1 |

---

## User Interface

### REQ-UI-001: Main Application Window

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-UI-001-01 | riff shall render its UI in an egui window hosted by eframe. | P0 |
| REQ-UI-001-02 | Window size shall be persisted between sessions. | P0 |
| REQ-UI-001-03 | Window position shall be persisted between sessions. | P0 |
| REQ-UI-001-04 | The window shall be resizable by the user. | P0 |
| REQ-UI-001-05 | On macOS and Windows, closing the window shall minimize the app to the system tray rather than quitting. | P0 |
| REQ-UI-001-06 | On Linux, closing the window shall quit the app. | P0 |

### REQ-UI-002: Library Explorer Panel

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-UI-002-01 | The library panel shall offer two views: Library view (metadata) and Folders view (disk tree). | P0 |
| REQ-UI-002-02 | The user shall be able to toggle between Library view and Folders view. | P0 |
| REQ-UI-002-03 | Each view shall remember its selection independently; switching views shall not lose the user's place. | P0 |
| REQ-UI-002-04 | Library view shall browse by artist, expanding to albums, expanding to tracks. | P0 |
| REQ-UI-002-05 | In Library view, albums shall be grouped by album artist. | P0 |
| REQ-UI-002-06 | In Library view, albums shall be sorted by year. | P0 |
| REQ-UI-002-07 | In Library view, tracks within an album shall be sorted by track number. | P0 |
| REQ-UI-002-08 | Folders view shall show each registered library path as a top-level node. | P0 |
| REQ-UI-002-09 | Folders view shall show only subdirectories that actually contain audio files. | P0 |
| REQ-UI-002-10 | Folders view shall load children on demand (lazy loading). | P0 |
| REQ-UI-002-11 | Double-clicking a track shall start playback. | P0 |
| REQ-UI-002-12 | Double-clicking a folder (Folders view) shall replace the queue with every track in that folder and its subfolders and start playing. | P0 |
| REQ-UI-002-13 | Right-clicking a track shall show a context menu with Play, Play Next, and Append to Queue. | P0 |
| REQ-UI-002-14 | Right-clicking a folder shall show a context menu with Play, Play Next, and Append to Queue. | P0 |
| REQ-UI-002-15 | The currently playing track shall be visually marked in both Library and Folders views. | P0 |

### REQ-UI-003: Player Control Bar

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-UI-003-01 | The control bar shall be pinned to the bottom of the window and always visible. | P0 |
| REQ-UI-003-02 | The control bar shall show a previous button. | P0 |
| REQ-UI-003-03 | The control bar shall show a play/pause button. | P0 |
| REQ-UI-003-04 | The control bar shall show a next button. | P0 |
| REQ-UI-003-05 | The control bar shall show a progress bar with elapsed and total time in MM:SS format. | P0 |
| REQ-UI-003-06 | Clicking the progress bar shall seek to that position. | P0 |
| REQ-UI-003-07 | The control bar shall show a volume slider (0–100 percent). | P0 |
| REQ-UI-003-08 | The control bar shall show a mute toggle that restores the previous volume level when unmuted. | P0 |
| REQ-UI-003-09 | The control bar shall show a shuffle toggle. | P0 |
| REQ-UI-003-10 | The control bar shall show a repeat toggle that cycles through no repeat, repeat all, repeat one. | P0 |
| REQ-UI-003-11 | The control bar shall show a queue position indicator (e.g. "3 / 42"). | P0 |
| REQ-UI-003-12 | The control bar shall show a stop button. | P0 |

### REQ-UI-004: Cover Art Display

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-UI-004-01 | Wherever a track is shown in the UI, riff shall display its resolved cover art. | P1 |
| REQ-UI-004-02 | The cover art display shall use the same resolution pipeline and LRU texture cache as Cover Art Resolution. | P1 |
| REQ-UI-004-03 | When no cover art is available, riff shall display a placeholder. | P1 |

### REQ-UI-005: Now Playing View

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-UI-005-01 | The user shall be able to toggle the Now Playing view open and closed. | P2 |
| REQ-UI-005-02 | The view shall display the current track's title. | P2 |
| REQ-UI-005-03 | The view shall display the current track's artist. | P2 |
| REQ-UI-005-04 | The view shall display the current track's album. | P2 |
| REQ-UI-005-05 | The view shall display the current track's album artist. | P2 |
| REQ-UI-005-06 | The view shall display the current track's year. | P2 |
| REQ-UI-005-07 | The view shall display the current track's genre. | P2 |
| REQ-UI-005-08 | The view shall display the current track's track number. | P2 |
| REQ-UI-005-09 | The view shall display the upcoming tracks in the queue. | P2 |
| REQ-UI-005-10 | The view shall display the current track's cover art at large size. | P2 |
| REQ-UI-005-11 | Clicking an upcoming-queue row in the view shall play that track next. | P2 |
| REQ-UI-005-12 | The view shall include a seekable progress bar. | P2 |

---

## System Integration

### REQ-SI-001: System Tray Icon

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-SI-001-01 | On macOS, riff shall show a system tray icon. | P1 |
| REQ-SI-001-02 | On Windows, riff shall show a system tray icon. | P1 |
| REQ-SI-001-03 | On Linux, riff shall not show a system tray icon. | P1 |
| REQ-SI-001-04 | The tray icon's tooltip shall show the current track in the format "Artist - Title". | P1 |
| REQ-SI-001-05 | Left-clicking the tray icon shall toggle the main window (show if hidden, hide if visible). | P1 |
| REQ-SI-001-06 | Right-clicking the tray icon shall show a context menu with Play/Pause. | P1 |
| REQ-SI-001-07 | The tray context menu shall include Next Track. | P1 |
| REQ-SI-001-08 | The tray context menu shall include Previous Track. | P1 |
| REQ-SI-001-09 | The tray context menu shall include Show Window. | P1 |
| REQ-SI-001-10 | The tray context menu shall include Quit. | P1 |
| REQ-SI-001-11 | Playback shall continue with the main window hidden when the tray is active. | P1 |
| REQ-SI-001-12 | Quitting from the tray shall stop playback and exit the application. | P1 |
| REQ-SI-001-13 | On macOS and Windows, closing the main window shall minimize the app to the tray. | P1 |

### REQ-SI-002: Cross-platform Support

| # | Criterion | Priority |
|---|-----------|----------|
| REQ-SI-002-01 | riff shall build and run on Linux. | P0 |
| REQ-SI-002-02 | riff shall build and run on Windows. | P0 |
| REQ-SI-002-03 | riff shall build and run on macOS. | P0 |
| REQ-SI-002-04 | On Linux, riff shall use ALSA/PipeWire/PulseAudio for audio output. | P0 |
| REQ-SI-002-05 | On Windows, riff shall use WASAPI for audio output. | P0 |
| REQ-SI-002-06 | On macOS, riff shall use CoreAudio for audio output. | P0 |
| REQ-SI-002-07 | Sample-rate fallback shall work on all platforms when the device does not accept a track's native rate. | P0 |
| REQ-SI-002-08 | Platform-specific differences (tray, folder picker) shall be confined to conditional compilation and shall not affect core playback or library logic. | P0 |

---

## Deferred Requirements

These criteria are recorded here so they are not lost when features are promoted from deferred to planned.

### Playlists (deferred)
- The user shall be able to create a named playlist containing an ordered list of tracks.
- Playlists shall be persisted to disk and loaded on next launch.
- The user shall be able to load a playlist into the queue.
- When a track's file path changes or the file is deleted, the playlist entry shall be marked invalid (not cause a crash).

### Gapless Playback (deferred)
- When playing consecutive tracks from the same album, there shall be no audible silence between tracks.
- The next track shall be decoded and staged before the current track ends.
- Gaplessness shall apply to all supported formats.

### Equalizer (deferred)
- The user shall be able to adjust per-band EQ settings.
- EQ settings shall apply to the playback signal in real time.
- EQ settings shall be savable and loadable.

### ReplayGain Normalization (deferred)
- riff shall read existing ReplayGain tags from audio metadata where present.
- Where ReplayGain tags exist, riff shall apply the gain in the volume-scaling step.
- For tracks without ReplayGain tags, no gain adjustment shall be applied.

### Lyrics Display (deferred)
- riff shall display lyrics that are embedded in audio file tags.
- riff shall not fetch lyrics from the internet.

### Internet-based Features (explicitly non-goals)
- riff shall not stream music from the internet.
- riff shall not perform online metadata lookup.
- riff shall not scrobble listening history.
