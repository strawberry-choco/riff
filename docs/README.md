# riff Documentation

Welcome to the documentation for **riff** — a lightweight, offline-first desktop music player built in Rust with egui. This is the single home for everything you need to understand the product, its architecture, and how to work on it.

riff is a Cargo workspace that plays local audio files (MP3, AAC, Opus, FLAC, OGG Vorbis, WAV) using pure-Rust libraries, manages a music library from one or more folders, and runs cross-platform on Linux, Windows, and macOS. It keeps no cloud dependencies by design.

## How this documentation is organized

The docs are split into four reader-facing buckets that follow how different readers approach the project. This structure is loosely inspired by the [Diátaxis](https://diataxis.fr/) framework, adapted for a small desktop application: one axis separates *using* the product from *understanding* it, and the other separates *working on* it from *looking things up*.

| Bucket | For | Question it answers |
|---|---|---|
| [Product](product/overview.md) | Users, prospective users, and anyone explaining the product | "What is riff, what does it do, and who is it for?" |
| [Technical](technical/architecture.md) | Maintainers and contributors | "How is it built and how does it work internally?" |
| [Engineering](engineering/development-setup.md) | Contributors | "How do I build, change, and release it correctly?" |
| [Reference](reference/glossary.md) | Everyone | "What does term X mean, and where does state Y live?" |

If you are new, start with [Product → Overview](product/overview.md). If you want to build or change riff, start with [Engineering → Development setup](engineering/development-setup.md) and read [Coding standards](engineering/coding-standards.md) before your first change.

## Document index

### Product

What riff is, what it does, and how to use it.

**Orientation**
- [Overview](product/overview.md) — what riff is, its offline-first philosophy, who it is for, and what it deliberately is not.
- [Personas](product/personas.md) — the target users (the collector, the minimalist, the archivist) and how riff serves each.
- [Features](product/features.md) — the canonical feature catalog: every epic and feature with status, priority, and dependencies, plus the deferred items.

**Product decisions**
- [001: Offline-first design](product/decisions/001-offline-first.md) — why riff never connects to the internet.
- [002: No system tray on Linux](product/decisions/002-no-tray-on-linux.md) — why Linux builds run window-only.
- [003: Track identity is the file path](product/decisions/003-track-identity-is-path.md) — why the full file path is the canonical track ID.
- [004: Library cache as JSON](product/decisions/004-library-cache-as-json.md) — superseded by ADR 0001; kept for historical context.
- [005: Native picker on macOS/Windows, text input on Linux](product/decisions/005-native-picker-platform-split.md) — why the add-library dialog differs by platform.
- [006: Local-only discovery and metadata strategy](product/decisions/006-local-only-discovery.md) — why discovery and metadata enrichment stay local: smart playlists from play history, no online lookups.

**Architecture decisions (ADRs)**

- [ADR 0001: SQLite is the authoritative Application Store](adr/0001-sqlite-is-the-authoritative-application-store.md) — supersedes decision 004.
- [ADR 0002: The UI reads the store through Session Projections](adr/0002-ui-reads-the-store-through-session-projections.md).
- [ADR 0003: Store query model](adr/0003-store-query-model.md).
- [ADR 0004: Dual-theme tokens despite a dark-only design source](adr/0004-dual-theme-tokens.md) — two palettes with High Contrast as a variant over each, and `crates/riff-gui/src/ui/theme.rs` as the single store and read source for every design value.
- [ADR 0005: Custom window chrome (frameless) on all platforms](adr/0005-frameless-window-chrome-on-all-platforms.md) — amended 2026-09-28: the decision stands on Windows and Linux; macOS keeps its native decorations, with a transparent title bar and the system's traffic lights as the window controls; the windowed macOS spike passed and its gate is closed, with the native path confirmed as the shipping path.
- [ADR 0006: Background workers behind app-layer service seams](adr/0006-background-workers-behind-app-layer-service-seams.md) — Tag Edit and Cover services replace the worker threads spawned inline by `RiffApp`.
- [ADR 0007: No write-side SessionStore facade](adr/0007-no-write-side-sessionstore-facade.md) — the three store-mutation ports stay separate; the store owns generation bumps, so a facade would be a pass-through.
- [ADR 0009: Vertical crate split of the backend](adr/0009-vertical-crate-split-of-the-backend.md) — the backend is split by capability into a strict, compiler-enforced dependency chain.
- [ADR 0010: Inline tag editor in the detail panel](adr/0010-inline-tag-editor-in-the-detail-panel.md) — the Edit Tags modal moves into the detail panel as an inline editor with album-level tag aggregation and batch save.
- [ADR 0011: Retire the grid browser layout and the content top bar](adr/0011-retire-grid-and-content-top-bar-search-in-titlebar.md) — the search field becomes shared titlebar chrome; the browser is permanently list-only and its persisted setting is migrated out of the store.
- [ADR 0012: A narrow port gets a real adapter, not a blanket impl](adr/0012-narrow-port-gets-a-real-adapter.md) — `StorePlaybackLibrary` replaces the blanket impl so the engine's port stays fakeable.
- [ADR 0013: ReplayGain 2.0 via BS.1770, never 1.0](adr/0013-replaygain-2-via-bs1770-never-1.md) — the measurement standard, the −18 LUFS reference, and the `ebur128` adapter behind the `LoudnessAnalyzer` port.
- [ADR 0014: File tags are the source of truth; the Store mirrors them](adr/0014-file-tags-are-the-source-of-truth.md) — file-first write ordering and the Library Scan as the reconciler of external edits, made the general rule of the scan's and Tag Edit's semantics.

The numbering skips 0008: no ADR 0008 was ever written, so a missing `0008` is expected rather than a lost file.

### Technical

How riff is built and how it works at runtime.

- [Architecture](technical/architecture.md) — the workspace crate split, the dependency chain, each crate's membership criterion, boundary rules, validation checklist, and anti-patterns.
- [Design tokens](engineering/coding-standards.md#design-tokens) — where a design value lives (`theme.rs`), what view code may read and derive, and the three source sweeps that enforce it.
- [Threading model](technical/threading-model.md) — the threads (all workers spawned by the Composition Root), the crossbeam channels between them, shared state, and real-time constraints.
- [Data flow](technical/data-flow.md) — step-by-step sequences for the three primary flows: play a track, scan a library, resolve cover art.
- [Data model](technical/data-model.md) — the domain entities, `AppState`, the Application Store ports, and the port traits.

### Engineering

How to work on riff correctly.

- [Development setup](engineering/development-setup.md) — prerequisites, the command set, and build-profile notes.
- [Coding standards](engineering/coding-standards.md) — layering rules, the per-OS branch rule, clippy and formatting configuration, the error-handling pattern, and the implementation gotchas.
- [Contributing](engineering/contributing.md) — how to orient yourself and the pull-request checklist.
- [Commit conventions](engineering/commit-conventions.md) — the commit-message profile the validator enforces, the type-to-section-to-bump table from `cliff.toml`, and the three enforcement points.
- [Testing strategy](engineering/testing-strategy.md) — the per-crate suites and the workspace-root integration/golden suite, plus prioritized recommendations.
- [Golden-image testing](engineering/golden-image-testing.md) — the snapshot-test harness for visual parity: authoring goldens, re-baselining, and reviewing image diffs.
- [Access-violation flake](engineering/access-violation-flake.md) — the open `STATUS_ACCESS_VIOLATION` crash of the `riff-tests` binary: hypotheses ruled in and out, and the next move.
- [Release and packaging](engineering/release-and-packaging.md) — the tag-triggered release pipeline in `.github/workflows/release.yml` (version guard, four-platform build matrix, artifact packaging, pre-release publish) and how to cut a release.

### Reference

Quick lookup.

- [Glossary](reference/glossary.md) — product and technical terms, alphabetized.

## Conventions

- All documents are plain Markdown with no YAML front-matter.
- Cross-links are relative, so the tree renders correctly anywhere Markdown is supported.
- "Current State" sections describe the repository as verified; "Recommendations" sections are clearly labeled suggestions that are not yet implemented.
- Source filenames in examples are the real ones from the workspace crates (`riff-backend/src/`, `riff-gui/src/`, and so on).
