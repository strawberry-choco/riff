use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use riff_playback::app::errors::PlaybackError;
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Slack added on top of the decode loop's backpressure watermark
/// (`sample_rate * channels * 2`) when sizing the ring. A full decode chunk
/// (4096 samples) always fits, so steady-state writes never block or drop
/// samples.
const CHUNK_SLACK_SAMPLES: usize = 4096;

/// How long [`CpalAudioOutput::write_samples`] may wait on the callback to free
/// ring space before giving up. Only trips when the stream is effectively
/// dead (device gone) while the ring is full.
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);

/// Wrapper to make `cpal::Stream` Send-safe.
/// `cpal::Stream` is !Send on some platforms, but in practice it is safe
/// to send across threads as long as it is only dropped on the creating thread.
/// We ensure this by only using it on the dedicated audio thread.
struct SendStream(Option<cpal::Stream>);
unsafe impl Send for SendStream {}

pub struct CpalAudioOutput {
    host: cpal::Host,
    device: Option<cpal::Device>,
    stream: SendStream,
    sample_rate: u32,
    /// The sample rate the cpal stream was ACTUALLY built with (Task 4.1).
    /// On Windows WASAPI shared mode the device often locks to its default
    /// rate (commonly 48 kHz) regardless of the requested rate, so this can
    /// differ from `sample_rate`. The gapless format gate compares against it.
    effective_sample_rate: u32,
    channels: u16,
    /// Producer half of the lock-free SPSC ring between the decode loop
    /// (here, on the audio engine thread) and the cpal callback. A fresh
    /// ring is built per `initialize`; the consumer half moves into the
    /// stream closure at `start`.
    producer: Option<HeapProd<f32>>,
    /// Consumer half parked until `start` hands it to the cpal callback.
    pending_consumer: Option<HeapCons<f32>>,
    /// Flush generation shared with the callback. A producer-side clear bumps
    /// it; the callback detects the change and drains the ring, which is how a
    /// producer-side clear works without locks (the consumer owns the read
    /// cursor, so only it can reclaim the space). No production path bumps it
    /// today — teardown discards leftovers by replacing the ring per session —
    /// but the callback side stays armed so a clear can be added without
    /// touching the stream closures.
    flush_epoch: Arc<AtomicU64>,
    volume: Arc<AtomicU32>,
    /// `ReplayGain` linear factor (Task 4.3), stored as f32 bits like `volume`
    /// so the audio callback can read it lock-free. Defaults to 1.0 (no
    /// adjustment); the engine sets it per track and resets it on stop.
    replaygain: Arc<AtomicU32>,
}

impl CpalAudioOutput {
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
            device: None,
            stream: SendStream(None),
            sample_rate: 44100,
            effective_sample_rate: 44100,
            channels: 2,
            producer: None,
            pending_consumer: None,
            flush_epoch: Arc::new(AtomicU64::new(0)),
            volume: Arc::new(AtomicU32::new(f32::to_bits(1.0))),
            replaygain: Arc::new(AtomicU32::new(f32::to_bits(1.0))),
        }
    }
}

