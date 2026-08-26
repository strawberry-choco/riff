# riff

riff is an offline-first desktop music player. This context covers the language used for its indexed music collection, user-created lists, preferences, and authoritative persistent state.

## Language

**View**:
One of the primary content areas selectable from the shared chrome — Library, Folders, or Settings; exactly one is visible at a time.
_Avoid_: page, stage, screen, tab

**Now Playing**:
A presentation mode that temporarily replaces the active View with the current Track's details; closing it always returns to the Library View.
_Avoid_: now-playing page, player view

**Application Store**:
The single authoritative persistent state of the application: the Library, Playlists, and Settings.
_Avoid_: database file, cache store

**Clear Library**:
The user action that deletes Library collection data from the Application Store while preserving Playlists and Settings.
_Avoid_: Clear Library Cache

**Library**:
The authoritative indexed collection of known Tracks, Artists, and Albums discovered from Library Paths.
_Avoid_: Library Cache

**Library Cache**:
Retired term for the former non-authoritative JSON copy of the Library; do not use it for the current Store.
_Avoid_ using it for the Application Store or Library

**Legacy JSON Files**:
Old `library_cache.json` and `playlists.json` artifacts left by the former persistence design; they are not authoritative.
_Avoid_: backup, export

**Track**:
One audio file known to the Library, including its Metadata and play-history facts.
_Avoid_: song, file

**TrackId**:
A Track's stable identity string, derived from its full file path; renaming or moving a file yields a new TrackId.
_Avoid_: track number, row ID

**Artist**:
A grouping of Albums credited to one Album Artist name.
_Avoid_: performer, contributor

**Album**:
A grouping of Tracks identified by Album Artist and album title.
_Avoid_: record, folder

**Album Artist**:
The primary artist credited for an Album, distinct from track-specific artists such as those on compilations.
_Avoid_: artist

**Playlist**:
A named, ordered list of Track references created and edited by the user.
_Avoid_: queue

**Smart Playlist**:
A read-only Playlist generated on demand from current Library and play-history facts; it is never stored as an entity.
_Avoid_: saved search

**Playback Queue**:
The transient ordered set of Tracks scheduled for playback; it is not part of persisted state.
_Avoid_: Playlist

**Queue Fill**:
When playback starts on a Track while the Playback Queue is empty, the whole Library becomes the queue with that Track current.
_Avoid_: auto-fill, auto-populate

**Settings**:
Persisted user preferences that are not music-collection data, such as Library Paths, volume, Watch States, and display toggles.
_Avoid_: config, options

**Watch State**:
The persisted watcher choice for a Library Path: Disabled, Enabled, or Warning carrying a diagnostic message.
_Avoid_: watcher status

**Readiness**:
The per-Library-Path health shown as a status dot in Settings: whether the path is present on disk and indexed into the Library; independent of its Watch State.
_Avoid_: watch status, Ready state

**Session Projection**:
A bounded in-memory view of Application Store query results used while rendering the UI; it is never authoritative.
_Avoid_: cache, AppState snapshot

**SessionViews**:
The single read interface over all Session Projections; UI code asks it for ready-to-render data and never touches the generation counter, loader wiring, staleness handling, or store-error fallbacks itself.
_Avoid_: projection manager, view cache

**Audio Engine**:
The module that turns Playback Commands into decoded audio and Playback Updates, owning decode scheduling, output startup, and gapless handoff; it decides nothing about queue order beyond filling an empty Playback Queue.
_Avoid_: playback thread, sound server

**PlaybackCoordinator**:
The module that applies Playback Updates to session state and owns playback continuation: committing play history before advancing, repeat-one re-play, auto-advance, and stopping when nothing follows. It is the decider of queue continuation; the Audio Engine only reports what happened.
_Avoid_: update processor, track-end handler

**Tag Edit**:
The user action of editing a Track's Metadata through the edit dialog; saving commits the file tags and the Store facts as one durable change, and a failure leaves the dialog open with the reason.
_Avoid_: metadata editor, tag writer

