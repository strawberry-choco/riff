# Coding Standards

This document describes the conventions that keep riff's codebase consistent and maintainable. It covers the layered architecture rules, the lint and formatting configuration, the error-handling pattern, and the implementation gotchas that trip up contributors. Read this before writing any non-trivial change, and pair it with [development-setup.md](./development-setup.md) for the commands that enforce these standards.

riff is organized as a Cargo workspace of capability crates. The single most important rule is that dependencies follow the crate chain: `riff-persistence` ← `riff-library`/`riff-playback` ← `riff-infra` ← `riff-backend` ← `riff-gui`, with the two capability slices as siblings and no edge between them. The compiler enforces this chain, so a misplaced dependency fails the build rather than rotting silently.

## Architecture Layering

What lives in which crate, and the criterion that decides it, is maintained in
[../technical/architecture.md](../technical/architecture.md#crate-definitions-and-membership-criteria).

### Dependency direction

Dependencies always follow the chain, and lower crates know nothing about higher ones:

- `riff-persistence` has zero dependencies — no `serde`, no I/O, nothing but `std`.
- `riff-library` and `riff-playback` depend only on `riff-persistence` plus pure-Rust utilities. They never import each other, never name a concrete adapter, and never touch `symphonia`, `cpal`, `lofty`, `image`, `rusqlite`, or `egui`.
- Each slice defines the port traits for its external dependencies; `riff-infra` implements them and depends on the slices — never the reverse.
- `riff-backend` is the only crate that names both ports and concrete adapters; that happens exclusively in `composition.rs`.
- `riff-gui` depends only on `riff-backend` (plus UI/platform crates).

### Trait abstraction and dependency injection

The slices never touch `symphonia`, `cpal`, `lofty`, `image`, or `rusqlite` directly. Instead they declare port traits, and `riff-infra` provides the concrete implementations. Construction and wiring happen exclusively in `riff-backend/src/composition.rs` via manual constructor injection; there is no DI framework. No file other than the Composition Root should call `::new()` on an adapter and hand it across a boundary.

### Platform branches

Where behavior must differ by OS, the difference is **one pure decision function**, and every call site that depends on it reads that function rather than its own `#[cfg]`. `riff-gui`'s `chrome::chrome_mode()` is the worked example: one `ChromeMode` answer consumed by both the launch viewport configuration and the titlebar renderer, so the two cannot disagree about which convention the window is running under.

The reason is testability, not tidiness. A `cfg` branch is invisible to every machine that is not the target OS, so on Linux and Windows CI the macOS branch can never be compiled — and therefore can never be asserted. A function that *returns* the decision as data is assertable everywhere: the unreachable half of a platform split becomes a value any test can name on any runner. Keep the wrapper (`chrome_mode()`) separate from the consumers that take the decision as a parameter (`viewport_builder_for(mode)`), so the parameterised half stays executable and testable on every host — the "assert both branches as data" test only exists because of that split.

Residual `#[cfg]`s are for **compilation, not for decisions**. They compile a branch's code out where it cannot run and carry a comment saying why — the unreachable match arm, a native-only API, a tray handle. A `cfg` that picks *behavior* (a different widget, a string, a layout) is the smell this rule exists to catch: it is only acceptable when the code cannot exist on the other platform at all, and then it belongs at the adapter edge, not in a view.

Design values that differ per platform are still design values, so they still live in `theme.rs`: the macOS traffic-light clearance is a token like any other, and the fixed constant is the floor a runtime measurement may raise, not a second number kept beside it.

The shape this rule produces is worth naming, because it is the generalisable form and not a one-off: **one platform leaf that touches the OS — and the `unsafe` with it — and every decision above it a pure function asserted on every platform.** `ui::traffic_lights` is the worked example. The module compiles on all three platforms; its `apply` is a no-op off macOS, which is what lets the single call site in `RiffApp::logic` stay one ungated line; only the `AppKit` body behind `#[cfg(target_os = "macos")]` is code no CI machine compiles, and that body is where the OS calls, the two `superview` walks, and their `SAFETY` arguments live. Everything that *decides* anything sits above that leaf as a plain function over `f64` — `traffic_light_plan`, `needs_reapply` — asserted in `tests/ui_tests.rs` with no `cfg` at all, so the Linux and Windows legs execute the geometry that positions macOS's traffic lights. The same shape in the dependency graph: a platform-only native dependency goes under a `[target.'cfg(...)'.dependencies]` section (`objc2-app-kit`, pinned to the version eframe already resolves so the types unify and the cost is zero), so other platforms' CI never compiles the leaf's OS binding at all. If a new platform branch cannot be drawn this way — a decision living inside the gated body — that is the signal the decision belongs in a function the other platforms can call.

### Validation checklist

Before submitting a change, confirm:

1. Each module sits in the crate whose membership criterion matches its responsibility.
2. The workspace dependency chain has no new bypasses; the slices have no native imports.
3. External dependencies cross the slice/infra boundary only through port traits.
4. The persistence contract stays dependency-free, and the slices stay pure Rust.
5. There is no `egui` code outside `riff-gui`, and no audio decoding in the frontend.
6. Only `riff-backend/src/composition.rs` constructs and wires infrastructure.
7. Any new per-OS branch is the one decision function every call site reads, and its non-native branch is asserted as data rather than assumed.
8. Any new platform-only OS interop is one leaf whose native binding is compiled out on the other platforms and declared under `[target.'cfg(...)'.dependencies]`, with every decision it makes a pure function asserted on all of them.

For the full treatment, including key flows and the threading model, see [../technical/architecture.md](../technical/architecture.md).

## Design Tokens

`crates/riff-gui/src/ui/theme.rs` is the only place a design value is written, and the only place view code reads one from (ADR 0004). It holds the colors and the `Palette` slots, the corner radii, the type scale, the spacing scale (`SPACE_*`), the chrome dimensions, and the component geometry under `theme::geometry`.

The rules, and what each one is for:

- **A view declares no design value of its own.** No `const ROW_H: f32 = 40.0;` in a view module, and no inline number where a token exists. Two surfaces each naming a bar `HEADER_H` at different heights is how the tokens drifted apart in the first place; under `theme::geometry` both keep their own name in their own namespace.
- **A view derives no color of its own.** No `palette.error.gamma_multiply(0.1)` at a call site. Color math belongs beside the tokens it reads: `theme::glow`, `theme::hero_glyph`, `theme::destructive_fill`, `theme::blend_over`. Add a helper here rather than composing a color in a view.
- **Contrast is a property of the tokens, not of the call sites.** If muted text is hard to read, fix `INK_3`; do not move the call sites to `INK_2`. The computed WCAG test holds every text token at 4.5:1 on the fills it paints on, in all four palette combinations.
- **Structural stays with the algorithm.** Scroll and paging math, texture cache keys, a raster resolution, and how many rows a view asks its read model for are consequences of code, not of the design, and live in the view that computes them.

Three mechanical sweeps in `tests/ui_tests.rs` enforce the first two rules against the source of `crates/riff-gui/src/ui/**` (`theme.rs` exempt, being the store): `test_view_code_contains_no_hardcoded_color_literals`, `test_view_code_declares_no_dimensions_of_its_own`, `test_view_code_sets_no_spacing_of_its_own`. A violation names the file and line, and the fix is always to move the value into `theme.rs` — never to widen a sweep.

## Linting with Clippy

Lint levels are configured in `Cargo.toml` under `[lints.clippy]`, and tool-level options live in `clippy.toml`. The configuration enables the pedantic group as warnings, explicitly allows the nursery group, and carves out a small set of additional allowances.

The five individually allowed lints are worth understanding, because they reflect deliberate project choices:

- `needless_pass_by_value` — passing small values by value is accepted where it reads more clearly.
- `module_name_repetitions` — type names may repeat their module name (for example `app::state::AppState`).
- `missing_errors_doc` — `Result`-returning functions are not required to document every error case.
- `missing_panics_doc` — functions that may panic are not required to document it.
- `must_use_candidate` — the project does not annotate `#[must_use]` aggressively.

`clippy.toml` sets the tool MSRV (matching the `rust-version` in every crate manifest) and the project's behavioral options. Run `cargo clippy` before committing. Pedantic lints are warnings, so the build will not fail on them, but a clean clippy run is the expected standard; see [contributing.md](./contributing.md) for the pull-request expectations.

## Formatting with rustfmt

There is no `rustfmt.toml` or `.rustfmt.toml` in the repository, so riff uses rustfmt's default style. Run `cargo fmt` before committing so that formatting is consistent and does not appear as noise in diffs. No hook does this for you — the only versioned hook is the optional commit-message validator (see [commit-conventions.md](./commit-conventions.md)) — so the formatting check in CI is the gate.

## Error Handling

Errors are typed per owner using the `thiserror` crate, and each owner's type is the one its ports answer with:

- `riff_persistence::errors::StoreError` — the persistence boundary: store failures and invalid operations.
- `riff_library::app::errors::LibraryError` — the collection capability: metadata read/write, cover load, scan, and I/O failures.
- `riff_playback::app::errors::PlaybackError` — the playback capability: decode and audio-output failures.

The conversion flow runs outward: `riff-infra` maps external crate errors into the owning port's error at the adapter boundary, the slices and services match on those typed errors, and the UI surfaces a user-appropriate message. Playback failures reach the session as typed notices through the event inbox's notice channel (source + severity) rather than as a cross-slice state write. Errors are never returned as bare `String` values across a port, and the UI never panics on a recoverable error; it surfaces a message instead. Structured logging uses the `tracing` crate, with levels chosen by severity (ERROR for failures, WARN for recoverable issues, INFO for state changes, DEBUG for detailed tracing).

## Key Gotchas

These implementation details are easy to get wrong and worth internalizing before editing the relevant code.

- **Session state is two structs, each behind its own `Arc<Mutex<>>`.** `PlaybackSession` (in `riff-playback`) holds the queue, playback state, position, volume, and mute; `LibrarySession` (in `riff-backend`) holds selection, views, search, library paths and statuses, scan status, and watch states. There is no nested locking, and that must stay true: never acquire a second lock while holding a session lock.
- **There is no decoded-cover cache in `riff-library`.** The `CoverService` keeps only `negative`, an artless-verdict LRU; `COVER_CACHE_CAP` (200) bounds that *and* the GUI's `cover_textures` map, which holds `egui::TextureHandle`s with LRU order tracked in `cover_lru_keys` — so one constant serves two unrelated structures, and a new cache bound gets its own name rather than joining them: the texture map is bounded twice over, by that entry count and by `COVER_TEXTURE_BYTE_BUDGET` (64 MB), because the three canonical boxes span 80x in bytes and a count alone bounds nothing the user can feel. The thing that survives a restart is the Thumbnail cache on disk (`<data_local_dir>/covers/`), which is bounded by nothing at all: entries are named for their source's `(mtime, len)`, so growth is reclaimed only by Settings → Clear Thumbnail cache. Do not add an eviction policy to it without a reason that survives the argument in `.scratch/thumbnail-cache/spec.md`.
- **Audio buffer management.** `CpalAudioOutput` (`riff-infra`) uses a lock-free SPSC ring buffer (`ringbuf`) shared between the decode loop (producer) and the cpal callback (consumer). The cpal callback must never block; it outputs silence when the buffer cannot keep up.
- **Oversize decoded packets.** `SymphoniaDecoder` buffers decoded packets that are too large to emit in one call in an internal `pending_samples` buffer. Preserve this behavior when touching the decoder.
- **Sample-rate fallback.** The output stream always opens at the **device default** sample rate; a track's requested rate is not consulted. This is common on Windows WASAPI shared mode at 48 kHz.
- **`TrackId` identity.** A track's identity is its full file path as a string (derived from `PathBuf::to_string_lossy()`). Renaming or moving a file therefore produces a new track identity.
