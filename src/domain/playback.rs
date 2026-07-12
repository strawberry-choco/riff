/// Playback state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Repeat mode for the queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RepeatMode {
    #[default]
    None,
    All,
    One,
}

/// Playback position information.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaybackPosition {
    pub current: std::time::Duration,
    pub total: Option<std::time::Duration>,
}

/// Commands sent to the playback engine.
#[derive(Debug, Clone)]
pub enum PlaybackCommand {
    Play(crate::domain::TrackId),
    Pause,
    Resume,
    Stop,
    Seek(std::time::Duration),
    SetVolume(f32),
    Next,
    Previous,
    ToggleVisibility,
    PlayNext(crate::domain::TrackId),
    AddToQueue(crate::domain::TrackId),
    PlayPause,
}

/// Updates sent from the playback engine to the UI.
#[derive(Debug, Clone)]
pub enum PlaybackUpdate {
    StateChanged(PlaybackState),
    PositionChanged(PlaybackPosition),
    TrackChanged(crate::domain::TrackId),
    TrackEnded,
    Error(String),
}
