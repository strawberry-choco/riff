# Player Control Bar — Specification

## Feature

**Player Control Bar** — always-visible bottom bar carrying transport controls, progress, volume, shuffle, repeat, queue indicator, and stop.

**Status**: Partial. Transport, seeking, volume, shuffle, repeat, and queue indicator are implemented. The Stop button is not yet present.

---

## Overview

The player control bar is pinned to the bottom of the main application window and is always visible during playback. It provides the primary interface for controlling audio playback: transport (previous, play/pause, stop, next), a seekable progress bar, volume control, shuffle and repeat toggles, and a queue position indicator.

---

## Components

| Component | Description |
|-----------|-------------|
| Previous button | Jumps to prior track or restarts current track. |
| Play/Pause button | Toggles between playing and paused states. |
| Stop button | Stops playback and resets position to zero. (Planned — not yet present.) |
| Next button | Advances to the next track in the queue. |
| Progress bar | Click-to-seek bar with elapsed/total time. |
| Elapsed time label | MM:SS showing position from start. |
| Total time label | MM:SS showing track duration. |
| Volume slider | 0–100% continuous slider. |
| Mute button | Toggles mute; restores previous level on unmute. |
| Shuffle toggle | Toggles randomized queue order on/off. |
| Repeat toggle | Cycles: no repeat → repeat all → repeat one. |
| Queue position indicator | Text such as "3 / 42". |

---

## Behavior

### Transport

1. **Previous button** — When pressed:
   - If the current track has played for more than a few seconds, the track shall restart from the beginning.
   - If the current track has played for fewer than a few seconds, the player shall jump to the previous track in the queue.
   - If no previous track exists (first track in queue), the behavior shall be defined and consistent (e.g. restart current or no-op).

2. **Play/Pause button** — When pressed:
   - If playback is active, it shall pause.
   - If playback is paused, it shall resume from the current position.
   - If no track is loaded, it shall be a no-op or show an appropriate empty-state indicator.

3. **Stop button** — When pressed (planned):
   - Playback shall stop immediately.
   - The current position shall reset to zero.
   - The queue position shall remain unchanged.
   - The next press of Play shall start from the beginning of the current track.

4. **Next button** — When pressed:
   - The player shall advance to the next track in the queue and start playback.
   - If the queue is empty or the current track is the last one, the behavior shall be consistent with the current repeat mode.

### Progress Bar

5. **Display** — The bar shall show elapsed time on the left (or leading side) and total time on the right (or trailing side), both in MM:SS format.

6. **Click-to-seek** — Clicking anywhere on the progress bar shall seek to the corresponding position in the track.

7. **Clamp behavior** — Seeking to a position beyond the end of the track shall clamp to the end, not wrap or produce an error.

8. **Continuous update** — The progress indicator shall update continuously during playback (at least several times per second).

9. **No track state** — When no track is loaded, the progress bar shall be disabled or show zero values.

### Volume

10. **Slider range** — The volume slider shall accept values from 0 to 100 percent.

11. **Click-free** — Volume changes shall apply without audible clicks or pops.

12. **Mute toggle** — When the mute button is pressed:
    - Volume shall be set to zero.
    - The previous volume level shall be remembered.
    - Pressing mute again shall restore the previous volume level.

### Shuffle

13. **Toggle behavior** — When shuffle is toggled on, the queue order shall be randomized.

14. **Visual state** — The shuffle toggle shall show its on/off state visually.

### Repeat

15. **Cycle behavior** — The repeat toggle shall cycle through three states:
    - **No repeat**: when the queue ends, playback stops.
    - **Repeat all**: the entire queue loops from the beginning.
    - **Repeat one**: the current track loops until the toggle is changed.

16. **Visual state** — The repeat toggle shall show which of the three states is active.

### Queue Position Indicator

17. **Format** — The indicator shall display "current / total" (e.g. "3 / 42"), where current is the 1-based index of the currently playing track and total is the queue length.

18. **Empty queue** — When the queue is empty, the indicator shall show an appropriate empty value (e.g. "0 / 0" or hidden).

---

## States

| State | Description |
|-------|-------------|
| Idle | No track loaded. Transport buttons disabled or show no-op state. |
| Playing | Audio is actively playing. Play/Pause shows "pause" icon. Progress bar advancing. |
| Paused | Playback is paused. Play/Pause shows "play" icon. Progress bar frozen at current position. |
| Stopped | Playback has been stopped. Position reset to zero. (Planned — requires Stop button.) |
| Seeking | User is dragging/clicking on progress bar. Playback is temporarily paused or continues. |
| Buffering | Decoder is catching up to playback. Progress bar may show indeterminate state. |

---

## Edge Cases

- **Seeking past the end**: Clamped to the end; no wrap, no error.
- **Previous at track start**: Restarts the current track rather than jumping to a non-existent prior track.
- **Volume at 0 then unmute**: Restores the previous level; does not stay at zero.
- **Shuffle with one track**: Shuffle has no visible effect but the toggle still changes state.
- **Repeat one with one track**: Loops indefinitely until the repeat mode is changed.
- **Queue cleared during playback**: Transport buttons become no-ops or show empty state.
- **Audio device disconnects**: Playback pauses; UI reflects paused state.

---

## Empty States

- **No track loaded**: Transport buttons are disabled or visually indicate no active track. Progress bar shows 00:00 / 00:00. Volume controls remain functional.
- **Queue empty**: Queue position indicator shows "0 / 0" or is hidden. Next/Previous are no-ops.

---

## Error States

- **Unplayable current file**: Playback stops; error message displayed (see error-states.md). Transport controls remain available for switching tracks.
- **Audio device unavailable**: Playback pauses; UI reflects paused state. A status indicator or tooltip informs the user.

---

## Keyboard / Input

- **Space bar**: Toggle play/pause. (Recommended but not yet specified — should be documented in the final implementation.)
- **Arrow keys**: Left arrow = previous, Right arrow = next. (Recommended.)
- **Progress bar**: Mouse click seeks. Mouse drag should seek continuously.

---

## Platform Differences

- No platform-specific behavior for the control bar. All transport, volume, and display behavior is identical across Linux, Windows, and macOS.

---

## Out of Scope

- Crossfade between tracks.
- Speed/pitch adjustment.
- Output-device selection.
- Equalizer controls.
- Visualizations.
- Per-track volume normalization.

---

## Verification Checklist

- [ ] Previous button restarts track after a few seconds; jumps to prior track before that.
- [ ] Play/Pause toggles correctly between playing and paused states.
- [ ] Next button advances to the next track.
- [ ] Progress bar displays elapsed and total time in MM:SS format.
- [ ] Clicking the progress bar seeks to the correct position.
- [ ] Seeking past the end clamps to the end.
- [ ] Progress updates continuously during playback.
- [ ] Volume slider accepts 0–100% values.
- [ ] Mute toggles mute and restores previous level.
- [ ] Volume changes apply without audible clicks.
- [ ] Shuffle toggles queue order randomization.
- [ ] Repeat cycles through no repeat → repeat all → repeat one.
- [ ] Queue position indicator shows "current / total" format.
- [ ] Control bar is always visible at the bottom of the window.
- [ ] Stop button stops playback and resets position. (Planned — verify when implemented.)
- [ ] Audio device disconnect pauses playback gracefully.
