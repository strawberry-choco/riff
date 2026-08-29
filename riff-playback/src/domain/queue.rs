use crate::domain::playback::{RepeatMode};
use fastrand::Rng;
use riff_persistence::track::TrackId;
use std::collections::VecDeque;

/// Manages the playback queue and shuffle/repeat state.
///
/// Shuffle order maintenance is LAZY (allocation plan 4.4): mutations mark
/// the order dirty instead of regenerating an O(n) permutation each time,
/// and the next [`Self::advance`] rebuilds it once. Between a mutation and
/// that advance, [`Self::upcoming`] reports the stale-but-valid previous
/// order — the same trade the plan sanctions. The order itself lives in a
/// `VecDeque`, so consuming it during playback pops the front in O(1)
/// instead of memmoving the whole tail per advance.
#[derive(Debug, Clone, Default)]
pub struct PlaybackQueue {
    pub tracks: Vec<TrackId>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Upcoming play order while shuffling (indices into `tracks`,
    /// excluding the current track). A `VecDeque` so advancing pops the
    /// front in O(1).
    pub shuffled_indices: VecDeque<usize>,
    pub shuffle_history: Vec<usize>,
    /// Set by mutations while shuffle is engaged; the order regenerates on
    /// the next `advance`. Private so the invariant (dirty ⇒ stale order)
    /// cannot be broken from outside.
    shuffle_dirty: bool,
}

impl PlaybackQueue {
    /// Create a new queue from the given tracks.
    pub fn new(tracks: Vec<TrackId>) -> Self {
        let mut q = Self::default();
        q.tracks = tracks;
        if !q.tracks.is_empty() {
            q.current_index = Some(0);
        }
        q
    }

    /// Clear the queue.
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
        self.shuffled_indices.clear();
        self.shuffle_history.clear();
        self.shuffle_dirty = false;
    }

    /// Append a track to the end of the queue.
    pub fn append(&mut self, track: TrackId) {
        let was_empty = self.tracks.is_empty();
        self.tracks.push(track);
        if was_empty {
            self.current_index = Some(0);
        }
        if self.shuffle {
            self.touch_shuffle();
        }
    }

    /// Append a batch in one mutation. With lazy regeneration this costs
    /// one O(n) reorder at the next advance instead of one per track
    /// (allocation plan 4.3).
    pub fn append_many(&mut self, tracks: Vec<TrackId>) {
        let was_empty = self.tracks.is_empty();
        self.tracks.extend(tracks);
        if was_empty && !self.tracks.is_empty() {
            self.current_index = Some(0);
        }
        if self.shuffle {
            self.touch_shuffle();
        }
    }

    /// Insert a track as the next to play (after current).
    pub fn insert_next(&mut self, track: TrackId) {
        let idx = self.current_index.map_or(0, |i| i + 1);
        self.tracks.insert(idx, track);
        if self.shuffle {
            self.touch_shuffle();
        }
    }

    /// Remove a track at the given index.
    pub fn remove(&mut self, index: usize) {
        if index >= self.tracks.len() {
            return;
        }
        self.tracks.remove(index);
        if self.shuffle {
            self.touch_shuffle();
        }
        if let Some(ci) = self.current_index {
            if ci >= index {
                if ci == index && self.tracks.is_empty() {
                    self.current_index = None;
                } else if ci > 0 {
                    self.current_index = Some(ci - 1);
                }
            }
        }
    }

    /// Enable or disable shuffle, regenerating the order lazily.
    pub fn set_shuffle(&mut self, enabled: bool) {
        if enabled == self.shuffle {
            return;
        }
        self.shuffle = enabled;
        if enabled {
            self.touch_shuffle();
        } else {
            self.shuffled_indices.clear();
            self.shuffle_history.clear();
            self.shuffle_dirty = false;
        }
    }

    /// Toggle repeat mode: None → All → One → None.
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
    ///
    /// In shuffle mode this is where a dirty order regenerates (lazy
    /// regeneration, allocation plan 4.4); consuming the order pops its
    /// front in O(1).
    pub fn advance(&mut self) -> Option<&TrackId> {
        if self.tracks.is_empty() {
            return None;
        }

        let next_idx = if self.shuffle {
            if self.shuffle_dirty {
                self.regenerate_shuffle();
            } else if self.shuffled_indices.is_empty() {
                if self.repeat == RepeatMode::All {
                    self.regenerate_shuffle();
                } else {
                    return None;
                }
            }
            self.shuffled_indices.front().copied()
        } else {
            self.current_index
                .map(|i| i + 1)
                .filter(|&i| i < self.tracks.len())
        };

        if let Some(idx) = next_idx {
            self.current_index = Some(idx);
            if self.shuffle {
                self.shuffle_history.push(idx);
                self.shuffled_indices.pop_front();
            }
            self.tracks.get(idx)
        } else if self.repeat == RepeatMode::All {
            self.current_index = Some(0);
            if self.shuffle {
                self.regenerate_shuffle();
                if let Some(idx) = self.shuffled_indices.front().copied() {
                    self.current_index = Some(idx);
                    self.shuffle_history.push(idx);
                    self.shuffled_indices.pop_front();
                }
            }
            self.tracks.first()
        } else {
            None
        }
    }

