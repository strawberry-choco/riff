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

impl Default for SymphoniaDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SymphoniaDecoder {
    /// Build a decoder together with the codec registry it decodes through:
    /// symphonia's enabled defaults plus the Opus adapter.
    ///
    /// This is the one place that knows which codecs riff registers — the
    /// same list the Library Scan's format set mirrors, declared once here
    /// rather than rebuilt by every caller. The registry is deliberately not
    /// shared: it is not `Clone`, so a fresh one is built per decoder instead
    /// of handed around, which is also what keeps `riff-backend` — a crate
    /// that declares no native dependency — from ever holding, naming, or
    /// rebuilding one.
    pub fn new() -> Self {
        let mut codec_registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut codec_registry);
        codec_registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
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

#[cfg(test)]
mod registry_tests {
    use super::*;
    use symphonia::core::codecs::audio::well_known::{
        CODEC_ID_AAC, CODEC_ID_AC3, CODEC_ID_ADPCM_G726, CODEC_ID_ADPCM_IMA_WAV, CODEC_ID_ADPCM_MS,
        CODEC_ID_ALAC, CODEC_ID_DCA, CODEC_ID_FLAC, CODEC_ID_MP1, CODEC_ID_MP2, CODEC_ID_MP3,
        CODEC_ID_OPUS, CODEC_ID_PCM_ALAW, CODEC_ID_PCM_F32LE, CODEC_ID_PCM_F64LE,
        CODEC_ID_PCM_MULAW, CODEC_ID_PCM_S16BE, CODEC_ID_PCM_S16LE, CODEC_ID_PCM_S24LE,
        CODEC_ID_PCM_S32LE, CODEC_ID_PCM_U8, CODEC_ID_VORBIS, CODEC_ID_WAVPACK, CODEC_ID_WMA,
    };
    // The one codec id the module itself already names; `well_known` does not
    // carry it.
    use symphonia::core::codecs::audio::AudioCodecId;

    /// The registry the composition root built by hand before `new()` took
    /// the job over: symphonia's enabled defaults plus the Opus adapter. It is
    /// the oracle the constructor is held against, so a codec added or dropped
    /// inside `new()` fails here rather than silently changing what riff can
    /// play.
    fn hand_built_registry() -> CodecRegistry {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        registry
    }

    /// Every audio codec a file the Library Scan indexes can present, keyed by
    /// the extension that brings it in: `.mp3` is the MPEG audio family,
    /// `.aac` and `.m4a` are AAC or ALAC, `.opus` is Opus, `.ogg` is Vorbis,
    /// `.flac` is FLAC, and `.wav` is one of the PCM, A-law, μ-law, or ADPCM
    /// families. This is what the registry is *for* as far as riff is
    /// concerned, so it is what the lock probes.
    const INDEXED_FORMAT_CODECS: &[(&str, &[AudioCodecId])] = &[
        ("mp3", &[CODEC_ID_MP1, CODEC_ID_MP2, CODEC_ID_MP3]),
        ("aac", &[CODEC_ID_AAC]),
        ("m4a", &[CODEC_ID_AAC, CODEC_ID_ALAC]),
        ("opus", &[CODEC_ID_OPUS]),
        ("ogg", &[CODEC_ID_VORBIS]),
        ("flac", &[CODEC_ID_FLAC]),
        (
            "wav",
            &[
                CODEC_ID_PCM_S16LE,
                CODEC_ID_PCM_S16BE,
                CODEC_ID_PCM_S24LE,
                CODEC_ID_PCM_S32LE,
                CODEC_ID_PCM_U8,
                CODEC_ID_PCM_F32LE,
                CODEC_ID_PCM_F64LE,
                CODEC_ID_PCM_ALAW,
                CODEC_ID_PCM_MULAW,
                CODEC_ID_ADPCM_IMA_WAV,
                CODEC_ID_ADPCM_MS,
            ],
        ),
    ];

    /// Codecs symphonia knows about that this build deliberately does not
    /// register — the other half of "same codecs": a widened registration must
    /// not pass either.
    const UNREGISTERED_CODECS: &[(&str, AudioCodecId)] = &[
        ("null audio", CODEC_ID_NULL_AUDIO),
        ("WavPack", CODEC_ID_WAVPACK),
        ("G.726 ADPCM", CODEC_ID_ADPCM_G726),
        ("AC-3", CODEC_ID_AC3),
        ("DCA", CODEC_ID_DCA),
        ("WMA", CODEC_ID_WMA),
    ];

    /// Whether `decoder`'s registry can mint a decoder for `id`.
    fn resolves(decoder: &SymphoniaDecoder, id: AudioCodecId) -> bool {
        decoder.codec_registry.get_audio_decoder(id).is_some()
    }

    /// `new()` builds, privately, exactly the registry the composition root
    /// used to build by hand — no codec added, none dropped, and none
    /// invented.
    #[test]
    fn new_builds_the_registry_the_composition_root_used_to_build() {
        let decoder = SymphoniaDecoder::new();
        let expected = hand_built_registry();

        for (extension, ids) in INDEXED_FORMAT_CODECS {
            for id in *ids {
                assert_eq!(
                    resolves(&decoder, *id),
                    expected.get_audio_decoder(*id).is_some(),
                    "the registry `new()` builds must answer {id} ({extension}) \
                     exactly as the hand-built registry does"
                );
                assert!(
                    resolves(&decoder, *id),
                    "a {extension} file's codec {id} must resolve"
                );
            }
        }

        for (name, id) in UNREGISTERED_CODECS {
            assert_eq!(
                resolves(&decoder, *id),
                expected.get_audio_decoder(*id).is_some(),
                "the registry `new()` builds must answer {name} ({id}) exactly \
                 as the hand-built registry does"
            );
            assert!(!resolves(&decoder, *id), "{name} must stay unregistered");
        }
    }

    /// Every extension the Library Scan indexes resolves at least one decoder,
    /// so no indexed format is silently unplayable.
    #[test]
    fn every_indexed_format_resolves_a_decoder() {
        let decoder = SymphoniaDecoder::new();

        for (extension, ids) in INDEXED_FORMAT_CODECS {
            assert!(
                ids.iter().any(|id| resolves(&decoder, *id)),
                "at least one of {extension}'s codecs must resolve: {ids:?}"
            );
        }
    }
}