impl Default for CpalAudioOutput {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a normalised f32 sample [-1.0, 1.0] to i16.
// The value is clamped to [-1.0, 1.0] before scaling, so the product is
// guaranteed to be within the i16 range; the cast below cannot truncate
// (the `as` cast is intentional sample quantization, hence the allow).
#[allow(clippy::cast_possible_truncation)]
#[inline]
fn f32_to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// Convert a normalised f32 sample [-1.0, 1.0] to u16.
/// u16 silence is at 32768 (half of 65535).
// The clamped-and-shifted value lies within [1, 65535] ⊂ u16, so the cast
// below can neither truncate nor lose a sign (hence the allow).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[inline]
fn f32_to_u16(v: f32) -> u16 {
    (v.clamp(-1.0, 1.0) * 32767.0 + 32768.0) as u16
}

/// Drain the ring if a `clear_buffer` flush happened since the callback last
/// looked. Called at the START of every callback invocation; stale samples
/// left by the flush are discarded so playback resumes at whatever the
/// producer writes after the clear.
#[inline]
fn absorb_flush_start(cons: &mut HeapCons<f32>, flush_epoch: &AtomicU64, seen: &mut u64) {
    let epoch = flush_epoch.load(Ordering::Acquire);
    if epoch != *seen {
        *seen = epoch;
        // Drop every queued sample; the producer-side clear cannot advance
        // the read cursor itself, so the consumer reclaims the space here.
        cons.skip(usize::MAX);
    }
}

/// Re-check for a flush at the END of a callback invocation. A clear may race
/// with samples already popped into the output buffer this invocation; those
/// must not be played, so the caller silences its whole buffer and drains.
#[inline]
fn absorb_flush_end(cons: &mut HeapCons<f32>, flush_epoch: &AtomicU64, seen: &mut u64) -> bool {
    let epoch = flush_epoch.load(Ordering::Acquire);
    if epoch == *seen {
        return false;
    }
    *seen = epoch;
    cons.skip(usize::MAX);
    true
}

fn audio_callback_f32(
    data: &mut [f32],
    cons: &mut HeapCons<f32>,
    flush_epoch: &AtomicU64,
    seen: &mut u64,
    volume: &AtomicU32,
    replaygain: &AtomicU32,
) {
    absorb_flush_start(cons, flush_epoch, seen);
    let vol = f32::from_bits(volume.load(Ordering::Relaxed));
    let rg = f32::from_bits(replaygain.load(Ordering::Relaxed));
    let filled = cons.pop_slice(data);
    if absorb_flush_end(cons, flush_epoch, seen) {
        // A clear raced with this callback: discard everything produced here.
        data.fill(0.0);
        return;
    }
    for sample in &mut data[..filled] {
        *sample *= vol * rg;
    }
    data[filled..].fill(0.0);
}

fn audio_callback_i16(
    data: &mut [i16],
    cons: &mut HeapCons<f32>,
    flush_epoch: &AtomicU64,
    seen: &mut u64,
    volume: &AtomicU32,
    replaygain: &AtomicU32,
) {
    absorb_flush_start(cons, flush_epoch, seen);
    let vol = f32::from_bits(volume.load(Ordering::Relaxed));
    let rg = f32::from_bits(replaygain.load(Ordering::Relaxed));
    for slot in data.iter_mut() {
        let f32_val = cons.try_pop().map_or(0.0, |s| s * vol * rg);
        *slot = f32_to_i16(f32_val);
    }
    if absorb_flush_end(cons, flush_epoch, seen) {
        // A clear raced with this callback: discard everything produced here.
        data.fill(0);
    }
}

fn audio_callback_u16(
    data: &mut [u16],
    cons: &mut HeapCons<f32>,
    flush_epoch: &AtomicU64,
    seen: &mut u64,
    volume: &AtomicU32,
    replaygain: &AtomicU32,
) {
    absorb_flush_start(cons, flush_epoch, seen);
    let vol = f32::from_bits(volume.load(Ordering::Relaxed));
    let rg = f32::from_bits(replaygain.load(Ordering::Relaxed));
    for slot in data.iter_mut() {
        let f32_val = cons.try_pop().map_or(0.0, |s| s * vol * rg);
        *slot = f32_to_u16(f32_val);
    }
    if absorb_flush_end(cons, flush_epoch, seen) {
        // A clear raced with this callback: discard everything produced here.
        data.fill(32768);
    }
}

impl CpalAudioOutput {
    /// Pick the default output device and build a fresh ring sized to the
    /// decode loop's backpressure watermark.
    fn initialize(&mut self, sample_rate: u32, channels: u16) -> Result<(), PlaybackError> {
        self.sample_rate = sample_rate;
        self.channels = channels;

        let device = self
            .host
            .default_output_device()
            .ok_or_else(|| PlaybackError::AudioOutput("No default output device".to_string()))?;

        // Fresh ring per session, sized to the decode loop's backpressure
        // watermark plus one chunk of slack (see `CHUNK_SLACK_SAMPLES`).
        // Replacing the ring also discards any samples left over from the
        // previous session without relying on callback cooperation.
        let capacity = (sample_rate as usize) * usize::from(channels) * 2 + CHUNK_SLACK_SAMPLES;
        let (producer, consumer) = HeapRb::<f32>::new(capacity.max(1)).split();
        self.producer = Some(producer);
        self.pending_consumer = Some(consumer);

        self.device = Some(device);
        Ok(())
    }

