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

use crate::app::errors::PlaybackError;
use crate::app::state::PlaybackSession;
use crate::domain::{PlaybackCommand, PlaybackPosition, PlaybackState, PlaybackUpdate, RepeatMode};
use crate::infra::ports::{AudioDecoder, AudioFormatInfo, AudioOutput, DecoderFactory};
use crossbeam_channel::{Receiver, Sender};
use riff_persistence::track::TrackId;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// Position updates: send [`PlaybackUpdate::PositionChanged`] only when
/// playback crosses a boundary of this many milliseconds, instead of once
/// per decoded chunk (~43/s), each of which wakes the update processor and
/// locks the Playback Session.
const POSITION_UPDATE_INTERVAL_MS: u64 = 250;

/// The audio engine: owns the decoders and audio output, processes
/// [`PlaybackCommand`]s, and drives decode/position/gapless logic.
pub struct AudioEngine {
    cmd_rx: Receiver<PlaybackCommand>,
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
        update_tx: Sender<PlaybackUpdate>,
        query: Box<dyn LibraryQueryStore + Send>,
        decoder_factory: DecoderFactory,
        output: Box<dyn AudioOutput + Send>,
        session: Arc<Mutex<PlaybackSession>>,
    ) -> Self {
        Self {
            cmd_rx,
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
    pub fn run(mut self) {
        let mut primary_decoder: Option<Box<dyn AudioDecoder>> = None;
        let mut output_started = false;
        let mut current_format: Option<AudioFormatInfo> = None;
        let mut current_track_id: Option<TrackId> = None;

        // Gapless pre-decode state
        let mut pre_decode_state = PreDecodeState::default();
        let pre_buffer_cap = |rate, ch| crate::app::gapless::pre_buffer_cap(rate, ch, PRE_BUFFER_SECONDS);

        // Position update throttling
        let mut last_position_update = Instant::now();

        // ReplayGain state
        let mut replaygain_gain: Option<f32> = None;
        let mut replaygain_peak: Option<f32> = None;

        // Audio output callback - closure that pulls decoded samples
        let mut decode_buffer = vec![0.0f32; DECODE_CHUNK_SAMPLES];

        while let Ok(cmd) = self.cmd_rx.recv() {
            match cmd {
                PlaybackCommand::Play(id) => {
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
                        let _ = self.update_tx.send(PlaybackUpdate::Error(format!("Track not found: {}", id.0)));
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
                        let session = self.session.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
                    primary_decoder = Some(decoder);

                    // Emit track changed
                    let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(id));
                    let _ = self.update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
                }

                PlaybackCommand::Pause => {
                    if output_started {
                        self.output.stop();
                        output_started = false;
                    }
                    let _ = self.update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
                }

                PlaybackCommand::Resume => {
                    if let Some(ref format) = current_format {
                        if let Err(e) = self.output.start(format.clone()) {
                            let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                        } else {
                            output_started = true;
                            let _ = self.update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
                        }
                    }
                }

                PlaybackCommand::Stop => {
                    if primary_decoder.is_some() {
                        primary_decoder.take();
                    }
                    if output_started {
                        self.output.stop();
                        output_started = false;
                    }
                    current_format = None;
                    current_track_id = None;
                    pre_decode_state = PreDecodeState::default();
                    let _ = self.update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
                }

                PlaybackCommand::Seek(pos) => {
                    if let Some(decoder) = primary_decoder.as_mut() {
                        let actual = decoder.seek(pos);
                        let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(PlaybackPosition {
                            current: actual,
                            total: decoder.duration(),
                        }));
                    }
                }

                PlaybackCommand::SetVolume(vol) => {
                    if output_started {
                        self.output.set_volume(vol.clamp(0.0, 1.0));
                    }
                    // Session volume is updated by the coordinator/transport
                }

                PlaybackCommand::Next => {
                    // Handled by coordinator via TrackEnded; engine just stops current
                    if primary_decoder.is_some() {
                        primary_decoder.take();
                        if output_started {
                            self.output.stop();
                            output_started = false;
                        }
                    }
                }

                PlaybackCommand::Previous => {
                    // Handled by coordinator; engine just stops current
                    if primary_decoder.is_some() {
                        primary_decoder.take();
                        if output_started {
                            self.output.stop();
                            output_started = false;
                        }
                    }
                }

                PlaybackCommand::PlayNext(id) => {
                    // Queue manipulation handled by coordinator; engine just notes
                    let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(id));
                }

                PlaybackCommand::AddToQueue(_id) => {
                    // Queue manipulation handled by coordinator
                }

                PlaybackCommand::AddMany(_ids) => {
                    // Queue manipulation handled by coordinator
                }

                PlaybackCommand::PlayPause => {
                    let state = self.session.lock().unwrap_or_else(std::sync::PoisonError::into_inner).playback_state;
                    match state {
                        PlaybackState::Playing => {
                            if output_started {
                                self.output.stop();
                                output_started = false;
                            }
                            let _ = self.update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
                        }
                        PlaybackState::Paused => {
                            if let Some(ref format) = current_format {
                                if let Err(e) = self.output.start(format.clone()) {
                                    let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                                } else {
                                    output_started = true;
                                    let _ = self.update_tx.send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
                                }
                            }
                        }
                        PlaybackState::Stopped => {
                            // No-op
                        }
                    }
                }
            }

            // Decode loop: pull from primary decoder and push to audio output
            let mut decoder_exhausted = false;
            if let Some(decoder) = primary_decoder.as_mut() {
                while let Some(samples) = decoder.next_frames(&mut decode_buffer) {
                    if samples == 0 {
                        break;
                    }

                    // Apply ReplayGain if enabled
                    if let Some(gain_db) = replaygain_gain {
                        let mut factor = 10f32.powf(gain_db / 20.0);
                        if let Some(peak) = replaygain_peak {
                            if peak > 0.0 {
                                let max_factor = 1.0 / peak;
                                if factor > max_factor {
                                    factor = max_factor;
                                }
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

                    // Throttled position updates
                    let elapsed = last_position_update.elapsed();
                    if elapsed >= Duration::from_millis(POSITION_UPDATE_INTERVAL_MS) {
                        let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(PlaybackPosition {
                            current: Duration::ZERO, // TODO: track actual position
                            total: decoder.duration(),
                        }));
                        last_position_update = Instant::now();
                    }
                }
                // Decoder returned None (EOF)
                decoder_exhausted = true;
            }

            // EOF handling - check for gapless handoff
            if decoder_exhausted {
                self.handle_eof(
                    &mut primary_decoder,
                    &mut pre_decode_state,
                    &mut current_format,
                    &mut output_started,
                    &mut current_track_id,
                );
            }

            // Pre-decode successor if conditions are met
            if primary_decoder.is_some() && pre_decode_state.pre_buffer.is_none() {
                let session = self.session.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let current_pos = Duration::ZERO; // TODO: track actual position
                let current_track = session.queue.current_track().cloned();

                if let Some(current_id) = current_track {
                    if let Ok(Some(track)) = self.query.get_track(&current_id) {
                        if let Some(duration) = track.duration {
                            let remaining = duration.saturating_sub(current_pos);
                            if remaining.as_secs_f32() <= PRE_ENCODE_SECONDS {
                                // Try to pre-decode successor
                                if let Some(next_id) = session.queue.upcoming(1).first().cloned() {
                                    let next_id_clone = next_id.clone();
                                    if let Ok(Some(next_track)) = self.query.get_track(&next_id) {
                                        let mut decoder = (self.decoder_factory)();
                                        if let Ok(format) = decoder.init(&next_track.file_path) {
                                            if let Some(ref cur_format) = current_format {
                                                if format.compatible_with(cur_format) {
                                                    pre_decode_state.format_compatible = true;
                                                    pre_decode_state.has_successor = true;
                                                    pre_decode_state.next_track_id = Some(next_id_clone);

                                                    // Pre-decode up to PRE_BUFFER_SECONDS
                                                    let cap = pre_buffer_cap(format.sample_rate, format.channels);
                                                    let mut pre_buffer = Vec::with_capacity(cap);
                                                    let mut buf = vec![0.0f32; DECODE_CHUNK_SAMPLES];
                                                    while pre_buffer.len() < cap {
                                                        if let Some(samples) = decoder.next_frames(&mut buf) {
                                                            if samples == 0 {
                                                                break;
                                                            }
                                                            pre_buffer.extend_from_slice(&buf[..samples]);
                                                        } else {
                                                            break;
                                                        }
                                                    }

                                                    if !pre_buffer.is_empty() {
                                                        pre_decode_state.pre_buffer = Some(decoder);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
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
    ) {
        let session = self.session.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

        if can_gapless && pre_decode_state.pre_buffer.is_some() {
            // Gapless handoff: swap to pre-buffered decoder
            *primary_decoder = pre_decode_state.pre_buffer.take();
            *pre_decode_state = PreDecodeState::default();

            // Emit track changed for the new track
            if let Some(ref next_id) = pre_decode_state.next_track_id.take() {
                *current_track_id = Some(next_id.clone());
                let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(next_id.clone()));
            }
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
}

/// Gapless pre-decode state (Task 4.1). Purely additive: when a valid,
/// format-compatible successor has been pre-buffered, EOF hands off without
/// stopping the cpal stream; in EVERY other case these fields are ignored and
/// the existing gapped path runs unchanged.
#[derive(Default)]
struct PreDecodeState {
    pre_buffer: Option<Box<dyn AudioDecoder>>,
    format_compatible: bool,
    has_successor: bool,
    next_track_id: Option<TrackId>,
}

impl PreDecodeState {
    fn reset(&mut self) {
        self.pre_buffer = None;
        self.format_compatible = false;
        self.has_successor = false;
        self.next_track_id = None;
    }
}

/// Trait for library query store (re-exported for engine use)
pub trait LibraryQueryStore {
    fn get_track(&self, id: &TrackId) -> Result<Option<riff_persistence::track::Track>, crate::app::errors::StoreError>;
}