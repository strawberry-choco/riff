# Error States

This document consolidates every error condition that a user can encounter in riff, with what they see, what the system does, and how they recover. It replaces ad-hoc error handling scattered across features.

---

## 1. Unplayable File

**Condition**: The current file cannot be decoded — corrupt header, truncated data, or unsupported sub-codec (e.g. ALAC in an M4A container).

**User-visible**: An error message is displayed in the UI: "Could not play [filename]: [reason]."

**System behavior**: Playback stops. The error is logged at ERROR level with the filename and decoder error. The UI is not crashed.

**Recovery**: The user can select a different track from the library or queue. The unplayable file remains in the library (it was indexed correctly; only playback failed).

**Status**: Implemented.

---

## 2. Missing or Corrupt Library Cache

**Condition**: The library_cache.json file is missing (deleted by user, not yet written) or contains malformed JSON.

**User-visible**: No error message. The library is empty on launch.

**System behavior**: riff starts with an empty library. The library cache is not partially loaded.

**Recovery**: The user adds library paths and triggers a scan. The cache is rebuilt from scratch.

**Status**: Implemented.

---

## 3. Failed Cache Write

**Condition**: The cache file cannot be written — disk full, permissions error, or filesystem error.

**User-visible**: No error shown to the user. The UI is not affected.

**System behavior**: The failure is logged at WARN level. The current library state in memory continues to function. The cache is simply not updated.

**Recovery**: Fix the underlying issue (free disk space, fix permissions). The next successful scan or path removal will write the cache again.

**Status**: Implemented.

---

## 4. Unavailable Library Path

**Condition**: A registered library path points to an ejected external drive, an unmounted network share, or a path that no longer exists.

**User-visible**: The path is shown in settings with an "Unavailable" status.

**System behavior**: The path is skipped during scans. Index entries for the path are preserved (not deleted automatically).

**Recovery**: Reconnect the drive or remount the share. Click "Scan" to re-index. If the path is permanently gone, remove it from settings to clean up the index.

**Status**: Implemented.

---

## 5. Folder Watch Unavailable

**Condition**: The OS cannot monitor a library path for changes — the path is a network mount, permissions are insufficient, or the Linux inotify limit is reached.

**User-visible**: The Watch toggle shows a warning state (distinct from On/Off), with an explanatory tooltip or adjacent text.

**System behavior**: Watching is not active for that path. Changes are not auto-detected.

**Recovery**: The user can fall back to a manual "Scan" click. On Linux, reducing the number of watched paths may free inotify slots.

**Status**: Implemented.

---

## 6. No Audio Device Available

**Condition**: No audio output device is available — headphones unplugged, audio driver crashed, or the audio subsystem is not responding at launch.

**User-visible**: Playback cannot start. A status indicator or error message informs the user.

**System behavior**: Audio output initialization fails. Playback commands that require output (play, seek) are no-ops or return errors.

**Recovery**: Connect an audio device or restart the audio driver. Once a device is available, playback can start normally.

**Status**: Implemented (graceful handling).

---

## 7. Audio Device Disconnects Mid-Playback

**Condition**: An audio output device disconnects while playback is active — Bluetooth headphones power off, wired headphones unplugged, audio output disconnected.

**User-visible**: Playback pauses. The UI reflects the paused state. A status indicator or tooltip may inform the user.

**System behavior**: The cpal output stream detects the device loss. Playback is paused (not stopped). The current position is preserved.

**Recovery**: Reconnect the device. Press play to resume from the paused position. No app restart needed.

**Status**: Implemented.

---

## 8. Cover Art Decode Failure

**Condition**: A cover image cannot be decoded — corrupt JPEG/PNG, unsupported image format, or the image is too large to decode within available memory.

**User-visible**: A placeholder is shown in place of the cover art. No error message is displayed in the UI.

**System behavior**: The failure is logged at WARN level with the image path. The placeholder is rendered. The LRU cache is not updated with the failed entry.

**Recovery**: Fix or replace the cover image file. The next scan will re-resolve cover art. The app does not need to be restarted.

**Status**: Implemented.

---

## 9. Unsupported File Format

**Condition**: A file with an unsupported format (WMA, MIDI, DSD, SACD, or DRM-protected files) is present in a library folder.

**User-visible**: The file is not indexed in the library. It does not appear in search or browsing results.

**System behavior**: The scanner filters by supported extensions. Files with unsupported extensions are silently skipped. No error is raised.

**Recovery**: Convert the file to a supported format (MP3, AAC, Opus, FLAC, OGG Vorbis, WAV) and add it to a library folder.

**Status**: Implemented.

---

## 10. Scan Interrupted

**Condition**: A library path becomes unavailable during an active scan — the drive is ejected, the network share disconnects, or the path is deleted.

**User-visible**: The scan for that path stops. Other paths continue scanning. A status indicator shows the interrupted path.

**System behavior**: The scan thread detects the I/O error and stops processing that path. Already-indexed entries are preserved. The cache is not rewritten until a scan completes successfully.

**Recovery**: Reconnect the drive and click "Scan" on the path. Or remove the path from settings.

**Status**: Implemented (graceful handling).

---

## 11. Empty Library State

**Condition**: No tracks are indexed — first launch, all paths removed, or cache was deleted.

**User-visible**: The library explorer shows an empty state (e.g. "No music found — add a library folder"). The control bar shows no track.

**System behavior**: No crash or error. The empty state is a valid, expected condition.

**Recovery**: Add a library path and scan.

**Status**: Implemented.

---

## 12. Failed Metadata Read

**Condition**: A file is readable as audio but its metadata cannot be parsed — corrupt tags, malformed ID3 frames, or other tag-level errors.

**User-visible**: The track is indexed and playable. Metadata fields that failed to read are simply not displayed (no "Unknown" placeholder).

**System behavior**: The metadata reader logs a WARN for the file. The track is added to the index with whatever metadata was successfully read. Missing fields are handled at the display level.

**Recovery**: Fix the tags with an external tagger and re-scan the path.

**Status**: Implemented.

---

## 13. Native Dialog Cancelled

**Condition**: The user clicks Cancel on the native folder picker (macOS/Windows) or closes the text input without confirming (Linux).

**User-visible**: No path is added. The settings page remains unchanged.

**System behavior**: The add-operation is a no-op. No error is raised.

**Recovery**: Click "Add Library" again when ready.

**Status**: Implemented.

---

## 14. Mutex / Poison Lock Recovery

**Condition**: A thread panics while holding a mutex, poisoning the lock.

**User-visible**: No user-visible impact.

**System behavior**: The `MutexExt::lock_or_recover` helper detects the poisoned lock, recovers it, and continues. The panic on one thread does not cascade into every other thread that shares the `AppState` mutex.

**Recovery**: Automatic — no user action required.

**Status**: Implemented.
