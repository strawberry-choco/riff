use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crate::app::traits::AudioOutput;
use crate::app::errors::AppError;

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
}

impl CpalAudioOutput {
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
            device: None,
            stream: SendStream(None),
            sample_rate: 44100,
            channels: 2,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(65536))),
        }
    }
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

        let config = device.default_output_config()
            .map_err(|e| AppError::AudioOutput(format!("Config error: {}", e)))?;

        let sample_format = config.sample_format();
        let config: cpal::StreamConfig = config.into();

        let buffer_clone = self.buffer.clone();

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let mut buf = match buffer_clone.try_lock() {
                            Ok(b) => b,
                            Err(_) => {
                                // Can't lock — fill with silence
                                for sample in data.iter_mut() {
                                    *sample = 0.0;
                                }
                                return;
                            }
                        };
                        for sample in data.iter_mut() {
                            *sample = buf.pop_front().unwrap_or(0.0);
                        }
                    },
                    move |err| {
                        eprintln!("Audio stream error: {}", err);
                    },
                    None,
                )
            }
            _ => {
                return Err(AppError::AudioOutput("Unsupported sample format".to_string()));
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
        if let Ok(mut buf) = self.buffer.lock() {
            buf.clear();
        }
        self.stream = SendStream(None);
        Ok(())
    }

    fn write_samples(&mut self, samples: &[f32]) -> Result<usize, AppError> {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.extend(samples.iter());
        }
        Ok(samples.len())
    }

    fn set_volume(&mut self, _volume: f32) {
        // Volume is handled by the app layer (main.rs audio engine)
    }
}
