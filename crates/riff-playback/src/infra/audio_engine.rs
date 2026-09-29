//! The audio engine: turns [`PlaybackCommand`]s into decoded audio and
//! [`PlaybackUpdate`]s.
//!
//! A deep module behind the port seams ([`AudioDecoder`] via a
//! [`DecoderFactory`], [`AudioOutput`], [`PlaybackLibrary`]): it owns
//! decode scheduling with backpressure, output startup, `ReplayGain`
//! resolution, command re-dispatch, and the gapless
//! pre-decode/handoff machinery — everything else is private implementation.
//!
//! It decides nothing about queue order. The *when* of a **Queue Fill** and of
//! the once-only idle auto-play is one shared operation, [`PlaybackStart`],
//! which every Playback Command that can start or grow the Playback Queue goes
//! through; the *what* — which Tracks a fill puts in the queue, which is
//! current, and what follows a skip — is **Continuation**'s answer. The engine
//! asks its Library port for one Track at a time and performs the load.
//!
//! Threading: the module exposes only the blocking [`AudioEngine::run`];
//! the Composition Root is the sole thread spawner and runs it on the
//! dedicated audio engine thread, and it is the only thing that asks the
//! loop to stop, through the flag it passes to [`AudioEngine::new`].
//!
//! Pure-Rust: uses only the port traits. Concrete decoder/output
//! implementations live in `riff-infra`.

use crate::app::state::{PlaybackSession, replaygain_factor};
use crate::domain::continuation::{Continuation, Trigger};
use crate::domain::{PlaybackCommand, PlaybackPosition, PlaybackState, PlaybackUpdate};
use crate::infra::ports::{
    AudioDecoder, AudioFormatInfo, AudioOutput, DecoderFactory, PlaybackLibrary, PlaybackStart,
    QueueStart,
};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use riff_persistence::sync::MutexExt;
use riff_persistence::track::TrackId;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Gapless (Task 4.1): how many seconds before EOF the engine starts
/// pre-decoding the successor track.
const PRE_ENCODE_SECONDS: f32 = 2.0;
/// Gapless (Task 4.1): max seconds of successor audio held in the pre-buffer
/// (~1.5 MB at 48 kHz stereo). Exactly one successor is buffered, so total
/// extra memory is bounded independently of queue length.
const PRE_BUFFER_SECONDS: f32 = 4.0;

/// Samples per decode chunk handed to [`AudioDecoder::next_frames`]. One
/// buffer is allocated per decode session and reused for every chunk
/// (allocation-optimization plan, task 3.2).
const DECODE_CHUNK_SAMPLES: usize = 4096;

/// While a track is loaded the engine polls for commands at this interval
/// between decode chunks, so a queued Pause/Seek lands immediately instead of
/// after the track decodes to EOF.
const COMMAND_POLL: Duration = Duration::from_millis(10);

/// The audio engine: owns the decoders and audio output, processes
/// [`PlaybackCommand`]s, and drives decode/position/gapless logic.
pub struct AudioEngine {
    cmd_rx: Receiver<PlaybackCommand>,
    /// A handle back onto the command channel, so queue-navigation commands
    /// (Next/Previous) and the idle auto-play re-dispatch a full `Play` instead
    /// of duplicating its stream-restart logic inline. The *when* of that
    /// re-dispatch is [`PlaybackStart`]'s, not the arm's.
    cmd_tx: Sender<PlaybackCommand>,
    update_tx: Sender<PlaybackUpdate>,
    /// The Library, narrowed to the Track lookups the engine's loads make.
    library: Arc<dyn PlaybackLibrary + Send>,
    /// The one shared operation for *when* playback starts — the **Queue
    /// Fill** trigger and the once-only idle auto-play. It is a field rather
    /// than a method so the rule is decidable without an audio port, a decoder
    /// factory, or a running loop.
    start: PlaybackStart,
    decoder_factory: DecoderFactory,
    output: Box<dyn AudioOutput + Send>,
    session: Arc<Mutex<PlaybackSession>>,
    /// Cooperative stop request from the Composition Root, honored between
    /// commands (see [`AudioEngine::run`]).
    stop: Arc<AtomicBool>,
}

