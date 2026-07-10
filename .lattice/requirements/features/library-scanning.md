---
feature: Library Scanning
epic: Music Library
status: implemented
priority: P0
depends_on: []
personas: ["Music Listener"]
source_docs: []
implementation_notes: |
  Implemented in infra/scanner.rs (AudioFileScanner using walkdir + extension
  filtering) and app/library_manager.rs (scan_and_add_tracks). Runs on a
  background thread with progress/cancel support via channels.
---

# Library Scanning

## Problem Statement

Music listeners organize their audio files in directories on their local filesystem. The player must discover all supported audio files within user-specified directories, efficiently scan large collections (potentially tens of thousands of files), and build an in-memory index for fast access.

## User / Personas

- **Music Listener**: A person with a large local music collection spread across multiple directories who wants the player to automatically find all playable files without manual enumeration.

## Scope

**In scope:**
- Add directories to the library watch list
- Recursively scan directories for audio files
- Filter files by supported extensions: `.mp3`, `.m4a`, `.aac`, `.opus`, `.ogg`, `.flac`, `.wav`
- Build an in-memory index mapping file paths to track IDs
- Detect new files, removed files, and moved files on rescan
- Case-insensitive extension matching
- Report scan progress (file count, current directory)

**Out of scope:**
- Real-time filesystem watching (inotify/FSEvents/KQueue) — periodic manual rescan only in MVP
- Network or cloud storage scanning
- Scanning inside archives (zip, rar)
- Duplicate detection
- File integrity verification beyond format detection

## Boundary Conditions

- Scanning a library of 50,000 files should complete in under 30 seconds on a modern SSD
- Very deep directory nesting (>100 levels) should not cause stack overflow
- Symbolic links should be followed once (not recursively) or ignored to prevent cycles
- Permission-denied directories should be skipped with a warning, not fail the entire scan
- A directory with no audio files should result in an empty library section, not an error

## Assumptions

- Audio files have standard extensions indicating their format
- Filesystem access is local and reasonably fast (SSD or HDD, not network)
- The user will trigger rescans manually when they add new music
- File paths are UTF-8 or at least representable as Rust OsString/PathBuf

## Scenarios

### Scenario 1: Initial library scan
A user adds a music directory for the first time.

**Acceptance Criteria:**
- Given a directory containing 1,000 mixed files (audio, images, text), when the user adds it to the library, then the player discovers all supported audio files and reports the count
- Given a directory tree with 3 levels of subdirectories, when scanned, then all audio files in all subdirectories are discovered
- Given a scan is in progress, when the user looks at the UI, then a progress indicator shows the current directory and files-found count

### Scenario 2: Rescan detects changes
A user adds and removes files, then rescans.

**Acceptance Criteria:**
- Given a library was previously scanned, when the user adds new audio files and rescans, then the new files appear in the library
- Given a library was previously scanned, when the user deletes audio files and rescans, then the removed files no longer appear in the library
- Given a file was moved within the library tree, when rescanned, then the file appears at the new path and not the old path

### Scenario 3: Handle edge cases
The library contains unusual files or directories.

**Acceptance Criteria:**
- Given a directory contains a symlink to another music directory, when scanned, then the symlink is followed once and its contents are included
- Given a directory requires elevated permissions, when scanned, then it is skipped with a warning and the rest of the scan continues
- Given a file has an uppercase extension (`.MP3`, `.FlAc`), when scanned, then it is recognized as a valid audio file

## Implementation Notes

1. **Directory walker**: Use `walkdir` or `ignore` crate for efficient recursive directory traversal with symlink handling
2. **Extension filter**: Maintain a HashSet of lowercase supported extensions for O(1) lookup
3. **Index structure**: Store a `Vec<Track>` where each Track has a unique ID and file path. Use a HashMap from path to ID for quick lookups during rescan.
4. **Progress reporting**: Yield progress events (current directory, files found) that the UI can display. Consider scanning in a background thread.
5. **Rescan diff**: On rescan, compare discovered paths against existing index. Add new, remove missing, update moved.

## Open Questions

- [ ] Should we follow symlinks or ignore them to prevent infinite loops?
- [ ] How often should we automatically rescan, if at all? (manual only in MVP?)
- [ ] Should hidden directories (starting with `.`) be skipped by default?

## Links
- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
