# Architecture

riff is a lightweight, offline-first desktop music player written in Rust on top of the egui immediate-mode UI framework. It is a Cargo **workspace** with no code-generation step and no plugin system; persistence runs through ordered, checksummed SQLite migrations. The architecture is a **vertical capability split**: the headless backend is divided into crates by capability with a strict, compiler-enforced dependency chain, infrastructure adapters that wrap external crates sit in one dedicated crate at the edge, and a single composition root wires everything together at startup.

This document is the reference for how the workspace is organized, how the crates are allowed to depend on one another, and what belongs where. For the runtime view of the system see [./threading-model.md](./threading-model.md) and [./data-flow.md](./data-flow.md); for the concrete types that flow between layers see [./data-model.md](./data-model.md). The split decision and its rationale are recorded in [ADR 0009](../adr/0009-vertical-crate-split-of-the-backend.md).

## Overview

The workspace members:

| Crate | Role | Responsibility |
|-------|------|----------------|
| `riff-persistence` | Persistence contract | The stored entities and the Application Store contract (store ports and DTOs). Pure `std` — no dependencies at all. |
| `riff-library` | Collection capability | Scanning, Session Projections, playlist management, cover resolution and service, the ports it consumes, its error type. |
| `riff-playback` | Playback capability | The Playback Queue, the audio engine, gapless logic, the playback coordinator, the Transport trait, the playback ports, the playback session, and the Up Next read model. |
| `riff-infra` | Adapter crate | Every port implementation and every native/external dependency, so toolchain requirements exist in exactly one place. |
| `riff-backend` | Application API | The Backend Events inbox (typed events and notices), the app-layer application services, the library session state, and the Composition Root that owns the worker threads. |
| `riff-gui-theme` | Design system | The design authority: the token and palette store, the vendored fonts, the icon set, and the assets those are compiled from. No `riff-*` dependency at all — `egui` and `resvg`, nothing else. |
| `riff-gui-components` | Component library | The presentation primitives: props in, a response or a typed intent out. No `riff-*` dependency at all — `riff-gui-theme` and `egui`, nothing else. |
| `riff-gui` | Frontend | egui UI, tray icon, native dialogs, fonts, and the `riff` binary entry point — a thin composition over `riff-backend`. |
| `tests` | Integration tests | The single workspace-root integration-test crate (cross-crate integration, UI, and golden-image suites). |

### Dependency chain

```
riff-gui (frontend, `riff` binary)
    | depends on
    +--> riff-backend (application API + Composition Root)
    |        | depends on all four below
    |        |
    |        |--> riff-infra (adapters + native deps)
    |        |        | depends on all three below
    |        |        v
    |        |--> riff-library -------> riff-persistence
    |        |--> riff-playback ------> riff-persistence
    |        +-----------------------> riff-persistence
    |
    +--> riff-gui-components (component library)
             | depends on
             v
          riff-gui-theme (design system)
```

`riff-gui-theme` and `riff-gui-components` are written to point at nothing else in the workspace: neither has an edge to `riff-backend`, to a slice, or to `riff-infra` (ADR 0018).

Read precisely, the edges are:

- `riff-persistence` depends on **nothing** (`std` only).
- `riff-library` and `riff-playback` each depend on `riff-persistence` (plus pure-Rust utilities such as `crossbeam-channel`, `thiserror`, `tracing`, `fastrand`). They are **true siblings with no edge between them** — neither can import a type from the other, and the compiler enforces it.
- `riff-infra` depends on all three (`riff-persistence`, `riff-library`, `riff-playback`) and implements their ports. This is where `rusqlite` (bundled SQLite, a C compiler), `cpal` (platform audio libraries), `symphonia`, `lofty`, `image`, `walkdir`, and `notify` live.
- `riff-backend` depends on all four. It is the only crate that names both the slice-defined ports and the concrete `riff-infra` adapters.
- `riff-gui-theme` depends on **nothing in the workspace** — `egui` and `resvg`, and the assets it compiles in. It is the design authority and no business crate may reach it, or it, them.
- `riff-gui-components` depends on `riff-gui-theme` and `egui`, and on nothing else in the workspace. A primitive's props are its own types; it names no business type at all.
- `riff-gui` depends on `riff-backend`, `riff-gui-components` and `riff-gui-theme` (plus UI/platform dependencies: `egui`/`eframe`, `resvg`, `tray-icon`/`muda`/`rfd` on non-Linux). It carries no adapter dependencies.
- `tests` depends on every crate by name (per-crate imports) so each suite reaches the type it needs directly.

