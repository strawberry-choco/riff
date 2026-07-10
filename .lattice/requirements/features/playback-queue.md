---
feature: Playback Queue
epic: Audio Engine
status: implemented
priority: P1
depends_on: ["Playback Control"]
personas: ["Music Listener"]
source_docs: []
implementation_notes: |
  Implemented in domain/queue.rs (PlaybackQueue) with support for track ordering,
  shuffle, repeat modes (none/all/one), and upcoming track peek.
---

# Playback Queue

## Problem Statement

Music listeners do not want to manually select each track after the current one finishes. They expect the player to automatically continue through an album or playlist, with the ability to reorder upcoming tracks, skip forward/backward, and optionally enable shuffle or repeat modes.

## User / Personas

- **Music Listener**: A person listening to an album or custom selection who wants continuous playback with easy control over what plays next.

## Scope

**In scope:**
- Maintain an ordered queue of tracks to play
- Automatically advance to the next track when the current one finishes
- Play the previous track on user request
- Add tracks to the end of the queue
- Remove tracks from the queue
- Reorder tracks in the queue via drag-and-drop (UI feature, but queue must support reordering)
- Shuffle mode: randomize upcoming track order
- Repeat modes: no repeat, repeat all, repeat one

**Out of scope:**
- Persistent playlist save/load (covered by deferred playlist management)
- Queue editing during playback of the track being removed (edge case: skip to next if current removed)
- Smart shuffle based on listening history
- Crossfade between queued tracks

## Boundary Conditions

- Queue should support at least 10,000 tracks without performance degradation
- When the last track finishes and repeat-all is off, playback stops and the queue remains intact
- When repeat-one is on and a track finishes, it restarts immediately
- When shuffle is toggled, the already-played history should not be reshuffled
- Removing the currently playing track should stop playback and advance to the next track

## Assumptions

- Queue state is maintained in memory only (no persistence in MVP)
- Track references in the queue are file paths (or internal IDs that resolve to paths)
- The queue can be modified from the UI thread and consumed from the audio thread safely

## Scenarios

### Scenario 1: Automatic track advance
A track finishes playing.

**Acceptance Criteria:**
- Given a queue with multiple tracks and the current track finishes, when playback reaches the end, then the next track in the queue begins playing within 1 second
- Given the last track in the queue finishes and repeat-all is off, when playback ends, then the player transitions to a stopped state with the queue unchanged

### Scenario 2: Manual next/previous
A user presses next or previous buttons.

**Acceptance Criteria:**
- Given a track is playing and the user presses next, when the action occurs, then playback jumps to the next queue track within 300ms
- Given a track is playing and the user presses previous, when the action occurs, then playback restarts the current track if >3s elapsed, otherwise jumps to the previous track

### Scenario 3: Enable shuffle
A user turns on shuffle mode.

**Acceptance Criteria:**
- Given a queue with 10 tracks and shuffle is off, when the user enables shuffle, then the upcoming track order is randomized
- Given shuffle is on and a track finishes, when the next track is selected, then it is chosen from the remaining unplayed tracks
- Given all tracks have played in shuffle mode, when the last track finishes and repeat-all is on, then the queue reshuffles and starts from the beginning

### Scenario 4: Enable repeat
A user cycles through repeat modes.

**Acceptance Criteria:**
- Given repeat is off and a track finishes, when it is the last track, then playback stops
- Given repeat-all is on and the last track finishes, when playback ends, then the queue restarts from the first track
- Given repeat-one is on and a track finishes, when playback ends, then the same track restarts from the beginning

## Implementation Notes

1. **Queue data structure**: A VecDeque or Vec of track identifiers with a current_index pointer. The queue owns the track list, the audio engine asks for the next track when current finishes.
2. **Shuffle implementation**: When shuffle is enabled, generate a shuffled index mapping. Keep played indices in history for "previous" navigation.
3. **Repeat implementation**: Repeat-all simply resets current_index to 0 when advancing past the end. Repeat-one keeps current_index unchanged on advance.
4. **Thread safety**: The queue lives in the UI/main thread. The audio thread sends a "track finished" message and receives the next track path in response.

## Open Questions

- [ ] Should the queue persist across application restarts in the MVP?
- [ ] How should we handle queue modification (add/remove/reorder) while audio is actively decoding the next track?

## Links
- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
