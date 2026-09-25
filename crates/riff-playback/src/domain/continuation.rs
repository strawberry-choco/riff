//! **Continuation** — the one answer to "what plays next".
//!
//! Pure over the [`PlaybackQueue`]: no store, no threads, no channels. A
//! track ending and a listener's skip are the same question asked by different
//! triggers, so they get their answer here and nowhere else; the queue's own
//! traversal (`advance` / `previous`) is private outside this module tree so a
//! second spelling of the policy cannot be written.
//!
//! The module owns the *move* as well as the answer: the returned track is
//! already current. What it deliberately does not own is the stop's
//! aftermath, because the two callers' stops are observably different — the
//! Playback Coordinator clears the current index and marks the session
//! stopped, while the Audio Engine leaves the last track current and tears
//! down its decoder and output. Collapsing those would change what the UI
//! shows after a skip past the end of the queue.
//!
//! # Contract
//!
//! [`Continuation::after`] is asked *after* the Playback Coordinator has
//! committed play history for the track that just ended. That ordering is
//! stated here rather than enforced at runtime: enforcing it would drag a
//! library-mutation port into the domain layer.

use crate::domain::queue::PlaybackQueue;
use riff_persistence::track::TrackId;

/// What prompted the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The current track ended and playback continues on its own.
    TrackEnded,
    /// A listener pressed Next.
    ManualNext,
    /// A listener pressed Previous.
    ManualPrevious,
}

/// What follows, with the queue already moved to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    /// This track is current now — play it.
    Play(TrackId),
    /// Nothing follows.
    Stop,
}

impl Continuation {
    /// Answer `trigger` against `queue`, moving it so the answer is current.
    pub fn after(queue: &mut PlaybackQueue, trigger: Trigger) -> Self {
        if queue.tracks.is_empty() {
            queue.current_index = None;
            return Self::Stop;
        }

        // Repeat-One loops the current track when a track ends on its own. A
        // listener's skip is that loop being overridden, so only `TrackEnded`
        // asks for the repeat.
        let next = match trigger {
            Trigger::TrackEnded if queue.repeats_one() => queue.current_track().cloned(),
            Trigger::TrackEnded | Trigger::ManualNext => queue.advance().cloned(),
            Trigger::ManualPrevious => queue.previous().cloned(),
        };

        match next {
            Some(id) => Self::Play(id),
            None => Self::Stop,
        }
    }

    /// What is current, given a `TrackId` a caller has already chosen: a tag
    /// edit that moved the playing track, or the Audio Engine announcing the
    /// track it switched to. Moves the queue's index onto `id` and answers
    /// with it. An id the queue does not hold changes nothing and answers
    /// `None` — the caller's choice stands, the queue simply has no slot for
    /// it.
    pub fn settled_on(queue: &mut PlaybackQueue, id: &TrackId) -> Option<TrackId> {
        let index = queue.tracks.iter().position(|queued| queued == id)?;
        queue.current_index = Some(index);
        Some(id.clone())
    }

    /// **Queue Fill** — the case where nothing is queued yet. The caller's
    /// `library` (every known Track, in the order the Application Store
    /// returned it: `ORDER BY path`) becomes the queue, and `wanted` becomes
    /// current. The fill is a case of continuation because it answers the same
    /// question — which Track is current, and in what order — so the ordering
    /// is decided here, not at the load site.
    ///
    /// Queue *mode* is a different question and stays the caller's: resetting
    /// shuffle when the queue is replaced is a mode write, not an ordering one.
    pub fn fill(queue: &mut PlaybackQueue, library: Vec<TrackId>, wanted: &TrackId) {
        let index = library.iter().position(|id| id == wanted);
        queue.tracks = library;
        queue.current_index = index;
    }
}