    /// Go to the previous track (respecting shuffle/repeat state).
    pub fn previous(&mut self) -> Option<&TrackId> {
        if self.tracks.is_empty() {
            return None;
        }

        let prev_idx = if self.shuffle {
            if self.shuffle_history.is_empty() {
                if self.repeat == RepeatMode::All {
                    self.regenerate_shuffle();
                } else {
                    return None;
                }
            }
            self.shuffle_history.last().copied()
        } else {
            self.current_index
                .and_then(|i| i.checked_sub(1))
        };

        if let Some(idx) = prev_idx {
            self.current_index = Some(idx);
            if self.shuffle {
                self.shuffled_indices.push_front(idx);
                self.shuffle_history.pop();
            }
            self.tracks.get(idx)
        } else if self.repeat == RepeatMode::All {
            let idx = self.tracks.len() - 1;
            self.current_index = Some(idx);
            if self.shuffle {
                self.regenerate_shuffle();
                if let Some(idx) = self.shuffled_indices.back().copied() {
                    self.current_index = Some(idx);
                    self.shuffled_indices.push_back(idx);
                    self.shuffle_history.pop();
                }
            }
            self.tracks.last()
        } else {
            None
        }
    }

    /// Current track id.
    pub fn current_track(&self) -> Option<&TrackId> {
        self.current_index.and_then(|i| self.tracks.get(i))
    }

    /// Upcoming tracks in queue order (respects shuffle).
    pub fn upcoming(&self, count: usize) -> Vec<&TrackId> {
        let mut out = Vec::with_capacity(count);
        if self.tracks.is_empty() {
            return out;
        }
        if self.shuffle {
            let mut iter = self.shuffled_indices.iter();
            if let Some(ci) = self.current_index {
                // Skip the current track in shuffled order
                iter.next();
            }
            for idx in iter.take(count) {
                if let Some(t) = self.tracks.get(*idx) {
                    out.push(t);
                }
            }
        } else {
            let start = self.current_index.map_or(0, |i| i + 1);
            for t in self.tracks.iter().skip(start).take(count) {
                out.push(t);
            }
        }
        out
    }

    /// Mark the shuffle order stale after a mutation. Regeneration stays
    /// deferred to the next `advance` (allocation plan 4.4); with shuffle
    /// off there is no order to invalidate.
    fn touch_shuffle(&mut self) {
        if self.shuffle {
            self.shuffle_dirty = true;
        }
    }

    fn regenerate_shuffle(&mut self) {
        self.shuffled_indices.clear();
        self.shuffle_history.clear();
        self.shuffle_dirty = false;
        if self.tracks.is_empty() {
            return;
        }
        let n = self.tracks.len();
        let mut rng = Rng::new();
        let mut indices: Vec<usize> = (0..n).collect();
        if let Some(ci) = self.current_index {
            // Put current index at position 0 in the permutation
            indices.swap(0, ci);
        }
        // Fisher-Yates shuffle the rest
        for i in 1..n {
            let j = rng.usize(i..n);
            indices.swap(i, j);
        }
        // Now `indices[0]` is the current track; the rest are upcoming
        for idx in indices.into_iter().skip(1) {
            self.shuffled_indices.push_back(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riff_persistence::track::TrackId;

    fn make_id(s: &str) -> TrackId {
        TrackId(s.to_string())
    }

    #[test]
    fn advance_empty_queue_returns_none() {
        let mut q = PlaybackQueue::default();
        assert_eq!(q.advance(), None);
    }

    #[test]
    fn advance_single_track_repeat_none_returns_none() {
        let mut q = PlaybackQueue::new(vec![make_id("a")]);
        // First advance moves to next (none), returns None
        assert_eq!(q.advance(), None);
    }

    #[test]
    fn advance_single_track_repeat_all_loops() {
        let mut q = PlaybackQueue::new(vec![make_id("a")]);
        q.repeat = RepeatMode::All;
        // First advance moves to next (wraps to same track with repeat All)
        assert_eq!(q.advance(), Some(&make_id("a")));
        assert_eq!(q.advance(), Some(&make_id("a")));
    }

    #[test]
    fn advance_multiple_tracks() {
        let mut q = PlaybackQueue::new(vec![make_id("a"), make_id("b"), make_id("c")]);
        // First advance returns second track
        assert_eq!(q.advance(), Some(&make_id("b")));
        assert_eq!(q.advance(), Some(&make_id("c")));
        assert_eq!(q.advance(), None);
    }

    #[test]
    fn advance_repeat_all_loops() {
        let mut q = PlaybackQueue::new(vec![make_id("a"), make_id("b")]);
        q.repeat = RepeatMode::All;
        // First advance returns second track
        assert_eq!(q.advance(), Some(&make_id("b")));
        // Second advance wraps to first
        assert_eq!(q.advance(), Some(&make_id("a")));
        assert_eq!(q.advance(), Some(&make_id("b")));
    }
}