# Glossary

This glossary defines the recurring terms used across riff's documentation and source code. It combines the product vocabulary (codecs, containers, library concepts) with the technical vocabulary (frameworks, concurrency primitives, and architectural roles) that appears throughout the engineering docs. Terms are listed alphabetically. For the architecture these terms plug into, see [../technical/architecture.md](../technical/architecture.md).

## Terms

| Term | Definition |
|---|---|
| **Album Artist** | The primary artist credited for an album, distinct from track-specific artists (for example, on compilations where each track has a different artist). |
| **ALSA** | Advanced Linux Sound Architecture — the Linux kernel audio subsystem. `cpal` uses it as the audio backend on Linux, and ALSA development headers are commonly required to compile riff there. |
| **App Runtime** | The composed application produced by one `AppRuntime::spawn` call in the Composition Root. `spawn` returns it as two halves: the `AppRuntime` the frontend renders with, and a `RuntimeLifecycle` that owns the worker threads along with the stop flags and cancel flag that end them — so the process that spawns the workers is also the process that joins them. Shutdown is an explicit `RuntimeLifecycle::shutdown` call, never a destructor. |
| **AppState** | Retired name for the former single shared application-state struct. The backend crate split (ADR 0009) replaced it with two session structs — `PlaybackSession` (in `riff-playback`) and `LibrarySession` (in `riff-backend`) — each shared across threads behind its own `Arc<Mutex<>>`. |
| **Application Store** | The single authoritative persistent state of the application — Library, Playlists, and Settings — in one embedded SQLite database (`riff.sqlite3`). |
| **Arc/Mutex** | Standard concurrency primitives for shared ownership (`Arc`) and interior mutability (`Mutex`). riff shares `PlaybackSession` and `LibrarySession` as `Arc<Mutex<_>>`, along with the backend event inbox; the audio buffer between the decode loop and the cpal callback is a lock-free `ringbuf` SPSC ring inside the output adapter. |
| **Codec** | A software component that encodes or decodes audio data in a specific format (MP3, AAC, Opus, FLAC, etc.). |
| **Column Identity** | What one entity-Column is, declared once where it renders: its Section, the depth its rows select at, and its Scroll Memory slot (`ui::column::ColumnIdentity`). Every action that Column answers reads the one value, so a gesture never re-supplies a Section or a level and the scroll bookkeeping cannot name a different list than the guard. Two shapes — a Section root and a deeper Column — and the differences between Columns of one shape are values, not copied blocks. |
| **Commit-message gates** | The three enforcement points that run the same commit-message validator over every commit: the optional local commit-msg hook (wired with `git config core.hooksPath .githooks`), the `commit messages (pull request)` CI job (the required branch-protection check), and the `commit messages (push)` CI job on pushes to `master`. |
| **Composition Root** | The single place where dependencies are constructed and wired together. In riff this is `AppRuntime::spawn` in `riff-backend/src/composition.rs` — the only code that names both the slice-defined ports and the concrete `riff-infra` adapters; the `riff` binary in `riff-gui` is a thin composition over it. |
| **Container** | A file format that wraps encoded audio data along with metadata tags and optionally cover art (M4A, OGG, FLAC, etc.). |
| **Conventional Commits profile** | The commit-message format riff's tooling enforces: a single Conventional Commit header (`type(scope)!: description`, body optional) with a closed set of twelve types, a free-form lowercase scope, a 72-character header limit, and no trailing period on the description. The release-notes generator parses it to group sections and derive version bumps. |
| **CoreAudio** | Apple's native audio framework on macOS. `cpal` uses it as the audio backend on that platform. |
| **Cover Art** | An image associated with an album or track, in two distinct senses. **Beside a Track** — embedded in the audio file's metadata, or, failing that, one of a fixed list of names (`cover`, `folder`, `album`, `front`, each in `.jpg`/`.jpeg`/`.png`) in the Track's directory, first name in that list winning. **On a Folders-tree row** — only that directory's own `cover.jpg`/`cover.jpeg`/`cover.png`, which replaces the folder glyph; never a `folder`/`album`/`front` sidecar, and never a Track's embedded image. `cover.gif` is excluded from both senses: only the JPEG and PNG decoders are built, so a GIF is reported as an unsupported container. |
| **cpal** | A cross-platform audio I/O library for Rust. riff uses it for audio output to the native device. |
| **crossbeam channel** | The `crossbeam-channel` crate, providing multi-producer, multi-consumer channels. riff uses unbounded channels for all cross-thread message passing. |
| **eframe** | The official application framework around egui, providing windowing and the event loop. |
| **egui** | An immediate-mode GUI library written in pure Rust. It is the foundation of riff's user interface. |
| **egui-elegance** | A theming crate for egui used by earlier versions of riff; retired — the interface is now styled from the project's own token constants in `riff-gui/src/ui/theme.rs`. |
| **Generated release notes** | The "what changed" section of each GitHub release, rendered by the release-notes generator (git-cliff, configured by `cliff.toml`) from the conventional commit messages between tags — each commit grouped by its type into a section, with the version bump derived from what landed. Not hand-edited; a non-conforming commit shows up in the render rather than being absorbed. |
| **Library** | The complete set of audio files discovered and indexed by the application. |
| **Library Cache** | Retired term for the former non-authoritative JSON copy of the Library. riff now persists everything in the Application Store; do not use this term for it. |
| **Listing Page** | A store-side type, not an app-level one: `Page<T>`, returned by one `*_page` method on `LibraryQueryStore` per listing, carrying a total and a window read under a **single connection acquisition**. The Session Views seam no longer hands one to a caller — it takes the total and the rows it needs from separate reads — so the term describes only the Application Store adapter's own read shape, and nothing in the UI holds one. It carries no generation; which generation a cached level was filled at is the Session Projection's concern. Not a View or a page of the UI. |
| **Track-menu host** | The single owner of the handles a right-click on a Track answers through (`app/track_menu.rs`): the Transport, the Playlist and Library store sections, the Inline Tag Editor, and the selection slot. It answers two questions — *a right-click happened on this Track* and *this item was chosen* — and a Track-row surface supplies only which Track and its row's context. Replaced an eight-field effects bag that each Track-row surface assembled by hand, and the Tracks Column's hand-rolled copy of it. Not a second dispatch, and not per-row: one per app, declared once. |
| **lofty** | A pure-Rust audio metadata reading/writing library. riff uses it to extract tags and embedded cover art. |
| **Metadata** | Descriptive information embedded in audio files (artist, album, title, genre, year, track number, etc.). |
| **notify** | A cross-platform filesystem-watching crate. riff uses it (through its infrastructure watcher) to detect new and deleted files in library folders. |
| **Playback Queue** | An ordered list of tracks scheduled for sequential playback. |
| **Port / Trait** | An interface defined by the crate that consumes it (for example `riff-playback`'s `AudioDecoder`/`AudioOutput`, `riff-library`'s `MetadataReader`/`CoverLoader`, `riff-persistence`'s store ports) and implemented by the adapter crate (`riff-infra`), keeping business logic decoupled from external crates. |
| **rfd** | Rust File Dialog — a crate providing native OS file and folder picker dialogs. riff uses it on macOS and Windows; on Linux it falls back to a text input field. |
| **Session Projection** | A bounded in-memory view of Application Store query results used while rendering; invalidated by a session-local generation counter after every committed mutation. |
| **Symphonia** | A pure-Rust audio media library used for format parsing and decoding. |
| **symphonia-adapter-libopus** | An adapter crate that provides Opus decoding for symphonia, which does not ship a native Opus decoder. |
| **TrackId** | A track's identity, represented as a string derived from its full file path via `PathBuf::to_string_lossy()`. Because identity is the path, moving or renaming a file produces a new `TrackId`. |
| **WASAPI** | Windows Audio Session API — the native Windows audio backend used by `cpal`. In shared mode it commonly runs at 48 kHz, which can trigger a fallback to the device default sample rate. |

## Related Reading

- [../technical/architecture.md](../technical/architecture.md) — the layered architecture and threading model these terms describe.
