# Threading Model

riff is a multithreaded application. The egui event loop must stay responsive, audio decoding must not stall the UI, and the operating system's audio callback must never block. To satisfy these constraints the work is split across several threads that communicate through `crossbeam_channel` messages and a small set of shared session structs. This document describes every thread, the channels that connect them, the shared state they touch, and the hard constraints that keep the system correct.

For the end-to-end sequences that these threads participate in, see [./data-flow.md](./data-flow.md). For the types that cross the channels, see [./data-model.md](./data-model.md).

## Threads

Every worker thread is spawned by the Composition Root — `AppRuntime::spawn` in `riff-backend/src/composition.rs` — except the tray thread, which the frontend spawns. The cpal callback thread is owned by the operating system audio stack.

| Thread | Responsibility | Spawned in |
|--------|----------------|------------|
| Main / egui event loop | Runs the eframe event loop, renders the UI, handles user input, reads the session structs each frame, polls incoming channels. Must never block. | `riff-gui/src/main.rs` (`eframe::run_native`) |
| Audio engine | Owns the decoder factory and the audio output. Drives the decode loop, writes samples into the output's ring buffer, handles every `PlaybackCommand`, and emits `PlaybackUpdate` messages. | `composition.rs` (`run_engine_thread`; engine loop is `AudioEngine::run` in `riff-playback/src/infra/audio_engine.rs`) |
| Playback Coordinator | Receives `PlaybackUpdate` messages and applies them to `Arc<Mutex<PlaybackSession>>`. Commits play history before advancing, replays on repeat-one, auto-advances on `TrackEnded`, stops when nothing follows, and emits typed notices on playback errors. | `composition.rs` (`PlaybackCoordinator::spawn` in `riff-playback`) |
| cpal callback | OS real-time audio thread. Pops samples from the lock-free ring buffer and writes them to the audio device. Never blocks. | Owned by cpal / OS |
| Library scan worker | Runs the `ScanService` worker loop: receives scan requests, walks the filesystem with `walkdir` (honoring the cancel flag), applies the freshness filter, and commits ~10-track durable batches. | `composition.rs` (worker from `ScanService::new` in `riff-library`) |
| Filesystem-event forwarder | Receives raw change paths from the `notify` watcher and forwards them to the `WatcherManager`, which debounces and triggers rescans through the `ScanService`. | `composition.rs` (`spawn_fs_watcher`) |
| Tag-edit worker | Processes `TagEdit` requests: writes file tags via lofty (source of truth) and commits the store facts as one durable change, reporting a single combined outcome. | `composition.rs` (`TagEditService::new` + worker run) |
| Cover worker | Receives cover requests, resolves embedded/filesystem art, decodes it to RGBA through the bounded LRU in the `CoverService`, and returns results to the UI. | `composition.rs` (`CoverService::new` + worker run) |
| Tray event thread (non-Linux) | Dispatches system tray menu events (play/pause, next, previous, show/hide, quit) through the tray's `FacadeTransport`. | `riff-gui/src/ui/tray.rs` |

The audio engine thread is the heart of playback. It is a single long-lived loop that blocks on `cmd_rx.recv()` waiting for a `PlaybackCommand`. When it receives `Play(track_id)` it resolves the Track through the store query port, opens a decoder through the injected `DecoderFactory` (which mints a fresh symphonia `CodecRegistry` per decoder, so the Opus adapter decodes correctly), starts the cpal stream, and enters an inner decode loop that runs until the track ends or a command interrupts it.

## Wiring at Startup

`AppRuntime::spawn` opens the Application Store first — open/migration failures are returned to the caller, never silently tolerated — and then wires everything:

```rust
let (settings, playlists, library_mutations, library_queries, generation,
     playlist_generation, changes_rx) = open_application_store(store_path)?;
let facade = Arc::new(Mutex::new(BackendFacade::default()));
let (playback, library) = (Arc::new(Mutex::new(PlaybackSession::default())),
                           Arc::new(Mutex::new(LibrarySession::default())));
let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
let (update_tx, update_rx) = unbounded::<PlaybackUpdate>();
```

One shared `SqliteStore` connection serves every store port view; both session generations (library and playlist) bump inside the store's mutation impls, and the `StoreChanged` stream feeds the facade. The UI's `Box<dyn Transport>` and the tray's transport are both `FacadeTransport`s wrapping the same facade, so every dispatched command is recorded onto one observable event inbox before being forwarded to the engine's command channel. The scan service, watcher manager, tag-edit service, and cover service are constructed over the real adapters here, and their workers' thread handles are owned by the runtime. The frontend then receives everything it renders with as one `AppRuntime` value.

## The Playback Coordinator in Detail

The coordinator is a thin loop: `while let Ok(update) = update_rx.recv()`, then match on the update and mutate `PlaybackSession`. Most variants are simple field writes (`StateChanged`, `PositionChanged`, `TrackChanged`). The interesting case is `TrackEnded`, which must advance the queue without holding the session lock across a channel send:

1. Lock `PlaybackSession`, decide the continuation, and clone the resulting `TrackId`.
2. Drop the lock before doing anything else.
3. If there is a next track, send `PlaybackCommand::Play(next_id)`; on repeat-one, replay the current track; otherwise re-lock and set the state to `Stopped`.

Play history is committed through the `LibraryMutationStore` port before the advance, so a crash between the two steps records the track that actually played. Playback errors surface as typed notices: the coordinator sends a pre-formatted message over the notice channel, and the facade stamps it with playback source and error severity — no cross-slice state write ever happens.

This drop-then-reacquire pattern is the canonical example of the "no nested locking / no long-held lock" rule: the lock is never held while sending on a channel, and each critical section is as short as possible.

