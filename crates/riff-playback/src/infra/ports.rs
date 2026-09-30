//! Port traits for playback infrastructure.
//!
//! Infrastructure implementations (symphonia decoder, cpal output, and
//! `StorePlaybackLibrary` — the Application Store narrowed to
//! [`PlaybackLibrary`]) live in `riff-infra` and implement these traits. This
//! module also carries the Audio Engine's narrowed Library read port
//! ([`PlaybackLibrary`]) and the rule that decides *when* playback starts
//! ([`PlaybackStart`]).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app::state::PlaybackSession;
use crate::domain::PlaybackCommand;
use crate::domain::continuation::Continuation;
use riff_persistence::errors::StoreError;
use riff_persistence::sync::MutexExt;
use riff_persistence::track::{Track, TrackId};

/// The Audio Engine's Library read port, narrowed from the whole
/// [`LibraryQueryStore`](riff_persistence::store::LibraryQueryStore) to the two
/// reads playback actually makes.
///
/// * [`Self::get_track`] — the one Track behind a `TrackId`: the load, a
///   resume's re-open, and the gapless pre-decode;
/// * [`Self::library_track_ids`] — every Track id in the Library in the
///   Application Store's canonical path order (ADR 0003), which is what a
///   **Queue Fill** hands to [`Continuation`].
///
/// Nothing else: no listing window, no count, no artist/album/genre read, no
/// folder query, no smart playlist. The engine decides nothing about the
/// Library's shape, so its dependency on the Library says so.
///
/// **The narrowing is the adapter's, not the call site's.** The port has a real
/// implementation on each side of the seam: `StorePlaybackLibrary`
/// (`riff-infra`, wrapping the Application Store) in production, and a
/// two-method fake wherever the engine is driven without audio. It is
/// deliberately *not* `impl<T: LibraryQueryStore> PlaybackLibrary for T`
/// (ADR 0012). A blanket impl admits the store by implication — tidy at the
/// Composition Root — but it is a hypothetical seam: the narrowing becomes
/// observable only from inside this crate, and because it is the *only* way to
/// satisfy the port, every fake of the engine's dependency inherits all 35
/// `LibraryQueryStore` methods to exercise these two. A review should not
/// re-add one as an ergonomic convenience; it silently converts a real seam
/// back into a notional one.
pub trait PlaybackLibrary {
    /// The Library's `Track` for `id`, or `None` when unknown. The Application
    /// Store is the sole authority for track metadata — there is no in-memory
    /// copy to read instead.
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError>;

    /// Every Track id in the Library, path-ascending — the canonical flat
    /// ordering. A **Queue Fill** takes this list *verbatim*: the order is the
    /// store's contract, not the engine's, so nothing here re-sorts it.
    fn library_track_ids(&self) -> Result<Vec<TrackId>, StoreError>;
}

