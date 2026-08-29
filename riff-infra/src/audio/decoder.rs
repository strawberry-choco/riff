use riff_playback::app::errors::PlaybackError;
use riff_playback::domain::{duration_from_frames, frames_from_duration};
use riff_playback::infra::ports::AudioDecoder as EngineAudioDecoder;
use riff_playback::infra::ports::AudioFormatInfo as EngineAudioFormatInfo;
use std::path::{Path, PathBuf};
use std::time::Duration;
use symphonia::core::audio::AudioSpec;
use symphonia::core::audio::layouts::CHANNEL_LAYOUT_STEREO;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;
use symphonia::default::get_probe;

/// Format facts reported when a source is opened: the stream's shape plus its
/// total duration when the container declares one.
#[derive(Debug, Clone)]
pub struct DecoderFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Option<Duration>,
}

pub struct SymphoniaDecoder {
    codec_registry: CodecRegistry,
    format_reader: Option<Box<dyn FormatReader>>,
    decoder: Option<Box<dyn symphonia::core::codecs::audio::AudioDecoder>>,
    track_id: u32,
    spec: Option<AudioSpec>,
    /// The file this decoder was initialized with (empty when closed).
    source_path: PathBuf,
    duration: Option<Duration>,
    pending_samples: Vec<f32>,
    /// Reusable interleaved-samples scratch for decoded packets. Kept across
    /// packets so steady-state decoding performs no per-packet heap
    /// allocations (allocation-optimization plan, task 3.2).
    scratch: Vec<f32>,
}

