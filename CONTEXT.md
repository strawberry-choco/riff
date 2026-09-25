# riff

riff is an offline-first desktop music player. This context covers the language used for its indexed music collection, user-created lists, preferences, and authoritative persistent state.

## Language

**View**:
One of the primary content areas selectable from the shared chrome — Library, Folders, or Settings; exactly one is visible at a time.
_Avoid_: page, stage, screen, tab

**Section**:
One of the four Library browse surfaces selectable from the sidebar — All Tracks, Artists, Albums, or Genres; exactly one is active at a time, and each keeps its own Scroll Memory while the app runs.
_Avoid_: view, tab, page

**Drill Column**:
A deeper list column revealed by selecting an entity in the browser path — an artist's Albums, a Genre's Artists, or a Genre artist's Albums. Selecting a different entity resets a Drill Column's scroll to the top.
_Avoid_: subview, drill-down page

**Scroll Memory**:
The in-memory record of each Section's list scroll position, kept only while the app runs; restarting clears it. A Section's root list restores its position when the Section is reselected; a Drill Column resets on any selection or content change (search, sort, rescan).
_Avoid_: scroll persistence, saved position

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

**Preferences**:
The module that owns the Settings round-trip — hydrating Settings into the sessions on launch, committing session changes back to the Application Store; a preference change is durable by construction, never by call-site discipline. Structural preferences were the exception: Library Paths and Watch States were committed by call-site discipline at three separate sites, and the Library Path module closes it — every structural write now happens inside one of that module's operations, and `Preferences` only hydrates and diff-commits the scalars.
_Avoid_: settings sync, persist helper

**Library Path**:
A root folder the Library is discovered from, registered by the user. It is one fact-set, not a string: the path, its Readiness, its Watch State, its running filesystem watcher, and its rows in the Application Store. Retiring a Library Path retires all five.
_Avoid_: root, directory, watched folder

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

**Listing Page**:
A Section's or Drill Column's total and its visible window, read from the Application Store as one fact under one connection acquisition, so no committed write can interleave the two halves. Which generation a listing was read at is the Session Projection's concern, not the page's.
_Avoid_: page, query result, count row

**Audio Engine**:
The module that turns Playback Commands into decoded audio and Playback Updates, owning decode scheduling, output startup, and gapless handoff. It decides nothing about queue order: the Queue Fill and both skips are answered by **Continuation**, and the engine only performs the load.
_Avoid_: playback thread, sound server

**Transport**:
The module the UI uses to command playback: user intents (play, seek, volume, queue changes) enter here and map onto Playback Commands; it owns seek clamping and effective-volume math so call sites never re-derive them.
_Avoid_: command sender, playback API

**PlaybackCoordinator**:
The module that applies Playback Updates to session state: it commits play history for the track that just ended before asking **Continuation** what follows, and stops playback — clearing the current index with it — when nothing does. The answer itself is not its own; the Audio Engine asks the same answer for a listener's skip.
_Avoid_: update processor, track-end handler

**Continuation**:
The answer to what plays next, given a playback event or a manual skip — which Track, or that nothing follows. It is the same question whether the trigger was a Track ending or a listener pressing Next, and one module in the Playback capability answers it, moving the Playback Queue to that answer. Which Tracks a **Queue Fill** puts in the queue and which is current, and what is current when a caller has already chosen the Track, are the same question asked of that module too.
_Avoid_: auto-advance, next-track logic, skip handling

**Library Scan**:
The operation that discovers audio files under a Library Path and commits them into the Library in durable batches; progress and completion are reported to the session, and an interrupted scan keeps committed batches.
_Avoid_: scanner thread, directory walker

**Detail Panel**:
The rightmost selection readout showing the currently selected entity or Track — its art, title, secondary line, tag rows, and detail rows. It follows the live selection: single-clicking a Track anywhere shows that Track; selecting an Album, Artist, or Genre in the browser shows the entity readout.
_Avoid_: inspector, selection panel, readout column

**Inline Tag Editor**:
The tag editing surface that lives inside the Detail Panel, replacing the retired Edit Tags modal. It renders one row per editable Metadata field; editing happens in place and saving commits through the Tag Edit service.
_Avoid_: edit dialog, tag modal

**Tag Aggregation**:
In an Album readout, the way one tag row summarizes that field across every Track of the Album: all Tracks share the value → the value itself; the values differ → the orange `(different)` state; no Track carries the value → the grey `(none)` state. Tag rows on a single-Track readout show that Track's value directly.
_Avoid_: merged value, average value

**Batch Tag Edit**:
Saving an edited tag row on an Album readout applies the edit to every Track of the Album. Each Track is its own durable change through the Tag Edit service, so a batch may partially fail; the failure is reported, never silent.
_Avoid_: multi-edit, bulk write

**Tag Edit**:
The user action of editing a Track's Metadata through the Inline Tag Editor; saving commits the file tags and the Store facts as one durable change, and a failure is reported inline with the reason.
_Avoid_: metadata editor, tag writer

**App Runtime**:
The composed application the Composition Root spawns: shared sessions, Application Store ports, service front ends, and worker threads wired in one place. It owns the worker threads' whole lifecycle — they start with it and shut down through it, never outliving it.
_Avoid_: global state, handle bag