/// What a Playback Command is starting, as data. The rule that answers it lives
/// once, in [`PlaybackStart::enqueue_and_start_if_idle`], so a fourth
/// queue-mutating Playback Command is one variant here plus one line at its arm
/// — never a fourth copy of the rule.
pub enum QueueStart<'a> {
    /// A `Play` command: the engine is the load itself, so it has no queue
    /// write of its own. It may still answer a **Queue Fill**.
    Fill { wanted: &'a TrackId },
    /// Play `id` next, after the current Track.
    Next { id: &'a TrackId },
    /// Append `id` behind everything queued.
    Append { id: &'a TrackId },
    /// Append a batch behind everything queued as one write, under one lock —
    /// the whole collection's **Add to Queue**.
    AppendMany { ids: &'a [TrackId] },
    /// Nothing to queue and nothing to start: an empty batch.
    Nothing,
}

/// **The Audio Engine's rule for *when* playback starts** — the **Queue Fill**
/// trigger and the once-only idle auto-play, in one place, answering every
/// Playback Command that can start or grow the Playback Queue.
///
/// The split is deliberate: the *what* of a Queue Fill — which Tracks the queue
/// holds, and which of them is current — is [`Continuation`]'s answer, and this
/// type only decides the *when*. It is what makes the engine's own claim
/// literally true: the engine holds a Library port, and asks it for nothing
/// about queue order.
///
/// It reads three facts and holds no audio: the Playback Session, the engine's
/// own command channel (how a start is re-dispatched as a `Play` the engine
/// then loads), and the Library read port. That is also what makes the rule
/// callable on its own — no decoder, no output, no thread.
pub struct PlaybackStart {
    session: Arc<Mutex<PlaybackSession>>,
    cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
    library: Arc<dyn PlaybackLibrary + Send>,
}

impl PlaybackStart {
    /// Wire the rule over the shared session, the engine's command channel,
    /// and the Library read port.
    #[must_use]
    pub fn new(
        session: Arc<Mutex<PlaybackSession>>,
        cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
        library: Arc<dyn PlaybackLibrary + Send>,
    ) -> Self {
        Self {
            session,
            cmd_tx,
            library,
        }
    }

    /// The one shared operation: **the Playback Queue is empty, so fill it from
    /// the Library, and start playing exactly once.**
    ///
    /// * **The Queue Fill's when** — the queue is empty and a `Play` is
    ///   arriving, so the Library becomes the queue. Asked here and nowhere
    ///   else, and only when the queue is in fact empty, so the answer is never
    ///   fetched in order to be discarded.
    /// * **Idleness** — nothing is currently playing, so the Track a
    ///   queue-mutating command has just queued starts **exactly once**, as one
    ///   `Play` re-dispatched into the engine's own channel. Something already
    ///   playing means the mutation only queues, which is what **Add to Queue**
    ///   means.
    ///
    /// Both answers are taken under one lock, so a batch is one mutation and
    /// the auto-play decision cannot be made against a queue another thread has
    /// changed in between.
    pub fn enqueue_and_start_if_idle(&self, start: QueueStart<'_>, now_playing: Option<&TrackId>) {
        let mut session = self.session.lock_or_recover();

        // The command's intent, answered once: its own single write to the
        // queue, and the Track it wants playing when nothing is.
        let start_id = match start {
            QueueStart::Fill { wanted } => {
                // **Queue Fill** — the Playback Queue is empty, so the Library
                // becomes it. The *what* (which Tracks, which is current) is
                // Continuation's answer, taken from the store's own order
                // verbatim; the *when* (an empty queue) is this line, and it is
                // the only place it is written. Shuffle is queue *mode*, not
                // order, so replacing the queue resets it here. The engine is
                // the load itself, so nothing is re-dispatched.
                if session.queue.tracks.is_empty()
                    && let Ok(library) = self.library.library_track_ids()
                    && !library.is_empty()
                {
                    Continuation::fill(&mut session.queue, library, wanted);
                    session.queue.set_shuffle(false);
                }
                None
            }
            QueueStart::Nothing => None,
            QueueStart::Next { id } => {
                session.queue.insert_next(id.clone());
                Some(id.clone())
            }
            QueueStart::Append { id } => {
                session.queue.append(id.clone());
                Some(id.clone())
            }
            QueueStart::AppendMany { ids } => {
                // One write, one lock, one shuffle regeneration for the batch.
                session.queue.append_many(ids.to_vec());
                ids.first().cloned()
            }
        };

        // Idleness is "nothing is currently playing" — the engine's own
        // current Track, which only its load sets and clears.
        if now_playing.is_none()
            && let Some(id) = start_id
        {
            // Exactly once. The `Play` is re-dispatched rather than performed
            // here so every start goes through the engine's one load path.
            let _ = self.cmd_tx.send(PlaybackCommand::Play(id));
        }
    }
}

/// Factory for audio decoders: mints a fresh [`AudioDecoder`] on every call.
/// The audio engine uses it for both its primary decoder and the gapless
/// pre-decode decoder, so each owns independent codec state.
pub type DecoderFactory = Box<dyn Fn() -> Box<dyn AudioDecoder> + Send>;

/// Trait for audio decoders (implemented by infrastructure).
pub trait AudioDecoder: Send {
    /// The source path this decoder is reading from.
    fn source_path(&self) -> &std::path::Path;

    /// Initialize the decoder for the given track. Returns the audio format
    /// info (sample rate, channels) or an error if the file cannot be read.
    fn init(
        &mut self,
        path: &std::path::Path,
    ) -> Result<AudioFormatInfo, crate::app::errors::PlaybackError>;

    /// Decode the next chunk of audio into `buf` (interleaved f32 samples).
    /// Returns the number of samples written, or `None` at EOF.
    fn next_frames(&mut self, buf: &mut [f32]) -> Option<usize>;

    /// Seek to the given position. Returns the actual position seeked to.
    fn seek(&mut self, position: Duration) -> Duration;

    /// Get the total duration of the source, if known.
    fn duration(&self) -> Option<Duration>;
}

/// Audio format information returned by decoder.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFormatInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioFormatInfo {
    /// Check if this format is compatible with another for gapless handoff.
    #[must_use]
    pub fn compatible_with(&self, other: &AudioFormatInfo) -> bool {
        self.sample_rate == other.sample_rate && self.channels == other.channels
    }
}

/// Trait for audio output (implemented by infrastructure).
pub trait AudioOutput: Send {
    /// Start the output stream with the given format.
    fn start(&mut self, format: AudioFormatInfo) -> Result<(), crate::app::errors::PlaybackError>;

    /// Write decoded audio samples to the output.
    /// Returns the number of samples accepted (may be less than input if buffer is full).
    fn write(&mut self, samples: &[f32]) -> usize;

    /// Stop the output stream.
    fn stop(&mut self);

    /// Set the output volume (0.0–1.0).
    fn set_volume(&mut self, volume: f32);

    /// Set the `ReplayGain` linear factor applied alongside volume in the
    /// sample-scaling step. Default no-op so mocks need not implement it;
    /// `1.0` means no adjustment.
    fn set_replaygain(&mut self, _factor: f32) {}
}
