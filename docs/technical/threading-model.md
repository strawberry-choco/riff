# Threading Model

riff is a multithreaded application. The egui event loop must stay responsive, audio decoding must not stall the UI, and the operating system's audio callback must never block. To satisfy these constraints the work is split across several threads that communicate through `crossbeam_channel` messages and two pieces of shared state. This document describes every thread, the channels that connect them, the shared state they touch, and the hard constraints that keep the system correct.

For the end-to-end sequences that these threads participate in, see [./data-flow.md](./data-flow.md). For the types that cross the channels, see [./data-model.md](./data-model.md).

## Threads

The table below lists the threads that make up a running instance. Four are spawned explicitly in `src/main.rs`, one is spawned inside the UI (`RiffApp::new` in `src/ui/app.rs`), one is spawned by the tray module on non-Linux platforms, and the cpal callback thread is owned by the operating system audio stack.

| Thread | Responsibility | Spawned in |
|--------|----------------|------------|
| Main / egui event loop | Runs the eframe event loop, renders the UI, handles user input, reads `AppState` each frame, polls incoming channels. Must never block. | `main.rs` (`eframe::run_native`) |
| Audio engine | Owns the decoder and the audio output. Drives the decode loop, writes samples into the shared buffer, handles every `PlaybackCommand`, and emits `PlaybackUpdate` messages. | `main.rs` (`run_audio_engine`) |
| Update processor | Receives `PlaybackUpdate` messages and mutates `Arc<Mutex<AppState>>`. Auto-advances the queue on `TrackEnded`. | `main.rs` |
| cpal callback | OS real-time audio thread. Pops samples from the shared buffer via `try_lock` and writes them to the audio device. Never blocks. | Owned by cpal / OS |
| Library scanner | Receives `LibraryCommand`s, walks the filesystem with `walkdir`, reads metadata in chunks, and emits `LibraryUpdate`s. | `main.rs` |
| Filesystem-event processor | Receives raw change paths from the `notify` watcher and forwards them to the `WatcherManager`, which debounces and triggers rescans. | `main.rs` |
| Cover loader | Receives `(TrackId, PathBuf)` cover requests, resolves and decodes the image through the shared `CoverResolver`, and returns `(String, Option<CoverImage>)`. Runs a receive loop for the life of the app. | `ui/app.rs` (`RiffApp::new`) |
| Tray event thread (non-Linux) | Dispatches system tray menu events (play/pause, next, quit) as `PlaybackCommand`s. | `ui/tray.rs` |

The audio engine thread is the heart of playback. It is a single long-lived loop that blocks on `cmd_rx.recv()` waiting for a `PlaybackCommand`. When it receives `Play(track_id)` it opens the decoder, starts the cpal stream, and enters an inner decode loop that runs until the track ends or a command interrupts it.

## Wiring at Startup

All of the core channels are created in `main.rs` before any thread is spawned:

```rust
let state = Arc::new(Mutex::new(AppState::new()));
let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
let (update_tx, update_rx) = unbounded::<PlaybackUpdate>();
let (library_cmd_tx, library_cmd_rx) = unbounded::<LibraryCommand>();
let (library_update_tx, library_update_rx) = unbounded::<LibraryUpdate>();
```

Senders are cloned before being moved into closures so that several producers can share one channel. The command sender, for instance, is cloned for the UI, for the audio engine (which re-sends commands to itself to sequence `Next`/`Previous`/`Resume`), and — on non-Linux builds — for the tray. The library command sender is likewise shared between the UI and the `WatcherManager`, so both a manual scan and a debounced filesystem-watch rescan flow through the same channel.

Before the audio engine runs, it builds a symphonia `CodecRegistry`, registers the default codecs, and registers the Opus adapter (`symphonia_adapter_libopus`) so that Opus files decode correctly. The decoder and the cpal output are then owned outright by the engine thread for its entire lifetime; they are never shared with another thread.

## The Update Processor in Detail

The update processor is a thin loop: `while let Ok(update) = update_rx.recv()`, then match on the update and mutate `AppState`. Most variants are simple field writes (`StateChanged`, `PositionChanged`, `TrackChanged`). The interesting case is `TrackEnded`, which must advance the queue without holding the state lock across a channel send:

1. Lock `AppState`, call `queue.next()`, and clone the resulting `TrackId`.
2. Drop the lock before doing anything else.
3. If there is a next track, send `PlaybackCommand::Play(next_id)`; otherwise re-lock and set `playback_state = Stopped`.

This drop-then-reacquire pattern is the canonical example of the "no nested locking / no long-held lock" rule: the lock is never held while sending on a channel, and each critical section is as short as possible.

## Communication

All cross-thread messaging uses unbounded `crossbeam_channel` channels created in `main.rs` (the cover channels are created in `RiffApp::new`). Senders are cloned before being moved into threads so that multiple producers can share a channel.

