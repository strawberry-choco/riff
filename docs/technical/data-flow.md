# Data Flow

This document walks through the three primary runtime flows in riff — playing a track, scanning a library, and resolving cover art — as step-by-step sequences. Each flow crosses several threads and crates; the filenames referenced are the real ones in the workspace. For the threads involved and the constraints that govern them, see [./threading-model.md](./threading-model.md).

## Flow 1: Play a Track

This is the central flow. It begins with a click in the UI and ends with samples reaching the audio device, with the Playback Coordinator auto-advancing when the track finishes.

```
User clicks "Play" in the UI (riff-gui/src/ui/)
  -> The UI's Box<dyn Transport> dispatches the intent: the ChannelTransport
     sends PlaybackCommand::Play(track_id) on the command channel (a dispatch is
     not an event; the event inbox carries only the store stream and notices)
  -> Audio engine thread (AudioEngine::run in riff-playback/src/infra/audio_engine.rs)
     receives the command
   -> Engine locks PlaybackSession and resolves the Track through the
        PlaybackLibrary port (the Application Store is the sole authority for
        track metadata; a store miss drops the play request)
        (if the queue is empty, the engine's one start rule fills the queue:
        the Library's Track ids in canonical path order become the queue,
        Continuation makes the requested track current, and shuffle resets)
  -> SymphoniaDecoder (minted by the injected DecoderFactory) is initialized on
       the track path and reports the stream's format (sample_rate, channels)
  -> output.start(format) starts the cpal stream — under the hood the
       initialize/start pair owns the device-default-rate fallback (common on
       Windows WASAPI shared mode at 48 kHz)
  -> Engine sends PlaybackUpdate::StateChanged(Playing) and PlaybackUpdate::TrackChanged(track_id)
  -> Decode loop begins (push model, on the engine thread):
       a. decoder.next_frames decodes a chunk of up to 4096 interleaved samples
       b. ReplayGain factor scaling is applied in place when enabled
       c. output.write(samples) pushes them into the lock-free ring buffer
          (a blocking write when full — this is the backpressure point)
       d. PlaybackUpdate::PositionChanged is sent, computed sample-accurately
       e. Command poll: try_recv for Pause / Stop / Seek / volume changes (10 ms cadence)
  -> cpal callback thread (OS audio thread): pops samples from the lock-free ring
     buffer and writes them to the device; never blocks, never locks
  -> On EOF (decoder's next_frames returns None):
       send PlaybackUpdate::TrackEnded, then drain the buffer through the callback
       before stopping; a Stop command during drain discards the remaining samples
  -> Playback Coordinator thread (riff-playback/src/app/playback_coordinator.rs)
     receives each PlaybackUpdate and mutates PlaybackSession
  -> On TrackEnded, the coordinator commits the play history through the
     LibraryMutationStore port, then decides the continuation: repeat-one replays
     the current track, a successor is played via PlaybackCommand::Play(next_id),
     and otherwise the state is set to Stopped
  -> UI reads the session structs each frame for the progress bar, play state,
     and any notice message
```

The `Resume` path reuses the same machinery: the engine remembers the current track id and the paused position, and on resume it re-issues `Play(track_id)` and seeks to the stored position before continuing.

## Flow 2: Scan a Library

Scanning is triggered from the settings view and runs entirely on the serial scan worker thread. The UI stays responsive by polling the outcome stream.

