---
feature: Multi-format Decoding
epic: Audio Engine
status: implemented
priority: P0
depends_on: []
personas: ["Music Listener"]
source_docs: []
implementation_notes: |
  Implemented via SymphoniaDecoder in infra/decoder.rs. All in-scope codecs
  (MP3, AAC, FLAC, OGG Vorbis, WAV) use symphonia native decoders.
  Opus uses symphonia-adapter-libopus (FFI wrapper around libopus) because
  symphonia 0.5's symphonia-codec-opus crate is a placeholder with no decoder.
  The adapter is registered into a custom CodecRegistry alongside the default
  symphonia codecs via symphonia::default::register_enabled_codecs.
---

# Multi-format Decoding

## Problem Statement

Music listeners have audio collections in many different formats (MP3, AAC/M4A, Opus, FLAC, OGG, WAV). Existing music players often have incomplete codec support or require external non-Rust dependencies. Users need a single application that can decode all common formats reliably using only pure Rust libraries.

## User / Personas

- **Music Listener**: A person with a local music collection containing files in various formats from different sources (purchased, ripped, downloaded). They expect the player to simply play any file they throw at it without worrying about codec compatibility.

## Scope

**In scope:**
- Decode MP3 files (MPEG-1/2 Audio Layer III)
- Decode AAC files in M4A containers (MPEG-4 Audio)
- Decode Opus files in OGG containers
- Decode FLAC files (Free Lossless Audio Codec)
- Decode OGG Vorbis files
- Decode WAV files (PCM)
- Pure Rust implementation with no external system codec dependencies
- Unified audio sample stream abstraction regardless of source format

**Out of scope:**
- Encoding or transcoding audio
- DRM-protected files (Apple FairPlay, etc.)
- Obscure or legacy formats (WMA, RealAudio, MIDI)
- Network streaming protocols (HTTP streaming, HLS)
- Bit-perfect DSD/SACD formats

## Boundary Conditions

- Files with corrupted headers or truncated data should fail gracefully with a clear error message
- Very large files (>2GB FLAC) should be handled via streaming, not loaded entirely into memory
- Variable bitrate (VBR) MP3s should report accurate duration if possible
- Files with unsupported sub-formats within a container should report which specific codec is missing

## Assumptions

- The symphonia crate provides sufficient codec coverage for all in-scope formats
- Audio files are stored on local filesystem with random read access
- Sample rate conversion (e.g., 96kHz → 48kHz) is handled by the audio output layer, not the decoder
- Mono, stereo, and multi-channel files are supported up to the audio output layer's channel limit

## Scenarios

### Scenario 1: Decode an MP3 file
A user opens an MP3 file from their library.

**Acceptance Criteria:**
- Given a valid MP3 file exists on disk, when the player attempts to decode it, then the decoder successfully produces a PCM audio sample stream
- Given a VBR MP3 file, when decoded, then the reported duration is within 5% of the actual playback duration
- Given a corrupted MP3 file, when decoding is attempted, then the decoder returns a structured error indicating the failure reason instead of panicking

### Scenario 2: Decode an M4A/AAC file
A user opens an M4A file containing AAC audio.

**Acceptance Criteria:**
- Given a valid M4A file with AAC audio, when the player attempts to decode it, then the decoder successfully produces a PCM audio sample stream
- Given an M4A file with ALAC (Apple Lossless) audio, when decoding is attempted, then the decoder returns a clear unsupported-codec error

### Scenario 3: Decode an Opus file
A user opens an OGG file containing Opus audio.

**Acceptance Criteria:**
- Given a valid OGG Opus file, when the player attempts to decode it, then the decoder successfully produces a PCM audio sample stream
- Given an OGG Vorbis file, when the player attempts to decode it, then the decoder successfully produces a PCM audio sample stream via the same code path

### Scenario 4: Decode a FLAC file
A user opens a FLAC file from their library.

**Acceptance Criteria:**
- Given a valid FLAC file, when the player attempts to decode it, then the decoder successfully produces a PCM audio sample stream
- Given a FLAC file larger than 100MB, when decoded, then memory usage remains bounded (streaming decode, not full memory load)

### Scenario 5: Handle unsupported format gracefully
A user attempts to play a file in an unsupported format.

**Acceptance Criteria:**
- Given a WMA file or other unsupported format, when the player attempts to decode it, then the decoder returns a clear error indicating the unsupported format
- Given a corrupted file of an otherwise supported format, when decoding fails, then the error message includes the file path and specific failure reason

## Implementation Notes

1. **Symphonia integration**: Use the `symphonia` crate with feature flags for all supported formats (`mp3`, `aac`, `opus`, `flac`, `ogg`, `wav`, `isomp4`)
2. **Unified decoder interface**: Create a `AudioDecoder` trait that abstracts over symphonia's format readers, providing a consistent API for sample extraction regardless of source format
3. **Error handling**: Define a custom `DecodeError` enum covering file-not-found, unsupported-format, corrupted-data, and io-error variants
4. **Memory management**: Ensure symphonia's packet-based decoding is used correctly so only small packet buffers are held at any time

## Open Questions

- [ ] Should we support MP3 files with ID3v2 tags at the end of the file (non-standard but common)?
- [ ] Do we need to handle sample rate conversion in-app if cpal doesn't support the file's native rate?

## Links
- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
