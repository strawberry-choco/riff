use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crate::app::traits::AudioOutput;
use crate::app::errors::AppError;
use crate::domain::PlaybackUpdate;

/// Shared audio buffer between decoder (producer) and cpal callback (consumer).
type AudioBuffer = Arc<Mutex<VecDeque<f32>>>;

/// Wrapper to make cpal::Stream Send-safe.
/// cpal::Stream is !Send on some platforms, but in practice it is safe
/// to send across threads as long as it is only dropped on the creating thread.
/// We ensure this by only using it on the dedicated audio thread.
struct SendStream(Option<cpal::Stream>);
unsafe impl Send for SendStream {}

pub struct CpalAudioOutput {
    host: cpal::Host,
    device: Option<cpal::Device>,
    stream: SendStream,
    sample_rate: u32,
    channels: u16,
    buffer: AudioBuffer,
    volume: Arc<AtomicU32>,
    error_tx: crossbeam_channel::Sender<PlaybackUpdate>,
}

impl CpalAudioOutput {
    pub fn new(error_tx: crossbeam_channel::Sender<PlaybackUpdate>) -> Self {
        Self {
            host: cpal::default_host(),
            device: None,
            stream: SendStream(None),
            sample_rate: 44100,
            channels: 2,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(65536))),
            volume: Arc::new(AtomicU32::new(f32::to_bits(1.0))),
            error_tx,
        }
    }
}

/// Convert a normalised f32 sample [-1.0, 1.0] to i16.
#[inline]
fn f32_to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// Convert a normalised f32 sample [-1.0, 1.0] to u16.
/// u16 silence is at 32768 (half of 65535).
#[inline]
fn f32_to_u16(v: f32) -> u16 {
    (v.clamp(-1.0, 1.0) * 32767.0 + 32768.0) as u16
}

impl AudioOutput for CpalAudioOutput {
    fn initialize(&mut self, sample_rate: u32, channels: u16) -> Result<(), AppError> {
        self.sample_rate = sample_rate;
        self.channels = channels;

        let device = self.host.default_output_device()
            .ok_or_else(|| AppError::AudioOutput("No default output device".to_string()))?;

        self.device = Some(device);
        Ok(())
    }

    fn start(&mut self) -> Result<(), AppError> {
        let device = self.device.as_ref()
            .ok_or_else(|| AppError::AudioOutput("Device not initialized".to_string()))?;

        let supported_config = device.default_output_config()
            .map_err(|e| AppError::AudioOutput(format!("Config error: {}", e)))?;

        let sample_format = supported_config.sample_format();

        // Build StreamConfig preferring the decoder's sample rate.
        // On Windows WASAPI shared mode, the device may only support its native
        // rate (commonly 48000 Hz), so we fall back to the default if the
        // requested rate is not supported.
        let stream_config = build_stream_config(device, self.sample_rate, self.channels, &supported_config);
        let buffer_clone = self.buffer.clone();
        let volume_clone = self.volume.clone();

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                device.build_output_stream(
                    stream_config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        audio_callback_f32(data, &buffer_clone, &volume_clone);
                    },
                    move |err| {
                        tracing::error!("Audio stream error: {}", err);
                    },
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                device.build_output_stream(
                    stream_config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        audio_callback_i16(data, &buffer_clone, &volume_clone);
                    },
                    move |err| {
                        tracing::error!("Audio stream error: {}", err);
                    },
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                device.build_output_stream(
                    stream_config,
                    move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                        audio_callback_u16(data, &buffer_clone, &volume_clone);
                    },
                    move |err| {
                        tracing::error!("Audio stream error: {}", err);
                    },
                    None,
                )
            }
            _ => {
                return Err(AppError::AudioOutput(
                    format!("Unsupported sample format: {:?}", sample_format)
                ));
            }
        }.map_err(|e| AppError::AudioOutput(format!("Stream error: {}", e)))?;

        stream.play()
            .map_err(|e| AppError::AudioOutput(format!("Play error: {}", e)))?;

        self.stream = SendStream(Some(stream));
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AppError> {
        if let Some(ref stream) = self.stream.0 {
            let _ = stream.pause();
        }
        self.stream = SendStream(None);
        Ok(())
    }

    fn buffer_len(&self) -> usize {
        if let Ok(buf) = self.buffer.lock() {
            buf.len()
        } else {
            0
        }
    }

    fn clear_buffer(&mut self) {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.clear();
        }
    }

    fn write_samples(&mut self, samples: &[f32]) -> Result<usize, AppError> {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.extend(samples.iter());
        }
        Ok(samples.len())
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume.store(f32::to_bits(volume), Ordering::Relaxed);
    }
}

/// Build a `StreamConfig` that tries to match the requested sample rate and
/// channels. Falls back to the device's default config when the requested
/// values are outside the device's supported range (common on Windows WASAPI
/// where shared mode is locked to the system sample rate).
fn build_stream_config(
    device: &cpal::Device,
    requested_rate: u32,
    requested_channels: u16,
    default_config: &cpal::SupportedStreamConfig,
) -> cpal::StreamConfig {
    let default_stream: cpal::StreamConfig = default_config.clone().into();
    
    // For Windows WASAPI shared mode, we need to be more careful about configuration.
    // The issue is that shared mode may not support arbitrary buffer sizes or sample rates.
    // Let's try to use the default configuration to avoid "Stream configuration is not supported" errors.
    let mut config = default_stream;
    
    // Only change channels if requested and supported
    if requested_channels != config.channels {
        // Check if the requested channels are supported
        if let Ok(mut supported_configs) = device.supported_output_configs() {
            if supported_configs.any(|range| {
                let channels_min = range.channels().min(2); // Use a reasonable default
                let channels_max = range.channels().max(2); // Use a reasonable default
                channels_min <= requested_channels && channels_max >= requested_channels
            }) {
                config.channels = requested_channels;
            }
        }
    }
    
    config
}

fn audio_callback_f32(data: &mut [f32], buffer: &AudioBuffer, volume: &Arc<AtomicU32>) {
    let mut buf = match buffer.try_lock() {
        Ok(b) => b,
        Err(_) => {
            data.fill(0.0);
            return;
        }
    };
    let vol = f32::from_bits(volume.load(Ordering::Relaxed));
    for sample in data.iter_mut() {
        *sample = buf.pop_front().unwrap_or(0.0) * vol;
    }
}

fn audio_callback_i16(data: &mut [i16], buffer: &AudioBuffer, volume: &Arc<AtomicU32>) {
    let mut buf = match buffer.try_lock() {
        Ok(b) => b,
        Err(_) => {
            data.fill(0);
            return;
        }
    };
    let vol = f32::from_bits(volume.load(Ordering::Relaxed));
    for sample in data.iter_mut() {
        let f32_val = buf.pop_front().unwrap_or(0.0) * vol;
        *sample = f32_to_i16(f32_val);
    }
}

fn audio_callback_u16(data: &mut [u16], buffer: &AudioBuffer, volume: &Arc<AtomicU32>) {
    let mut buf = match buffer.try_lock() {
        Ok(b) => b,
        Err(_) => {
            data.fill(32768);
            return;
        }
    };
    let vol = f32::from_bits(volume.load(Ordering::Relaxed));
    for sample in data.iter_mut() {
        let f32_val = buf.pop_front().unwrap_or(0.0) * vol;
        *sample = f32_to_u16(f32_val);
    }
}
