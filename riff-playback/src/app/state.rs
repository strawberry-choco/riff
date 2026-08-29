use crate::domain::{PlaybackPosition, PlaybackQueue, PlaybackState};

/// Compute the linear playback-gain multiplier for `ReplayGain` (Task 4.3).
///
/// Disabled, or no gain tag → `1.0` (no adjustment). Otherwise the dB value is
/// converted to a linear factor (`10^(dB/20)`). When a peak is known and
/// positive, the factor is capped at `1.0 / peak` so `factor * peak <= 1.0`
/// and amplified samples cannot clip. Pure f32 math — no external crates.
pub fn replaygain_factor(enabled: bool, gain_db: Option<f32>, peak: Option<f32>) -> f32 {
    if !enabled {
        return 1.0;
    }
    let Some(g) = gain_db else {
        return 1.0;
    };
    let mut linear = 10f32.powf(g / 20.0);
    if let Some(p) = peak
        && p > 0.0
    {
        linear = linear.min(1.0 / p);
    }
    linear
}

/// The Playback Session: exactly the fields the audio engine, playback
/// coordinator, and transport touch. Lives behind its own `Arc<Mutex<>>`,
/// separate from the Library Session, so no code path ever holds both
/// session locks at once.
///
/// The UI reads this through a per-frame [`Clone`] snapshot and writes back
/// only the UI-owned fields (volume, mute, shuffle, repeat, replay-gain) at
/// frame end — the engine and coordinator write `playback_state`,
/// `current_position`, and the queue's traversal index, none of which the UI
/// ever mutates, so the targeted write-back cannot clobber them.
#[derive(Debug, Clone)]
pub struct PlaybackSession {
    pub queue: PlaybackQueue,
    pub playback_state: PlaybackState,
    pub current_position: PlaybackPosition,
    pub current_volume: f32,
    /// Mute flag: independent of `current_volume` — the slider keeps its
    /// value while muted. The engine always receives
    /// [`Self::effective_volume`], so a muted app stays silent until unmuted.
    pub muted: bool,
    /// `ReplayGain` flag: opt-in loudness normalization. When `true`, the
    /// engine applies each track's `REPLAYGAIN_TRACK_GAIN` (peak-capped) in
    /// the audio output's volume-scaling step.
    pub replaygain_enabled: bool,
}

impl Default for PlaybackSession {
    fn default() -> Self {
        Self {
            queue: PlaybackQueue::default(),
            playback_state: PlaybackState::Stopped,
            current_position: PlaybackPosition::default(),
            current_volume: 1.0,
            muted: false,
            replaygain_enabled: false,
        }
    }
}

impl PlaybackSession {
    /// Effective volume the engine should use (respects mute).
    pub fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.current_volume }
    }
}
