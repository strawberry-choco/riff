# User Guide

This guide walks through using riff day to day: getting it running, building a library, browsing and playing music, and the platform-specific behaviors worth knowing about. It is written for listeners, not developers — if you want the feature-level reference see [./features.md](./features.md), and for what riff is and is not, see [./overview.md](./overview.md).

## Getting it running

riff is a single Rust program. From a checkout of the source:

```bash
cargo run
```

This builds and launches the development build. For everyday use, build the optimized release binary instead — it is compiled with full optimization, link-time optimization, and stripped symbols, so it is noticeably smaller and faster:

```bash
cargo build --release
```

The resulting `riff` executable (in `target/release/`) is self-contained; you can move it anywhere and run it directly. Building requires a recent Rust toolchain (the project's minimum supported Rust version is 1.92).

## First launch

The first time you open riff, the library is empty — no tracks, no artists, no albums — and the settings page shows no library paths. This is expected: riff has nothing indexed yet and, by design, does not assume where your music lives. There are no error messages on a fresh start; an empty state simply invites you to add a music folder.

On every subsequent launch, riff loads your previously scanned library from an on-disk cache before the first frame is drawn, so a large collection is browsable almost immediately rather than re-scanning the disk each time. The cache is refreshed automatically whenever a scan completes.

## Adding a music library

Library folders are managed from the settings page, opened from the gear icon in the top bar. Click **Add Library** to register a folder that contains your music.

- **macOS and Windows:** the operating system's native folder picker opens. Browse to your music folder (for example your Music directory or an external drive), confirm, and the path is added to the list. Cancelling the dialog adds nothing.
- **Linux:** there is no native picker; instead a plain text input appears. Type or paste the full directory path and confirm.

A few behaviors are the same everywhere:

- Adding a path that is already in the list does nothing — you will never see duplicate entries.
- The list of paths is saved automatically and restored on the next launch.
- You can register several locations — an internal drive, an external SSD, a mounted NAS share — and riff combines them into one unified library. Network mounts work because your operating system presents them as ordinary folders.
- Removing a path from the list (the delete control on its row) only removes it from riff's index. **Your files on disk are never deleted.**

If a registered path disappears — an ejected external drive, an unmounted share — riff shows it as unavailable rather than silently dropping it, and skips it during scans. You decide when to remove it.

## Scanning your library

After adding paths, trigger a scan to index the files. Each library row in settings has its own **Scan** button, and there is a **Scan All** button that scans every registered path. Scanning runs in the background; the interface stays responsive while it works, and new tracks appear as the scan completes. When a scan finishes, the library cache is rewritten so the next launch is instant.

You do not need to rescan manually every time your collection changes. Each library path has a **Watch** toggle: with it enabled, riff monitors the folder and automatically rescans when files appear or disappear. Rapid changes are coalesced — copying an album of a dozen tracks triggers a single rescan after a short quiet period, not twelve. Deleted files are removed from the index, so search results never point at tracks that no longer exist. Watch state is remembered across restarts.

Watching is best-effort and depends on the operating system. On a filesystem that cannot report change events (some network mounts), or if permissions or system limits get in the way (on Linux, the kernel caps the number of watched directories), the toggle shows a warning state with an explanation instead of silently doing nothing. You can always fall back to a manual scan.

## Browsing your library

The library panel on the left offers two views of the same collection, switched with a toggle. Each view remembers what you had selected, so switching back and forth does not lose your place.

**Library view** browses by metadata: a sorted list of artists, expanding to their albums (grouped by album artist, so compilations stay together, and ordered by year), expanding to tracks in track-number order. This is the view to use when you think in terms of artists and records.

**Folders view** mirrors your disk. Each registered library path is a top-level node; expanding it shows only the subdirectories that actually contain audio, and children load on demand, so even very large trees expand quickly. This is the view to use when you organize by directory structure and want to see exactly what is on disk.

The **search box** filters whichever view is active. In Library view it matches artist, album artist, album, and title; in Folders view it prunes the tree to folders containing matching tracks.

To play something:

- **Double-click a track** to start it. The queue is replaced with the surrounding album context when available.
- **Double-click a folder** (Folders view) to replace the queue with every track in that folder and its subfolders and start playing from the top — an instant ad-hoc playlist.
- **Right-click** any track or folder for a context menu: Play, Play Next (insert right after the current track), or Append to Queue (add to the end without interrupting playback).

The track that is currently playing is marked in both views, so you can always see where you are.

## Playing music

The control bar is pinned to the bottom of the window and always visible.

- **Transport:** previous, play/pause, and next. Previous jumps back to the prior track, or restarts the current one if more than a few seconds have already played.
- **Progress bar:** shows elapsed and total time as MM:SS / MM:SS. Click anywhere on it to seek to that position; seeking past the end clamps to the end of the track.
- **Volume:** a slider from 0 to 100 percent, plus a mute toggle that restores your previous level when unmuted. Volume changes apply smoothly, without clicks.
- **Shuffle:** toggles randomized queue order.
- **Repeat:** cycles through no repeat, repeat all, and repeat one.
- **Queue position:** an indicator such as "3 / 42" shows where you are in the queue.

Playback starts within a moment of selecting a track, the UI never blocks while audio is decoding, and position updates continuously during playback. If your audio device disconnects mid-track (Bluetooth headphones powering off, for example), playback pauses gracefully and you can resume once an output device is available again — no restart needed.

One platform note: on Windows, the shared-mode audio system commonly runs at 48 kHz. If a track's native sample rate is not supported by the device, riff falls back to the device's default rate automatically; you do not need to configure anything.

## Cover art

Wherever a track is shown, riff displays its cover art when one can be found, using a fixed resolution order:

1. **Embedded artwork** in the file's tags takes priority — this is what most tagged libraries use.
2. Otherwise, an **image file next to the audio file**: cover, folder, album, or front, in JPEG or PNG, matched case-insensitively (so `Cover.PNG` works on any file system).
3. If neither exists, a **placeholder** is shown.

Cover images are decoded in the background and cached, so scrolling through a large library stays smooth. Corrupt or oversized images fall back to the placeholder rather than causing errors. riff never downloads artwork: consistent with its offline design, art comes only from your files and folders.

## Now Playing view

A toggle opens the Now Playing view, a focused panel for the current track showing its title, artist, and album along with the upcoming tracks in the queue. It is the view for leaning back and listening rather than browsing. Note that this view is still maturing: large cover art, the full metadata field set (album artist, year, genre, track number), and clickable up-next rows are not all in place yet — see [./features.md](./features.md) for the current status and [./roadmap.md](./roadmap.md) for plans.

## System tray

**macOS and Windows:** riff runs with a system tray icon. Close the window and playback continues with the app tucked into the tray; the icon's tooltip shows the current track as "Artist - Title". Left-click the icon to show or hide the window. Right-click for a menu with Play/Pause, Next Track, Previous Track, Show Window, and Quit — enough to control playback without ever opening the window. Quit from the tray to stop playback and exit fully.

**Linux:** there is no tray icon. Linux builds run as an ordinary windowed application; closing the window closes the app. This is a deliberate platform decision, not a missing feature — the tray technology is not reliably available across Linux distributions.

## Settings and library management

Everything about your libraries lives in the settings page (gear icon, top bar):

- The list of registered library paths, each with its status and its own **Scan** button.
- A per-path **Watch** toggle for automatic change detection.
- **Add Library** and **Scan All** controls.
- A delete control per path (removes the path from riff only — never your files).

Window size and position, your library paths, and watch preferences are all persisted between sessions.

## Where riff keeps its data

riff stores nothing in your music folders. Its own data lives in the standard per-user locations for your operating system:

- **Library cache** (`library_cache.json` — the scanned index of tracks, artists, and albums):
  - Linux: `~/.local/share/riff/library_cache.json`
  - macOS: `~/Library/Application Support/com.riff.riff/library_cache.json`
  - Windows: `C:\Users\<user>\AppData\Local\riff\riff\library_cache.json`
- **Library paths and window state:** stored through the egui/eframe persistence mechanism alongside the application's saved state.

If the cache is ever missing or unreadable, riff simply starts with an empty library and a scan rebuilds it — nothing is lost, because your music was never in the cache to begin with.
