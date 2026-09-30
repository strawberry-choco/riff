# riff

riff is an offline-first desktop music player. This context covers the language used for its indexed music collection, user-created lists, preferences, and authoritative persistent state.

## Language

**View**:
One of the primary content areas selectable from the shared chrome — Library, Folders, or Settings; exactly one is visible at a time.
_Avoid_: page, stage, screen, tab

**Section**:
One of the Library browse sections selectable from the sidebar; exactly one is active at a time, and each keeps its own Scroll Memory while the app runs.
_Avoid_: view, tab, page

**Artists Column**:
A list column of Artists — the Artists Section's root list, or a Genre's Artists.
_Avoid_: Drill Column, artist list, subview

**Albums Column**:
A list column of Albums — the Albums Section's root list, an artist's Albums, or a Genre artist's Albums.
_Avoid_: Drill Column, album list, subview

**Genres Column**:
The Genres Section's root list of Genres. Selecting one reveals an Artists Column.
_Avoid_: Drill Column, genre list, subview

**Tracks Column**:
The list column of an Album's Tracks, revealed by selecting that Album. The All Tracks Section shows its Tracks as a flat full-width list, not a Tracks Column.
_Avoid_: Drill Column, track list, All Tracks

**Scroll Memory**:
The in-memory record of each Section's list scroll position, kept only while the app runs; restarting clears it. A Section's root list restores its position when the Section is reselected; any deeper column resets on any selection or content change (search, sort, rescan). Which list a record belongs to is the owning Column's own fact, never something a gesture supplies: an action that moves the selection resets **the Column it happened in**, so a Column cannot leave its own scroll stale and no Column can reset a neighbour's.
_Avoid_: scroll persistence, saved position

**Column Identity**:
What one entity-Column is, declared once where it renders: its **Section**, the **depth** its rows select at, and the **Scroll Memory slot** it owns. It is stated as data and is then never supplied again — a click, a right-click and a sort flip all read the one value, and the scroll bookkeeping reads the same one, so a Column's list and its reset behaviour cannot come to disagree. There are exactly two shapes, a Section root and a deeper Column, and what differs between two Columns of one shape is a value rather than a block of code.
_Avoid_: column config, per-action section, drill level argument

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

**Favourite**:
A Track's user-set starred flag, independent of its Metadata and of whether its file still exists.
_Avoid_: like, star, rating, love

**TrackId**:
A Track's stable identity string, derived from its full file path; renaming or moving a file yields a new TrackId.
_Avoid_: track number, row ID

**Cover**:
The artwork a Track or a directory displays — embedded in a Track's own file, or a cover file in a Track's parent folder; a directory's is a cover file inside it. Every Track has exactly one Cover, and one that cannot be found is artless.
_Avoid_: thumbnail, artwork, art, image, album art

**Thumbnail**:
A Cover reduced to a fixed pixel box for display in the UI. Derived, display-only, and never a source for a larger size.
_Avoid_: cover, artwork, icon, texture

**Cover Cache**:
What the application knows about a Cover — which are wanted, at which size, which are in flight, and which have arrived. It holds no picture: a Cover becomes a Thumbnail in the View, and the View owns that. The two are separate facts and neither answers for the other.
_Avoid_: texture cache, cover LRU, image cache, Thumbnail cache

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

**Add to Queue**:
The user action that puts a Track — or every Track of an Album, Artist, or Genre — into the Playback Queue. One action, one name, whether it is one Track or many.
_Avoid_: Append to Queue, append, enqueue

**Queue Fill**:
When playback starts on a Track while the Playback Queue is empty, the whole Library becomes the queue with that Track current.
_Avoid_: auto-fill, auto-populate

**Settings**:
Persisted user preferences that are not music-collection data, such as Library Paths, volume, Watch States, and display toggles.
_Avoid_: config, options

**Preferences**:
The module that owns the Settings round-trip — hydrating Settings into the sessions on launch, committing session changes back to the Application Store; a preference change is durable by construction, never by call-site discipline. `Preferences` only hydrates and diff-commits the scalars; every structural write happens inside a `Library Path` module operation.
_Avoid_: settings sync, persist helper

**Library Path**:
A root folder the Library is discovered from, registered by the user. It is one fact-set, not a string: the path, its Readiness, its Watch State, its running filesystem watcher, and its rows in the Application Store. One operation owns each edge: retiring a Library Path retires all five, and restoring one takes them back — the restored Watch State is acted on rather than copied in, so Enabled starts its watcher, Disabled starts nothing, and a Warning is retried so the verdict is the current one. The App Runtime restores them at launch, so a Library Path the user enabled watching on is watched before any frame is drawn.
_Avoid_: root, directory, watched folder

**Watch State**:
The persisted watcher choice for a Library Path: Disabled, Enabled, or Warning carrying a diagnostic message. A Warning is a recorded failed attempt, retried at every launch, so it never outlives the condition that caused it.
_Avoid_: watcher status

**Readiness**:
The per-Library-Path health shown as a status dot in Settings: whether the path is present on disk and indexed into the Library; independent of its Watch State.
_Avoid_: watch status, Ready state

**Session Projection**:
A bounded in-memory view of Application Store query results used while rendering the UI; it is never authoritative. A listing is read as two of its own — a row by its index, and the listing's total — and the projection owns what makes those two cheap: the window a row is fetched in, the refetch, and the reuse of rows it already holds.
_Avoid_: cache, AppState snapshot

