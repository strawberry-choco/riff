# Glossary

This glossary defines the recurring terms used across riff's documentation and source code. It combines the product vocabulary (codecs, containers, library concepts) with the technical vocabulary (frameworks, concurrency primitives, and architectural roles) that appears throughout the engineering docs. Terms are listed alphabetically. For the architecture these terms plug into, see [../technical/architecture.md](../technical/architecture.md); for where related state is stored, see [./configuration.md](./configuration.md).

## Terms

| Term | Definition |
|---|---|
| **Album Artist** | The primary artist credited for an album, distinct from track-specific artists (for example, on compilations where each track has a different artist). |
| **ALSA** | Advanced Linux Sound Architecture — the Linux kernel audio subsystem. `cpal` uses it as the audio backend on Linux, and ALSA development headers are commonly required to compile riff there. |
| **AppState** | The single large shared application-state struct holding the library, playback queue, playback state, theme, and UI state. It is shared across threads behind an `Arc<Mutex<>>`. |
| **Arc/Mutex** | Standard concurrency primitives for shared ownership (`Arc`) and interior mutability (`Mutex`). riff shares `AppState` as `Arc<Mutex<AppState>>` and the audio buffer as `Arc<Mutex<VecDeque<f32>>>`; the `parking_lot` crate is also a dependency. |
| **Codec** | A software component that encodes or decodes audio data in a specific format (MP3, AAC, Opus, FLAC, etc.). |
| **Composition Root** | The single place where dependencies are constructed and wired together. In riff this is `src/main.rs`, the only file that touches all four architectural layers. |
| **Container** | A file format that wraps encoded audio data along with metadata tags and optionally cover art (M4A, OGG, FLAC, etc.). |
| **CoreAudio** | Apple's native audio framework on macOS. `cpal` uses it as the audio backend on that platform. |
| **Cover Art** | An image associated with an album or track, typically embedded in audio file metadata or stored as `cover.jpg`/`cover.png` in the same directory. |
| **cpal** | A cross-platform audio I/O library for Rust. riff uses it for audio output to the native device. |
| **crossbeam channel** | The `crossbeam-channel` crate, providing multi-producer, multi-consumer channels. riff uses unbounded channels for all cross-thread message passing. |
| **eframe** | The official application framework around egui, providing windowing, the event loop, and persistence. riff enables its `persistence` feature and stores settings via `eframe::Storage`. |
| **egui** | An immediate-mode GUI library written in pure Rust. It is the foundation of riff's user interface. |
| **egui-elegance** | A theming and styling crate for egui used by riff to customize the appearance of the interface. |
| **Library** | The complete set of audio files discovered and indexed by the application. |
| **Library Cache** | A persistent on-disk JSON copy of scanned tracks, artists, and albums that avoids re-scanning on startup. |
| **lofty** | A pure-Rust audio metadata reading/writing library. riff uses it to extract tags and embedded cover art. |
| **Metadata** | Descriptive information embedded in audio files (artist, album, title, genre, year, track number, etc.). |
| **notify** | A cross-platform filesystem-watching crate. riff uses it (through its infrastructure watcher) to detect new and deleted files in library folders. |
| **Playback Queue** | An ordered list of tracks scheduled for sequential playback. |
| **Port / Trait** | An interface defined in the application layer (`AudioDecoder`, `AudioOutput`, `MetadataReader`, `CoverLoader`) and implemented by the infrastructure layer, keeping business logic decoupled from external crates. |
| **rfd** | Rust File Dialog — a crate providing native OS file and folder picker dialogs. riff uses it on macOS and Windows; on Linux it falls back to a text input field. |
| **Symphonia** | A pure-Rust audio media library used for format parsing and decoding. |
| **symphonia-adapter-libopus** | An adapter crate that provides Opus decoding for symphonia 0.5, which does not ship a native Opus decoder. |
| **TrackId** | A track's identity, represented as a string derived from its full file path via `PathBuf::to_string_lossy()`. Because identity is the path, moving or renaming a file produces a new `TrackId`. |
| **WASAPI** | Windows Audio Session API — the native Windows audio backend used by `cpal`. In shared mode it commonly runs at 48 kHz, which can trigger a fallback to the device default sample rate. |

## Terms by Category

The same terms grouped thematically, to help you find related concepts:

- **Audio formats and decoding:** Codec, Container, Symphonia, symphonia-adapter-libopus, Metadata, Album Artist, Cover Art.
- **Audio output backends:** cpal, WASAPI, ALSA, CoreAudio.
- **UI framework:** egui, eframe, egui-elegance.
- **Library and state:** Library, Library Cache, Playback Queue, TrackId, AppState.
- **Architecture roles:** Composition Root, Port / Trait.
- **Concurrency and messaging:** Arc/Mutex, crossbeam channel.
- **System integration and files:** notify, rfd.
- **Supporting libraries:** lofty, image (cover decoding), walkdir (scanning).

## Related Reading

- [../technical/architecture.md](../technical/architecture.md) — the layered architecture and threading model these terms describe.
- [./configuration.md](./configuration.md) — where the Library Cache, settings, and cover-art cache live on disk.
- [./troubleshooting.md](./troubleshooting.md) — common issues involving WASAPI, ALSA, and the Library Cache.