    fn write_samples(&mut self, samples: &[f32]) -> Result<usize, PlaybackError> {
        let Some(producer) = self.producer.as_mut() else {
            return Err(PlaybackError::AudioOutput(
                "Audio output not initialized".to_string(),
            ));
        };
        // Push as much as fits, waiting for the callback to free space rather
        // than dropping samples (the old shared VecDeque grew unboundedly; the
        // ring is bounded). Under normal playback the watermark plus slack
        // guarantees the whole chunk fits on the first push; the retry loop
        // matters for the gapless handoff's bulk pre-buffer write, which the
        // callback drains in real time.
        let deadline = Instant::now() + WRITE_TIMEOUT;
        let mut written = 0usize;
        while written < samples.len() {
            written += producer.push_slice(&samples[written..]);
            if written == samples.len() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(PlaybackError::AudioOutput(format!(
                    "Audio output buffer stalled: wrote {written}/{} samples",
                    samples.len()
                )));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(written)
    }

    pub fn effective_sample_rate(&self) -> u32 {
        self.effective_sample_rate
    }
}

/// The layer the port adds over an adapter failure.
///
/// This port used to reach `start`'s stream-opening half through an inherent
/// method and wrap that method's whole result, so every refusal was reported
/// twice: once as the adapter's `AudioOutput` reason and once as the port's.
/// The engine forwards that text to the frontend as `PlaybackUpdate::Error`, so
/// the fold that inlined the body keeps the wrap — applied at each fallible
/// step, where the forwarder applied it over the whole result — rather than
/// letting a message quietly lose a prefix.
fn wrap_as_port_error(e: PlaybackError) -> PlaybackError {
    PlaybackError::AudioOutput(e.to_string())
}

/// The playback engine's output port, served over the richer backend port
/// above: the engine's single `start(format)` maps onto the backend's
/// `initialize` + `start` pair (which owns the device-default-rate fallback).
/// A stalled ring write is swallowed here rather than surfaced, because the
/// engine's response to it is to keep decoding — the port that used to carry
/// the short-write count upstream dropped it at both call sites anyway.
impl riff_playback::infra::ports::AudioOutput for CpalAudioOutput {
    fn start(
        &mut self,
        format: riff_playback::infra::ports::AudioFormatInfo,
    ) -> Result<(), riff_playback::app::errors::PlaybackError> {
        self.initialize(format.sample_rate, format.channels)
            .map_err(wrap_as_port_error)?;

        // The stream-opening half this port used to forward to an inherent
        // `start`, inlined: every fallible step below is wrapped by
        // `wrap_as_port_error` where that forwarder wrapped the whole result,
        // so a refused start reports exactly the text it always has.
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| PlaybackError::AudioOutput("Device not initialized".to_string()))
            .map_err(wrap_as_port_error)?;
        let mut consumer = self
            .pending_consumer
            .take()
            .ok_or_else(|| PlaybackError::AudioOutput("Output not initialized".to_string()))
            .map_err(wrap_as_port_error)?;

        let supported_config = device
            .default_output_config()
            .map_err(|e| PlaybackError::AudioOutput(format!("Config error: {e}")))
            .map_err(wrap_as_port_error)?;

        let sample_format = supported_config.sample_format();

        // Build StreamConfig preferring the decoder's sample rate.
        // On Windows WASAPI shared mode, the device may only support its native
        // rate (commonly 48000 Hz), so we fall back to the default if the
        // requested rate is not supported.
        let stream_config =
            build_stream_config(device, self.sample_rate, self.channels, &supported_config);
        // Record the rate the stream is ACTUALLY built with (Task 4.1).
        self.effective_sample_rate = stream_config.sample_rate;
        let volume_clone = self.volume.clone();
        let replaygain_clone = self.replaygain.clone();
        let flush_epoch = self.flush_epoch.clone();
        // The callback tracks the flush generation it has already absorbed.
        let mut seen_flush = flush_epoch.load(Ordering::Acquire);

