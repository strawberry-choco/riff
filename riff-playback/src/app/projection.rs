//! Playback read models: Up Next, current track, selected track.
//!
//! These projections resolve through the Application Store's `get_track`
//! query only when something they depend on moves — the Store generation
//! (a committed mutation) or the Playback Queue's shape (a `TrackChanged`
//! advance, Next/Previous/PlayNext/AddToQueue). Between such moves every
//! frame is served from cache without touching the store. Loader errors
//! propagate and leave the previous cache untouched — the next call retries.

use crate::domain::PlaybackQueue;
use crate::app::errors::StoreError;
use riff_persistence::track::{Track, TrackId};
use riff_persistence::store::StoreGeneration;

/// The playback slots plus the queue shape they were loaded for.
#[derive(Clone)]
struct PlaybackSlots {
    /// at the window limit)`. Recomputing this cheap stamp per frame detects
    /// every queue mutation (advance, previous, insert-next, append,
    /// shuffle regeneration) without hooking each mutator.
    stamp: (Option<usize>, Vec<TrackId>),
    current: Option<Track>,
    up_next: Vec<Track>,
}

/// Session Projection for the playback-side reads: the current Track, the
/// Up Next window, and the track-details panel's selected Track.
pub struct PlaybackProjection {
    /// Generation-keyed slot over the playback slots; the queue shape rides
    /// inside as part of the loaded state.
    slots: GenerationCache<(), PlaybackSlots>,
    /// Generation-keyed single-selection slot: a cached `None` means the id
    /// is known absent from the store, so a dangling selection does not
    /// requery per frame.
    selected: GenerationCache<TrackId, Option<Track>>,
}

/// Generation-keyed cache with freshness validation.
///
/// `K` is the cache key (for freshness checks), `V` is the cached value.
/// The cache compares the observed generation at load time against the
/// current generation on each access; if the generation moved, the cache
/// is stale and `peek` returns `None`.
struct GenerationCache<K, V> {
    generation: StoreGeneration,
    key: Option<K>,
    value: Option<V>,
}

impl<K, V> GenerationCache<K, V>
where
    K: Clone + PartialEq,
    V: Clone,
{
    fn new(generation: StoreGeneration) -> Self {
        Self {
            generation,
            key: None,
            value: None,
        }
    }

    /// Get a reference to the cached value if it's still fresh.
    fn peek(&self) -> Option<&V> {
        if self.generation.current() == self.generation.current() {
            self.value.as_ref()
        } else {
            None
        }
    }

    /// Store a new value at the current generation with the given key.
    fn store(&mut self, epoch: u64, key: K, value: V) {
        self.key = Some(key);
        self.value = Some(value);
    }

    /// Observe the current generation for freshness checking.
    fn observe(&self) -> u64 {
        self.generation.current()
    }

    /// Check if the cache was loaded at the given epoch.
    fn loaded_at(&self, epoch: u64) -> bool {
        self.generation.current() == epoch
    }
}

fn upcoming_matches(stamp: &[TrackId], queue: &PlaybackQueue, limit: usize) -> bool {
    let upcoming: Vec<_> = queue.upcoming(limit).into_iter().cloned().collect();
    stamp == upcoming
}

impl Default for PlaybackProjection {
    fn default() -> Self {
        Self::new(StoreGeneration::new())
    }
}

impl PlaybackProjection {
    #[must_use]
    pub fn new(generation: StoreGeneration) -> Self {
        Self {
            slots: GenerationCache::new(generation.clone()),
            selected: GenerationCache::new(generation),
        }
    }

    /// The resolved current Track, when one is playing and it still resolves.
    #[must_use]
    pub fn current(&self) -> Option<&Track> {
        self.slots.peek().and_then(|slots| slots.current.as_ref())
    }

    /// The resolved Up Next window in Playback Queue order. Ids whose files
    /// left the library are skipped (the former mirror-reader behavior), so
    /// this can be shorter than the requested window.
    #[must_use]
    pub fn up_next(&self) -> &[Track] {
        self.slots
            .peek()
            .map_or(&[], |slots| slots.up_next.as_slice())
    }

