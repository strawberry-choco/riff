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

use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app::MutexExt;
use crate::app::errors::AppError;
use crate::app::gapless::{
    GaplessConditions, QueueConditions, elapsed_from_samples, formats_gapless_compatible,
    is_gapless_eligible, pre_buffer_cap, repeat_one_handoff_eligible, samples_from_duration,
};
use crate::app::state::{AppState, replaygain_factor};
use crate::app::store::LibraryQueryStore;
use crate::app::traits::{AudioDecoder, AudioFormatInfo, AudioOutput, DecoderFactory};
use crate::domain::{
    PlaybackCommand, PlaybackPosition, PlaybackState, PlaybackUpdate, RepeatMode, TrackId,
};

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

/// Position updates (task 3.3): send [`PlaybackUpdate::PositionChanged`] only
/// when playback crosses a boundary of this many milliseconds, instead of once
/// per decoded chunk (~43/s), each of which wakes the update processor and
/// locks `AppState`.
const POSITION_UPDATE_INTERVAL_MS: u64 = 250;

/// The audio engine: owns the decoders and audio output, processes
/// [`PlaybackCommand`]s, and drives decode/position/gapless logic.
pub struct AudioEngine {
    cmd_rx: Receiver<PlaybackCommand>,
    update_tx: Sender<PlaybackUpdate>,
    cmd_tx: Sender<PlaybackCommand>,
    state: Arc<Mutex<AppState>>,
    /// Store query port for playback-time track resolution: playing a track
    /// resolves it by `TrackId` from the Application Store, never from an
    /// in-memory copy. `Send` because the engine lives on its own thread.
    library_queries: Box<dyn LibraryQueryStore + Send>,
    decoder: Box<dyn AudioDecoder>,
    next_decoder: Box<dyn AudioDecoder>,
    audio_output: Box<dyn AudioOutput>,
    current_track_id: Option<TrackId>,
    paused_position: Option<Duration>,
    /// Set on a gapless handoff: the `TrackEnded` we emit makes the update
    /// processor re-send Play(current) for the track that is ALREADY playing;
    /// that duplicate must be swallowed or it would tear down the stream.
    gapless_dup_expected: bool,
    pre_decode: PreDecodeState,
}

impl AudioEngine {
    /// Wire an engine over its ports. `cmd_tx` is the engine's own handle to
    /// the command channel for self re-dispatch (`PlayPause`, `Resume`,
    /// `Next`/`Previous`, queue-triggered plays); `decoder_factory` mints a
    /// fresh decoder for the primary and the gapless pre-decode slots.
    pub fn new(
        cmd_rx: Receiver<PlaybackCommand>,
        cmd_tx: Sender<PlaybackCommand>,
        update_tx: Sender<PlaybackUpdate>,
        state: Arc<Mutex<AppState>>,
        library_queries: Box<dyn LibraryQueryStore + Send>,
        decoder_factory: DecoderFactory,
        audio_output: Box<dyn AudioOutput>,
    ) -> Self {
        Self {
            cmd_rx,
            update_tx,
            cmd_tx,
            state,
            library_queries,
            decoder: decoder_factory(),
            next_decoder: decoder_factory(),
            audio_output,
            current_track_id: None,
            paused_position: None,
            gapless_dup_expected: false,
            pre_decode: PreDecodeState::default(),
        }
    }

    /// Block processing commands until every command sender is dropped.
    /// Spawns nothing; run this on the dedicated audio engine thread.
    pub fn run(mut self) {
        while let Ok(cmd) = self.cmd_rx.recv() {
            self.dispatch(cmd);
        }
    }

