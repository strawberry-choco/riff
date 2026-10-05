use crate::domain::{PlaybackPosition, PlaybackQueue, PlaybackState};

/// Which `ReplayGain` pair a Track plays under. The `ReplayGain` toggle turns
/// application off entirely; the Mode has no off of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayGainMode {
    /// Apply each Track's own measured pair.
    #[default]
    Track,
    /// Apply the Album's shared pair to every Track of the Album, preserving
    /// the Album's internal dynamics; falls back to the Track pair when the
    /// Album value is absent.
    Album,
}

/// Compute the linear playback-gain multiplier for `ReplayGain` (Task 4.3).
///
/// Disabled, or the Mode's pair carries no gain → `1.0` (no adjustment).
/// Track mode uses the track pair; Album mode uses the album pair, falling
/// back to the track pair when the album gain is absent. The dB value is
/// converted to a linear factor (`10^(dB/20)`). When the Mode's peak is known
/// and positive, the factor is capped at `1.0 / peak` so `factor * peak <= 1.0`
/// and amplified samples cannot clip — the peak follows the pair, so Album
/// mode is protected exactly as Track mode is. A gain that is not finite —
/// the recorded verdict for a Track with no signal — carries no loudness
/// information and is treated like no gain, so the factor this returns is
/// always finite. Pure f32 math — no external crates.
pub fn replaygain_factor(
    enabled: bool,
    mode: ReplayGainMode,
    track_gain: Option<f32>,
    track_peak: Option<f32>,
    album_gain: Option<f32>,
    album_peak: Option<f32>,
) -> f32 {
    if !enabled {
        return 1.0;
    }
    // Album mode falls back to the track pair when the album value is
    // absent; the peak follows the pair.
    let (gain_db, peak) = if mode == ReplayGainMode::Album && album_gain.is_some() {
        (album_gain, album_peak)
    } else {
        (track_gain, track_peak)
    };
    let Some(g) = gain_db else {
        return 1.0;
    };
    // A Track with no signal is measured as an infinite gain, and the peak cap
    // below cannot rescue it (a silent Track's peak is 0.0, so `1.0 / peak` is
    // itself infinite). The factor is multiplied into every sample in the audio
    // callback, where an infinite one turns digital silence into NaN and quiet
    // content into full-scale infinities — non-finite samples at the device.
    if !g.is_finite() {
        return 1.0;
    }
    let mut linear = 10f32.powf(g / 20.0);
    if let Some(p) = peak
        && p > 0.0
    {
        linear = linear.min(1.0 / p);
    }
    linear
}

/// How far a hand-typed `ReplayGain` gain may reach, in either direction
/// (the Inline Tag Editor clamps its gain inputs into this range). Wide
/// enough for any real measurement; narrow enough that a typo cannot
/// command an extreme playback factor. Lives beside the factor math whose
/// input it bounds.
pub const REPLAYGAIN_GAIN_LIMIT_DB: f32 = 51.0;

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
    /// engine applies the pair the [`ReplayGainMode`] selects (peak-capped)
    /// in the audio output's volume-scaling step.
    pub replaygain_enabled: bool,
    /// The listener's **`ReplayGain` Mode**: which pair the engine levels at the
    /// volume-scaling stage. A persisted preference the Settings surface owns,
    /// not a fact the play gesture decides — starting a queue as a whole-Album
    /// play leaves this untouched.
    pub replaygain_mode: ReplayGainMode,
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
            replaygain_mode: ReplayGainMode::default(),
        }
    }
}

impl PlaybackSession {
    /// Effective volume the engine should use (respects mute).
    pub fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.current_volume }
    }
}
