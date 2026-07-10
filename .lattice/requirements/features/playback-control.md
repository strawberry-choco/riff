---
feature: Playback Control
epic: Audio Engine
status: implemented
priority: P0
depends_on: ["Multi-format Decoding"]
personas: ["Music Listener"]
source_docs: []
implementation_notes: |
  Implemented in main.rs run_audio_engine() with SymphoniaDecoder + CpalAudioOutput.
  Push-model: decoder writes to shared Arc<Mutex<VecDeque<f32>>> buffer; cpal callback pulls from it via try_lock.
  Backpressure: decode loop throttles when buffer ≥ 2s of audio (sample_rate × channels × 2), sleeps 10ms + polls commands.
  Drain on EOF: after decoder returns None, buffer drains through cpal callback before stopping stream.
  Stop no longer clears buffer; explicit clear_buffer() called on new track, Stop command, or Seek.
  Supports play, pause, resume, stop, seek, volume, next, previous.
---

# Playback Control

## Problem Statement

Music listeners need basic transport controls (play, pause, stop) and advanced controls (seek, volume adjustment) to interact with their audio playback. The playback system must manage an audio output stream across all supported platforms and provide smooth, gap-free control transitions.

## User / Personas

- **Music Listener**: A person listening to music who expects standard media player controls to work intuitively and responsively, with no perceptible delay between pressing a button and hearing the result.

## Scope

**In scope:**
- Play audio from a file path or decoded stream
- Pause playback (retain position, mute output)
- Resume playback from paused position
- Stop playback (reset position to start)
- Seek to arbitrary position in the track
- Adjust playback volume (0% to 100%)
- Report current playback position and total duration
- Audio output via cpal on Linux (ALSA/PipeWire/PulseAudio), Windows (WASAPI), macOS (CoreAudio)

**Out of scope:**
- Crossfade between tracks
- Speed/pitch adjustment (playback rate change)
- Per-channel balance (panning)
- Output device selection (use system default only)
- Output to multiple devices simultaneously

## Boundary Conditions

- Seeking to a position beyond track duration should clamp to the end
- Volume changes should apply immediately without audible clicks or pops
- Playback should gracefully handle the output device being disconnected (e.g., Bluetooth headphones turning off)
- If the audio thread panics, the main application should remain responsive and report the error
- Position reporting should update at least once per second during playback

## Assumptions

- cpal provides stable audio output across all three target platforms
- The audio output thread can run independently from the UI thread
- A shared `Arc<Mutex<VecDeque<f32>>>` buffer with backpressure-based throttling provides sufficient latency for interactive controls without unbounded memory growth
- 50-200ms audio latency is acceptable for a music player (not a real-time instrument)
- The buffer is not bounded by a fixed capacity — `VecDeque` grows as needed, but backpressure at 2 seconds prevents runaway allocation

## Scenarios

### Scenario 1: Play a track
A user selects a track and presses play.

**Acceptance Criteria:**
- Given a valid audio file, when the user initiates playback, then audio begins playing within 500ms
- Given a track is playing, when the user looks at the UI, then the progress bar shows the current position advancing and the total duration

### Scenario 2: Pause and resume
A user pauses playback and later resumes.

**Acceptance Criteria:**
- Given a track is playing, when the user presses pause, then audio output stops within 100ms and the current position is preserved
- Given a track is paused, when the user presses play, then audio resumes from the exact paused position within 500ms
- Given a track is paused, when the user presses stop, then the track resets to the beginning and the paused state is cleared

### Scenario 3: Seek within a track
A user drags the progress bar to a different position.

**Acceptance Criteria:**
- Given a track is playing, when the user seeks to a new position, then audio jumps to that position and resumes playback within 300ms
- Given a track is paused, when the user seeks to a new position, then the position updates and the UI reflects the new position without starting playback
- Given the user seeks to a position beyond track duration, when the seek is attempted, then the position clamps to the track end and playback stops

### Scenario 4: Adjust volume
A user changes the volume slider.

**Acceptance Criteria:**
- Given a track is playing, when the user drags the volume slider to 50%, then the perceived loudness is approximately halved without audible artifacts
- Given the volume is at 0%, when a track is playing, then no audible output is produced
- Given the volume is adjusted during playback, when the change happens, then it applies to the next audio buffer without a perceptible delay

### Scenario 5: Handle output device disconnection
A user's audio output device is disconnected during playback.

**Acceptance Criteria:**
- Given a track is playing through Bluetooth headphones, when the headphones disconnect, then playback pauses gracefully and the UI updates to show a paused state
- Given a device disconnection occurs, when a new default device becomes available, then the user can resume playback without restarting the application

## Implementation Notes

1. **Audio engine thread architecture**: A single dedicated OS thread (`run_audio_engine` in `main.rs`) owns both the decoder and audio output. It receives `PlaybackCommand` messages via `crossbeam_channel::unbounded()`, runs the decode loop inline (not in a separate decoder thread), and sends `PlaybackUpdate` messages to a separate update processor thread
2. **State machine**: The playback state is a clear enum: `Stopped | Paused { position: Duration } | Playing { start_time: Instant, paused_offset: Duration }`
3. **Buffer management**: Decoded PCM samples are pushed into a shared `Arc<Mutex<VecDeque<f32>>>` buffer. The decode loop checks buffer fill level before each decode and applies backpressure when the buffer holds ≥ 2 seconds of audio (`sample_rate × channels × 2` samples). This prevents the decoder from racing ahead of playback and consuming unbounded memory. The cpal callback (on a separate real-time thread) pulls samples from this buffer via `try_lock` — on lock failure, it outputs silence rather than blocking
4. **Volume scaling**: Apply volume as a floating-point multiplier on each sample. Applied in the decode loop before writing to the buffer
5. **Position tracking**: Position is tracked via `Instant::elapsed()` from playback start, sent as `PlaybackUpdate::PositionChanged` each decode iteration. Since backpressure throttles the decode loop, position updates are paced by the fill rate rather than a fixed timer — typically multiple updates per second
6. **Track end handling**: When the decoder returns `None` (EOF), the engine sends `TrackEnded` and enters a drain loop that waits for the shared buffer to empty through the cpal callback before stopping the audio stream. A `Stop` command during drain discards remaining samples
7. **Stop vs. Pause**: `stop()` pauses the cpal stream but does NOT clear the shared buffer. Buffer clearing is explicit via `clear_buffer()` and happens only at: new track start, explicit Stop command, and Seek

## Open Questions

- [ ] Should we pre-buffer the next track during playback to enable faster track transitions?
- [x] What is the target buffer size in milliseconds? (200ms? 500ms?) — **Resolved: 2 seconds** (`sample_rate × channels × 2`). Chosen as a balance between memory usage (~350 KB at 44.1kHz stereo f32) and resilience against decode latency spikes. The buffer is unbounded in capacity but backpressure prevents it from exceeding this threshold in steady state.
- [ ] Do we need a separate audio thread per platform, or can cpal handle all platforms with one pattern?

## Links
- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