```
User clicks "Scan" (or "Scan All") in the settings view (riff-gui/src/ui/settings.rs)
  -> UI marks the path Scanning in LibrarySession and requests the scan through
     the Scans seam (ScanService::request in riff-library)
  -> Library scan worker thread (composition.rs) picks the request up
  -> AudioFileScanner (riff-infra/src/filesystem/scanner.rs) walks the directory
     tree with walkdir and returns all audio file paths
     (honoring the AtomicBool cancel flag shared with the service)
   -> The store freshness filter keeps paths whose metadata is already current
        (one indexed lookup per path through the LibraryQueryStore); if the
        check errors, the path is scanned anyway (fail-open)
        It is gated by the store's metadata version, read ONCE per scan through
        the same port and never per path: while the recorded version is behind
        the binary's `METADATA_VERSION`, every path in the batch is treated as
        fresh, so a tag added since the store was last written reaches a
        library indexed before the column existed
        -> For each chunk of ~10 new/changed paths:
             -> build_tracks(chunk, &LoftyMetadataReader) from riff-library reads
                tags, duration, cover source, and format; per-file failures are
                logged and skipped so a scan never aborts on one bad file
             -> The chunk commits as ONE immediate durable transaction through the
                LibraryMutationStore port (apply_scan_batch), preserving existing
                play history for known tracks
             -> On success the store bumps the session generation counters, so
                Session Projections refetch on the next frame
             -> ScanOutcome progress is reported to the UI's outcome stream
  -> When all chunks are processed, the scan records its summary and stamps
     the store's metadata version forward through the mutation port — only on
     this completed branch, so a cancelled or failed scan leaves the store
     behind and the next scan finishes the re-read — then the terminal
     ScanOutcome reports the total
  -> UI sets the path status to Scanned(total) and renders the refreshed views
```

A cancellation request sets the shared cancel flag, which the scanner checks between chunks; the service keeps every already-committed batch (an interrupted scan never rolls back committed work). If a commit fails, the outcome stream reports the failure and the path status is reset. Neither a cancellation nor a failure stamps the metadata version. The scan worker never touches the session structs: it reads the store through the query port and commits through the mutation port.

Watched folders feed the same seam from the other side: the filesystem-event forwarder hands `notify` events to the `WatcherManager` (riff-backend), which debounces batches and requests rescans through the shared `ScanService` — a manual scan and a watch-triggered rescan are the same flow.

Settings' **Clear Library** is the manual equivalent of a version-lag re-read: it drops the collection so the next scan indexes every file from scratch again. The version exists so a metadata-shape change does not require that — the user keeps their play history and favorites through the backfill.

## Flow 3: Resolve Cover Art

Cover resolution is asynchronous. The UI requests a cover, the cover service worker resolves and decodes it, and the UI uploads the result to an egui texture the next time it polls.

```
A track becomes current or is selected for display (riff-gui/src/ui/)
  -> the UI sends a cover request through the Covers seam (CoverService in
     riff-library), skipping tracks whose texture is already cached
  -> Cover service worker thread (composition.rs) receives the request
  -> It resolves the art through the CoverResolver (riff-library), which asks
     LoftyMetadataReader::read_cover_source(path) for the cover source
  -> Priority: embedded art first, filesystem fallback second
       -> If CoverSource::Embedded(bytes): ImageCoverLoader decodes the bytes to RGBA
       -> If CoverSource::None: CoverResolver scans the track's directory for a cover image
          (cover.jpg/png, folder.jpg/png, album.jpg/png, front.jpg/png — case-insensitive)
          and, if found, ImageCoverLoader decodes that file
       -> If CoverSource::Filesystem(path): ImageCoverLoader decodes that file directly
  -> The resolved Source's file (the image itself for a folder cover, the track's
     own file for embedded art) names a rung in the Thumbnail cache:
       -> hit: the stored JPEG/PNG is handed to the same decode-and-fit step, and
          the source file is never read or decoded — but an *embedded* cover has
          still cost the lofty tag parse that produced the bytes
       -> miss: decode as above, then encode and store the rung (a failed store is
          logged and ignored; it must never fail the resolve)
     Keyed on the Source rather than the track, so every track of one album shares
     the folder cover's ladder. Nothing is stored for an artless result.
  -> The decoded image is delivered to the UI over the response poll. The service
     keeps no decoded cover between requests; its only in-memory cache is the
     artless-verdict LRU (`negative`), capped at `COVER_CACHE_CAP`
  -> UI drains the response channel:
       -> builds an egui::ColorImage from the RGBA bytes and loads it as a texture
       -> inserts it into cover_textures and touches the LRU, then trims the map
         back inside both bounds: `COVER_CACHE_CAP` entries *or*
         `COVER_TEXTURE_BYTE_BUDGET` bytes, whichever is reached first. The shared
         placeholder tile is exempt from both.
  -> Subsequent frames fetch the texture from that in-memory LRU without asking at
     all; a frame that finds nothing there gets a placeholder plus a request, and
     a request that the disk already answers costs one stat and one open
```

