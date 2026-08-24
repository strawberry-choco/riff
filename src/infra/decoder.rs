use crate::app::errors::AppError;
use crate::app::gapless::{duration_from_frames, frames_from_duration};
use crate::app::traits::{AudioDecoder, AudioFormatInfo};
use std::path::Path;
use std::time::Duration;
use symphonia::core::audio::layouts::CHANNEL_LAYOUT_STEREO;
use symphonia::core::audio::AudioSpec;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;
use symphonia::default::get_probe;

pub struct SymphoniaDecoder {
    codec_registry: CodecRegistry,
    format_reader: Option<Box<dyn FormatReader>>,
    decoder: Option<Box<dyn symphonia::core::codecs::audio::AudioDecoder>>,
    track_id: u32,
    spec: Option<AudioSpec>,
    duration: Option<Duration>,
    pending_samples: Vec<f32>,
}

impl SymphoniaDecoder {
    pub fn new(codec_registry: CodecRegistry) -> Self {
        Self {
            codec_registry,
            format_reader: None,
            decoder: None,
            track_id: 0,
            spec: None,
            duration: None,
            pending_samples: Vec::new(),
        }
    }
}

impl AudioDecoder for SymphoniaDecoder {
    fn open(&mut self, path: &Path) -> Result<AudioFormatInfo, AppError> {
        let source = std::fs::File::open(path)
            .map_err(|e| AppError::Decode(format!("Failed to open file: {e}")))?;

        let mss = symphonia::core::io::MediaSourceStream::new(
            Box::new(source),
            symphonia::core::io::MediaSourceStreamOptions::default(),
        );

        let hint = Hint::new();
        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();
        let decoder_opts = AudioDecoderOptions::default();

        // In symphonia 0.6 the probe returns the format reader directly.
        let format: Box<dyn FormatReader> = get_probe()
            .probe(&hint, mss, format_opts, metadata_opts)
            .map_err(|e| AppError::Decode(format!("Probe error: {e}")))?;

        let tracks = format.tracks();

        let track = tracks
            .iter()
            .find(|t| {
                matches!(
                    t.codec_params,
                    Some(CodecParameters::Audio(ref p)) if p.codec != CODEC_ID_NULL_AUDIO
                )
            })
            .ok_or_else(|| AppError::Decode("No audio track found".to_string()))?;

        let track_id = track.id;
        let audio_params = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(p)) => p.clone(),
            _ => return Err(AppError::Decode("No audio track found".to_string())),
        };
        let track_channels = audio_params.channels.clone();
        let sample_rate = audio_params
            .sample_rate
            .ok_or_else(|| AppError::Decode("Unknown sample rate".to_string()))?;
        let channels = track_channels
            .as_ref()
            .map_or(2, |c| u16::try_from(c.count()).unwrap_or(u16::MAX));

        let duration = track
            .num_frames
            .map(|frames| duration_from_frames(frames, sample_rate));

        let decoder = self
            .codec_registry
            .make_audio_decoder(&audio_params, &decoder_opts)
            .map_err(|e| AppError::Decode(format!("Decoder creation failed: {e}")))?;

        self.track_id = track_id;
        self.duration = duration;
        self.spec = Some(AudioSpec::new(
            sample_rate,
            track_channels.unwrap_or(CHANNEL_LAYOUT_STEREO),
        ));

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

        let format = self
            .format_reader
            .as_mut()
            .ok_or_else(|| AppError::Decode("Decoder not open".to_string()))?;
        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| AppError::Decode("Decoder not initialized".to_string()))?;

        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            // In symphonia 0.6 end-of-stream is signalled with `Ok(None)`.
            Ok(None) => return Ok(None),
            Err(e) => return Err(AppError::Decode(format!("Packet read error: {e}"))),
        };

        if packet.track_id != self.track_id {
            return self.next_frames(max_samples);
        }

        let decoded_audio = decoder
            .decode(&packet)
            .map_err(|e| AppError::Decode(format!("Decode error: {e}")))?;

        let spec = decoded_audio.spec().clone();
        if self.spec.as_ref() != Some(&spec) {
            self.spec = Some(spec);
        }

        // Copy the decoded (any sample format) audio into interleaved f32 samples.
        let mut samples: Vec<f32> = Vec::with_capacity(decoded_audio.samples_interleaved());
        decoded_audio.copy_to_vec_interleaved::<f32>(&mut samples);

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
        let format = self
            .format_reader
            .as_mut()
            .ok_or_else(|| AppError::Decode("Decoder not open".to_string()))?;

        let sample_rate = self
            .spec
            .as_ref()
            .map_or(44100, symphonia::core::audio::AudioSpec::rate);
        let sample = frames_from_duration(position, sample_rate);

        let _ = format
            .seek(
                SeekMode::Accurate,
                SeekTo::Timestamp {
                    track_id: self.track_id,
                    ts: Timestamp::new(sample.cast_signed()),
                },
            )
            .map_err(|e| AppError::Decode(format!("Seek error: {e}")))?;

        self.pending_samples.clear();

        if let Some(decoder) = self.decoder.as_mut() {
            decoder.reset();
        }

        Ok(())
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn close(&mut self) {
        self.format_reader = None;
        self.decoder = None;
        self.spec = None;
        self.duration = None;
        self.pending_samples.clear();
    }
}