The arrows point inward in the inversion-of-control sense: slices define ports and code against `Box<dyn Port>`; `riff-infra` depends on the slices and implements their traits; runtime control flows slices → adapters while the dependency arrow points adapters → slices.

## Crate Definitions and Membership Criteria

Each crate owns the types and ports it consumes; there is no shared catch-all crate.

### `riff-persistence` — types that cross the persistence boundary

**Membership criterion**: a type belongs here iff it crosses the persistence boundary.

It owns the stored entities (`Track`, `TrackId`, `TrackMetadata`, `Album`, `Artist`, `Playlist`, `PlaylistId`, `SmartPlaylistKind`, `CoverSource`) and `METADATA_VERSION`, the Application Store ports (migrations, settings, playlists, library query, library mutation), and the store DTOs (`Settings`, `ScalarSettings`, `WatchState`, `PlaylistEntry`, `StoreGeneration`, `StoreChanged`, the Lost Gems threshold) plus the `StoreError` type. It is `std`-only and implements nothing — the SQLite adapter lives in `riff-infra`. The settings port and DTOs live here — not in a capability slice — because ports must sit below the adapter crate and the Application Store is one table family with one generation scheme; this is a contract placement, not a capability claim.

**A paged listing is read as two reads, not one.** The read seam answers **"give me row *i*"** and, separately, **"how many rows does this listing have"**. Both land on the same `WindowedListProjection` in `riff-library`, which owns the whole paged-listing protocol: the window a row is fetched in, the refetch that happens only when the row leaves the window already in hand, the reuse of rows it already holds, and the total beside them, all at one generation. Nothing else derives a position inside a window — a render site names a row index and receives that row, and it never opens a page to learn a total. The arithmetic this removed was written once per paged read in the seam and re-derived in every render site, where an off-by-one was a rendering bug only a golden image could catch; it is now a seam test failure (`docs/engineering/testing-strategy.md`).

The store's own read shape is unchanged and is a separate fact: each paged listing still declares one `*_page` method on `LibraryQueryStore` (`tracks_page`, `search_page`, `hit_albums_page`, `hit_artists_page`, `artists_page`, `albums_page`, `genres_page`, `artists_in_genre_page`, `artist_albums_in_genre_page`) returning a `Page<T>` — a total and a window taken under **one connection acquisition**, carrying no generation. A seam count read reaches that method asking for **zero** rows, so the total costs no window; the seam takes the total alone and discards nothing it fetched. Everything a cached level needs to decide — observe the generation once, serve while it is current, otherwise load and commit — is `GenerationCache::level` in `levels.rs`, beside the `GenerationCache` primitive it drives. Placement follows the cache primitive's own rule: the two capability crates are siblings with no edge between them, and playback's generation caches adopt the same procedure later.

`METADATA_VERSION` sits beside `TrackMetadata` because it versions that struct's shape, and the scan reads and stamps it through the two Library ports (`LibraryQueryStore::metadata_version`, `LibraryMutationStore::stamp_metadata_version`) rather than through `SettingsStore`: the scan worker holds the Library pair, and a value the app writes on a scan's behalf is not a user preference — it never enters `ScalarSettings` and never renders in Settings, though `app_settings` is where the column physically lives.

### `riff-library` — the collection capability

**Membership criterion**: collection use cases and the ports they consume.