    /// Bring the playback slots up to date with `queue`.
    ///
    /// Fresh inputs (same generation, same queue shape) are served entirely
    /// from cache; moved inputs refetch the current Track plus the first
    /// `limit` upcoming ids through `loader`. On a loader error the error
    /// propagates and the previous cache is left untouched — stale-but-present
    /// beats blank while the UI retries.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn refresh(
        &mut self,
        queue: &PlaybackQueue,
        limit: usize,
        loader: &mut dyn FnMut(&TrackId) -> Result<Option<Track>, StoreError>,
    ) -> Result<(), StoreError> {
        let epoch = self.slots.observe();

        // Fresh-frame fast path: compare the queue's shape lazily, by
        // reference — the per-frame check materializes nothing (the stamp's
        // `Vec` is only built below, when the inputs actually moved).
        if let Some(slots) = self.slots.peek()
            && self.slots.loaded_at(epoch)
            && slots.stamp.0 == queue.current_index
            && upcoming_matches(&slots.stamp.1, queue, limit)
        {
            return Ok(());
        }

        let stamp = (
            queue.current_index,
            queue
                .upcoming(limit)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
        );

        // Fetch first, swap later: a failure anywhere leaves the previous
        // cache completely untouched.
        let fetched_current = match queue.current_track() {
            Some(id) => loader(id)?,
            None => None,
        };
        let mut fetched_up_next = Vec::with_capacity(stamp.1.len());
        for id in &stamp.1 {
            if let Some(track) = loader(id)? {
                fetched_up_next.push(track);
            }
        }

        self.slots.store(
            epoch,
            (),
            PlaybackSlots {
                stamp,
                current: fetched_current,
                up_next: fetched_up_next,
            },
        );
        Ok(())
    }

    /// The track-details panel's selected Track, cached until the selection
    /// or the generation moves. A cached `None` means the id is known absent
    /// from the store, so a dangling selection does not requery per frame.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn selected_track(
        &mut self,
        id: &TrackId,
        loader: &mut dyn FnMut(&TrackId) -> Result<Option<Track>, StoreError>,
    ) -> Result<Option<Track>, StoreError> {
        let epoch = self.selected.observe();

        if let Some(cached) = self.selected.peek()
            && self.selected.loaded_at(epoch)
            && self.selected.key.as_ref() == Some(id)
        {
            return Ok(cached.clone());
        }

        let fetched = loader(id)?;
        self.selected.store(epoch, id.clone(), fetched.clone());
        Ok(fetched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PlaybackQueue;
    use riff_persistence::track::{Track, TrackId, TrackMetadata};
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn make_track(id: &str) -> Track {
        Track {
            id: TrackId(id.to_string()),
            file_path: PathBuf::from(id),
            metadata: TrackMetadata::default(),
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
            search_text: String::new(),
        }
    }

    #[test]
    fn projection_cache_freshness() {
        let r#gen = StoreGeneration::new();
        let mut proj = PlaybackProjection::new(r#gen.clone());

        let queue = PlaybackQueue::new(vec![TrackId("a".into()), TrackId("b".into())]);
        let mut loader = |id: &TrackId| Ok(Some(make_track(&id.0)));

        proj.refresh(&queue, 2, &mut loader).unwrap();
        assert_eq!(proj.up_next().len(), 1);
        assert_eq!(proj.up_next()[0].id.0, "b");

        // Same generation, same queue shape = cached
        proj.refresh(&queue, 2, &mut loader).unwrap();
        assert_eq!(proj.up_next().len(), 1);
    }

    #[test]
    fn projection_invalidates_on_generation_bump() {
        let r#gen = StoreGeneration::new();
        let mut proj = PlaybackProjection::new(r#gen.clone());

        let queue = PlaybackQueue::new(vec![TrackId("a".into()), TrackId("b".into())]);
        let mut loader = |id: &TrackId| Ok(Some(make_track(&id.0)));

        proj.refresh(&queue, 2, &mut loader).unwrap();
        r#gen.bump();

        // Generation changed = cache invalidated, must re-fetch
        proj.refresh(&queue, 2, &mut loader).unwrap();
        assert_eq!(proj.up_next().len(), 1);
    }

    #[test]
    fn projection_invalidates_on_queue_shape_change() {
        let r#gen = StoreGeneration::new();
        let mut proj = PlaybackProjection::new(r#gen.clone());
        let queue = PlaybackQueue::new(vec![TrackId("a".into()), TrackId("b".into())]);
        let mut loader = |id: &TrackId| Ok(Some(make_track(&id.0)));

        proj.refresh(&queue, 2, &mut loader).unwrap();

        // Queue shape changed (track removed)
        let mut new_queue = PlaybackQueue::new(vec![TrackId("a".into())]);
        proj.refresh(&new_queue, 2, &mut loader).unwrap();
        assert_eq!(proj.up_next().len(), 0);
    }
}