/// Build the shared codec registry: symphonia's enabled defaults plus the
/// Opus adapter. The native codec dependencies live in this crate, so the
/// registry is built here and the composition root never names symphonia.
#[must_use]
pub fn default_codec_registry() -> CodecRegistry {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

impl SymphoniaDecoder {
    pub fn new(codec_registry: CodecRegistry) -> Self {
        Self {
            codec_registry,
            format_reader: None,
            decoder: None,
            track_id: 0,
            spec: None,
            source_path: PathBuf::new(),
            duration: None,
            pending_samples: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// Open the file at `path`, probe its container, and mint a decoder for
    /// the first audio track.
    pub fn open(&mut self, path: &Path) -> Result<DecoderFormat, PlaybackError> {
        self.source_path = path.to_path_buf();
        let source = std::fs::File::open(path)
            .map_err(|e| PlaybackError::Decode(format!("Failed to open file: {e}")))?;

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
            .map_err(|e| PlaybackError::Decode(format!("Probe error: {e}")))?;

        let tracks = format.tracks();

        let track = tracks
            .iter()
            .find(|t| {
                matches!(
                    t.codec_params,
                    Some(CodecParameters::Audio(ref p)) if p.codec != CODEC_ID_NULL_AUDIO
                )
            })
            .ok_or_else(|| PlaybackError::Decode("No audio track found".to_string()))?;

        let track_id = track.id;
        let audio_params = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(p)) => p.clone(),
            _ => return Err(PlaybackError::Decode("No audio track found".to_string())),
        };
        let track_channels = audio_params.channels.clone();
        let sample_rate = audio_params
            .sample_rate
            .ok_or_else(|| PlaybackError::Decode("Unknown sample rate".to_string()))?;
        let channels = track_channels
            .as_ref()
            .map_or(2, |c| u16::try_from(c.count()).unwrap_or(u16::MAX));

        let duration = track
            .num_frames
            .map(|frames| duration_from_frames(frames, sample_rate));

        let decoder = self
            .codec_registry
            .make_audio_decoder(&audio_params, &decoder_opts)
            .map_err(|e| PlaybackError::Decode(format!("Decoder creation failed: {e}")))?;

        self.track_id = track_id;
        self.duration = duration;
        self.spec = Some(AudioSpec::new(
            sample_rate,
            track_channels.unwrap_or(CHANNEL_LAYOUT_STEREO),
        ));

        self.pending_samples.clear();
        self.scratch.clear();
        self.decoder = Some(decoder);
        self.format_reader = Some(format);

        Ok(DecoderFormat {
            sample_rate,
            channels,
            duration,
        })
    }

    pub fn next_frames(&mut self, out: &mut [f32]) -> Result<usize, PlaybackError> {
        // Drain leftover samples from a previous oversized decode before decoding more.
        if !self.pending_samples.is_empty() {
            let available = self.pending_samples.len();
            let to_return = out.len().min(available);
            out[..to_return].copy_from_slice(&self.pending_samples[..to_return]);
            // Shift the remainder to the front in place — no reallocation.
            self.pending_samples.copy_within(to_return.., 0);
            self.pending_samples.truncate(available - to_return);
            return Ok(to_return);
        }

        let format = self
            .format_reader
            .as_mut()
            .ok_or_else(|| PlaybackError::Decode("Decoder not open".to_string()))?;
        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| PlaybackError::Decode("Decoder not initialized".to_string()))?;

        loop {
            let packet = match format.next_packet() {
                Ok(Some(packet)) => packet,
                // In symphonia 0.6 end-of-stream is signalled with `Ok(None)`.
                Ok(None) => return Ok(0),
                Err(e) => return Err(PlaybackError::Decode(format!("Packet read error: {e}"))),
            };

            if packet.track_id != self.track_id {
                continue;
            }

            let decoded_audio = decoder
                .decode(&packet)
                .map_err(|e| PlaybackError::Decode(format!("Decode error: {e}")))?;

            let spec = decoded_audio.spec().clone();
            if self.spec.as_ref() != Some(&spec) {
                self.spec = Some(spec);
            }

            // Copy the decoded (any sample format) audio into interleaved f32
            // samples, reusing the scratch buffer across packets.
            self.scratch.clear();
            decoded_audio.copy_to_vec_interleaved::<f32>(&mut self.scratch);

            let total = self.scratch.len();
            if total == 0 {
                // Empty packet (e.g. codec padding): keep pulling so `Ok(0)`
                // unambiguously means end of stream.
                continue;
            }

            if total <= out.len() {
                out[..total].copy_from_slice(&self.scratch);
                return Ok(total);
            }

            // Packet had more samples than requested — keep the rest without
            // reallocating (`pending_samples` retains its capacity).
            out.copy_from_slice(&self.scratch[..out.len()]);
            self.pending_samples.clear();
            self.pending_samples
                .extend_from_slice(&self.scratch[out.len()..]);
            return Ok(out.len());
        }
    }

    pub fn seek(&mut self, position: Duration) -> Result<(), PlaybackError> {
        let format = self
            .format_reader
            .as_mut()
            .ok_or_else(|| PlaybackError::Decode("Decoder not open".to_string()))?;

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
            .map_err(|e| PlaybackError::Decode(format!("Seek error: {e}")))?;

        self.pending_samples.clear();

        if let Some(decoder) = self.decoder.as_mut() {
            decoder.reset();
        }

        Ok(())
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Release the currently open file's resources without opening a new
    /// file. Safe to call when nothing is open.
    pub fn close(&mut self) {
        self.format_reader = None;
        self.decoder = None;
        self.spec = None;
        self.duration = None;
        self.source_path = PathBuf::new();
        self.pending_samples.clear();
        self.scratch.clear();
    }
}

/// The playback engine's decoder port, served over the same symphonia
/// internals as the richer inherent API above. EOF (`Ok(0)` on the decoder)
/// maps to `None`; a mid-stream decode error also reads as EOF because the
/// engine's loop has no error arm.
impl EngineAudioDecoder for SymphoniaDecoder {
    fn source_path(&self) -> &Path {
        &self.source_path
    }

    fn init(
        &mut self,
        path: &Path,
    ) -> Result<EngineAudioFormatInfo, riff_playback::app::errors::PlaybackError> {
        let info = self.open(path)?;
        Ok(EngineAudioFormatInfo {
            sample_rate: info.sample_rate,
            channels: info.channels,
        })
    }

    fn next_frames(&mut self, buf: &mut [f32]) -> Option<usize> {
        match self.next_frames(buf) {
            Ok(0) => None,
            Ok(n) => Some(n),
            Err(e) => {
                tracing::warn!("Decode error treated as end of stream: {e}");
                None
            }
        }
    }

    fn seek(&mut self, position: Duration) -> Duration {
        let _ = self.seek(position);
        position
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }
}
