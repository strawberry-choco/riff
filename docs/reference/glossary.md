# Glossary

This glossary defines the recurring terms used across riff's documentation and source code. It combines the product vocabulary (codecs, containers, library concepts) with the technical vocabulary (frameworks, concurrency primitives, and architectural roles) that appears throughout the engineering docs. Terms are listed alphabetically. For the architecture these terms plug into, see [../technical/architecture.md](../technical/architecture.md); for where related state is stored, see [./configuration.md](./configuration.md).

## Terms

| Term | Definition |
|---|---|
| **Album Artist** | The primary artist credited for an album, distinct from track-specific artists (for example, on compilations where each track has a different artist). |
| **ALSA** | Advanced Linux Sound Architecture — the Linux kernel audio subsystem. `cpal` uses it as the audio backend on Linux, and ALSA development headers are commonly required to compile riff there. |
| **AppState** | Retired name for the former single shared application-state struct. The backend crate split (ADR 0009) replaced it with two session structs — `PlaybackSession` (in `riff-playback`) and `LibrarySession` (in `riff-backend`) — each shared across threads behind its own `Arc<Mutex<>>`. |
| **Arc/Mutex** | Standard concurrency primitives for shared ownership (`Arc`) and interior mutability (`Mutex`). riff shares `PlaybackSession` and `LibrarySession` as `Arc<Mutex<_>>`, along with the backend facade; the audio buffer between the decode loop and the cpal callback is a lock-free `ringbuf` SPSC ring inside the output adapter. |
| **Codec** | A software component that encodes or decodes audio data in a specific format (MP3, AAC, Opus, FLAC, etc.). |
| **Composition Root** | The single place where dependencies are constructed and wired together. In riff this is `AppRuntime::spawn` in `riff-backend/src/composition.rs` — the only code that names both the slice-defined ports and the concrete `riff-infra` adapters; the `riff` binary in `riff-gui` is a thin composition over it. |
| **Container** | A file format that wraps encoded audio data along with metadata tags and optionally cover art (M4A, OGG, FLAC, etc.). |
| **CoreAudio** | Apple's native audio framework on macOS. `cpal` uses it as the audio backend on that platform. |
| **Cover Art** | An image associated with an album or track, typically embedded in audio file metadata or stored as `cover.jpg`/`cover.png` in the same directory. |
| **cpal** | A cross-platform audio I/O library for Rust. riff uses it for audio output to the native device. |
| **crossbeam channel** | The `crossbeam-channel` crate, providing multi-producer, multi-consumer channels. riff uses unbounded channels for all cross-thread message passing. |
| **eframe** | The official application framework around egui, providing windowing and the event loop. |
| **Application Store** | The single authoritative persistent state of the application — Library, Playlists, and Settings — in one embedded SQLite database (`riff.sqlite3`). |
| **Session Projection** | A bounded in-memory view of Application Store query results used while rendering; invalidated by a session-local generation counter after every committed mutation. |
| **egui** | An immediate-mode GUI library written in pure Rust. It is the foundation of riff's user interface. |
| **egui-elegance** | A theming crate for egui used by earlier versions of riff; retired — the interface is now styled from the project's own token constants in `riff-gui/src/ui/theme.rs`. |
| **Library** | The complete set of audio files discovered and indexed by the application. |
| **Library Cache** | Retired term for the former non-authoritative JSON copy of the Library. riff now persists everything in the Application Store; do not use this term for it. |
| **lofty** | A pure-Rust audio metadata reading/writing library. riff uses it to extract tags and embedded cover art. |
| **Metadata** | Descriptive information embedded in audio files (artist, album, title, genre, year, track number, etc.). |
| **notify** | A cross-platform filesystem-watching crate. riff uses it (through its infrastructure watcher) to detect new and deleted files in library folders. |
| **Playback Queue** | An ordered list of tracks scheduled for sequential playback. |
| **Port / Trait** | An interface defined by the crate that consumes it (for example `riff-playback`'s `AudioDecoder`/`AudioOutput`, `riff-library`'s `MetadataReader`/`CoverLoader`, `riff-persistence`'s store ports) and implemented by the adapter crate (`riff-infra`), keeping business logic decoupled from external crates. |
| **rfd** | Rust File Dialog — a crate providing native OS file and folder picker dialogs. riff uses it on macOS and Windows; on Linux it falls back to a text input field. |
| **Symphonia** | A pure-Rust audio media library used for format parsing and decoding. |
| **symphonia-adapter-libopus** | An adapter crate that provides Opus decoding for symphonia, which does not ship a native Opus decoder. |
| **TrackId** | A track's identity, represented as a string derived from its full file path via `PathBuf::to_string_lossy()`. Because identity is the path, moving or renaming a file produces a new `TrackId`. |
| **WASAPI** | Windows Audio Session API — the native Windows audio backend used by `cpal`. In shared mode it commonly runs at 48 kHz, which can trigger a fallback to the device default sample rate. |

## Terms by Category

The same terms grouped thematically, to help you find related concepts:

- **Audio formats and decoding:** Codec, Container, Symphonia, symphonia-adapter-libopus, Metadata, Album Artist, Cover Art.
- **Audio output backends:** cpal, WASAPI, ALSA, CoreAudio.
- **UI framework:** egui, eframe, egui-elegance.
- **Library and state:** Application Store, Session Projection, Library, Clear Library, Playback Queue, TrackId, AppState.
- **Architecture roles:** Composition Root, Port / Trait.
- **Concurrency and messaging:** Arc/Mutex, crossbeam channel.
- **System integration and files:** notify, rfd.
- **Supporting libraries:** lofty, image (cover decoding), walkdir (scanning).

## Related Reading

- [../technical/architecture.md](../technical/architecture.md) — the layered architecture and threading model these terms describe.
- [./configuration.md](./configuration.md) - where the Application Store, settings, and cover-art cache live on disk.
- [./troubleshooting.md](./troubleshooting.md) - common issues involving WASAPI, ALSA, and the Application Store.
