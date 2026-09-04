//! Audio adapters: the symphonia decoder and the cpal output, serving
//! riff-playback's [`AudioDecoder`](riff_playback::infra::ports::AudioDecoder)
//! and [`AudioOutput`](riff_playback::infra::ports::AudioOutput) ports.

pub mod audio_output;
pub mod decoder;

pub use audio_output::CpalAudioOutput;
pub use decoder::SymphoniaDecoder;