**SessionViews**:
The single read interface over all Session Projections; UI code asks it for ready-to-render data and never touches the generation counter, loader wiring, staleness handling, or store-error fallbacks itself. For a paged listing it answers two questions in two reads — row *i* of the listing, and how many rows it has — so no caller holds a window's rows or derives a position inside one.
_Avoid_: projection manager, view cache

**Audio Engine**:
The module that turns Playback Commands into decoded audio and Playback Updates, owning decode scheduling, output startup, and gapless handoff. It decides nothing about queue order: the Queue Fill's content and both skips are answered by **Continuation**, and the engine only performs the load. What the engine does own is the *when* — that an empty Playback Queue triggers a Queue Fill, and that idleness (nothing is currently playing) starts playback exactly once — and it is one operation every Playback Command that can start or grow the Playback Queue goes through.
_Avoid_: playback thread, sound server

**Transport**:
The module the UI uses to command playback: user intents (play, seek, volume, queue changes) enter here and map onto Playback Commands; it owns seek clamping and effective-volume math so call sites never re-derive them.
_Avoid_: command sender, playback API

**PlaybackCoordinator**:
The module that applies Playback Updates to session state: it commits play history for the track that just ended before asking **Continuation** what follows, and stops playback — clearing the current index with it — when nothing does. The answer itself is not its own; the Audio Engine asks the same answer for a listener's skip.
_Avoid_: update processor, track-end handler

**Continuation**:
The answer to what plays next, given a playback event or a manual skip — which Track, or that nothing follows. It is the same question whether the trigger was a Track ending or a listener pressing Next, and one module in the Playback capability answers it, moving the Playback Queue to that answer. Which Tracks a **Queue Fill** puts in the queue and which is current, and what is current when a caller has already chosen the Track, are the same question asked of that module too. It answers from what it is handed: no store, no thread, no channel and no IO reach it, and *when* a fill or an idle auto-play happens is the **Audio Engine**'s decision, not this module's.
_Avoid_: auto-advance, next-track logic, skip handling

**Library Scan**:
The operation that discovers audio files under a Library Path and commits them into the Library in durable batches; progress and completion are reported to the session, and an interrupted scan keeps committed batches.
_Avoid_: scanner thread, directory walker

**Detail Panel**:
The rightmost selection readout showing the currently selected entity or Track — its art, title, secondary line, tag rows, and detail rows. It follows the live selection: single-clicking a Track anywhere shows that Track; selecting an Album, Artist, or Genre in the browser shows the entity readout. A readout displays rather than acts: it carries no action buttons and no menu of its own, because an entity's actions belong to the entity. Its one way inward is the tag rows, which open the Inline Tag Editor.
_Avoid_: inspector, selection panel, readout column

**Inline Tag Editor**:
The tag editing surface that lives inside the Detail Panel, replacing the retired Edit Tags modal. It renders one row per editable Metadata field; editing happens in place and saving commits through the Tag Edit service. It has exactly **one door**: the **Track-menu host** opens it, from whichever surface the gesture came from, and the Detail Panel's tag rows open the same draft through the same controller. A Track row's "Edit Tags" item is therefore not a second way in — it is that one opening, reached from a row.
_Avoid_: edit dialog, tag modal, second tag editor

**Track-menu host**:
The one place a right-click on a Track becomes an effect, and the answer to only two questions: *a right-click happened on this Track*, and *this item was chosen*. It owns the handles a Track's actions need — the Playback Queue's Transport, the Application Store's Playlist and Library sections, the Inline Tag Editor, and the selection slot — so every Track-row surface answers a right-click identically by naming a Track and its row's context, and nothing else. There is one per app: the handle set is declared once, so a fifth Track-row surface cannot wire a different one. A Track menu is not a second way to dispatch: a surface paints the menu and hands the host what it reported.
_Avoid_: effects bag, track menu effects, per-row menu context

**Tag Aggregation**:
In an Album readout, the way one tag row summarizes that field across every Track of the Album: all Tracks share the value → the value itself; the values differ → the orange `(different)` state; no Track carries the value → the grey `(none)` state. Tag rows on a single-Track readout show that Track's value directly.
_Avoid_: merged value, average value

**Batch Tag Edit**:
Saving an edited tag row on an Album readout applies the edit to every Track of the Album. Each Track is its own durable change through the Tag Edit service, so a batch may partially fail; the failure is reported, never silent.
_Avoid_: multi-edit, bulk write

**Tag Edit**:
The user action of editing a Track's Metadata through the Inline Tag Editor; saving commits the file tags and the Store facts as one durable change, and a failure is reported inline with the reason.
_Avoid_: metadata editor, tag writer

**Frame**:
One pass of the read-decide-draw cycle: the backend's events are drained, the services are polled, what the user did is applied, and the View is handed what to draw. Its order is part of what it is — a fact filled in after the compose is a frame late — so the Frame is one module with one interface, not a sequence of steps threaded through the render path.
_Avoid_: tick, update loop, render pass, frame callback

**App Runtime**:
The composed application the Composition Root spawns: shared sessions, Application Store ports, service front ends, and worker threads wired in one place. It owns the worker threads' whole lifecycle — they start with it and shut down through it, never outliving it.
_Avoid_: global state, handle bag