        // Only one arm runs, but each needs to own the consumer half.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    audio_callback_f32(
                        data,
                        &mut consumer,
                        &flush_epoch,
                        &mut seen_flush,
                        &volume_clone,
                        &replaygain_clone,
                    );
                },
                move |err| {
                    tracing::error!("Audio stream error: {}", err);
                },
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                stream_config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    audio_callback_i16(
                        data,
                        &mut consumer,
                        &flush_epoch,
                        &mut seen_flush,
                        &volume_clone,
                        &replaygain_clone,
                    );
                },
                move |err| {
                    tracing::error!("Audio stream error: {}", err);
                },
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                stream_config,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    audio_callback_u16(
                        data,
                        &mut consumer,
                        &flush_epoch,
                        &mut seen_flush,
                        &volume_clone,
                        &replaygain_clone,
                    );
                },
                move |err| {
                    tracing::error!("Audio stream error: {}", err);
                },
                None,
            ),
            _ => {
                return Err(wrap_as_port_error(PlaybackError::AudioOutput(format!(
                    "Unsupported sample format: {sample_format:?}"
                ))));
            }
        }
        .map_err(|e| PlaybackError::AudioOutput(format!("Stream error: {e}")))
        .map_err(wrap_as_port_error)?;

        stream
            .play()
            .map_err(|e| PlaybackError::AudioOutput(format!("Play error: {e}")))
            .map_err(wrap_as_port_error)?;

        self.stream = SendStream(Some(stream));
        Ok(())
    }

    fn write(&mut self, samples: &[f32]) {
        // The verdict stays inside: a ring that never frees is reported by
        // `write_samples` as a stall carrying how far it got, which this port
        // now swallows the only way it can — the caller has no count to act
        // on, and must never have one. Everything it decides (bounded ring,
        // `WRITE_TIMEOUT` deadline, no dropped samples) stays right here.
        let _ = self.write_samples(samples);
    }

    fn stop(&mut self) {
        if let Some(ref stream) = self.stream.0 {
            let _ = stream.pause();
        }
        // Dropping the stream joins the callback and drops the consumer half
        // parked inside its closure. Any samples left in the ring are
        // discarded when the next session's `initialize` builds a fresh ring.
        self.stream = SendStream(None);
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume.store(f32::to_bits(volume), Ordering::Relaxed);
    }

    fn set_replaygain(&mut self, factor: f32) {
        self.replaygain
            .store(f32::to_bits(factor), Ordering::Relaxed);
    }
}

/// Build a `StreamConfig` that tries to match the requested sample rate and
/// channels. Falls back to the device's default config when the requested
/// values are outside the device's supported range (common on Windows WASAPI
/// where shared mode is locked to the system sample rate).
fn build_stream_config(
    device: &cpal::Device,
    _requested_rate: u32,
    requested_channels: u16,
    default_config: &cpal::SupportedStreamConfig,
) -> cpal::StreamConfig {
    let default_stream: cpal::StreamConfig = (*default_config).into();

    // For Windows WASAPI shared mode, we need to be more careful about configuration.
    // The issue is that shared mode may not support arbitrary buffer sizes or sample rates.
    // Let's try to use the default configuration to avoid "Stream configuration is not supported" errors.
    let mut config = default_stream;

    // Only change channels if requested and supported
    if requested_channels != config.channels {
        // Check if the requested channels are supported
        if let Ok(mut supported_configs) = device.supported_output_configs()
            && supported_configs.any(|range| {
                let channels_min = range.channels().min(2); // Use a reasonable default
                let channels_max = range.channels().max(2); // Use a reasonable default
                channels_min <= requested_channels && channels_max >= requested_channels
            })
        {
            config.channels = requested_channels;
        }
    }

    config
}

