//! The minimal playback queue shape the playback-side read models ride on.

use crate::domain::{RepeatMode, TrackId};
use std::collections::VecDeque;

/// Minimal playback queue for projection use (avoids riff-playback dependency).
pub struct PlaybackQueue {
    pub tracks: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub shuffled_indices: VecDeque<usize>,
    pub shuffle_history: Vec<usize>,
    #[allow(dead_code, reason = "mirrors riff-playback's queue shape")]
    shuffle_dirty: bool,
}

impl PlaybackQueue {
    pub fn current_track(&self) -> Option<&TrackId> {
        self.current_index.and_then(|i| self.tracks.get(i))
    }

    pub fn upcoming(&self, limit: usize) -> Vec<&TrackId> {
        let mut out = Vec::with_capacity(limit);
        if self.tracks.is_empty() {
            return out;
        }
        if self.shuffle {
            let mut iter = self.shuffled_indices.iter();
            if let Some(_ci) = self.current_index {
                iter.next();
            }
            for idx in iter.take(limit) {
                if let Some(t) = self.tracks.get(*idx) {
                    out.push(t);
                }
            }
        } else {
            let start = self.current_index.map_or(0, |i| i + 1);
            for t in self.tracks.iter().skip(start).take(limit) {
                out.push(t);
            }
        }
        out
    }
}
