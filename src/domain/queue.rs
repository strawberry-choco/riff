use crate::domain::{RepeatMode, TrackId};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

/// Manages the playback queue and shuffle/repeat state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaybackQueue {
    pub tracks: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub shuffled_indices: Vec<usize>,
    pub shuffle_history: Vec<usize>,
}

impl PlaybackQueue {
    pub fn new(tracks: Vec<TrackId>) -> Self {
        Self {
            tracks,
            current_index: None,
            shuffle: false,
            repeat: RepeatMode::None,
            shuffled_indices: Vec::new(),
            shuffle_history: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
        self.shuffled_indices.clear();
        self.shuffle_history.clear();
    }

    pub fn append(&mut self, track: TrackId) {
        self.tracks.push(track);
        if self.shuffle {
            self.regenerate_shuffle();
        }
    }

    pub fn insert_next(&mut self, track: TrackId) {
        let insert_idx = self.current_index.map_or(0, |i| i + 1);
        if insert_idx <= self.tracks.len() {
            self.tracks.insert(insert_idx, track);
        } else {
            self.tracks.push(track);
        }
        if self.shuffle {
            self.regenerate_shuffle();
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.tracks.len() {
            self.tracks.remove(index);
            if let Some(current) = self.current_index {
                if index < current {
                    self.current_index = Some(current - 1);
                } else if index == current {
                    self.current_index = None;
                }
            }
            if self.shuffle {
                self.regenerate_shuffle();
            }
        }
    }

    pub fn set_shuffle(&mut self, enabled: bool) {
        if self.shuffle == enabled {
            return;
        }
        self.shuffle = enabled;
        if enabled {
            self.regenerate_shuffle();
        } else {
            self.shuffled_indices.clear();
            self.shuffle_history.clear();
        }
    }

    pub fn toggle_repeat(&mut self) {
        self.repeat = match self.repeat {
            RepeatMode::None => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::None,
        };
    }

    /// Advance to the next track (respecting shuffle/repeat state) and make
    /// it current. Named `advance` rather than `next` to avoid confusion
    /// with `std::iter::Iterator::next`.
    pub fn advance(&mut self) -> Option<&TrackId> {
        if self.tracks.is_empty() {
            return None;
        }

        let next_idx = if self.shuffle {
            if self.shuffled_indices.is_empty() {
                if self.repeat == RepeatMode::All {
                    self.regenerate_shuffle();
                } else {
                    return None;
                }
            }
            self.shuffled_indices.first().copied()
        } else {
            self.current_index
                .map(|i| i + 1)
                .filter(|&i| i < self.tracks.len())
        };

        if let Some(idx) = next_idx {
            self.current_index = Some(idx);
            if self.shuffle {
                self.shuffle_history.push(idx);
                if !self.shuffled_indices.is_empty() {
                    self.shuffled_indices.remove(0);
                }
            }
            self.tracks.get(idx)
        } else if self.repeat == RepeatMode::All {
            self.current_index = Some(0);
            if self.shuffle {
                self.regenerate_shuffle();
                if let Some(idx) = self.shuffled_indices.first().copied() {
                    self.current_index = Some(idx);
                    self.shuffle_history.push(idx);
                    self.shuffled_indices.remove(0);
                }
            }
            self.tracks.first()
        } else {
            None
        }
    }

    pub fn previous(&mut self) -> Option<&TrackId> {
        if self.tracks.is_empty() {
            return None;
        }

        if self.shuffle && !self.shuffle_history.is_empty() {
            if let Some(idx) = self.shuffle_history.pop() {
                self.current_index = Some(idx);
                return self.tracks.get(idx);
            }
        }

        if let Some(current) = self.current_index {
            if current > 0 {
                self.current_index = Some(current - 1);
                return self.tracks.get(current - 1);
            }
        }

        self.current_index = Some(0);
        self.tracks.first()
    }

    pub fn current_track(&self) -> Option<&TrackId> {
        self.current_index.and_then(|i| self.tracks.get(i))
    }

    pub fn upcoming(&self, count: usize) -> Vec<&TrackId> {
        let mut result = Vec::new();
        if self.tracks.is_empty() {
            return result;
        }

        let start = self.current_index.map_or(0, |i| i + 1);

        if self.shuffle && !self.shuffled_indices.is_empty() {
            for &idx in self.shuffled_indices.iter().take(count) {
                if let Some(track) = self.tracks.get(idx) {
                    result.push(track);
                }
            }
        } else {
            for i in start..(start + count).min(self.tracks.len()) {
                if let Some(track) = self.tracks.get(i) {
                    result.push(track);
                }
            }
        }

        result
    }

    fn regenerate_shuffle(&mut self) {
        let mut indices: Vec<usize> = (0..self.tracks.len()).collect();
        if let Some(current) = self.current_index {
            indices.retain(|&i| i != current);
        }
        let mut rng = rand::thread_rng();
        indices.shuffle(&mut rng);
        self.shuffled_indices = indices;
    }
}
