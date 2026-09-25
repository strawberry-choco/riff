//! Playback read models: Up Next, current track, selected track.
//!
//! These projections resolve through the Application Store's `get_track`
//! query only when something they depend on moves — the Store generation
//! (a committed mutation) or the Playback Queue's shape (a `TrackChanged`
//! advance, Next/Previous/PlayNext/AddToQueue). Between such moves every
//! frame is served from cache without touching the store. Loader errors
//! propagate and leave the previous cache untouched — the next call retries.

use crate::app::errors::StoreError;
use crate::domain::PlaybackQueue;
use riff_persistence::store::{GenerationCache, StoreGeneration};
use riff_persistence::track::{Track, TrackId};

/// The playback slots plus the queue shape they were loaded for.
///
/// `Default` is what lets [`GenerationCache::level`] fill the bundle: an
/// all-empty value is never served, because `level` only asks `read` of a cache
/// that already holds an entry at the generation it observed.
#[derive(Clone, Default)]
struct PlaybackSlots {
    /// Queue shape the slots were loaded for: the (current index, upcoming
    /// ids at the window limit) pair. Recomputing this cheap stamp detects
    /// every queue mutation (advance, previous, insert-next, append,
    /// shuffle regeneration) without hooking each mutator.
    stamp: (Option<usize>, Vec<TrackId>),
    current: Option<Track>,
    up_next: Vec<Track>,
}

/// Session Projection for the playback-side reads: the current Track, the
/// Up Next window, and the track-details panel's selected Track.
///
/// Both caches declare themselves on [`GenerationCache::level`] — the same
/// primitive the collection capability's projections use, placed in the
/// persistence contract crate so this crate could adopt it without an edge
/// crossing the sibling split (see
/// `docs/adr/0002-ui-reads-the-store-through-session-projections.md`). What
/// these levels declare on top of the shared procedure is queue *shape*: the
/// slots' `read` answers only when the cached stamp still matches the live
/// queue, so a `level` miss means "the queue moved", not "the generation
/// moved". The two pure reads with no loader to run — [`Self::current`] and
/// [`Self::up_next`] — still guard a [`GenerationCache::peek`] by hand.
pub struct PlaybackProjection {
    /// Generation-keyed slot over the playback slots; the queue shape rides
    /// inside as part of the loaded state.
    slots: GenerationCache<(), PlaybackSlots>,
    /// Generation-keyed single-selection slot: a cached `None` means the id
    /// is known absent from the store, so a dangling selection does not
    /// requery per frame.
    selected: GenerationCache<TrackId, Option<Track>>,
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
    /// `None` while the slots are stale (the generation moved since the last
    /// successful [`Self::refresh`]) — the caller-side freshness guard of
    /// the canonical `GenerationCache::peek`.
    #[must_use]
    pub fn current(&self) -> Option<&Track> {
        if self.slots.loaded_at(self.slots.observe()) {
            self.slots.peek().and_then(|slots| slots.current.as_ref())
        } else {
            None
        }
    }

    /// The resolved Up Next window in Playback Queue order. Ids whose files
    /// left the library are skipped (the former mirror-reader behavior), so
    /// this can be shorter than the requested window. Empty while the slots
    /// are stale — the caller-side freshness guard of the canonical
    /// `GenerationCache::peek`.
    #[must_use]
    pub fn up_next(&self) -> &[Track] {
        if self.slots.loaded_at(self.slots.observe()) {
            self.slots
                .peek()
                .map_or(&[], |slots| slots.up_next.as_slice())
        } else {
            &[]
        }
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
        self.slots.level(
            &(),
            // Fresh-frame fast path: the bundle answers only for the queue
            // shape it was loaded for. The comparison stays by reference, so
            // the per-frame check materializes nothing — the stamp `Vec` is
            // only built below, when the inputs actually moved.
            |slots| {
                (slots.stamp.0 == queue.current_index
                    && upcoming_matches(&slots.stamp.1, queue, limit))
                .then_some(())
            },
            || {
                let stamp = (
                    queue.current_index,
                    queue
                        .upcoming(limit)
                        .into_iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                );

                // Fetch first, swap later: a failure anywhere leaves the
                // previous cache completely untouched.
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
                Ok::<_, StoreError>(PlaybackSlots {
                    stamp,
                    current: fetched_current,
                    up_next: fetched_up_next,
                })
            },
            |slots, fetched| *slots = fetched,
        )
    }

    /// The track-details panel's selected Track, cached until the selection
    /// or the generation moves. A cached `None` means the id is known absent
    /// from the store, so a dangling selection does not requery per frame —
    /// the answer lives in the slot, and `read` hands it out as-is.
    ///
    /// # Errors
    /// Propagates loader failures without touching the cache.
    pub fn selected_track(
        &mut self,
        id: &TrackId,
        loader: &mut dyn FnMut(&TrackId) -> Result<Option<Track>, StoreError>,
    ) -> Result<Option<Track>, StoreError> {
        self.selected.level(
            id,
            |cached| Some(cached.clone()),
            || loader(id),
            |slot, fetched| {
                slot.clone_from(&fetched);
                fetched
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PlaybackQueue;
    use riff_persistence::track::{Track, TrackId, TrackMetadata};
    use std::path::PathBuf;

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
            favorite: false,
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
        let new_queue = PlaybackQueue::new(vec![TrackId("a".into())]);
        proj.refresh(&new_queue, 2, &mut loader).unwrap();
        assert_eq!(proj.up_next().len(), 0);
    }
}
