---
feature: Player Control Bar
epic: User Interface
status: partial
priority: P0
depends_on: [Playback Control]
personas: []
source_docs: []
implementation_gaps: |
  Done: Previous/Play-Pause/Next buttons, progress bar with click-to-seek,
  time display (MM:SS / MM:SS), volume slider, shuffle/repeat toggles,
  queue position indicator.
  Missing: Stop button.
---

# Player Control Bar

## Problem Statement

Users need persistent, always-visible playback controls at the bottom of the window. The control bar must show transport buttons (previous, play/pause, next), a progress bar with current time and total duration, volume control, and basic track information (title, artist, album).

## User / Personas

**Active Listener**: A user who frequently skips tracks, seeks to specific parts, and adjusts volume. They need controls that are always visible and responsive.

## Scope

**In scope:**
- Transport buttons: Previous, Play/Pause (toggle), Next, Stop
- Progress bar: Shows elapsed time and total duration, clickable to seek
- Time display: "MM:SS / MM:SS" format
- Volume slider: 0% to 100%, with mute button
- Track info display: Title, Artist, Album (2-line format: Title on top, Artist - Album below)
- Shuffle toggle button
- Repeat mode toggle button (cycles: no repeat → repeat all → repeat one → no repeat)
- Queue length indicator ("3 / 42")

**Out of scope:**
- Mini player / floating widget
- Visualizations (spectrum analyzer, waveform)
- Lyrics display
- Rating stars or favorite button
- Output device selector
- Detailed audio format info (bitrate, sample rate, codec) — can be added to a details popup

## Boundary Conditions

- Buttons must be large enough to hit easily (minimum 32x32px)
- Progress bar must allow seeking to any point, even for files where duration is initially unknown (VBR MP3 without Xing header)
- Volume slider must not produce audible jumps when dragged
- Track info must truncate with ellipsis if too long for the available space
- Control bar is fixed at the bottom of the window and always visible

## Assumptions

- Users expect the standard horizontal control bar layout found in most music players
- The control bar receives real-time updates from the playback engine (position, state changes)
- Buttons use standard Unicode symbols or simple text labels (no custom SVG icons needed for MVP)

## Scenarios

### Scenario 1: Control playback
A user interacts with the basic transport controls.

**Acceptance Criteria:**
- Given a track is loaded, when the user clicks the Play button, then the track begins playing and the button changes to a Pause icon
- Given a track is playing, when the user clicks the Pause button, then playback pauses and the button changes to a Play icon
- Given a track is playing, when the user clicks the Next button, then playback advances to the next track in the queue
- Given a track is playing (not the first), when the user clicks the Previous button, then playback goes to the previous track (or restarts current if >5s elapsed)

### Scenario 2: Seek within a track
A user clicks on the progress bar to jump to a specific time.

**Acceptance Criteria:**
- Given a track is playing, when the user clicks at the 50% position on the progress bar, then playback jumps to approximately the middle of the track
- Given the user is dragging the progress bar slider, when they release the mouse, then playback seeks to the released position
- Given the track duration is unknown (VBR MP3 without duration info), when the user clicks the progress bar, then seeking uses the current best estimate of duration

### Scenario 3: Adjust volume
A user drags the volume slider.

**Acceptance Criteria:**
- Given the volume is at 50%, when the user drags the slider to 75%, then the volume is updated smoothly and the audio output reflects the new level
- Given the volume is unmuted, when the user clicks the mute button, then the volume is silenced and the mute button indicates the muted state
- Given the volume is muted, when the user clicks the mute button again, then the previous volume level is restored

## Implementation Notes

1. **Layout**: Use an `egui::TopBottomPanel::bottom` for the control bar. Divide it into sections: track info (left), transport controls (center), volume + extras (right).
2. **Progress bar**: Implement a custom progress bar using `egui::Response::interact` and `ui.add(egui::Slider::new())` or a custom `ProgressBar` widget that supports click-to-seek. Use `egui::Rect` and `ui.allocate_response` for a clickable bar.
3. **State synchronization**: The control bar reads from the shared `PlaybackEngine` state (playing/paused/stopped, current position, duration, volume). Buttons send commands back to the engine via a channel.
4. **Real-time updates**: Use `egui::Context::request_repaint()` at regular intervals (e.g., every 100ms) while playing to update the progress bar and time display.
5. **Unicode icons**: Use simple Unicode characters for buttons: ▶ (Play), ⏸ (Pause), ⏹ (Stop), ⏮ (Previous), ⏭ (Next), 🔀 (Shuffle), 🔁 (Repeat). These render on all platforms without custom font assets.

## Open Questions

- [ ] Should we show remaining time instead of total time? (Format: "MM:SS / -MM:SS"?) (Non-blocking: show both elapsed and total for MVP)
- [ ] Should the progress bar show a waveform or just a plain bar? (Non-blocking: plain bar for MVP)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