impl AudioEngine {
    /// Construct an engine with the given ports and channels. `stop` is the
    /// Composition Root's cooperative stop request: the loop observes it
    /// between commands and returns, which is what lets the runtime shut the
    /// audio thread down without closing the command channel.
    #[must_use]
    // The engine's ports are its constructor's whole argument list; grouping
    // them would only add an indirection between the Composition Root and
    // the wiring it owns.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cmd_rx: Receiver<PlaybackCommand>,
        cmd_tx: Sender<PlaybackCommand>,
        update_tx: Sender<PlaybackUpdate>,
        library: Box<dyn PlaybackLibrary + Send>,
        decoder_factory: DecoderFactory,
        output: Box<dyn AudioOutput + Send>,
        session: Arc<Mutex<PlaybackSession>>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let library: Arc<dyn PlaybackLibrary + Send> = Arc::from(library);
        Self {
            cmd_rx,
            start: PlaybackStart::new(Arc::clone(&session), cmd_tx.clone(), Arc::clone(&library)),
            cmd_tx,
            update_tx,
            library,
            decoder_factory,
            output,
            session,
            stop,
        }
    }

    /// Run the engine's main loop. Blocks until the command channel is closed
    /// or a stop request arrives; either way the loop returns only after the
    /// command it has already received is fully processed, so a stop never
    /// truncates work in flight. This is the ONLY blocking entry point — the
    /// composition root spawns it on the dedicated audio thread.
    // The engine loop is one continuous command pump; splitting it would
    // scatter the shared decode state across many small helpers.
    #[allow(clippy::too_many_lines)]
    pub fn run(mut self) {
        let mut primary_decoder: Option<Box<dyn AudioDecoder>> = None;
        let mut output_started = false;
        let mut current_format: Option<AudioFormatInfo> = None;
        let mut current_track_id: Option<TrackId> = None;

        // Gapless pre-decode state
        let mut pre_decode_state = PreDecodeState::default();
        let pre_buffer_cap =
            |rate, ch| crate::app::gapless::pre_buffer_cap(rate, ch, PRE_BUFFER_SECONDS);
        // ReplayGain: resolved once at track load via `replaygain_factor`
        // and pushed to the audio output's port method, which multiplies
        // every sample in the callback. The write loop no longer scales
        // samples; the factor lives in the output's atomic.

        // Audio output callback - closure that pulls decoded samples
        let mut decode_buffer = vec![0.0f32; DECODE_CHUNK_SAMPLES];

        // Sample-accurate playback position for the current track.
        let mut position = Duration::ZERO;

        loop {
            // Always poll, never block indefinitely. The same 10 ms tick that
            // lets a queued command land between decode chunks (instead of
            // after EOF) is also what lets an IDLE engine observe a stop
            // request — a blocking `recv()` here would strand the thread
            // until the command channel closed.
            let cmd = match self.cmd_rx.recv_timeout(COMMAND_POLL) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };

            if let Some(cmd) = cmd {
                match cmd {
                    PlaybackCommand::Play(id) => {
                        // The Playback Queue is empty, so the Library becomes
                        // it: the *when* of a **Queue Fill** is the shared
                        // operation's, and Continuation owns the *what*. This
                        // command is the load itself, so it re-dispatches
                        // nothing — that half of the rule belongs to a
                        // queue-mutating command, not to this one.
                        self.start.enqueue_and_start_if_idle(
                            QueueStart::Fill { wanted: &id },
                            current_track_id.as_ref(),
                        );

                        // A dispatch of the already-current track (the
                        // coordinator's auto-Play after a handoff TrackEnded)
                        // is swallowed: the handoff already switched without a
                        // stream restart.
                        if current_track_id.as_ref() == Some(&id) && primary_decoder.is_some() {
                            continue;
                        }

                        // Stop current playback
                        if primary_decoder.is_some() {
                            primary_decoder.take();
                            if output_started {
                                self.output.stop();
                                output_started = false;
                            }
                            current_format = None;
                        }

                        // Clear pre-buffer on explicit Play
                        pre_decode_state = PreDecodeState::default();

                        // Load track metadata
                        let track = self.library.get_track(&id);
                        let Ok(Some(track)) = track else {
                            let _ = self
                                .update_tx
                                .send(PlaybackUpdate::Error(format!("Track not found: {}", id.0)));
                            continue;
                        };

                        // Initialize decoder
                        let mut decoder = (self.decoder_factory)();
                        let format = match decoder.init(&track.file_path) {
                            Ok(f) => f,
                            Err(e) => {
                                let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                                continue;
                            }
                        };

                        // Start output if format changed
                        if current_format.as_ref() != Some(&format) {
                            if output_started {
                                self.output.stop();
                            }
                            if let Err(e) = self.output.start(format.clone()) {
                                let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                                continue;
                            }
                            output_started = true;
                            current_format = Some(format.clone());
                        }

                        // Resolve ReplayGain for this track and push the
                        // linear factor to the audio output. The port method
                        // multiplies every sample in the callback, so the
                        // write loop below no longer scales samples.
                        {
                            let session = self.session.lock_or_recover();
                            let factor = replaygain_factor(
                                session.replaygain_enabled,
                                track.metadata.replaygain_track_gain,
                                track.metadata.replaygain_track_peak,
                            );
                            self.output.set_replaygain(factor);
                        }

                        current_track_id = Some(id.clone());
                        position = Duration::ZERO;
                        primary_decoder = Some(decoder);

                        // Emit track changed
                        let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(id));
                        let _ = self
                            .update_tx
                            .send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
                    }

                    PlaybackCommand::Pause => {
                        if output_started {
                            self.output.stop();
                            output_started = false;
                        }
                        let _ = self
                            .update_tx
                            .send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
                    }

                    PlaybackCommand::Resume => {
                        if let Some(ref format) = current_format {
                            if let Err(e) = self.output.start(format.clone()) {
                                let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                            } else {
                                output_started = true;
                                // Re-open the decoder and seek back to the
                                // recorded pause position, so the pre-pause
                                // audio is not replayed.
                                if let Some(decoder) = primary_decoder.as_mut()
                                    && let Some(ref id) = current_track_id
                                    && let Ok(Some(track)) = self.library.get_track(id)
                                    && decoder.init(&track.file_path).is_ok()
                                {
                                    let _ = decoder.seek(position);
                                }
                                if let Some(id) = current_track_id.clone() {
                                    // Re-announce the track: a paused UI session
                                    // lost its Now Playing binding.
                                    let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(id));
                                }
                                let _ = self
                                    .update_tx
                                    .send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
                            }
                        }
                    }

                    PlaybackCommand::Stop => {
                        // The decoder stays parked: an idle Seek (re-scrubbing
                        // the stopped track's position) still reaches it.
                        if output_started {
                            self.output.stop();
                            output_started = false;
                        }
                        current_format = None;
                        current_track_id = None;
                        pre_decode_state = PreDecodeState::default();
                        let _ = self
                            .update_tx
                            .send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
                    }

                    PlaybackCommand::Seek(pos) => {
                        if let Some(decoder) = primary_decoder.as_mut() {
                            let actual = decoder.seek(pos);
                            position = actual;
                            let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(
                                PlaybackPosition {
                                    current: actual,
                                    total: decoder.duration(),
                                },
                            ));
                        }
                    }

                    PlaybackCommand::SetVolume(vol) => {
                        self.output.set_volume(vol.clamp(0.0, 1.0));
                        // Session volume is updated by the coordinator/transport
                    }

                    PlaybackCommand::Next => self.skip(
                        SkipDirection::Forward,
                        &mut primary_decoder,
                        &mut current_format,
                        &mut output_started,
                        &mut current_track_id,
                    ),

                    PlaybackCommand::Previous => self.skip(
                        SkipDirection::Backward,
                        &mut primary_decoder,
                        &mut current_format,
                        &mut output_started,
                        &mut current_track_id,
                    ),

                    PlaybackCommand::PlayNext(id) => {
                        self.start.enqueue_and_start_if_idle(
                            QueueStart::Next { id: &id },
                            current_track_id.as_ref(),
                        );
                    }

                    PlaybackCommand::AddToQueue(id) => {
                        self.start.enqueue_and_start_if_idle(
                            QueueStart::Append { id: &id },
                            current_track_id.as_ref(),
                        );
                    }

                    PlaybackCommand::AddMany(ids) => {
                        // One write, one lock, one shuffle regeneration for the
                        // whole batch (folder "play all" enqueues N tracks), and
                        // the same idle auto-play as every other queue mutation.
                        self.start.enqueue_and_start_if_idle(
                            QueueStart::AppendMany { ids: &ids },
                            current_track_id.as_ref(),
                        );
                    }

                    PlaybackCommand::PlayPause => {
                        let state = self.session.lock_or_recover().playback_state;
                        match state {
                            PlaybackState::Playing => {
                                if output_started {
                                    self.output.stop();
                                    output_started = false;
                                }
                                let _ = self
                                    .update_tx
                                    .send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
                            }
                            PlaybackState::Paused => {
                                if let Some(ref format) = current_format {
                                    if let Err(e) = self.output.start(format.clone()) {
                                        let _ = self
                                            .update_tx
                                            .send(PlaybackUpdate::Error(e.to_string()));
                                    } else {
                                        output_started = true;
                                        let _ = self.update_tx.send(PlaybackUpdate::StateChanged(
                                            PlaybackState::Playing,
                                        ));
                                        if let Some(id) = current_track_id.clone() {
                                            // Re-announce the track: a paused UI
                                            // session lost its Now Playing binding.
                                            let _ = self
                                                .update_tx
                                                .send(PlaybackUpdate::TrackChanged(id));
                                        }
                                    }
                                }
                            }
                            PlaybackState::Stopped => {
                                // No-op
                            }
                        }
                    }
                }
            }

            // Stop only BETWEEN commands: whatever command was just received
            // has been fully processed by now, so the engine's self-dispatch
            // — Next / Previous / AddToQueue re-dispatching a fresh `Play`
            // through our own `cmd_tx` for the gapless handoff — can never be
            // truncated by a stop request.
            if self.stop.load(Ordering::Relaxed) {
                break;
            }

            // Decode one chunk per tick while the stream runs; the command
            // loop stays in charge between chunks, so queued commands are
            // honored mid-track and position updates flow while decoding.
            if output_started && let Some(decoder) = primary_decoder.as_mut() {
                match decoder.next_frames(&mut decode_buffer) {
                    Some(samples) if samples > 0 => {
                        // Write to audio output (blocking if buffer full)
                        if output_started {
                            self.output.write(&decode_buffer[..samples]);
                        }

                        // Sample-accurate position update for the UI.
                        let (rate, channels) = current_format
                            .as_ref()
                            .map_or((44_100, 2), |f| (f.sample_rate, f.channels));
                        position +=
                            crate::app::gapless::elapsed_from_samples(samples, rate, channels);
                        let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(
                            PlaybackPosition {
                                current: position,
                                total: decoder.duration(),
                            },
                        ));

                        // Pre-decode the successor once the track's tail is
                        // in range: a format-compatible successor hands off
                        // without stopping the stream. The DECODER's duration
                        // drives the window - the store's track row may carry
                        // no duration at all.
                        if pre_decode_state.pre_decoder.is_none()
                            && let Some(total) = decoder.duration()
                            && total.saturating_sub(position).as_secs_f32() <= PRE_ENCODE_SECONDS
                        {
                            let successor_id = {
                                let session = self.session.lock_or_recover();
                                session.queue.upcoming(1).first().map(|id| (*id).clone())
                            };
                            if let Some(next_id) = successor_id
                                && let Ok(Some(next_track)) = self.library.get_track(&next_id)
                            {
                                let mut successor = (self.decoder_factory)();
                                if let Ok(format) = successor.init(&next_track.file_path)
                                    && let Some(ref cur_format) = current_format
                                    && format.compatible_with(cur_format)
                                {
                                    pre_decode_state.format_compatible = true;
                                    pre_decode_state.has_successor = true;
                                    pre_decode_state.next_track_id = Some(next_id.clone());

                                    // Pre-decode up to the pre-buffer cap; the
                                    // decoder stays parked on the successor for
                                    // the post-handoff continuation.
                                    let cap = pre_buffer_cap(format.sample_rate, format.channels);
                                    let mut pre_buffer = Vec::with_capacity(cap);
                                    let mut buf = vec![0.0f32; DECODE_CHUNK_SAMPLES];
                                    while pre_buffer.len() < cap {
                                        match successor.next_frames(&mut buf) {
                                            Some(0) | None => break,
                                            Some(samples) => {
                                                pre_buffer.extend_from_slice(&buf[..samples]);
                                            }
                                        }
                                    }

                                    if !pre_buffer.is_empty() {
                                        pre_decode_state.samples = pre_buffer;
                                        pre_decode_state.pre_decoder = Some(successor);
                                        pre_decode_state.format = Some(format);
                                    }
                                }
                            }
                        }
                    }
                    Some(_) => {} // empty packet (codec padding)
                    None => {
                        self.handle_eof(
                            &mut primary_decoder,
                            &mut pre_decode_state,
                            &mut current_format,
                            &mut output_started,
                            &mut current_track_id,
                            &mut position,
                        );
                    }
                }
            }
        }
    }

    /// Handle end-of-track logic including gapless handoff.
    fn handle_eof(
        &mut self,
        primary_decoder: &mut Option<Box<dyn AudioDecoder>>,
        pre_decode_state: &mut PreDecodeState,
        current_format: &mut Option<AudioFormatInfo>,
        output_started: &mut bool,
        current_track_id: &mut Option<TrackId>,
        position: &mut Duration,
    ) {
        let session = self.session.lock_or_recover();
        let gapless_conditions = crate::app::gapless::GaplessConditions {
            shuffle: session.queue.shuffle,
            repeat_one: session.queue.repeats_one(),
            format_compatible: pre_decode_state.format_compatible,
            has_successor: pre_decode_state.has_successor,
        };

        let can_gapless = if session.queue.repeats_one() {
            crate::app::gapless::repeat_one_handoff_eligible(
                session.queue.shuffle,
                true,
                pre_decode_state.format_compatible,
                pre_decode_state.has_successor,
            )
        } else {
            crate::app::gapless::is_gapless_eligible(gapless_conditions)
        };

        if can_gapless && pre_decode_state.pre_decoder.is_some() {
            // Gapless handoff: flush the pre-buffered successor audio and
            // swap in its decoder WITHOUT stopping the output stream.
            if let Some(decoder) = pre_decode_state.pre_decoder.take() {
                if *output_started && !pre_decode_state.samples.is_empty() {
                    self.output.write(&pre_decode_state.samples);
                }
                // The finished track still ends: the coordinator commits its
                // play history and re-dispatches Play(successor), which the
                // engine swallows because the handoff already switched.
                let _ = self.update_tx.send(PlaybackUpdate::TrackEnded);
                if let Some(ref format) = pre_decode_state.format {
                    *position = crate::app::gapless::elapsed_from_samples(
                        pre_decode_state.samples.len(),
                        format.sample_rate,
                        format.channels,
                    );
                }
                *primary_decoder = Some(decoder);
                // Emit track changed for the new track
                if let Some(next_id) = pre_decode_state.next_track_id.take() {
                    *current_track_id = Some(next_id.clone());
                    let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(next_id));
                }
            }
            *pre_decode_state = PreDecodeState::default();
        } else {
            // Normal EOF — emit TrackEnded, coordinator handles continuation
            primary_decoder.take();
            if *output_started {
                self.output.stop();
                *output_started = false;
            }
            *current_format = None;
            let _ = self.update_tx.send(PlaybackUpdate::TrackEnded);
        }
    }

    /// Manual skip (Next/Previous): move the session queue — the one
    /// traversal state — and re-dispatch a full `Play` for the new current
    /// track, exactly as if the user had started it. When nothing follows in
    /// the requested direction, stop the stream and mark playback stopped,
    /// like the coordinator does at a queue end.
    fn skip(
        &mut self,
        direction: SkipDirection,
        primary_decoder: &mut Option<Box<dyn AudioDecoder>>,
        current_format: &mut Option<AudioFormatInfo>,
        output_started: &mut bool,
        current_track_id: &mut Option<TrackId>,
    ) {
        let next_id = {
            let mut session = self.session.lock_or_recover();
            let trigger = match direction {
                SkipDirection::Forward => Trigger::ManualNext,
                SkipDirection::Backward => Trigger::ManualPrevious,
            };
            match Continuation::after(&mut session.queue, trigger) {
                Continuation::Play(id) => Some(id),
                Continuation::Stop => None,
            }
        };
        if let Some(id) = next_id {
            let _ = self.cmd_tx.send(PlaybackCommand::Play(id));
            return;
        }
        if *output_started {
            self.output.stop();
            *output_started = false;
        }
        primary_decoder.take();
        *current_format = None;
        *current_track_id = None;
        let _ = self
            .update_tx
            .send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
    }
}

