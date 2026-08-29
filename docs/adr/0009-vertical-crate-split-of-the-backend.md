# Vertical Crate Split of the Backend

**Status**: Accepted
**Date**: 2026-08-28

The headless backend lives in one crate (`riff-backend`) containing four different
concerns: the music collection (scanning, browsing, playlists, tag editing, covers),
playback (decode, output, queue, gapless), the SQLite Application Store contract, and
every infrastructure adapter. The layers inside it are clean on paper — use cases code
against Port / Trait interfaces that infrastructure implements — but nothing enforces
the boundary. One use-case module already imports a concrete adapter directly
(the watcher manager names `FilesystemWatcher`), an adapter imports application-layer
math (the decoder imports gapless frame conversions), and the whole crate drags a C
compiler (bundled SQLite) and platform audio libraries into every build and test run of
pure business logic. The frontend still carries transitional adapter dependencies
because the Composition Root lives in the binary crate.

The backend is split vertically by capability into five crates with a strict dependency
chain, keeping the existing Port / Trait inversion of control and making it
compiler-enforced:

- **`riff-persistence`** — everything that crosses the persistence boundary: the stored
  entities and the Application Store contract (ports and DTOs). Pure std.
- **`riff-library`** — the collection capability: scanning, filesystem watching, Session
  Projections and views, playlist management, tag editing, cover resolution and service,
  the ports it consumes, its own error type, and the library half of session state.
- **`riff-playback`** — the playback capability: the Playback Queue, the audio engine,
  gapless logic, the playback coordinator, the Transport trait, the playback ports, its
  own error type, the playback half of session state, and the Up Next read model.
- **`riff-infra`** — every port implementation and every native/external dependency, so
  toolchain requirements exist in exactly one place.
- **`riff-backend`** — the application API: the Backend Facade, typed events and
  notices, and the Composition Root that owns the worker threads.

`riff-library` and `riff-playback` are true siblings with no edge between them; both
depend only on `riff-persistence`. `riff-infra` depends on all three and implements
their ports. `riff-backend` depends on all four. The frontend depends only on
`riff-backend`.

## Considered Options

- **Keep one backend crate (rejected)**: no boundary enforcement; native dependencies
  leak into every build and test of pure business logic; the Composition Root stays in
  the binary crate.
- **Horizontal layer split — separate domain / app / infra crates (rejected)**: too
  fine-grained, and the "domain" crate becomes a dumping ground for ports, errors, and
  settings DTOs that are not domain entities. It also gives no compile isolation
  between capabilities.
- **Vertical capability split with a persistence contract crate (chosen)**: each crate
  owns the types and ports it consumes; the two capabilities are independent siblings;
  native dependencies are quarantined in one adapter crate; the persistence contract is
  a small std-only crate both slices share without depending on each other.

## Consequences

- The dependency chain is pure: `riff-persistence`, `riff-library`, and `riff-playback`
  are pure Rust and build/test anywhere without a C compiler or platform audio
  libraries. Only `riff-infra` (and, transitively, `riff-backend`) needs the native
  toolchain.
- Crate boundaries become compile errors instead of conventions. The two existing
  violations are fixed as prerequisites: the watcher manager gains a `FileWatcher`
  port, and the gapless frame/duration math moves to `riff-playback` so the decoder
  adapter imports it from there.
- The single `AppState` splits into a playback session and a library session, each
  behind its own `Arc<Mutex<>>`, owned by the slice that mutates it. The one
  cross-slice write (the playback coordinator setting a scan-status message on playback
  error) becomes a typed notice through the facade's existing notice channel.
- The single `AppError` splits by owner: decode and audio-output variants become the
  playback error; metadata, cover, scan, and IO variants become the library error.
- The Up Next / playback read model (a Session Projection that reads the Playback
  Queue) moves from the library's views into `riff-playback`, so the library slice
  never imports a playback type. The frontend holds it beside the Session Views.
- The Composition Root moves into `riff-backend` (a facade-owned run that constructs
  the real adapters, wires them into the ports, and spawns the worker threads),
  realizing the shape already named in the frontend crate's manifest. The frontend's
  transitional adapter dependencies are deleted.
- TrackId identity, the Application Store schema and durability semantics, port method
  contracts, the threading model, and Session Projection generation mechanics
  (ADRs 0001, 0002, 0003) are unchanged. The split re-homes ownership; it changes no
  data model, persisted format, or observable behavior.
- `riff-infra` preserves clean internal module seams (store / audio / media /
  filesystem) so it can be split further later without redesign if compile times ever
  demand it.
