use crate::app::errors::PlaybackError;
use crate::app::traits::AudioOutput;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Slack added on top of the engine's backpressure watermark
/// (`sample_rate * channels * 2`) when sizing the ring. After the engine's
/// `buffer_len() < max_buffer_samples` check a full decode chunk (4096
/// samples) always fits, so steady-state writes never block or drop samples.
const CHUNK_SLACK_SAMPLES: usize = 4096;

/// How long [`AudioOutput::write_samples`] may wait on the callback to free
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
    /// Flush generation shared with the callback. `clear_buffer` bumps it;
    /// the callback detects the change and drains the ring, which is how a
    /// producer-side clear works without locks (the consumer owns the read
    /// cursor, so only it can reclaim the space).
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

impl AudioOutput for CpalAudioOutput {
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

    fn start(&mut self) -> Result<(), PlaybackError> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| PlaybackError::AudioOutput("Device not initialized".to_string()))?;
        let mut consumer = self
            .pending_consumer
            .take()
            .ok_or_else(|| PlaybackError::AudioOutput("Output not initialized".to_string()))?;

        let supported_config = device
            .default_output_config()
            .map_err(|e| PlaybackError::AudioOutput(format!("Config error: {e}")))?;

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
                return Err(PlaybackError::AudioOutput(format!(
                    "Unsupported sample format: {sample_format:?}"
                )));
            }
        }
        .map_err(|e| PlaybackError::AudioOutput(format!("Stream error: {e}")))?;

        stream
            .play()
            .map_err(|e| PlaybackError::AudioOutput(format!("Play error: {e}")))?;

        self.stream = SendStream(Some(stream));
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlaybackError> {
        if let Some(ref stream) = self.stream.0 {
            let _ = stream.pause();
        }
        // Dropping the stream joins the callback and drops the consumer half
        // parked inside its closure. Any samples left in the ring are
        // discarded when the next session's `initialize` builds a fresh ring.
        self.stream = SendStream(None);
        Ok(())
    }

    fn buffer_len(&self) -> usize {
        // Lock-free: the producer side can observe the fill level while the
        // callback consumes concurrently.
        self.producer.as_ref().map_or(0, Observer::occupied_len)
    }

    fn clear_buffer(&mut self) {
        // Producer-side clear: bump the flush generation. The callback notices
        // on its next invocation (and re-checks at the end of the current one,
        // so a mid-callback clear can never play stale samples), drains the
        // ring, and thereby hands the space back to the producer. Every
        // `clear_buffer` call site either has a live stream to service the
        // drain or is followed by `initialize`, which replaces the ring.
        if self.producer.is_some() {
            self.flush_epoch.fetch_add(1, Ordering::Release);
        }
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

    fn set_volume(&mut self, volume: f32) {
        self.volume.store(f32::to_bits(volume), Ordering::Relaxed);
    }

    fn set_replaygain(&mut self, factor: f32) {
        self.replaygain
            .store(f32::to_bits(factor), Ordering::Relaxed);
    }

    fn effective_sample_rate(&self) -> u32 {
        self.effective_sample_rate
    }
}

/// The playback engine's output port, served over the richer backend port
/// above: the engine's single `start(format)` maps onto the backend's
/// `initialize` + `start` pair (which owns the device-default-rate fallback),
/// and a stalled ring write surfaces as a short-write instead of an error.
impl riff_playback::infra::ports::AudioOutput for CpalAudioOutput {
    fn start(
        &mut self,
        format: riff_playback::infra::ports::AudioFormatInfo,
    ) -> Result<(), riff_playback::app::errors::PlaybackError> {
        AudioOutput::initialize(self, format.sample_rate, format.channels)
            .map_err(|e| riff_playback::app::errors::PlaybackError::AudioOutput(e.to_string()))?;
        AudioOutput::start(self)
            .map_err(|e| riff_playback::app::errors::PlaybackError::AudioOutput(e.to_string()))
    }

    fn write(&mut self, samples: &[f32]) -> usize {
        AudioOutput::write_samples(self, samples).unwrap_or(0)
    }

    fn stop(&mut self) {
        let _ = AudioOutput::stop(self);
    }

    fn set_volume(&mut self, volume: f32) {
        AudioOutput::set_volume(self, volume);
    }

    fn latency(&self) -> u32 {
        // Approximate the buffered latency in frames from the ring fill level.
        u32::try_from(self.buffer_len() / usize::from(self.channels.max(1))).unwrap_or(u32::MAX)
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
