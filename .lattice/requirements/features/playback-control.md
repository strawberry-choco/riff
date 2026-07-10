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
- A ring buffer or channel-based approach will provide sufficient latency for interactive controls
- 50-200ms audio latency is acceptable for a music player (not a real-time instrument)

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

1. **Audio thread architecture**: Spawn a dedicated audio output thread that owns the cpal stream and receives commands (play, pause, seek, volume) via a bounded channel
2. **State machine**: The playback state should be a clear enum: `Stopped | Paused { position: Duration } | Playing { start_time: Instant, paused_offset: Duration }`
3. **Decoding thread**: Consider a separate decoding thread that fills a bounded audio buffer ahead of the playback position, to smooth over seek latency and disk I/O variance
4. **Volume scaling**: Apply volume as a floating-point multiplier on each sample. Consider a logarithmic volume curve for perceptually-linear slider response
5. **Position tracking**: Track position independently from the audio clock to account for buffering. Use `Instant` elapsed time plus paused offsets for active playback, and decoder-reported sample position for seek validation

## Open Questions

- [ ] Should we pre-buffer the next track during playback to enable faster track transitions?
- [ ] What is the target buffer size in milliseconds? (200ms? 500ms?)
- [ ] Do we need a separate audio thread per platform, or can cpal handle all platforms with one pattern?

## Links
- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