| Direction | Message type | Contents |
|-----------|--------------|----------|
| UI / tray -> audio engine | `PlaybackCommand` | `Play(TrackId)`, `Pause`, `Resume`, `Stop`, `Seek(Duration)`, `SetVolume(f32)`, `Next`, `Previous`, `PlayNext(TrackId)`, `AddToQueue(TrackId)`, `PlayPause`, `ToggleVisibility` |
| Audio engine -> update processor | `PlaybackUpdate` | `StateChanged(PlaybackState)`, `PositionChanged(PlaybackPosition)`, `TrackChanged(TrackId)`, `TrackEnded`, `Error(String)` |
| UI / watcher -> library scanner | `LibraryCommand` | `ScanDirectory(PathBuf)`, `CancelScan` |
| Library scanner -> UI | `LibraryUpdate` | `ScanProgress { path, files_found, current_dir }`, `ScanComplete { path, total_files }`, `ScanError { path, message }` |
| `notify` watcher -> fs-event processor | `PathBuf` | The path that changed, forwarded to `WatcherManager::on_fs_event` |
| UI -> cover loader | `(TrackId, PathBuf)` | A request to resolve and decode cover art for a track |
| Cover loader -> UI | `(String, Option<CoverImage>)` | The track id key and the decoded RGBA image, if any |

Because the channels are unbounded, producers never block on a slow consumer. Backpressure for audio is enforced separately in the decode loop (see below), not by the channel.

## Shared State

Two values are shared between threads behind `Arc<Mutex<_>>`:

- **`Arc<Mutex<AppState>>`** — the entire application state: library, queue, playback state, current position, volume, view mode, library paths and statuses, and watch states. It is read by the UI every frame and written primarily by the update processor thread. The audio engine and the UI also take short locks for specific mutations (for example, reading a track path or inserting into the queue).
- **`Arc<Mutex<VecDeque<f32>>>`** — the audio ring buffer shared between the audio engine thread (producer) and the cpal callback thread (consumer). The engine pushes decoded samples; the callback pops them. There is no hard upper bound on the buffer; the decode loop's backpressure check prevents unbounded growth.

The `WatcherManager` is also shared as `Arc<Mutex<Option<WatcherManager>>>` between the filesystem-event thread and the UI, which polls it and notifies it of scan completion.

## Constraints

These constraints are load-bearing. Violating any of them causes audio glitches, deadlocks, or a frozen UI.

### The cpal callback never blocks

The cpal callback runs on a real-time OS audio thread. It acquires the buffer lock with `try_lock()` rather than `lock()`. If the lock is unavailable — because the engine thread is momentarily holding it — the callback outputs silence for that buffer instead of waiting. Blocking on this thread would cause audible dropouts and can destabilize the audio device.

### Decode-loop backpressure

Before each decode batch, the audio engine checks whether the shared buffer already holds at least two seconds of audio, computed as `sample_rate * channels * 2` samples. If the buffer is full, the engine sleeps for 10 milliseconds and polls the command channel for `Pause`, `Stop`, `Seek`, or volume changes, then re-checks. This prevents the decoder from racing ahead of playback and consuming unbounded memory. Decoding proceeds in batches of up to 4096 frames via `decoder.next_frames(4096)`.

### The UI never blocks

The main thread must return from each frame quickly. All heavy work — scanning, metadata reading, image decoding — happens on background threads. The UI only ever performs non-blocking `try_recv` polls on its incoming channels and short locked reads of `AppState`.

### No nested locking

There is a single application-state mutex. Code must never hold the `AppState` lock while acquiring another lock, and must never hold it across a long-running operation. Where a command needs both a state read and a follow-up action (for example, the update processor advancing the queue on `TrackEnded`), the lock is dropped before the next acquisition. The `MutexExt::lock_or_recover` helper is used everywhere so that a poisoned lock is recovered rather than causing a panic.

## End-of-File Drain Behavior

When the decoder reaches the end of a track (`next_frames` returns `Ok(None)`), the engine does not stop the stream immediately, because the shared buffer still holds samples that have not yet reached the device. Instead it:

1. Sends `PlaybackUpdate::TrackEnded` so the update processor can advance the queue.
2. Enters a drain loop that waits for the buffer to empty, sleeping 50 ms per iteration and polling the command channel.
3. If a `Stop` command arrives during the drain, clears the buffer immediately and breaks out, discarding the remaining samples.
4. Once the buffer is empty, breaks out of the decode loop and stops the stream.

The buffer is otherwise drained naturally through the callback. `clear_buffer()` is called explicitly only at the start of a new track, on a `Stop` command, or on a `Seek`.

## Cover Loader Thread Lifecycle

The cover loader is a single persistent worker spawned once in `RiffApp::new`. It owns a clone of the `Arc<Mutex<CoverResolver>>` and runs a `while let Ok((track_id, path)) = cover_rx.recv()` loop for the lifetime of the application, processing requests sequentially. For each request it locks the resolver, calls `resolve(&path)` (which reads embedded art or falls back to a filesystem image and decodes it to RGBA), and sends `(track_id.0, Option<CoverImage>)` back on the response channel. The UI drains that response channel in `update_cover_cache`, uploads successful images to egui textures, and manages a 50-entry LRU. If resolution fails, the error is logged with `tracing::warn!` and a `None` is returned so the UI simply shows no cover.

## See also

- [./data-flow.md](./data-flow.md) — the playback, scanning, and cover-resolution sequences these threads execute.
- [./architecture.md](./architecture.md) — layer boundaries and where each thread's code lives.
