# Data Flow

This document walks through the three primary runtime flows in riff — playing a track, scanning a library, and resolving cover art — as step-by-step sequences. Each flow crosses several threads and layers; the filenames referenced are the real ones in the source tree. For the threads involved and the constraints that govern them, see [./threading-model.md](./threading-model.md). For how state persists in the Application Store, see [./persistence.md](./persistence.md).

## Flow 1: Play a Track

This is the central flow. It begins with a click in the UI and ends with samples reaching the audio device, with the queue auto-advancing when the track finishes.

```
User clicks "Play" in the UI (src/ui/app.rs)
  -> UI sends PlaybackCommand::Play(track_id) over the command channel
  -> Audio engine thread (run_audio_engine in src/main.rs) receives the command
   -> Engine locks AppState and resolves the Track through the LibraryQueryStore
        port (the Application Store is the sole authority for track metadata;
        a store miss drops the play request)
        (if the queue is empty, Queue Fill populates it from the whole library
        via all_track_ids() in canonical path order, makes the requested track
        current, and resets shuffle)
  -> SymphoniaDecoder::open(path) opens the file and returns AudioFormatInfo
       (sample_rate, channels, duration)
  -> CpalAudioOutput::initialize(sample_rate, channels) configures the stream
  -> CpalAudioOutput::start() starts the cpal stream
  -> CpalAudioOutput::clear_buffer() discards leftover samples from any previous track
  -> Engine sends PlaybackUpdate::StateChanged(Playing) and PlaybackUpdate::TrackChanged(track_id)
  -> Decode loop begins (push model, on the engine thread):
       a. Backpressure check: while buffer >= 2s of audio (sample_rate * channels * 2),
          sleep 10ms and poll the command channel
       b. decoder.next_frames(4096) decodes a batch of PCM samples
       c. audio_output.write_samples(&samples) pushes them into Arc<Mutex<VecDeque<f32>>>
       d. PlaybackUpdate::PositionChanged is sent with elapsed / total
       e. Command poll: try_recv for Pause / Stop / Seek / SetVolume
  -> cpal callback thread (OS audio thread): try_lock the buffer, pop samples, write to device;
     on lock failure, output silence
  -> On EOF (decoder returns Ok(None)):
       send PlaybackUpdate::TrackEnded, then drain the buffer (50ms per poll) before stopping;
       a Stop command during drain clears the buffer and discards remaining samples
  -> Update processor thread (src/main.rs) receives each PlaybackUpdate and mutates AppState
  -> On TrackEnded, the update processor calls queue.next(); if there is a next track it sends
     PlaybackCommand::Play(next_id), otherwise it sets playback_state = Stopped
  -> UI reads AppState each frame for the progress bar, play state, and any error message
```

The `Resume` path reuses the same machinery: the engine remembers the current track id and the paused position, and on resume it re-issues `Play(track_id)` and seeks to the stored position before continuing.

## Flow 2: Scan a Library

Scanning is triggered from the settings view and runs entirely on the library scanner thread. The UI stays responsive by only polling for progress updates.

```
User clicks "Scan" (or "Scan All") in the settings view (src/ui/settings.rs)
  -> UI sets the path's status to Scanning and sends LibraryCommand::ScanDirectory(path)
  -> Library scanner thread (src/main.rs) receives the command
  -> AudioFileScanner (src/infra/scanner.rs) walks the directory tree with walkdir
       and returns all audio file paths (honoring the AtomicBool cancel flag)
   -> For each chunk of 10 paths:
        -> Paths the store already knows are skipped (one indexed lookup per
           path through the LibraryQueryStore) so rescans do not re-read
           unchanged metadata; if the check errors, the path is scanned anyway
        -> build_tracks(chunk, &LoftyMetadataReader) from src/app/scan.rs reads
           tags, duration, cover source, and format; per-file failures are
           logged and skipped so a scan never aborts on one bad file
        -> The chunk commits as ONE immediate durable transaction through the
           LibraryMutationStore port (apply_scan_batch), preserving existing play
           history for known tracks
        -> On success the mutation adapter bumps the session generation counter,
           so Session Projections refetch on the next frame
        -> LibraryUpdate::Progress { path, files_found, current_dir } is sent to the UI
  -> When all chunks are processed:
       LibraryUpdate::Complete { path, total_files } is sent to the UI
  -> UI (poll_library_updates in src/ui/app.rs) receives Complete:
       -> sets the path status to Scanned(total_files)
       -> notifies the WatcherManager so any queued rescan can fire
  -> UI re-renders the library view from its projections with the new tracks
```