## Communication

All cross-thread messaging uses unbounded `crossbeam_channel` channels created in `composition.rs`. Senders are cloned before being moved into threads so that multiple producers can share a channel.

| Direction | Message type | Contents |
|-----------|--------------|----------|
| UI / tray -> audio engine | `PlaybackCommand` | `Play(TrackId)`, `Pause`, `Resume`, `Stop`, `Seek(Duration)`, `SetVolume(f32)`, `Next`, `Previous`, `PlayNext(TrackId)`, `AddToQueue(TrackId)`, `PlayPause`, `ToggleVisibility` |
| Audio engine -> playback coordinator | `PlaybackUpdate` | `StateChanged(PlaybackState)`, `PositionChanged(PlaybackPosition)`, `TrackChanged(TrackId)`, `TrackEnded`, `Error(String)` |
| Coordinator -> engine | `PlaybackCommand` | The `Play(next_id)` issued on auto-advance |
| Playback Coordinator -> facade | `String` notice | A pre-formatted playback failure, stamped by the facade with source and severity |
| Store mutations -> facade | `StoreChanged` | Committed-mutation notifications that drive the facade's event surface |
| UI / watcher -> scan service | Scan request | A path to scan, serviced by the serial scan worker (one request at a time, cancelable) |
| Scan service -> UI | `ScanOutcome` stream | Progress cadence and a terminal outcome (completed total or failure) |
| `notify` watcher -> fs-event forwarder | `Vec<PathBuf>` | The paths that changed, forwarded to `WatcherManager::on_fs_events` |
| UI -> tag-edit service | `TagEditSubmission` | A tag edit; the service answers over an outcome poll |
| UI -> cover service | Cover request | Resolve and decode the art for a track; results come back over the response poll |

Because the channels are unbounded, producers never block on a slow consumer. Backpressure for audio is enforced inside the output adapter (see below), not by the channel.

## Shared State

A small set of values is shared between threads behind `Arc<Mutex<_>>`:

- **`Arc<Mutex<PlaybackSession>>`** — the playback half of the session state: queue, playback state, current position, volume, mute. Read by the engine, mutated by the Playback Coordinator, read by the tray and UI.
- **`Arc<Mutex<LibrarySession>>`** — the library half: selection, views, search, library paths and statuses, scan status, watch states. Owned by the frontend's rendering loop and the library services.
- **`Arc<Mutex<BackendFacade>>`** — the facade's event inbox; both transports record onto it and the UI drains it.
- **`Arc<Mutex<Option<WatcherManager>>>`** — shared between the filesystem-event forwarder and the UI, which reconfigures watch states through it.

The two session mutexes are independent: code must never hold one while acquiring the other. The audio ring buffer between the decode loop and the cpal callback is a lock-free SPSC ring (`ringbuf`) inside `CpalAudioOutput` (`riff-infra`) and is not part of the application surface.

## Constraints

These constraints are load-bearing. Violating any of them causes audio glitches, deadlocks, or a frozen UI.

### The cpal callback never blocks

The cpal callback runs on a real-time OS audio thread and only pops samples from the lock-free ring buffer. It performs no locking, allocation, or channel traffic. Blocking on this thread would cause audible dropouts and can destabilize the audio device.

### Decode-loop backpressure

The engine pushes decoded samples into the output adapter with a blocking write: when the ring buffer is full, the write parks the engine thread until the callback drains space, so the decoder can never run ahead of playback unboundedly. Between batches the engine also polls the command channel (a 10 ms poll cadence) so `Pause`, `Stop`, `Seek`, and volume changes are honored promptly. Decoding proceeds in chunks of up to 4096 samples via `decoder.next_frames`, and position updates are computed sample-accurately from the decoded count.

### The UI never blocks

The main thread must return from each frame quickly. All heavy work — scanning, tag writing, metadata reading, image decoding — happens on worker threads. The UI only ever performs non-blocking polls on its incoming channels and short locked reads of the session structs.

### No nested locking

The session structs each have exactly one mutex. Code must never hold a session lock while acquiring another lock, and must never hold it across a long-running operation. Where a command needs both a state read and a follow-up action (for example, the coordinator advancing the queue on `TrackEnded`), the lock is dropped before the next acquisition. The `MutexExt::lock_or_recover` helper is used everywhere so that a poisoned lock is recovered rather than causing a panic.

## End-of-File Drain Behavior

When the decoder reaches the end of a track, the engine does not stop the stream immediately, because the output buffer still holds samples that have not yet reached the device. Instead it:

1. Sends `PlaybackUpdate::TrackEnded` so the coordinator can advance the queue.
2. Enters a drain loop that waits for the buffer to empty, polling the command channel as it waits.
3. If a `Stop` command arrives during the drain, discards the remaining samples and breaks out immediately.
4. Once the buffer is empty, breaks out of the decode loop and stops the stream.

The buffer is otherwise drained naturally through the callback. The buffer is cleared explicitly only at the start of a new track, on a `Stop` command, or on a `Seek`.

## Gapless Handoff

For gapless-eligible transitions (compatible formats, no shuffle, correct repeat mode — pinned by `gapless::is_gapless_eligible`), the engine pre-decodes the successor track's first samples into a bounded pre-buffer while the current track finishes, then hands off without waiting on a fresh `init` on the hot path. The eligibility math and frame/duration conversions live in `riff-playback/src/app/gapless.rs`; the pre-buffer cap is 4 seconds of audio at the track's rate.

## See also

- [./data-flow.md](./data-flow.md) — the playback, scanning, and cover-resolution sequences these threads execute.
- [./architecture.md](./architecture.md) — the crate boundaries and where each thread's code lives.