If resolution fails at any point, the worker logs a warning and reports no image, so the UI simply displays no cover rather than surfacing an error; the service's negative cache prevents retry storms for artless tracks.

## Flow 4: Read a Paged Listing

Every paged listing in the app — the flat All Tracks list, the search results, the hit-album and hit-artist roots, the three browse roots, and the two genre drill-downs — is read the same way. All of it happens on the UI thread, inside one frame, and none of it is a worker round trip.

```
A surface needs to draw a listing (e.g. the Artists column)
  -> It asks the Session Views seam (riff-backend/src/app/views.rs) TWO things:
       1. "how many rows does this listing have"  -> artist_count(direction)
       2. "give me row i"                          -> artist_row(direction, i)
  -> Both land on ONE Session Projection, the windowed-list projection
     (riff-library/src/app/projection/windowed_list.rs), keyed by the listing's
     query signature (which list, plus the A-Z / Z-A direction)
     -> The projection retargets: a keystroke, a genre switch or a sort flip
        changes the signature, which drops the cached rows and total even at an
        unchanged generation
     -> Each read declares itself on GenerationCache::level (riff-persistence
        src/levels.rs), which observes the generation ONCE, serves the cached
        answer while it is current, and otherwise loads and commits at that same
        epoch. A failed load leaves the cache untouched, so the next frame retries
     -> A row read aligns i down to the window size, reads one window if the one
        in hand does not already cover i, and returns that single row as a shared
        handle. A count read asks the store's page read for the listing's total
        and ZERO rows
  -> The store read (riff-infra/src/store/sqlite.rs) takes its connection once
    and returns a Page<T>: the total and the window, read as one fact. The seam
    takes the half it asked for; a count read discards no rows because it
    requested none
  -> On a store error the seam logs a tracing::warn! with its context and answers
    the projection's LAST GOOD row or total (or the default, None / 0, if there
    is none). The UI never sees a Result, and a header never blanks over rows
    that are still on screen
  -> The surface draws the row it was handed. It never learns the window size,
     the window a row lives in, or when a refetch is due
```

What this costs, precisely: **one count read plus one window read per listing per generation**, not one per row. The projection holds up to eight windows per query signature, so scrolling within those eight is served entirely from memory, and a row already in a window in hand is a refcount bump rather than a store round trip. The window size itself is private to the projection; the store is asked for a window and never told what a surface is paging by.

The one guarantee this does *not* give: a count and a window are two reads, so a committed write can land between them. What it does give is that both are stamped at the same generation, so a frame's header and its rows are never reading across a committed write — and, on failure, that each keeps its last good answer rather than blanking.

## Flow 5: Answer a Column's Action

Everything below happens on the UI thread after a column has painted, inside the same frame.

```
A list column's widget reports what the listener did (ui/browser.rs's BrowserAction,
or ui/detail.rs's DetailReport for the Tracks column)
  -> The owning render site hands the column's ONE stated identity to a shared
     dispatch binding (RiffApp::drain_root_actions / ::drain_drill_actions in
     riff-gui/src/ui/app/browser_pane.rs)
     -> The identity is ui::column::ColumnIdentity: its Section, the depth its
        rows select at, and its Scroll Memory slot. It was built where the column
        renders and is read here; nothing re-supplies it per action
  -> For each action, in the order it was reported:
       -> THE GUARD: column.note_scroll_for(&action) answers the column's OWN slot
          for an action that moves the selection, and nothing for one that does
          not (a sort flip). The same value the scroll handshake above used, so a
          list and its reset cannot name different slots
            -> ScrollMemory::note_selection_in(slot) — the module decides whether
               that slot's selection is drill bookkeeping
       -> Then the action itself, from one of three arms:
          Select(key)   -> apply_entity_selection(key, column, library)
                           -> the key's MEANING is decided by the column's Section
                              and depth, and library.select_at(depth, …) truncates
                              anything deeper
          ContextMenu   -> apply_collection_menu(key, intents, column, effects)
                           -> selects the row FIRST (opening a menu is what moves
                              the selection), then resolves the row's Track batch
                              through the seam AT DISPATCH TIME and applies each
                              collection-menu intent
          ToggleSort    -> root: flip library.browser_sort_desc
                           drill: flip library.drill_sort_desc AND
                                  scroll_memory.reset_drill_scroll(column.drill_slot())
```