A `LibraryCommand::CancelScan` sets the shared cancel flag, which the scanner checks between chunks to stop early. If the scanner itself fails, it sends `LibraryUpdate::Error { path, message }`, the UI resets the path status to `Idle`, and the message is shown in the status line. The scan thread never touches `AppState`: it reads the store through the query port and commits through the mutation port.

## Flow 3: Resolve Cover Art

Cover resolution is asynchronous. The UI requests a cover, a dedicated worker thread resolves and decodes it, and the UI uploads the result to an egui texture the next time it polls.

```
A track becomes current or is selected for display (src/ui/app.rs)
  -> request_cover(track_id, file_path) sends (track_id, file_path) on the cover request channel
       (skipped if a texture for that track id is already cached)
  -> Cover loader thread (spawned in RiffApp::new) receives the request
  -> It locks the shared CoverResolver (src/app/cover_resolver.rs) and calls resolve(path)
  -> CoverResolver asks LoftyMetadataReader::read_cover_source(path) for the cover source
  -> Priority: embedded art first, filesystem fallback second
       -> If CoverSource::Embedded(bytes): ImageCoverLoader decodes the bytes to RGBA
       -> If CoverSource::None: CoverResolver scans the track's directory for a cover image
          (cover.jpg/png, folder.jpg/png, album.jpg/png, front.jpg/png — case-insensitive)
          and, if found, ImageCoverLoader decodes that file
       -> If CoverSource::Filesystem(path): ImageCoverLoader decodes that file directly
  -> The loader sends (track_id_string, Option<CoverImage>) on the response channel
  -> UI (update_cover_cache in src/ui/app.rs) drains the response channel:
       -> builds an egui::ColorImage from the RGBA bytes and loads it as a texture
       -> inserts it into cover_textures and touches the LRU (max 50 entries)
  -> Subsequent frames fetch the texture from the LRU cache without re-decoding
```

If resolution fails at any point, the worker logs a warning and returns `None`, so the UI simply displays no cover rather than surfacing an error.

## Key Design Decisions

### Push model

The audio engine pushes decoded samples into a shared buffer; the cpal callback pulls from it. The engine never responds to callback requests directly. This one-directional flow keeps the real-time audio thread free of any blocking call and isolates it from the decoder's pacing.

### Backpressure

Before each decode batch the engine checks whether the buffer already holds at least two seconds of audio (`sample_rate * channels * 2` samples). If it does, the engine sleeps 10 ms and polls for commands instead of decoding. This bounds memory use and keeps the engine from racing ahead of playback, while still letting it react promptly to pause, stop, and seek.

### Drain on EOF

When the decoder reaches end-of-file, the engine waits for the remaining buffer to drain through the callback before stopping the stream, so the tail of the track is not clipped. A `Stop` command during the drain discards the remaining samples immediately.

### Buffer lifetime

The shared buffer is not cleared on a natural stop — it drains on its own through the callback. `clear_buffer()` is invoked explicitly only at the start of a new track, on a `Stop` command, or on a `Seek`. This avoids both stale samples bleeding into a new track and unnecessary clearing during normal playback.

### Cover priority

Embedded cover art always wins. Only when a track has no embedded art does the resolver fall back to a filesystem image in the track's directory, checking a fixed list of common names (`cover`, `folder`, `album`, `front` with `.jpg`/`.jpeg`/`.png` extensions, matched case-insensitively). Decoded covers are cached as egui textures in a 50-entry LRU so a track's cover is decoded at most once per session.

## See also

- [./threading-model.md](./threading-model.md) — the threads and constraints behind these flows.
- [./persistence.md](./persistence.md) — how state persists in the Application Store.
- [./data-model.md](./data-model.md) — the types that flow through these sequences.
