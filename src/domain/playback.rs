/// Playback state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Repeat mode for the queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
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
    /// Append a batch of tracks in one command, so the queue mutates once
    /// under one lock (folder "play all" enqueues N tracks without N lock
    /// round-trips and N shuffle regenerations).
    AddMany(Vec<crate::domain::TrackId>),
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
