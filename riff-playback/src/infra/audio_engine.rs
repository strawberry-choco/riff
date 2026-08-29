//! The audio engine: turns [`PlaybackCommand`]s into decoded audio and
//! [`PlaybackUpdate`]s.
//!
//! A deep module behind the port seams ([`AudioDecoder`] via a
//! [`DecoderFactory`], [`AudioOutput`], [`LibraryQueryStore`]): it owns
//! decode scheduling with backpressure, output startup, `ReplayGain`
//! resolution, Queue Fill, command re-dispatch, and the gapless
//! pre-decode/handoff machinery — everything else is private implementation.
//! It decides nothing about queue order beyond filling an empty Playback
//! Queue.
//!
//! Threading: the module exposes only the blocking [`AudioEngine::run`];
//! `main.rs` remains the sole thread spawner and runs it on the dedicated
//! audio engine thread.
//!
//! Pure-Rust: uses only the port traits. Concrete decoder/output
//! implementations live in `riff-infra`.

use crate::app::state::PlaybackSession;
use crate::domain::{PlaybackCommand, PlaybackPosition, PlaybackState, PlaybackUpdate, RepeatMode};
use crate::infra::ports::{AudioDecoder, AudioFormatInfo, AudioOutput, DecoderFactory};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use riff_persistence::store::LibraryQueryStore;
use riff_persistence::track::TrackId;
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
    /// (Next/Previous, idle auto-play after PlayNext/AddToQueue) can
    /// re-dispatch a full `Play` instead of duplicating its stream-restart
    /// logic inline.
    cmd_tx: Sender<PlaybackCommand>,
    update_tx: Sender<PlaybackUpdate>,
    query: Box<dyn LibraryQueryStore + Send>,
    decoder_factory: DecoderFactory,
    output: Box<dyn AudioOutput + Send>,
    session: Arc<Mutex<PlaybackSession>>,
}

impl AudioEngine {
    /// Construct an engine with the given ports and channels.
    #[must_use]
    pub fn new(
        cmd_rx: Receiver<PlaybackCommand>,
        cmd_tx: Sender<PlaybackCommand>,
        update_tx: Sender<PlaybackUpdate>,
        query: Box<dyn LibraryQueryStore + Send>,
        decoder_factory: DecoderFactory,
        output: Box<dyn AudioOutput + Send>,
        session: Arc<Mutex<PlaybackSession>>,
    ) -> Self {
        Self {
            cmd_rx,
            cmd_tx,
            update_tx,
            query,
            decoder_factory,
            output,
            session,
        }
    }

    /// Run the engine's main loop. Blocks until the command channel is closed.
    /// This is the ONLY blocking entry point — the composition root spawns it
    /// on the dedicated audio thread.
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

        // ReplayGain state
        let mut replaygain_gain: Option<f32> = None;
        let mut replaygain_peak: Option<f32> = None;

        // Audio output callback - closure that pulls decoded samples
        let mut decode_buffer = vec![0.0f32; DECODE_CHUNK_SAMPLES];

        // Sample-accurate playback position for the current track.
        let mut position = Duration::ZERO;