It owns the scan-side Track construction and the Library Scan Service (`scan.rs`, `scan_service.rs`), playlist entry validity (`playlist_manager.rs`), cover resolution and the cover service (`cover_resolver.rs`, `cover_service.rs`), the Library Session Projections (`projection/` — one module per projection: browsing, counts, folders, genres, hits, playlists, smart, and the generic `windowed_list` every paged read is built on, whose `row` and `count` are the seam's two reads and which owns the window alignment, the refetch decision, and the window-start arithmetic behind them), the ports it consumes (`traits.rs`: `MetadataReader`, `MetadataWriter` with the `TagEdit` DTO, `CoverLoader` with the decoded-image DTO, `FilesystemWatch`), its own `LibraryError` type, and re-exports of the store contract from `riff-persistence`. It has no edge to `riff-playback` and no native dependencies.

### `riff-playback` — the playback capability

**Membership criterion**: playback use cases and the ports they consume.

It owns the Playback Queue and repeat mode (`domain/queue.rs`), the playback command/update/state/position types, the playback session (`PlaybackSession` — the half of the former `AppState` the engine, coordinator, and transport touch), its ports (`infra/ports.rs`: `AudioDecoder`, `DecoderFactory`, `AudioOutput`, `AudioFormatInfo`), its own `PlaybackError` type, the pure-Rust audio engine (`infra/audio_engine.rs` — decode scheduling and gapless handoff over the port traits, no codec code), gapless eligibility and frame/duration math (`gapless.rs`), the playback coordinator (`playback_coordinator.rs` — commits play history for the finished track, then asks `domain/continuation.rs`, the pure arbiter of queue continuation, for the answer; the engine asks the same arbiter for a listener's skip), the `Transport` trait with `ChannelTransport` (`transport.rs` — the adapter owns the intent→command mapping, seek clamping, and effective-volume math), and the Up Next / playback read model (`projection.rs` — a Session Projection that reads the Playback Queue, placed here so the library slice never imports a playback type).

### `riff-infra` — every port implementation and every native dependency

**Membership rule**: an item belongs here iff it implements a port defined in another crate or wraps a native/external dependency — nothing else.

Concretely: the SQLite Application Store (`store/`: `SqliteStore`, migrations, corruption recovery, plus `StorePlaybackLibrary` — the store's read side narrowed to the Audio Engine's two queries, a named adapter rather than a blanket impl so the port stays fakeable, ADR 0012), the symphonia decoder and cpal output (`audio/`), the lofty metadata reader/writer and image cover loader (`media/`), and the walkdir scanner and notify watcher (`filesystem/`). The reason is isolation: bundled SQLite needs a C compiler and cpal needs platform audio libraries (ALSA on Linux), so keeping adapters out of the slices is what makes the slices pure Rust and testable anywhere. The crate preserves clean internal module seams (store / audio / media / filesystem) so it can be split further later without redesign if compile times ever demand it.

### `riff-backend` — the application API

**Membership criterion**: the frontend-facing event surface, the app-layer application services, and the one place that knows both ports and adapters.

It owns the Backend Events inbox (`events.rs` — the `BackendEvent` enum, notices with source and severity, and the two inputs that feed it: recorded transport commands and the store's `StoreChanged` stream), the app-layer application services (the Session Views read seam `views.rs`, the Tag Edit service `tag_edit_service.rs`, the Watcher Manager `watcher_manager.rs`), the library half of the session state (`state.rs`: `LibrarySession`, `ViewMode`, `BrowseMode`, `LibraryStatus`, `UiFlags` — the playback half lives in `riff-playback`), the Composition Root (`composition.rs`: `AppRuntime::spawn` opens the Application Store, constructs every real adapter, wires them into the slice-defined ports, and spawns the worker threads), and the re-export surface that keeps historical `riff_backend::…` import paths resolving for the frontend and the test suite. The crate carries no native dependencies of its own and no UI crate dependencies.

### `riff-gui-theme` — the design system

**Membership criterion**: a design value, or the asset it is compiled from.

It owns the token and palette store (`theme.rs` — every color, corner radius, type size, spacing step and component dimension, per ADR 0004), the vendored Inter faces and the system CJK fallback wiring (`fonts.rs`), the vendored Lucide glyphs with their tinted-texture cache (`icons.rs`), the assets those two are compiled in from (`assets/fonts`, `assets/icons`, with the OFL and ISC licences), and the two helpers that are token-derived rather than widget-shaped (`styled_font`, `search_ring_stroke`). It has no `riff-*` dependency — `egui` and `resvg`, nothing else — so a crate that means anything about the Library, playback or the store cannot reach it. What it holds is the design's own arithmetic (a contrast ratio, a tween's endpoints, the derived light palette) and never a decision about the product.

### `riff-gui-components` — the component library

**Membership criterion**: a presentation primitive — props in, a response or a typed intent out.

It owns the reusable presentation and interaction widgets that views compose from: the button family (with the Track menu's chrome, the prompts, and `ghost_icon_button`), the row band, the linear control, the text field, the toggle and checkbox, the empty state, the elastic stage, and the scroll-area wrapper. A primitive receives the `egui` context, the active `Palette`, an icon cache where needed, presentation values, and **caller-owned** buffers; it never names `RiffApp`, a session, `SessionViews`, a service front end, a store port, `Transport`, an application generation, or a native integration, and it never holds a shared handle to one. It has no `riff-*` dependency — `riff-gui-theme` and `egui`, nothing else — which is the point: the rule is a manifest edge the compiler checks on every line, where it used to be a substring sweep over a list of file names that no thirteenth module had to join. Its vocabulary is its own rather than the business side's (`menu` declares its own sort key and id keys, and `riff-gui` owns the transform), and a module whose seam is shared policy, a shared contract, or a business-shaped API rather than a name — `artwork`, with the cover service's eviction cap and the `RequestedSize` its loader also consumes, and `feedback`, whose every operation dispatches on the business provenance type and which is not scheduled to move — is a frontend module, not a primitive.

### `riff-gui` — the frontend

**Membership criterion**: rendering, input, and platform integration.

It owns the feature composites (`app.rs`, `app/`, `browser`, `chrome`, `detail`, `playerbar`, `sidebar`, `settings`, `selection`, `now_playing`, `frame`, and the presentation owners that name application types: `artwork`, `cover_cache`, `column`, `scroll_memory`), the host adapters (`tray`, `window_visibility`, `traffic_lights`, and the `impl RiffApp` blocks), and the `riff` binary — a thin composition over `riff_backend::composition::AppRuntime::spawn` that opens the store at its default location and hands the returned `AppRuntime` handles to the UI and tray. It depends on `riff-backend`, `riff-gui-components` and `riff-gui-theme`. The cover-texture LRU lives here because it is at once an egui-specific concern (`egui::TextureHandle`) and a store-facing one.

### Inside the frontend: the widgetry crates, and the two tiers that remain

The shared widgetry is **two crates below the frontend, not a layer inside it** (ADR 0018): `riff-gui-theme` holds the design authority and `riff-gui-components` the presentation primitives, and neither can name a `riff-*` crate. New UI work starts by asking which of three homes it belongs to:

- **`riff-gui-theme`** — a design value, or the asset it is compiled from: a token, a palette, a font, a glyph, a geometry constant, a helper that derives one. Names may be riff-flavoured (`geometry::playerbar`, `geometry::seek`) because naming a surface is not knowing anything about it, and the store still makes no decision.
- **`riff-gui-components`** — a presentation primitive on the props-in / intents-out contract, whose vocabulary is its own. If a widget has to know what a `TrackSort` or a `NoticeSeverity` *is*, it is not this crate's, and the mapping lives in the frontend.
- **`riff-gui`** — a feature composite or a host adapter. **Feature composites** are the recognizably riff-specific views: they combine primitives with content, and they are where application state is legitimately held. **Host adapters** are the `impl RiffApp` blocks (including `app/library_picker.rs` and `app/tag_editor.rs`) plus `tray` and `window_visibility`: they resolve the props, map emitted intents onto `Transport` / store / session / native effects, and keep platform behavior (the folder picker, Linux's text-path flow, the no-tray policy) explicit rather than hidden inside a generic widget.

*A module whose seam is shared policy or a shared contract rather than a name stays up.* `artwork` names the cover service's eviction cap and the `RequestedSize` the backend's own loader consumes, so it is a frontend module and not a primitive — importing either would put shared policy inside the crate whose rule is that it holds neither.

`app/track_menu.rs` is the one place in that layer where the mapping is *shared* rather than per-site. A right-click on a Track is one gesture, and it used to be answered through an eight-field effects bundle that each Track-row surface assembled by hand — one attach point for the six flat/smart/folder/playlist listings, and a second, differently-populated copy in the Tracks Column that intercepted a report the detail-action applier could not answer. The **Track-menu host** now owns those handles and answers exactly two questions (*a right-click happened on this Track*, *this item was chosen*), and a surface supplies only a `TrackMenuSubject` — which Track, and the one fact about where its row sits. `RiffApp::track_menu` is the app's single declaration of the handle set, and the test suite calls that same constructor, so a handle added to or dropped from the host breaks the app and its tests together rather than leaving a mirror to drift.

The host is **borrowed per gesture, not stored on the app**: storing it would mean moving four handles out of `RiffApp` and re-reaching them at every other site in a 4,000-line file to save a borrow that costs nothing. The scope that matters is where the handles are *named*, and that is one function. A **per-row host was rejected** for the reason that makes the per-app one worth having: a per-row host is that bundle again with a constructor, so every surface would still name what the host needs, and one that named it wrong would fail the same way and go unnoticed the same way.

### Inside the browser stage: one Column identity, two drains

The six entity-Columns — three Section roots and three deeper Columns — used to open their action drain with six copies of the same block, and to re-supply their Section, their depth and their Scroll Memory slot on every action. They are **not byte-identical**, and `app/browser_pane.rs`'s module doc says so rather than leaving it to be discovered: normalised they were **two shapes**, so the collapse target is **two bindings**, `drain_root_actions` and `drain_drill_actions`. What differs between the two is a *rule* — a root's A–Z flip does not force its list back to the top, a drill's does, because the control sits above the list and the list's egui state cannot see it — so the rule stays in the rule. What differs between two Columns of the same shape is a *value*: `ui::column::ColumnIdentity`, holding the Section, the depth and the Scroll Memory slot, stated once where the Column renders.

That makes the selecting-action-resets-scroll guard **structural**. It used to be `if action.selects_a_row() { note_selection_in(<a slot this site spelled out>) }`, written six times; it is now `ColumnIdentity::note_scroll_for`, a method on the datum, so the Scroll Memory wiring and the guard read the *same* value and cannot name different lists. A seventh entity-Column states its identity and inherits the guard. The module is pure — it names no `RiffApp`, no port and no session — so its decisions are answerable directly at `crates/riff-gui/tests/column_dispatch_tests.rs`, the same way the Scroll Memory's are.

The dispatch entry points follow: `apply_entity_selection`, `entity_track_ids` and `apply_collection_menu` take a `ColumnIdentity` rather than a Section and a level, so no per-action call re-supplies a Column's identity anywhere. Nothing here moves a handle: each Column still reaches the Transport, the Session Views seam and the store ports by `impl RiffApp` inheritance, exactly as before.

The Tracks Column's Track-menu report is a **separate report type** (`menu::TrackMenuReport`, carried in `detail::DetailReport`'s second variant) rather than a `DetailAction`. That is what makes the detail-action applier's former no-op arm *unnecessary* rather than merely removed: the applier takes a type that cannot carry a menu report, so it needs no arm to discard one.

The theme, fonts, and icons are one design authority in `riff-gui-theme`, and both crates above it read their values from there rather than keeping a second token store — the arrangement this section anticipated, moved together rather than split (ADR 0018).

## Dependency Rules

The core rule is that **dependencies follow the chain above and nothing bypasses it**. Specific rules:

- `riff-persistence` has zero dependencies — not even `serde`. It performs no I/O.
- `riff-library` and `riff-playback` depend only on `riff-persistence` and pure-Rust utilities. They never name a concrete adapter, never import from each other, and never import `egui`, `rusqlite`, `symphonia`, `cpal`, `lofty`, or `image`.
- Each slice defines the port traits for every external dependency it has, and `riff-infra` implements them. A slice never names a concrete adapter type.
- `riff-infra` implements slice-defined ports; it contains no business logic (no play-order or shuffle decisions, no scan policy).
- `riff-backend` is the only place that names both a port and its concrete implementation (`composition.rs`). Concrete adapters are not re-exported to the frontend.
- `riff-gui` depends on `riff-backend`, `riff-gui-components` and `riff-gui-theme` (plus UI/platform crates). It reads the backend's re-exported read-side surface — entities, Session Views, projections, Transport — and never touches an adapter.
- `riff-gui-theme` and `riff-gui-components` never gain a dependency on a `riff-*` business crate — not `riff-backend`, not a slice, not `riff-infra`. Neither holds a decision, a port, a session or a shared handle, and a widget that has to know what an application type means is a frontend module instead.

Inside each slice, the historical layering is preserved as module convention: `domain/` (pure types and logic), `app/` (use cases, session state, projections), `infra/` (port traits the adapter crate implements). What the split added is compiler enforcement of the boundaries *between* capabilities.

### Data crossing boundaries

- **Persistence contract to slices**: owned stored entities and DTOs (`Track`, `TrackId`, `Settings`, `PlaylistEntry`).
- **Theme to the widgetry crates**: tokens, palettes and presentation values flow down (`riff-gui-theme` → `riff-gui-components` → `riff-gui`), and a response or a typed intent flows back up. No session, port, service front end, or domain type crosses this boundary in either direction.
- **Slices to infrastructure**: trait method calls passing owned data or immutable references (for example `MetadataReader::read_all(&self, path: &Path)`).
- **Infrastructure to slices**: results returned through trait methods. Infrastructure never holds a reference to application state.
- **Backend to frontend**: the `AppRuntime` handles — the two session mutexes, the event inbox, the transports, the service front ends, and the store port views — plus channel messages drained by the UI each frame.

## Boundary Rules

### Synchronous calls, channels, and shared state

Three communication mechanisms are used, each for a different kind of interaction:

- **Synchronous calls** for fast, deterministic operations: queue manipulation, library queries, state reads.
- **Channels** (`crossbeam_channel::unbounded`) for all cross-thread communication: UI/tray to audio engine, engine to playback coordinator, scan service to worker, watcher events to the manager, tag-edit and cover request/response pairs, store change notifications to the event inbox.
- **Shared state** for read-heavy concurrent access: `Arc<Mutex<PlaybackSession>>` and `Arc<Mutex<LibrarySession>>` (the split of the former single `AppState`, each behind its own mutex), plus `Arc<Mutex<BackendEvents>>`. The audio ring buffer between the decode loop and the cpal callback lives inside `riff-infra`'s output adapter and is not part of the application surface.

### Manual dependency injection

There is no DI container. `composition.rs` performs manual constructor injection: it builds the concrete adapters and passes them — boxed as trait objects where appropriate — into the services and engine that need them. The `CoverResolver`, for example, is constructed over a `MetadataReader` and a `CoverLoader` port; it never knows those are backed by lofty and the `image` crate. No file outside the Composition Root constructs an adapter.

### Thread boundaries

- The egui event loop runs on the main thread (an egui/eframe requirement).
- Audio decoding runs on a dedicated audio engine thread; the Playback Coordinator runs on its own thread.
- Library scanning, filesystem-watch event processing, tag editing, and cover decoding each run on dedicated worker threads — all spawned by the Composition Root in `riff-backend`, not by the frontend.
- The cpal callback runs on an OS-owned real-time audio thread.

All thread-to-thread communication uses `crossbeam_channel`. See [./threading-model.md](./threading-model.md) for the full thread inventory and constraints.

### Error propagation

- Errors are typed per owner: `StoreError` in `riff-persistence`, `LibraryError` in `riff-library`, `PlaybackError` in `riff-playback` — each defined with `thiserror`, string-based so adapters can map into whichever port's error they answer.
- Infrastructure maps external crate errors into the owning port's error at the adapter boundary, so crate-specific error types never leak above `riff-infra`.
- Playback failures surface to the session as typed notices through the event inbox's notice channel (source + severity), not as a cross-slice state write.
- The UI displays user-friendly messages and never panics on a recoverable error.
- Mutex access uses the `MutexExt::lock_or_recover` helper, defined once in `riff-persistence/src/sync.rs` and re-exported by `riff-backend`, which recovers a poisoned lock instead of panicking, so a panic on one thread does not cascade into every other thread that shares a session mutex.

## Validation Checklist

Use this checklist when adding or reviewing a component:

- [ ] **Crate placement**: each module is in the crate whose membership criterion it satisfies (see above), and in the conventional `domain/`/`app/`/`infra/` layer for its responsibility.
- [ ] **Dependency direction**: the crate's `Cargo.toml` gains no edge that violates the chain. The slices never gain a dependency on `riff-infra`, on each other, or on any native crate.
- [ ] **Trait abstraction**: every external dependency of a slice crosses a port trait defined in that slice and is implemented in `riff-infra`.
- [ ] **Purity**: `riff-persistence` builds with no dependencies; the slices stay pure Rust. If a change needs a C compiler or platform audio libraries, it belongs in `riff-infra`.
- [ ] **Thread safety**: shared state between threads is synchronized with `Arc<Mutex<_>>` (one mutex per session — never nested), and cross-thread communication uses channels.
- [ ] **Error handling**: errors are owned by the crate that raises them; `riff-infra` maps external errors into the owning port's error; the UI shows user-friendly messages.
- [ ] **No UI in logic**: `egui` code lives only in `riff-gui`, `riff-gui-components` and `riff-gui-theme`, and there is no audio decoding in any of them. Neither widgetry crate names a `riff-*` business crate, and neither holds a decision, a port, a session or a shared handle.
- [ ] **Composition root**: only `riff-backend/src/composition.rs` constructs and wires infrastructure; no other file calls `::new()` on a concrete adapter and injects it across a boundary.

## Anti-Patterns

- **Native crates in a slice**: a `Cargo.toml` of `riff-library` or `riff-playback` gains `rusqlite`, `cpal`, `symphonia`, `lofty`, or `image`. Move the code to `riff-infra` behind a port.
- **A slice imports the other slice**: `riff-library` code touches a `riff-playback` type (or vice versa). Move the shared type down into `riff-persistence`, or move the code into `riff-backend`.
- **egui outside the frontend**: any crate other than `riff-gui`, `riff-gui-components` and `riff-gui-theme` imports `egui::`. UI concerns belong in the frontend.
- **A business import in a widgetry crate**: `riff-gui-theme` or `riff-gui-components` gains `riff-backend`, `riff-library`, `riff-playback`, `riff-persistence` or `riff-infra` as a dependency. A primitive's props are its own types; what an application type means is the frontend's to translate, at the boundary.
- **Second composition root**: a file other than `composition.rs` calls `SymphoniaDecoder::new()` (or any concrete adapter constructor) and injects it across a boundary. Route the wiring through the Composition Root.
- **UI thread blocking**: a `riff-gui` handler performs scanning or image decoding synchronously. Move it to a worker thread and use a channel.
- **Callback spaghetti**: an audio callback calls UI methods directly. Use channels for all thread-to-thread communication; never call UI code from a non-UI thread.
- **Stringly typed errors**: errors passed as bare `String` across a port. Use the owning crate's typed error enum so callers can match on variants.

## Ambiguity Signals

These are decisions with more than one defensible answer. Surface them explicitly rather than choosing silently:

- **Where the app-layer services belong.** The Session Views seam, Tag Edit service, and Watcher Manager live in `riff-backend` because they orchestrate the event surface the frontend renders; the use cases and ports beneath them live in `riff-library`. If a service ever needs to serve a non-frontend consumer, moving it into its slice is the considered step.
- **Where cover caching belongs.** The in-memory cover texture LRU lives in `riff-gui` because it stores egui-specific `TextureHandle`s. If caching policy (how long to keep covers) ever becomes a business rule, it may warrant a home in a slice.
- **Shared state versus message passing for playback position.** Position flows as a `PlaybackUpdate::PositionChanged` channel message rather than a shared atomic. The channel approach is more explicit and easier to trace; an atomic would be marginally faster.
- **Recovery from corrupted files.** Whether the decoder should skip a bad frame and continue or stop playback is a product decision with valid arguments on both sides.
- **Library index structure.** The Application Store is the single implementation of collection semantics: tracks, albums, and artists live in SQLite, and every view reads them through Session Projections over store queries or direct port calls. There is no second in-memory copy to keep consistent; if a future feature needs a different index shape, the decision is which store query (and projection) serves it.

## Error Handling Patterns

- **Per-owner error types**: `riff_persistence::errors::StoreError` (store/invalid-operation failures), `riff_library::app::errors::LibraryError` (metadata read/write, cover load, scan, I/O, track lookup), and `riff_playback::app::errors::PlaybackError` (decode, audio output) — all `thiserror`-derived and `Clone`, variants carrying a `String` message.
- **Infrastructure mapping**: each adapter in `riff-infra` converts its crate's error type into the appropriate owning port's error at the boundary, typically with `map_err`, so external error types never cross above the adapter crate.
- **Typed notices for playback failures**: the Playback Coordinator sends pre-formatted failure messages over the notice channel; BackendEvents stamps them with playback source and error severity. No slice ever writes into another slice's session state.
- **UI display**: the frontend matches on results and shows brief, user-facing messages. Technical detail goes to the log, not the screen.
- **Logging**: the `tracing` crate provides structured logging, initialized in `riff-gui/src/main.rs` via `tracing_subscriber::fmt::init()` with env-filter support. Use ERROR for failures, WARN for recoverable issues (for example, a failed store write or a failed cover decode), INFO for notable state changes, and DEBUG for detailed tracing.
- **Lock poisoning**: `MutexExt::lock_or_recover` recovers from a poisoned mutex instead of panicking, so an isolated thread panic does not take down the whole application.

## See also

- [./threading-model.md](./threading-model.md) — threads, channels, shared state, and real-time constraints.
- [./data-flow.md](./data-flow.md) — step-by-step sequences for playback, scanning, and cover resolution.
- [./data-model.md](./data-model.md) — the entities, the two session structs, the store ports, and the port traits.
- [ADR 0009](../adr/0009-vertical-crate-split-of-the-backend.md) — the crate-split decision and its consequences.