The two arms are the one real difference between the shapes, and it is a rule rather
than a per-Column value: a drill Column's A–Z control lives above its list, so the flip is
content identity the list's egui state cannot see and the slot is forced back to the top; a
root's is not. Two Columns of the same shape therefore differ only in the identity they
state.

The Tracks column is deliberately not one of these six: its rows select Tracks, not
entities, so it asks no guard, and its menu report is answered by the per-app **Track-menu
host** rather than by this dispatch — see Flow 1's sibling note in
[../technical/architecture.md](../technical/architecture.md).

## Key Design Decisions

### Push model

The audio engine pushes decoded samples into the output adapter's ring buffer; the cpal callback pulls from it. The engine never responds to callback requests directly. This one-directional flow keeps the real-time audio thread free of any blocking call and isolates it from the decoder's pacing.

### Continuation ownership

**Continuation** — `riff-playback/src/domain/continuation.rs` — decides what plays next. The Playback Coordinator commits play history for the finished track and then asks it, and the Audio Engine asks the same arbiter for a listener's Next or Previous, so repeat-one replay, the shuffle order, the stop-at-end rule and the Queue Fill's ordering live in one pure module rather than in the two callers. The callers keep only their own aftermath: the coordinator marks the session stopped and drops the current index, the engine tears down its decoder and output. The **when** is the Audio Engine's, not this module's: an empty queue triggers a Queue Fill, and idleness (nothing is currently playing) starts playback exactly once — both decided in one operation, `PlaybackStart::enqueue_and_start_if_idle` in `riff-playback/src/infra/ports.rs`, that every queue-mutating Playback Command goes through. Continuation is asked for the content, never for the trigger. Playback failures surface as typed notices instead of state writes.

### Cover priority

Embedded art first, filesystem fallback second — the resolution order and the names the fallback accepts are defined in the **Cover Art** entry of [../reference/glossary.md](../reference/glossary.md).

### One Column identity, two drains

A list column's actions are answered by a shared dispatch, not by the render site that painted
the column. What a Column is — its Section, its depth, its Scroll Memory slot — is stated once
where it renders, and the dispatch reads that one value, so no gesture re-supplies a Section
or a level and a column's list cannot disagree with the code that resets it. The
selecting-action-resets-scroll rule is therefore a property of the Column rather than a
statement six drains each repeat. Two shapes exist, not one: a Section root and a deeper
Column differ in a real rule (a drill's sort flip resets its list, a root's does not), so
there are two bindings and the difference between two Columns of a shape is a value. See Flow
5 above.

### Row-at-*i* at the read seam

The seam answers "give me row *i*" and "how many rows does this listing have" as **two reads**, never one bundled answer. The arithmetic that makes a bounded listing cheap — which window a row belongs to, when a window must be refetched, where inside a window a row sits — lives in the windowed-list projection and nowhere else. A render site names a row index; it holds no window, decides no refetch, and subtracts no offset. The alternative was writing that arithmetic once per paged read in the seam and again in every render site, where an off-by-one produces a wrong-but-plausible row that only a golden image catches, weeks later, for no visible reason. With the arithmetic behind the seam it is a seam test failure — see Flow 4 above and the boundary tests named in [../engineering/testing-strategy.md](../engineering/testing-strategy.md).

## See also

- [./threading-model.md](./threading-model.md) — the threads and constraints behind these flows.
- [./data-model.md](./data-model.md) — the types that flow through these sequences.