        loop {
            // Block while idle; while a track is loaded, poll so a queued
            // command lands between decode chunks instead of after EOF.
            let cmd = if primary_decoder.is_some() {
                match self.cmd_rx.recv_timeout(COMMAND_POLL) {
                    Ok(cmd) => Some(cmd),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match self.cmd_rx.recv() {
                    Ok(cmd) => Some(cmd),
                    Err(_) => break,
                }
            };

            if let Some(cmd) = cmd {
                match cmd {
                    PlaybackCommand::Play(id) => {
                        // Queue Fill: playing into an empty queue loads the
                        // whole Library in canonical flat ordering (path
                        // ascending) so Next/Previous and auto-advance work;
                        // the requested track becomes current and shuffle
                        // resets with the replaced queue.
                        {
                            let mut session = self
                                .session
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if session.queue.tracks.is_empty()
                                && let Ok(all_ids) = self.query.all_track_ids()
                                && !all_ids.is_empty()
                            {
                                session.queue.tracks = all_ids;
                                session.queue.current_index = session
                                    .queue
                                    .tracks
                                    .iter()
                                    .position(|queue_id| queue_id == &id);
                                session.queue.set_shuffle(false);
                            }
                        }

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
                        let track = self.query.get_track(&id);
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

                        // Apply ReplayGain if enabled
                        {
                            let session = self
                                .session
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if session.replaygain_enabled {
                                if let Some(gain) = track.metadata.replaygain_track_gain {
                                    replaygain_gain = Some(gain);
                                    replaygain_peak = track.metadata.replaygain_track_peak;
                                } else {
                                    replaygain_gain = None;
                                    replaygain_peak = None;
                                }
                            } else {
                                replaygain_gain = None;
                                replaygain_peak = None;
                            }
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
                                    && let Ok(Some(track)) = self.query.get_track(id)
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
                        // Queue manipulation lives here in the engine: the
                        // session queue is the one traversal state, and the
                        // coordinator only hears about what happened.
                        let idle = {
                            let mut session = self
                                .session
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            session.queue.insert_next(id.clone());
                            current_track_id.is_none()
                        };
                        if idle {
                            let _ = self.cmd_tx.send(PlaybackCommand::Play(id));
                        }
                    }

                    PlaybackCommand::AddToQueue(id) => {
                        let idle = {
                            let mut session = self
                                .session
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            session.queue.append(id.clone());
                            current_track_id.is_none()
                        };
                        if idle {
                            let _ = self.cmd_tx.send(PlaybackCommand::Play(id));
                        }
                    }

                    PlaybackCommand::AddMany(ids) => {
                        // One lock, one queue mutation for the whole batch
                        // (folder "play all" enqueues N tracks without N
                        // shuffle regenerations). Same idle-auto-play
                        // contract as AddToQueue: the first id starts
                        // playback when idle.
                        let first = ids.first().cloned();
                        let idle = {
                            let mut session = self
                                .session
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            session.queue.append_many(ids);
                            current_track_id.is_none()
                        };
                        if idle && let Some(first) = first {
                            let _ = self.cmd_tx.send(PlaybackCommand::Play(first));
                        }
                    }

                    PlaybackCommand::PlayPause => {
                        let state = self
                            .session
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .playback_state;
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

            // Decode one chunk per tick while the stream runs; the command
            // loop stays in charge between chunks, so queued commands are
            // honored mid-track and position updates flow while decoding.
            if output_started && let Some(decoder) = primary_decoder.as_mut() {
                match decoder.next_frames(&mut decode_buffer) {
                    Some(samples) if samples > 0 => {
                        // Apply ReplayGain if enabled
                        if let Some(gain_db) = replaygain_gain {
                            let mut factor = 10f32.powf(gain_db / 20.0);
                            if let Some(peak) = replaygain_peak
                                && peak > 0.0
                            {
                                let max_factor = 1.0 / peak;
                                if factor > max_factor {
                                    factor = max_factor;
                                }
                            }
                            for s in &mut decode_buffer[..samples] {
                                *s *= factor;
                            }
                        }

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
                                let session = self
                                    .session
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                session.queue.upcoming(1).first().map(|id| (*id).clone())
                            };
                            if let Some(next_id) = successor_id
                                && let Ok(Some(next_track)) = self.query.get_track(&next_id)
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
        let session = self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let gapless_conditions = crate::app::gapless::GaplessConditions {
            shuffle: session.queue.shuffle,
            repeat_one: session.queue.repeat == RepeatMode::One && !session.queue.shuffle,
            format_compatible: pre_decode_state.format_compatible,
            has_successor: pre_decode_state.has_successor,
        };

        let can_gapless = if session.queue.repeat == RepeatMode::One && !session.queue.shuffle {
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
            let mut session = self
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match direction {
                SkipDirection::Forward => session.queue.advance().cloned(),
                SkipDirection::Backward => session.queue.previous().cloned(),
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
