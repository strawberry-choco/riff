use std::path::PathBuf;
use std::time::Duration;
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_codecs;
use symphonia::default::get_probe;
use crate::app::traits::{AudioDecoder, AudioFormatInfo};
use crate::app::errors::AppError;

pub struct SymphoniaDecoder {
    format_reader: Option<Box<dyn FormatReader>>,
    decoder: Option<Box<dyn symphonia::core::codecs::Decoder>>,
    track_id: u32,
    sample_buffer: Option<SampleBuffer<f32>>,
    spec: Option<SignalSpec>,
    duration: Option<Duration>,
    /// Samples left over from a previous decoded packet that exceeded max_samples.
    pending_samples: Vec<f32>,
}

impl SymphoniaDecoder {
    pub fn new() -> Self {
        Self {
            format_reader: None,
            decoder: None,
            track_id: 0,
            sample_buffer: None,
            spec: None,
            duration: None,
            pending_samples: Vec::new(),
        }
    }
}

impl AudioDecoder for SymphoniaDecoder {
    fn open(&mut self, path: &PathBuf) -> Result<AudioFormatInfo, AppError> {
        let source = std::fs::File::open(path)
            .map_err(|e| AppError::Decode(format!("Failed to open file: {}", e)))?;

        let mss = symphonia::core::io::MediaSourceStream::new(
            Box::new(source),
            symphonia::core::io::MediaSourceStreamOptions::default(),
        );

        let hint = Hint::new();
        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();
        let decoder_opts = DecoderOptions::default();

        let probed = get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| AppError::Decode(format!("Probe error: {}", e)))?;

        let format = probed.format;
        let tracks = format.tracks();

        let track = tracks.iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| AppError::Decode("No audio track found".to_string()))?;

        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate
            .ok_or_else(|| AppError::Decode("Unknown sample rate".to_string()))?;
        let channels = track.codec_params.channels
            .map(|c| c.count() as u16)
            .unwrap_or(2);

        let duration = track.codec_params.n_frames
            .map(|frames| Duration::from_secs_f64(frames as f64 / sample_rate as f64));

        let decoder = get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|e| AppError::Decode(format!("Decoder creation failed: {}", e)))?;

        self.track_id = track_id;
        self.duration = duration;
        self.spec = Some(SignalSpec::new(sample_rate, track.codec_params.channels.unwrap_or(symphonia::core::audio::Channels::FRONT_LEFT | symphonia::core::audio::Channels::FRONT_RIGHT)));

        self.sample_buffer = None;
        self.pending_samples.clear();
        self.decoder = Some(decoder);
        self.format_reader = Some(format);

        Ok(AudioFormatInfo {
            sample_rate,
            channels,
            duration,
        })
    }

    fn next_frames(&mut self, max_samples: usize) -> Result<Option<Vec<f32>>, AppError> {
        // Drain leftover samples from a previous oversized decode before decoding more.
        if !self.pending_samples.is_empty() {
            let available = self.pending_samples.len();
            let to_return = max_samples.min(available);
            let result: Vec<f32> = self.pending_samples.drain(..to_return).collect();
            return Ok(Some(result));
        }

        let format = self.format_reader.as_mut()
            .ok_or_else(|| AppError::Decode("Decoder not open".to_string()))?;
        let decoder = self.decoder.as_mut()
            .ok_or_else(|| AppError::Decode("Decoder not initialized".to_string()))?;

        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(e) => return Err(AppError::Decode(format!("Packet read error: {}", e))),
        };

        if packet.track_id() != self.track_id {
            return self.next_frames(max_samples);
        }

        let decoded = decoder.decode(&packet)
            .map_err(|e| AppError::Decode(format!("Decode error: {}", e)))?;

        let spec = *decoded.spec();
        let spec_changed = self.spec.as_ref().map_or(true, |s| s != &spec);
        if spec_changed || self.sample_buffer.is_none() {
            let duration = decoded.capacity() as u64;
            self.sample_buffer = Some(SampleBuffer::<f32>::new(duration, spec));
            if spec_changed {
                self.spec = Some(spec);
            }
        }

        let sample_buffer = self.sample_buffer.as_mut().unwrap();
        sample_buffer.copy_interleaved_ref(decoded);
        let samples = sample_buffer.samples().to_vec();

        let total = samples.len();
        if total <= max_samples {
            Ok(Some(samples))
        } else {
            // Packet had more samples than requested — buffer the rest.
            self.pending_samples = samples[max_samples..].to_vec();
            Ok(Some(samples[..max_samples].to_vec()))
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), AppError> {
        let format = self.format_reader.as_mut()
            .ok_or_else(|| AppError::Decode("Decoder not open".to_string()))?;

        let sample_rate = self.spec.as_ref().map(|s| s.rate).unwrap_or(44100);
        let sample = (position.as_secs_f64() * sample_rate as f64) as u64;

        let _ = format.seek(
            symphonia::core::formats::SeekMode::Accurate,
            symphonia::core::formats::SeekTo::TimeStamp {
                track_id: self.track_id,
                ts: sample,
            },
        ).map_err(|e| AppError::Decode(format!("Seek error: {}", e)))?;

        self.pending_samples.clear();

        if let Some(decoder) = self.decoder.as_mut() {
            decoder.reset();
        }

        Ok(())
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }
}
