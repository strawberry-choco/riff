# Data Flow

This document walks through the three primary runtime flows in riff — playing a track, scanning a library, and resolving cover art — as step-by-step sequences. Each flow crosses several threads and crates; the filenames referenced are the real ones in the workspace. For the threads involved and the constraints that govern them, see [./threading-model.md](./threading-model.md). For how state persists in the Application Store, see [./persistence.md](./persistence.md).

## Flow 1: Play a Track

This is the central flow. It begins with a click in the UI and ends with samples reaching the audio device, with the Playback Coordinator auto-advancing when the track finishes.

```
User clicks "Play" in the UI (riff-gui/src/ui/)
  -> The UI's Box<dyn Transport> dispatches the intent: the FacadeTransport
     records the command onto the shared facade's event inbox, then the
     ChannelTransport sends PlaybackCommand::Play(track_id) on the command channel
  -> Audio engine thread (AudioEngine::run in riff-playback/src/infra/audio_engine.rs)
     receives the command
   -> Engine locks PlaybackSession and resolves the Track through the
        LibraryQueryStore port (the Application Store is the sole authority for
        track metadata; a store miss drops the play request)
        (if the queue is empty, Queue Fill populates it from the whole library
        via all_track_ids() in canonical path order, makes the requested track
        current, and resets shuffle)
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
  -> When all chunks are processed, the terminal ScanOutcome reports the total
  -> UI sets the path status to Scanned(total) and renders the refreshed views
```

A cancellation request sets the shared cancel flag, which the scanner checks between chunks; the service keeps every already-committed batch (an interrupted scan never rolls back committed work). If a commit fails, the outcome stream reports the failure and the path status is reset. The scan worker never touches the session structs: it reads the store through the query port and commits through the mutation port.

Watched folders feed the same seam from the other side: the filesystem-event forwarder hands `notify` events to the `WatcherManager` (riff-backend), which debounces batches and requests rescans through the shared `ScanService` — a manual scan and a watch-triggered rescan are the same flow.

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
  -> The decoded image is cached in the service's bounded LRU (cap 50) and
     delivered to the UI over the response poll
  -> UI drains the response channel:
       -> builds an egui::ColorImage from the RGBA bytes and loads it as a texture
       -> inserts it into cover_textures and touches the LRU (max 50 entries)
  -> Subsequent frames fetch the texture from the LRU cache without re-decoding
```

If resolution fails at any point, the worker logs a warning and reports no image, so the UI simply displays no cover rather than surfacing an error; the service's negative cache prevents retry storms for artless tracks.

## Key Design Decisions

### Push model

The audio engine pushes decoded samples into the output adapter's ring buffer; the cpal callback pulls from it. The engine never responds to callback requests directly. This one-directional flow keeps the real-time audio thread free of any blocking call and isolates it from the decoder's pacing.

### Backpressure

Backpressure lives inside the output adapter: the engine's write blocks when the lock-free ring buffer is full, so the decoder can never outrun playback. The engine still polls the command channel on a 10 ms cadence between chunks, so pause, stop, and seek stay responsive.

### Drain on EOF

When the decoder reaches end-of-file, the engine waits for the remaining buffer to drain through the callback before stopping the stream, so the tail of the track is not clipped. A `Stop` command during the drain discards the remaining samples immediately.

### Buffer lifetime

The shared buffer is not cleared on a natural stop — it drains on its own through the callback. It is cleared explicitly only at the start of a new track, on a `Stop` command, or on a `Seek`. This avoids both stale samples bleeding into a new track and unnecessary clearing during normal playback.

### Continuation ownership

The Audio Engine only reports what happened; the Playback Coordinator decides what happens next. History-before-advance, repeat-one replay, auto-advance, and the stop-at-end rule all live in one place, and playback failures surface as typed notices instead of state writes.

### Cover priority

Embedded cover art always wins. Only when a track has no embedded art does the resolver fall back to a filesystem image in the track's directory, checking a fixed list of common names (`cover`, `folder`, `album`, `front` with `.jpg`/`.jpeg`/`.png` extensions, matched case-insensitively). Decoded covers are cached in the service's LRU (cap 50) and rendered textures in the UI's LRU (max 50), so a track's cover is decoded at most once per session.

## See also

- [./threading-model.md](./threading-model.md) — the threads and constraints behind these flows.
- [./persistence.md](./persistence.md) — how state persists in the Application Store.
- [./data-model.md](./data-model.md) — the types that flow through these sequences.