/// Which way a manual skip moves through the queue.
enum SkipDirection {
    Forward,
    Backward,
}

/// Gapless pre-decode state (Task 4.1). Purely additive: when a valid,
/// format-compatible successor has been pre-buffered, EOF hands off without
/// stopping the cpal stream; in EVERY other case these fields are ignored and
/// the existing gapped path runs unchanged.
#[derive(Default)]
struct PreDecodeState {
    /// Pre-decoded successor audio, flushed to the output at handoff.
    samples: Vec<f32>,
    /// The successor's decoder, advanced through the pre-buffered span; it
    /// continues decoding after the handoff.
    pre_decoder: Option<Box<dyn AudioDecoder>>,
    /// The format the samples were decoded at.
    format: Option<AudioFormatInfo>,
    format_compatible: bool,
    has_successor: bool,
    next_track_id: Option<TrackId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use riff_persistence::errors::StoreError;
    use riff_persistence::track::{Track, TrackMetadata};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn id(path: &str) -> TrackId {
        TrackId(path.to_string())
    }

    /// Nothing is currently playing — the one fact the idle auto-play reads.
    fn idle() -> Option<&'static TrackId> {
        None
    }

    fn track(path: &str) -> Track {
        Track {
            id: id(path),
            file_path: PathBuf::from(path),
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

    /// The Library as playback reads it: the narrowed port's two reads over a
    /// canned map, and nothing else — no listing window, no count, no
    /// artist/album/genre/folder read, because the engine asks for none. The
    /// rule never resolves a Track (the load does), but the port it is built on
    /// genuinely offers the lookup, so the fake implements that port rather than
    /// a trait narrowed to suit this test.
    struct FakeLibrary {
        tracks: HashMap<TrackId, Track>,
    }

    impl FakeLibrary {
        fn holding(paths: &[&str]) -> Self {
            Self {
                tracks: paths.iter().map(|path| (id(path), track(path))).collect(),
            }
        }
    }

    impl PlaybackLibrary for FakeLibrary {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            Ok(self.tracks.get(id).cloned())
        }

        /// The Application Store's canonical flat order (ADR 0003): every id
        /// path-ascending, which the fill takes verbatim.
        fn library_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            let mut ids: Vec<TrackId> = self.tracks.keys().cloned().collect();
            ids.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(ids)
        }
    }

    /// The shared operation over the three facts it reads — the session, the
    /// engine's command channel, the Library port — and nothing else: no
    /// decoder, no output, no spawned thread. The *when* playback starts is a
    /// decision, not a decode, so it is driven directly rather than through
    /// the engine loop's audio ports.
    struct Fixture {
        start: PlaybackStart,
        session: Arc<Mutex<PlaybackSession>>,
        /// Everything the operation re-dispatched into the engine's own
        /// command channel: the "exactly once" evidence.
        commands: Receiver<PlaybackCommand>,
    }

    fn fixture(paths: &[&str]) -> Fixture {
        let (cmd_tx, commands) = unbounded();
        let session = Arc::new(Mutex::new(PlaybackSession::default()));
        let start = PlaybackStart::new(
            Arc::clone(&session),
            cmd_tx,
            Arc::new(FakeLibrary::holding(paths)),
        );
        Fixture {
            start,
            session,
            commands,
        }
    }

    /// The **Queue Fill**'s *when*, and Continuation's *what*: an empty queue
    /// plus a `Play` means the whole Library becomes the queue, in the store's
    /// own order, with the wanted Track current. The engine is the load here,
    /// so nothing is re-dispatched.
    #[test]
    fn an_empty_queue_is_filled_from_the_library_with_the_wanted_track_current() {
        let f = fixture(&["music/a.wav", "music/b.wav", "music/c.wav"]);
        f.session.lock_or_recover().queue.set_shuffle(true);
        let wanted = id("music/b.wav");

        f.start
            .enqueue_and_start_if_idle(QueueStart::Fill { wanted: &wanted }, idle());

        let session = f.session.lock_or_recover();
        assert_eq!(
            session.queue.tracks,
            vec![id("music/a.wav"), id("music/b.wav"), id("music/c.wav")],
            "the whole Library becomes the queue, in the store's own order"
        );
        assert_eq!(
            session.queue.current_index,
            Some(1),
            "Continuation makes the wanted Track current at its slot in that order"
        );
        assert!(
            !session.queue.shuffle,
            "replacing the queue resets shuffle — a mode write, not an ordering one"
        );
        drop(session);
        assert!(
            f.commands.try_recv().is_err(),
            "the caller is the load itself, so nothing is re-dispatched"
        );
    }

    /// The same trigger answered "no": a **Queue Fill** is a *when*, and a
    /// Playback Queue that already holds Tracks is not that when. Nothing is
    /// filled and, because the fill's shuffle reset belongs to the fill, the
    /// queue's mode is left alone too.
    #[test]
    fn a_loaded_queue_is_not_filled() {
        let f = fixture(&["music/a.wav", "music/b.wav"]);
        {
            let mut session = f.session.lock_or_recover();
            session.queue.set_shuffle(true);
            session.queue.append(id("music/a.wav"));
        }
        let wanted = id("music/b.wav");

        f.start
            .enqueue_and_start_if_idle(QueueStart::Fill { wanted: &wanted }, idle());

        let session = f.session.lock_or_recover();
        assert_eq!(
            session.queue.tracks,
            vec![id("music/a.wav")],
            "the queue the caller found is left exactly as it was"
        );
        assert!(
            session.queue.shuffle,
            "and so is its mode — the reset belongs to the fill, not to Play"
        );
        assert_eq!(session.queue.current_index, Some(0));
    }

    /// An empty Library is not a fill either: there is nothing to put in the
    /// queue, so nothing is current — the same answer Continuation gives for a
    /// Track the store did not return.
    #[test]
    fn an_empty_library_leaves_the_queue_empty_and_nothing_current() {
        let f = fixture(&[]);
        let wanted = id("music/gone.wav");

        f.start
            .enqueue_and_start_if_idle(QueueStart::Fill { wanted: &wanted }, idle());

        let session = f.session.lock_or_recover();
        assert!(
            session.queue.tracks.is_empty(),
            "an empty Library fills nothing"
        );
        assert_eq!(session.queue.current_index, None);
    }

    /// The idle auto-play half of the same operation, driven the way a
    /// queue-mutating Playback Command drives it: the batch lands in the queue
    /// under one lock, and because nothing is currently playing the first of
    /// it starts — **exactly once**.
    #[test]
    fn an_idle_queue_mutation_starts_the_first_track_exactly_once() {
        let f = fixture(&[]);
        let batch = vec![id("music/a.wav"), id("music/b.wav")];

        f.start
            .enqueue_and_start_if_idle(QueueStart::AppendMany { ids: &batch }, idle());

        assert_eq!(
            f.commands.try_recv().ok(),
            Some(PlaybackCommand::Play(id("music/a.wav"))),
            "nothing is currently playing, so the first of the batch starts"
        );
        assert!(
            f.commands.try_recv().is_err(),
            "exactly once — a second dispatch would restart the Track"
        );
        assert_eq!(
            f.session.lock_or_recover().queue.tracks,
            batch,
            "the batch is queued under the same lock"
        );
    }

    /// The other half of idleness: something IS currently playing, so the same
    /// mutation queues without interrupting it. This is what **Add to Queue**
    /// means, and it is why the rule cannot be "always start".
    #[test]
    fn a_queue_mutation_while_something_plays_starts_nothing() {
        let f = fixture(&[]);
        let playing = id("music/playing.wav");
        let queued = id("music/a.wav");

        f.start
            .enqueue_and_start_if_idle(QueueStart::Append { id: &queued }, Some(&playing));

        assert!(
            f.commands.try_recv().is_err(),
            "the current Track plays straight through the queue mutation"
        );
        assert_eq!(
            f.session.lock_or_recover().queue.tracks,
            vec![queued],
            "and the queue still grew"
        );
    }

    /// **Add to Queue** as the UI sends it: one Track appended behind what is
    /// already queued, and nothing started while a Track is playing.
    #[test]
    fn add_to_queue_appends_behind_the_queue_without_starting_it() {
        let f = fixture(&[]);
        {
            let mut session = f.session.lock_or_recover();
            session.queue.append(id("music/a.wav"));
            session.queue.current_index = Some(0);
        }
        let playing = id("music/a.wav");
        let queued = id("music/b.wav");

        f.start
            .enqueue_and_start_if_idle(QueueStart::Append { id: &queued }, Some(&playing));

        assert_eq!(
            f.session.lock_or_recover().queue.tracks,
            vec![id("music/a.wav"), queued]
        );
        assert!(f.commands.try_recv().is_err());
    }

    /// A fourth queue-mutating Playback Command would be one [`QueueStart`]
    /// variant plus one line at its arm — the rule itself is not restated.
    /// This is the shape that line has.
    #[test]
    fn a_play_next_command_queues_behind_the_current_track() {
        let f = fixture(&[]);
        {
            let mut session = f.session.lock_or_recover();
            session.queue.append(id("music/a.wav"));
            session.queue.append(id("music/c.wav"));
            session.queue.current_index = Some(0);
        }
        let next = id("music/b.wav");

        f.start
            .enqueue_and_start_if_idle(QueueStart::Next { id: &next }, idle());

        assert_eq!(
            f.session.lock_or_recover().queue.tracks,
            vec![id("music/a.wav"), id("music/b.wav"), id("music/c.wav")],
            "queued to play next, after the current Track"
        );
        assert_eq!(
            f.commands.try_recv().ok(),
            Some(PlaybackCommand::Play(next)),
            "and started, because nothing was playing"
        );
    }
}