    fn dispatch(&mut self, cmd: PlaybackCommand) {
        match cmd {
            PlaybackCommand::Play(track_id) => self.handle_play(track_id),
            PlaybackCommand::Pause => {
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
            }
            PlaybackCommand::Resume => {
                if let Some(track_id) = self.current_track_id.clone() {
                    let _ = self.cmd_rx.try_recv(); // clear any pending
                    let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::Stop => {
                self.paused_position = None;
                self.gapless_dup_expected = false;
                let _ = self.audio_output.stop();
                self.decoder.close();
                // Discard any gapless pre-decode state (Task 4.1).
                self.pre_decode.reset();
                self.next_decoder.close();
                // Drop any stale per-track ReplayGain factor (Task 4.3).
                self.audio_output.set_replaygain(1.0);
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
            }
            PlaybackCommand::Seek(pos) => {
                let _ = self.decoder.seek(pos);
            }
            PlaybackCommand::SetVolume(vol) => {
                self.audio_output.set_volume(vol);
            }
            // User navigation invalidates any pending gapless Play dup.
            PlaybackCommand::Next => self.jump_queue(true),
            PlaybackCommand::Previous => self.jump_queue(false),
            PlaybackCommand::ToggleVisibility => {
                // Close-to-tray (REQ-SI-001): the tray thread cannot touch the
                // window, so flip the shared flag; the UI's per-frame `logic`
                // reconciles it with the real viewport visibility (Show Window
                // and the left-click toggle both route through here).
                let mut s = self.state.lock_or_recover();
                s.window_visible = !s.window_visible;
            }
            PlaybackCommand::PlayPause => {
                let current_state = {
                    let state = self.state.lock_or_recover();
                    state.playback_state
                };
                match current_state {
                    PlaybackState::Playing => {
                        let _ = self.cmd_tx.send(PlaybackCommand::Pause);
                    }
                    _ => {
                        let _ = self.cmd_tx.send(PlaybackCommand::Resume);
                    }
                }
            }
            PlaybackCommand::PlayNext(track_id) => {
                {
                    let mut state = self.state.lock_or_recover();
                    state.queue.insert_next(track_id.clone());
                }
                if self.current_track_id.is_none() {
                    let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::AddToQueue(track_id) => {
                {
                    let mut state = self.state.lock_or_recover();
                    state.queue.append(track_id.clone());
                }
                if self.current_track_id.is_none() {
                    let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
                }
            }
            PlaybackCommand::AddMany(track_ids) => {
                // One lock, one queue mutation for the whole batch
                // (allocation plan 4.3). Same idle-auto-play contract as
                // AddToQueue: the first id starts playback when idle.
                let first = track_ids.first().cloned();
                {
                    let mut state = self.state.lock_or_recover();
                    state.queue.append_many(track_ids);
                }
                if self.current_track_id.is_none()
                    && let Some(first) = first
                {
                    let _ = self.cmd_tx.send(PlaybackCommand::Play(first));
                }
            }
        }
    }

    /// Next/Previous navigation: move the queue and play the new current
    /// track.
    fn jump_queue(&mut self, advance: bool) {
        self.gapless_dup_expected = false;
        let target = {
            let mut state = self.state.lock_or_recover();
            if advance {
                state.queue.advance().cloned()
            } else {
                state.queue.previous().cloned()
            }
        };
        if let Some(track_id) = target {
            self.current_track_id = Some(track_id.clone());
            let _ = self.audio_output.stop();
            let _ = self.cmd_tx.send(PlaybackCommand::Play(track_id));
        }
    }

    fn handle_play(&mut self, track_id: TrackId) {
        // Gapless dedup (Task 4.1): after a gapless handoff the update
        // processor's TrackEnded handling re-sends Play() for the track that
        // is ALREADY playing. Swallow exactly that duplicate so it cannot
        // tear down the live stream; any other Play clears the expectation
        // and runs normally.
        if self.gapless_dup_expected && self.current_track_id.as_ref() == Some(&track_id) {
            self.gapless_dup_expected = false;
            tracing::debug!("Gapless: ignored auto-Play for the already-playing track");
            return;
        }
        self.gapless_dup_expected = false;

        // Reset pre-decode state left over from a previous session.
        self.pre_decode.reset();

        self.audio_output.clear_buffer();
        let _ = self.audio_output.stop();
        let is_resuming =
            self.current_track_id.as_ref() == Some(&track_id) && self.paused_position.is_some();
        if !is_resuming {
            self.paused_position = None;
        }
        self.current_track_id = Some(track_id.clone());

        let Some((path, replaygain)) = self.resolve_track(&track_id) else {
            return;
        };

        // Apply the per-track ReplayGain factor before samples flow.
        self.audio_output.set_replaygain(replaygain);
        let mut info = match self.decoder.open(&path) {
            Ok(info) => info,
            Err(e) => {
                let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                return;
            }
        };
        if is_resuming && let Some(pos) = self.paused_position.take() {
            let _ = self.decoder.seek(pos);
        }
        let _ = self.update_tx.send(PlaybackUpdate::TrackChanged(track_id));

        if let Err(e) = self.start_output(info.sample_rate, info.channels) {
            let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
            return;
        }

        self.audio_output.clear_buffer();
        let _ = self
            .update_tx
            .send(PlaybackUpdate::StateChanged(PlaybackState::Playing));
        let should_stop_audio = self.run_decode_loop(&mut info);

        if should_stop_audio {
            let _ = self.audio_output.stop();
        }
    }

    /// Initialize and start the audio output for the given format.
    fn start_output(&mut self, sample_rate: u32, channels: u16) -> Result<(), AppError> {
        self.audio_output.initialize(sample_rate, channels)?;
        self.audio_output.start()?;
        Ok(())
    }
}

// __PART2__

impl AudioEngine {
    /// Look up the file path and `ReplayGain` factor for a track. When
    /// playing from the library with an empty queue, populate the queue so
    /// that Next/Previous/auto-advance work.
    fn resolve_track(&mut self, track_id: &TrackId) -> Option<(PathBuf, f32)> {
        let mut state = self.state.lock_or_recover();
        if state.queue.tracks.is_empty() {
            // Queue Fill (glossary): the whole Library loads into the empty
            // Playback Queue in canonical flat ordering (path ascending,
            // ADR 0003 — deliberately replacing the mirror's HashMap-luck
            // order), the requested TrackId becomes current, and shuffle
            // resets. The data source is the Application Store; extraction
            // of this use case above the engine seam is Part 2 step 2.
            match self.library_queries.all_track_ids() {
                Ok(all_ids) if !all_ids.is_empty() => {
                    state.queue.tracks = all_ids;
                    state.queue.current_index =
                        state.queue.tracks.iter().position(|id| id == track_id);
                    // Reset shuffle state since the queue was replaced
                    state.queue.shuffle = false;
                    state.queue.shuffled_indices.clear();
                    state.queue.shuffle_history.clear();
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("Failed to fill the queue from the store: {e}");
                }
            }
        }
        // Playback resolves the track by TrackId from the Application Store.
        let stored = match self.library_queries.get_track(track_id) {
            Ok(track) => track,
            Err(e) => {
                tracing::error!("Failed to resolve track {track_id:?} from the store: {e}");
                None
            }
        };
        // ReplayGain (Task 4.3): compute the peak-capped factor from the
        // track's tags and the user preference. Untagged tracks (or the
        // disabled setting) yield 1.0 — no adjustment.
        let (gain_db, peak) = stored.as_ref().map_or((None, None), |t| {
            (
                t.metadata.replaygain_track_gain,
                t.metadata.replaygain_track_peak,
            )
        });
        let factor = replaygain_factor(state.replaygain_enabled, gain_db, peak);
        stored.map(|t| (t.file_path.clone(), factor))
    }

    /// The decode loop for the currently open track: decode + write with
    /// backpressure, position updates, gapless handoff at EOF, pre-decode,
    /// and inline command polling. Returns `true` when the audio output
    /// should be stopped afterwards (every break except Pause/Stop, which
    /// manage the stream themselves).
    fn run_decode_loop(&mut self, info: &mut AudioFormatInfo) -> bool {
        // Sample-exact elapsed tracking (Task 4.1): every decoded chunk adds
        // its length; elapsed = samples / (rate * ch). Replaces the old
        // wall-clock timing and resets to 0 at each gapless handoff.
        let mut accumulated_samples: usize = 0;
        let mut is_playing = true;
        let mut should_stop_audio = true;
        let mut max_buffer_samples = (info.sample_rate as usize) * usize::from(info.channels) * 2;
        // Reused decode chunk (task 3.2): allocated once per session.
        let mut chunk = vec![0.0f32; DECODE_CHUNK_SAMPLES];
        // Last 250 ms boundary reported to the UI (task 3.3).
        let mut position_bucket: Option<u64> = None;

        loop {
            if !is_playing {
                break;
            }

            // Backpressure: don't decode when the buffer is already full
            self.wait_for_buffer_space(
                max_buffer_samples,
                &mut is_playing,
                &mut should_stop_audio,
                &mut accumulated_samples,
                info,
            );

            if !is_playing {
                break;
            }

            match self.decoder.next_frames(&mut chunk) {
                Ok(0) => {
                    // End of stream: final position at the track's true total.
                    if let Some(dur) = info.duration {
                        let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(
                            PlaybackPosition {
                                current: dur,
                                total: Some(dur),
                            },
                        ));
                    }

                    if self.end_of_track(info, &mut accumulated_samples, &mut max_buffer_samples)
                        == EndOfTrack::HandedOff
                    {
                        continue;
                    }
                    break;
                }
                Ok(n) => {
                    if let Err(e) = self.audio_output.write_samples(&chunk[..n]) {
                        let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                        break;
                    }
                    accumulated_samples += n;
                    let elapsed =
                        elapsed_from_samples(accumulated_samples, info.sample_rate, info.channels);
                    // Throttled position reporting (task 3.3): send only when
                    // playback crosses a 250 ms boundary. The final position at
                    // EOF above is sent unconditionally.
                    let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
                    let bucket = Some(millis / POSITION_UPDATE_INTERVAL_MS);
                    if position_bucket != bucket {
                        position_bucket = bucket;
                        let _ = self.update_tx.send(PlaybackUpdate::PositionChanged(
                            PlaybackPosition {
                                current: elapsed,
                                total: info.duration,
                            },
                        ));
                    }
                }
                Err(e) => {
                    let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                    break;
                }
            }

            self.maybe_pre_encode(info, accumulated_samples);

            if let Ok(cmd) = self.cmd_rx.try_recv()
                && self.handle_loop_command(
                    cmd,
                    &mut is_playing,
                    &mut should_stop_audio,
                    &mut accumulated_samples,
                    info,
                )
            {
                break;
            }
        }

        // The decode loop ended for a reason other than a gapless handoff
        // (Pause, Stop, error, or the gapped EOF path): discard any leftover
        // pre-decode state so nothing leaks into the next session. Pause and
        // the gapped path already clear it; this covers Stop and decode-error
        // breaks that bypass those sites.
        self.pre_decode.reset();
        self.next_decoder.close();

        should_stop_audio
    }

    /// Block until the output buffer has space, processing any commands that
    /// arrive while waiting.
    fn wait_for_buffer_space(
        &mut self,
        max_buffer_samples: usize,
        is_playing: &mut bool,
        should_stop_audio: &mut bool,
        accumulated_samples: &mut usize,
        info: &AudioFormatInfo,
    ) {
        while self.audio_output.buffer_len() >= max_buffer_samples {
            if let Ok(cmd) = self.cmd_rx.try_recv()
                && self.handle_loop_command(
                    cmd,
                    is_playing,
                    should_stop_audio,
                    accumulated_samples,
                    info,
                )
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Process one command received mid-session (during the backpressure wait
    /// or the per-chunk poll). Handles the gapless Play-dedup, records the
    /// pause position, and rebases elapsed tracking after seeks. Returns
    /// `true` when the decode loop should end.
    fn handle_loop_command(
        &mut self,
        cmd: PlaybackCommand,
        is_playing: &mut bool,
        should_stop_audio: &mut bool,
        accumulated_samples: &mut usize,
        info: &AudioFormatInfo,
    ) -> bool {
        // Gapless dedup (Task 4.1): swallow the post-handoff auto-Play
        // instead of tearing down the stream.
        if self.gapless_dup_expected
            && matches!(&cmd, PlaybackCommand::Play(id)
                if self.current_track_id.as_ref() == Some(id))
        {
            self.gapless_dup_expected = false;
            return false;
        }
        if matches!(cmd, PlaybackCommand::Pause) {
            self.paused_position = Some(elapsed_from_samples(
                *accumulated_samples,
                info.sample_rate,
                info.channels,
            ));
            // Discard the pre-buffer; resume re-does pre-encode
            // (safe/simple).
            self.pre_decode.reset();
        }
        if let PlaybackCommand::Seek(pos) = &cmd {
            // Elapsed continues from the seek target (exact integer math).
            *accumulated_samples = samples_from_duration(*pos, info.sample_rate, info.channels);
        }
        self.apply_command(cmd, is_playing, should_stop_audio)
    }

    /// Apply one command to the output/decoder/queue. Returns `true` when the
    /// command ends the current decode session.
    fn apply_command(
        &mut self,
        cmd: PlaybackCommand,
        is_playing: &mut bool,
        should_stop_audio: &mut bool,
    ) -> bool {
        match cmd {
            PlaybackCommand::Pause => {
                *is_playing = false;
                *should_stop_audio = false;
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Paused));
                true
            }
            PlaybackCommand::Stop => {
                self.audio_output.clear_buffer();
                let _ = self.audio_output.stop();
                // Drop any stale per-track ReplayGain factor (Task 4.3).
                self.audio_output.set_replaygain(1.0);
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::StateChanged(PlaybackState::Stopped));
                *should_stop_audio = false;
                *is_playing = false;
                true
            }
            PlaybackCommand::Seek(pos) => {
                let _ = self.decoder.seek(pos);
                self.audio_output.clear_buffer();
                false
            }
            PlaybackCommand::SetVolume(vol) => {
                self.audio_output.set_volume(vol);
                false
            }
            PlaybackCommand::Play(_) | PlaybackCommand::Next | PlaybackCommand::Previous => {
                let _ = self.cmd_tx.send(cmd);
                *should_stop_audio = true;
                *is_playing = false;
                true
            }
            PlaybackCommand::PlayNext(track_id) => {
                let mut s = self.state.lock_or_recover();
                s.queue.insert_next(track_id);
                false
            }
            PlaybackCommand::AddToQueue(track_id) => {
                let mut s = self.state.lock_or_recover();
                s.queue.append(track_id);
                false
            }
            PlaybackCommand::AddMany(track_ids) => {
                let mut s = self.state.lock_or_recover();
                s.queue.append_many(track_ids);
                false
            }
            _ => false,
        }
    }

    /// Gapless pre-decode (Task 4.1): once playback nears EOF, opportunistically
    /// decode up to `PRE_BUFFER_SECONDS` of the natural successor on this same
    /// thread (symphonia decodes faster than real time, so the ring buffer
    /// cannot starve). Best-effort: any failure leaves `pre_decode.track_id`
    /// unset and the gapped path runs unchanged at EOF.
    fn maybe_pre_encode(&mut self, info: &AudioFormatInfo, accumulated_samples: usize) {
        let ready = !self.pre_decode.attempted
            && self.pre_decode.track_id.is_none()
            && info.duration.is_some_and(|d| {
                elapsed_from_samples(accumulated_samples, info.sample_rate, info.channels)
                    >= d.saturating_sub(Duration::from_secs_f32(PRE_ENCODE_SECONDS))
            });
        if !ready {
            return;
        }
        self.pre_decode.attempted = true;

        let successor = {
            let s = self.state.lock_or_recover();
            let next_id = if s.queue.repeat == RepeatMode::One {
                // Repeat-one loops the SAME track gaplessly.
                s.queue.current_track().cloned()
            } else {
                s.queue.upcoming(1).into_iter().next().cloned()
            };
            // The successor's path resolves from the Application Store.
            next_id.and_then(|id| match self.library_queries.get_track(&id) {
                Ok(Some(t)) => Some((id, t.file_path)),
                Ok(None) => None,
                Err(e) => {
                    tracing::error!("Failed to resolve successor {id:?}: {e}");
                    None
                }
            })
        };
        let Some((next_id, path)) = successor else {
            return;
        };

        match self.next_decoder.open(&path) {
            Ok(fmt) => {
                let cap = pre_buffer_cap(fmt.sample_rate, fmt.channels, PRE_BUFFER_SECONDS);
                let mut failed = false;
                // Reused decode chunk (task 3.2): allocated once per
                // pre-decode session.
                let mut chunk = vec![0.0f32; DECODE_CHUNK_SAMPLES];
                while self.pre_decode.buffer.len() < cap {
                    match self.next_decoder.next_frames(&mut chunk) {
                        // Short track: fully buffered already.
                        Ok(0) => break,
                        Ok(n) => {
                            self.pre_decode.buffer.extend_from_slice(&chunk[..n]);
                        }
                        Err(e) => {
                            tracing::warn!("Gapless pre-decode error: {}", e);
                            failed = true;
                            break;
                        }
                    }
                }
                if failed || self.pre_decode.buffer.is_empty() {
                    self.pre_decode.buffer.clear();
                    self.next_decoder.close();
                } else {
                    tracing::info!(
                        "Gapless: pre-buffered successor {:?} ({} samples)",
                        path,
                        self.pre_decode.buffer.len()
                    );
                    self.pre_decode.format = Some(fmt);
                    self.pre_decode.track_id = Some(next_id);
                    self.pre_decode.track_path = Some(path);
                }
            }
            Err(e) => {
                tracing::warn!("Gapless pre-decode open failed: {}", e);
                self.next_decoder.close();
            }
        }
    }

    /// End-of-track handling (Task 4.1): attempt the gapless handoff when a
    /// valid, format-compatible, pre-buffered natural successor exists;
    /// otherwise run the gapped path (`TrackEnded` → drain buffer → stop).
    /// Purely additive: any ineligible case falls through unchanged.
    fn end_of_track(
        &mut self,
        info: &mut AudioFormatInfo,
        accumulated_samples: &mut usize,
        max_buffer_samples: &mut usize,
    ) -> EndOfTrack {
        let promoted = self
            .pre_decode
            .track_id
            .take()
            .zip(self.pre_decode.format.take())
            .map(|(id, fmt)| (id, fmt, self.pre_decode.track_path.take()));

        if let Some((promoted_id, promoted_format, promoted_path)) = promoted {
            let eligible = {
                let s = self.state.lock_or_recover();
                let repeat_one = s.queue.repeat == RepeatMode::One;
                let natural = if repeat_one {
                    s.queue.current_track().cloned()
                } else {
                    s.queue.upcoming(1).into_iter().next().cloned()
                };
                let has_successor =
                    natural.as_ref() == Some(&promoted_id) && !self.pre_decode.buffer.is_empty();
                let compatible = formats_gapless_compatible(
                    self.audio_output.effective_sample_rate(),
                    info.channels,
                    promoted_format.sample_rate,
                    promoted_format.channels,
                );
                is_gapless_eligible(GaplessConditions {
                    queue: QueueConditions {
                        shuffle: s.queue.shuffle,
                        repeat_one,
                    },
                    formats_compatible: compatible,
                    has_successor,
                }) || repeat_one_handoff_eligible(
                    s.queue.shuffle,
                    repeat_one,
                    compatible,
                    has_successor,
                )
            };

            if eligible {
                // Expect (and swallow) the update processor's automatic
                // Play() for the promoted track.
                self.gapless_dup_expected = true;
                // TrackEnded keeps play-count and auto-advance bookkeeping
                // intact; the resulting Play dup is neutralized by the dedup
                // guard.
                let _ = self.update_tx.send(PlaybackUpdate::TrackEnded);
                // 1. Flush the successor's pre-buffer. The cpal stream is
                //    NEVER stopped/re-initialized across this boundary; the
                //    ring buffer is NOT cleared.
                if let Err(e) = self.audio_output.write_samples(&self.pre_decode.buffer) {
                    let _ = self.update_tx.send(PlaybackUpdate::Error(e.to_string()));
                    return EndOfTrack::SessionEnded;
                }
                // 2. Promote the pre-decode decoder to primary and retire the
                //    old primary.
                std::mem::swap(&mut self.decoder, &mut self.next_decoder);
                self.next_decoder.close();
                // 3. Position tracking restarts for the new track.
                *accumulated_samples = 0;
                self.pre_decode.buffer.clear();
                self.pre_decode.attempted = false;
                // ReplayGain (Task 4.3): apply the promoted track's factor at
                // the boundary; its tags resolve from the Application Store.
                let rg = {
                    let s = self.state.lock_or_recover();
                    let stored = self.library_queries.get_track(&promoted_id).ok().flatten();
                    let (g, p) = stored.map_or((None, None), |t| {
                        (
                            t.metadata.replaygain_track_gain,
                            t.metadata.replaygain_track_peak,
                        )
                    });
                    replaygain_factor(s.replaygain_enabled, g, p)
                };
                self.audio_output.set_replaygain(rg);
                // 4. Announce the new track at the sample boundary.
                self.current_track_id = Some(promoted_id.clone());
                let _ = self
                    .update_tx
                    .send(PlaybackUpdate::TrackChanged(promoted_id.clone()));
                tracing::info!("Gapless handoff to {:?} ({:?})", promoted_id, promoted_path);
                // 5. Continue the decode loop with the new track.
                *info = promoted_format;
                *max_buffer_samples = (info.sample_rate as usize) * usize::from(info.channels) * 2;
                return EndOfTrack::HandedOff;
            }

            // Successor was pre-buffered but the handoff is ineligible (format
            // mismatch, shuffle/jump, or pre-buffer issues). The gapped path
            // below runs unchanged and discards the pre-buffer.
            tracing::info!(
                "Gapless: skipping handoff to {:?} (ineligible); gapped path",
                promoted_id
            );
        }

        // --- Existing gapped path, unchanged ---
        let _ = self.update_tx.send(PlaybackUpdate::TrackEnded);
        // Wait for the remaining buffer to drain before stopping
        while self.audio_output.buffer_len() > 0 {
            if let Ok(PlaybackCommand::Stop) = self.cmd_rx.try_recv() {
                self.audio_output.clear_buffer();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // Discard any unused pre-buffer (format mismatch, stale successor, or
        // none); the next Play re-arms pre-decode.
        self.pre_decode.reset();
        self.next_decoder.close();
        EndOfTrack::SessionEnded
    }
}

/// Gapless pre-decode state (Task 4.1). Purely additive: when a valid,
/// format-compatible successor has been pre-buffered, EOF hands off without
/// stopping the cpal stream; in EVERY other case these fields are ignored and
/// the existing gapped path runs unchanged.
#[derive(Default)]
struct PreDecodeState {
    buffer: Vec<f32>,
    track_id: Option<TrackId>,
    track_path: Option<PathBuf>,
    format: Option<AudioFormatInfo>,
    /// Whether the (one-shot) pre-encode attempt has run for this track.
    attempted: bool,
}

impl PreDecodeState {
    fn reset(&mut self) {
        self.buffer.clear();
        self.track_id = None;
        self.track_path = None;
        self.format = None;
        self.attempted = false;
    }
}

/// Outcome of the end-of-track handling in the decode loop.
#[derive(PartialEq, Eq)]
enum EndOfTrack {
    /// Gapless handoff happened; continue the loop with the promoted track.
    HandedOff,
    /// Session ended (gapped path or mid-handoff write error).
    SessionEnded,
}