#[cfg(test)]
mod write_policy_tests {
    use super::*;
    use riff_playback::infra::ports::AudioOutput;

    /// The stall deadline the adapter's write policy runs on, asserted so the
    /// 500 ms figure the port's contract leans on cannot drift silently.
    /// What it does not lock is the shape of the loop itself — that is what
    /// the two `write_samples` tests below are for.
    #[test]
    fn the_stall_deadline_is_five_hundred_milliseconds() {
        assert_eq!(WRITE_TIMEOUT, Duration::from_millis(500));
        // The ring's slack is sized so a full decode chunk always fits: a
        // ring without it would turn a steady-state write into a stall.
        assert_eq!(CHUNK_SLACK_SAMPLES, 4096);
    }

    /// A fresh output with a producer half installed and nobody draining it:
    /// the ring is full and stays full, the shape a dead device leaves behind.
    fn output_with_ring(ring_samples: usize) -> (CpalAudioOutput, HeapCons<f32>) {
        let mut output = CpalAudioOutput::new();
        let (producer, consumer) = HeapRb::<f32>::new(ring_samples).split();
        output.producer = Some(producer);
        (output, consumer)
    }

    /// The device-gone path: with no producer the write reports a failure, it
    /// does not quietly accept nothing and call it a short write. The port
    /// swallows that failure either way — before this change it surfaced as a
    /// `0` return, now as no return at all — so both spellings must leave the
    /// adapter untouched.
    #[test]
    fn a_write_without_an_initialized_stream_reports_a_failure_not_a_short_write() {
        let mut output = CpalAudioOutput::new();

        let samples = vec![0.0_f32; 64];
        assert!(
            output.write_samples(&samples).is_err(),
            "no producer means no stream, which is a failure — not a short write"
        );

        // Through the port, the failure stays swallowed: the call completes and
        // the adapter is left exactly as it was.
        output.write(&samples);
        output.write(&samples);
        assert!(output.producer.is_none(), "the adapter stays uninitialized");
        assert!(output.device.is_none(), "and still holds no device");
        assert_eq!(
            output.effective_sample_rate(),
            44100,
            "and its rate is its default"
        );
    }

    /// A ring that never frees: the write waits out the deadline and then
    /// reports a stall carrying how far it got. This is the bounded ring plus
    /// the 500 ms deadline, and it must survive the port losing its count.
    #[test]
    fn a_write_into_a_full_ring_reports_a_stall_after_the_deadline() {
        let (mut output, _sealed_consumer) = output_with_ring(4);
        let samples = vec![0.0_f32; 8];

        let started = Instant::now();
        let stall = output
            .write_samples(&samples)
            .expect_err("a ring that never frees must report a stall");
        let waited = started.elapsed();

        assert!(
            waited >= WRITE_TIMEOUT,
            "the write waits the full deadline before giving up (waited {waited:?})"
        );
        assert!(
            stall.to_string().contains("stalled"),
            "the stall names itself: {stall}"
        );
        assert!(
            stall.to_string().contains("4/8"),
            "and reports how far it got into the chunk: {stall}"
        );
    }

    /// A ring with room takes the whole chunk in one push and never reports a
    /// short write — the steady-state case the slack is there to guarantee.
    #[test]
    fn a_write_that_fits_the_ring_takes_the_whole_chunk() {
        let (mut output, _consumer) = output_with_ring(4096);

        let samples = vec![0.5_f32; 3000];
        let written = output
            .write_samples(&samples)
            .expect("a roomy ring takes the whole chunk");

        assert_eq!(
            written,
            samples.len(),
            "no chunk that fits is ever reported short"
        );
    }
}

#[cfg(test)]
mod port_surface_tests {
    use riff_playback::infra::ports::{AudioFormatInfo, AudioOutput};

    use super::*;

    /// The port's volume write lands in the atomic the audio callback reads
    /// lock-free. Volume is a level, not a stack: a second write replaces it.
    #[test]
    fn set_volume_through_the_port_reaches_the_callback() {
        let mut output = CpalAudioOutput::new();

        AudioOutput::set_volume(&mut output, 0.25);
        assert_eq!(output.volume.load(Ordering::Relaxed), f32::to_bits(0.25));

        AudioOutput::set_volume(&mut output, 1.0);
        assert_eq!(
            output.volume.load(Ordering::Relaxed),
            f32::to_bits(1.0),
            "a later volume replaces the earlier one"
        );
    }

