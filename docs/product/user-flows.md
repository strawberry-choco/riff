# User Flows

This document describes the end-to-end journeys a user takes through riff. Each flow is a numbered sequence of user actions and system responses, describing how features compose into a complete experience. Individual button behaviors are documented in [interaction-specs.md](./interaction-specs.md); error conditions in [error-states.md](./error-states.md).

---

## Flow 1: First Launch & Setup

**Goal**: A new user opens riff, adds a music folder, and has an indexed library ready to browse.

1. **User launches riff.**
   - The main window opens.
   - The library is empty: no tracks, no artists, no albums.
   - The settings page shows no library paths.
   - No error messages are shown; the empty state invites the user to add a music folder.

2. **User opens settings** (gear icon in the top bar).

3. **User clicks "Add Library."**
   - **macOS/Windows**: The native OS folder picker opens.
   - **Linux**: A plain text input appears for typing or pasting a directory path.

4. **User selects a music folder and confirms.**
   - The path is added to the library paths list in settings.
   - The path shows a status indicator (initially unscanned).

5. **User clicks the path's "Scan" button (or "Scan All").**
   - Scanning begins on a background thread.
   - The UI remains responsive.
   - New tracks appear in the library as the scan completes.

6. **Scan completes.**
   - The library cache is written to disk.
   - The library panel now shows the indexed tracks, artists, and albums.
   - The user can browse and play immediately.

**Branch — Adding a second library path later**: The user repeats steps 2–5 with another folder. Both paths contribute to a unified library.

---

## Flow 2: Daily Use — Browse & Play

**Goal**: A user with an existing library browses by artist and album, then plays a track.

1. **User launches riff.**
   - The library cache loads on the first frame.
   - The library is browsable almost immediately.

2. **User opens the Library view** (metadata tree).
   - A sorted list of artists is shown.

3. **User expands an artist.**
   - Albums appear, grouped by album artist and sorted by year.

4. **User expands an album.**
   - Tracks appear in track-number order.

5. **User double-clicks a track.**
   - The track starts playing.
   - The currently playing track is marked in the library tree.
   - The player control bar shows the track and transport controls.

6. **User toggles shuffle** on the control bar.
   - The queue order is randomized.

7. **User toggles repeat** on the control bar.
   - The repeat mode cycles through its states.

**Branch — Track ends naturally**: The next track in the queue plays automatically. The mark in the library tree moves to the new track.

---

## Flow 3: Daily Use — Search & Play

**Goal**: A user searches for something specific and plays the result.

1. **User launches riff** (library already indexed).

2. **User types a query into the search box.**
   - The library tree filters in real time.
   - In Library view: only matching artists, albums, and tracks remain visible.
   - In Folders view: only folders containing matching tracks remain visible.

3. **User browses the filtered results.**

4. **User right-clicks a track and selects "Play."**
   - The track starts playing.
   - The queue is replaced with the surrounding context (album context when available).

5. **User clears the search box.**
   - The full library tree is restored.
   - The user's selection (the playing track) is preserved.

---

## Flow 4: Ad-Hoc Folder Playback

**Goal**: A user plays an entire folder of music (e.g. a newly copied album).

1. **User switches to Folders view.**

2. **User navigates the folder tree** to find a specific directory.

3. **User double-clicks a folder.**
   - The queue is replaced with every track in that folder and its subfolders.
   - Playback starts from the top of the queue.

**Branch — Right-click context menu**: The user can right-click a folder and choose "Play Next" (insert after current) or "Append to Queue" instead of replacing the entire queue.

---

## Flow 5: Adding New Music — Automatic

**Goal**: A user copies new files into a watched folder; they appear in the library without manual action.

1. **Folder watching is enabled** for the library path (Watch toggle is on).

2. **User copies an album folder** into a watched library path.

3. **The filesystem watcher detects the changes.**

4. **After a two-second quiet period**, the watcher triggers an incremental rescan.

5. **The rescan indexes the new files.**
   - New tracks appear in the library.
   - Metadata is extracted.
   - Cover art is resolved.

6. **The library cache is updated.**

**Branch — Watch unavailable**: If the path cannot be watched (network mount, permission issue, inotify limit), the toggle shows a warning state with an explanation. The user can fall back to a manual "Scan" click.

---

## Flow 6: Tray Playback

**Goal**: A user on macOS or Windows minimizes the window and controls playback from the tray.

1. **User closes the main window** (clicks the close button).
   - The window is hidden.
   - The app continues running in the system tray.
   - Playback continues.

2. **User hovers over the tray icon.**
   - The tooltip shows "Artist - Title".

3. **User right-clicks the tray icon.**
   - A context menu appears with Play/Pause, Next Track, Previous Track, Show Window, and Quit.

4. **User selects "Next Track."**
   - Playback advances to the next track.

5. **User left-clicks the tray icon.**
   - The main window is restored.

**Branch — Quit from tray**: User right-clicks → Quit. Playback stops. The app exits. The tray icon is removed.

**Branch — Linux**: No tray icon exists. Closing the window closes the app.

---

## Flow 7: Adding a Second Library Path

**Goal**: A user with one library adds another location (e.g. an external drive).

1. **User opens settings.**

2. **User clicks "Add Library" and selects the new path.**
   - The new path is added to the list.

3. **User clicks "Scan" on the new path.**
   - Files from the new path are indexed.
   - The library now contains tracks from both paths, unified.

4. **User browses the combined library.**

---

## Flow 8: Unavailable Library Path

**Goal**: A user ejects an external drive and manages the stale path.

1. **User ejects an external drive** that is a registered library path.

2. **The path shows as unavailable** in settings (rather than being silently dropped).

3. **Scans skip the unavailable path.**

4. **User decides to remove the path.**
   - The user clicks the delete control on the path's row.
   - Index entries for that path are removed.
   - The files on the drive are untouched.

5. **User reconnects the drive later.**
   - The user re-adds the path.
   - A scan re-indexes the files.

---

## Flow 9: Cover Art Fallback Chain

**Goal**: A track with no embedded cover art still displays appropriate artwork.

1. **Track is displayed in the UI.**

2. **riff checks for embedded artwork** in the file's metadata (ID3v2 APIC, FLAC/Vorbis picture, M4A covr).

3. **No embedded artwork found.** riff checks the track's directory for a filesystem cover image, matching case-insensitively in priority order: cover → folder → album → front (JPEG or PNG).

4. **Filesystem cover found.** The image is displayed.

5. **No filesystem cover found either.** A placeholder is shown.

6. **The cover image** is decoded on a background thread and cached in the LRU texture cache.

---

## Flow 10: Audio Device Disconnect

**Goal**: Bluetooth headphones disconnect mid-playback; playback resumes when reconnected.

1. **User is playing music** through Bluetooth headphones.

2. **Headphones disconnect.**
   - Playback pauses gracefully.
   - No crash, no error dialog.
   - The UI reflects the paused state.

3. **User reconnects the headphones.**

4. **User presses play.**
   - Playback resumes from the paused position.

5. **No restart of the app is needed.**