    /// The port's `ReplayGain` write lands in its own atomic, starts at 1.0
    /// (no adjustment), and leaves volume alone: the two are separate knobs
    /// the callback multiplies together.
    #[test]
    fn set_replaygain_through_the_port_reaches_the_callback() {
        let mut output = CpalAudioOutput::new();
        assert_eq!(
            output.replaygain.load(Ordering::Relaxed),
            f32::to_bits(1.0),
            "the default is no adjustment"
        );

        AudioOutput::set_replaygain(&mut output, 0.5);
        assert_eq!(output.replaygain.load(Ordering::Relaxed), f32::to_bits(0.5));
        assert_eq!(
            output.volume.load(Ordering::Relaxed),
            f32::to_bits(1.0),
            "setting ReplayGain leaves volume untouched"
        );
    }

    /// `stop` through the port on an output that never started tears down
    /// nothing and is safe to repeat.
    ///
    /// With a producer half installed the lock gets teeth: `stop` tears down
    /// the stream and nothing else — the ring is `initialize`'s to replace, per
    /// session — so an inline that also cleared the ring (or forgot the
    /// teardown entirely) fails here. The pause-and-drop half needs a live
    /// stream, which needs a device; that part stays in the adapter's own
    /// hands.
    #[test]
    fn stop_through_the_port_on_an_unstarted_output_is_a_no_op() {
        let mut output = CpalAudioOutput::new();

        AudioOutput::stop(&mut output);
        assert!(output.stream.0.is_none(), "there is no stream to tear down");

        AudioOutput::stop(&mut output);
        assert!(output.stream.0.is_none(), "and stopping again is safe");

        // A producer half stands in for the ring a session installs.
        let (producer, _consumer) = HeapRb::<f32>::new(16).split();
        output.producer = Some(producer);
        AudioOutput::stop(&mut output);
        assert!(
            output.producer.is_some(),
            "stop tears down the stream, never the ring a session owns"
        );
    }

    /// `start` through the port reports a failure the way it always has.
    ///
    /// The port opens the device (`initialize`) and then the stream, wrapping
    /// every failure of its own — that text is what the engine forwards to the
    /// frontend as `PlaybackUpdate::Error`. Two starts are attempted because
    /// the refusal a machine can produce differs: where there is no default
    /// output device the first refuses with "No default output device", and
    /// where there is one, every `initialize` installs a fresh consumer half,
    /// so a start that reaches the stream succeeds and no failure is producible
    /// from outside the adapter. So the lock is conditional: **when** a start
    /// refuses, its reason arrives under the port's own `AudioOutput` layer, on
    /// top of the adapter's — never bare.
    #[test]
    fn start_through_the_port_reports_a_failure_the_way_it_always_has() {
        let mut output = CpalAudioOutput::new();
        let format = AudioFormatInfo {
            sample_rate: 44_100,
            channels: 2,
        };

        let first = AudioOutput::start(&mut output, format.clone());
        let first_accepted = first.is_ok();
        let second = AudioOutput::start(&mut output, format);

        let failures: Vec<PlaybackError> = [first, second]
            .into_iter()
            .filter_map(Result::err)
            .collect::<Vec<_>>();
        for failure in &failures {
            assert!(
                failure
                    .to_string()
                    .starts_with("Audio output error: Audio output error: "),
                "the port keeps its layer over the adapter's own reason: {failure}"
            );
        }

        // Whatever the machine could produce, the adapter is left holding
        // exactly it: an accepted start's stream, or nothing at all.
        if first_accepted {
            assert!(
                output.stream.0.is_some(),
                "an accepted start leaves its stream held"
            );
        } else {
            assert!(
                output.stream.0.is_none(),
                "a refused start leaves no stream behind"
            );
        }
    }
}
