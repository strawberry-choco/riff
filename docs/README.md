# riff Documentation

Welcome to the documentation for **riff** — a lightweight, offline-first desktop music player built in Rust with egui. This is the single home for everything you need to understand the product, its architecture, and how to work on it.

riff is a single Cargo crate that plays local audio files (MP3, AAC, Opus, FLAC, OGG Vorbis, WAV) using pure-Rust libraries, manages a music library from one or more folders, and runs cross-platform on Linux, Windows, and macOS. It keeps no cloud dependencies by design.

## How this documentation is organized

The docs are split into four buckets that follow how different readers approach the project. This structure is loosely inspired by the [Diátaxis](https://diataxis.fr/) framework, adapted for a small desktop application: one axis separates *using* the product from *understanding* it, and the other separates *working on* it from *looking things up*.

| Bucket | For | Question it answers |
|---|---|---|
| [Product](product/overview.md) | Users, prospective users, and anyone explaining the product | "What is riff, what does it do, and who is it for?" |
| [Technical](technical/architecture.md) | Maintainers and contributors | "How is it built and how does it work internally?" |
| [Engineering](engineering/development-setup.md) | Contributors | "How do I build, change, and release it correctly?" |
| [Reference](reference/glossary.md) | Everyone | "What does term X mean, and where does state Y live?" |

If you are new, start with [Product → Overview](product/overview.md). If you want to build or change riff, start with [Engineering → Development setup](engineering/development-setup.md) and read [Coding standards](engineering/coding-standards.md) before your first change. If you are debugging a build or runtime problem, go straight to [Troubleshooting](reference/troubleshooting.md).

## Document index

### Product

What riff is, what it does, and how to use it.

**Orientation**
- [Overview](product/overview.md) — what riff is, its offline-first philosophy, who it is for, and what it deliberately is not.
- [Personas](product/personas.md) — the target users (the collector, the minimalist, the archivist) and how riff serves each.
- [Features](product/features.md) — the canonical feature catalog: every epic and feature with status, priority, and dependencies, plus the deferred items.
- [Roadmap](product/roadmap.md) — deferred items with their reasons, and recommended near-term improvements.
- [User guide](product/user-guide.md) — how to run riff, build a library, browse, and play music, with platform-specific notes.

**Specifications**
- [Requirements](product/requirements.md) — atomic acceptance criteria for every feature, organized by epic. The canonical reference for what each feature must do.
- [User flows](product/user-flows.md) — end-to-end user journeys across the product: first launch, daily browsing, search, folder playback, tray playback, and error recovery.
- [Interaction specs](product/interaction-specs.md) — per-widget interaction behavior: what happens on every click, double-click, right-click, and toggle across the entire UI.
- [Error states](product/error-states.md) — every error condition a user can encounter, what they see, what the system does, and how to recover.
- [Specs](product/specs/) — detailed per-feature specifications for the features currently marked partial:
  - [Player control bar](product/specs/player-control-bar.md)
  - [Now Playing view](product/specs/now-playing-view.md)
  - [System tray](product/specs/system-tray.md)

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

### Technical

How riff is built and how it works at runtime.

- [Architecture](technical/architecture.md) — the four-layer structure, dependency rules, boundary rules, per-layer rules, validation checklist, and anti-patterns.
- [Deepening plan](technical/deepening-plan.md) — the settled two-part refactor plan from the 2026-08-23 architecture review (One Library, playback engine seam) with per-step status tracking for resumability.
- [Threading model](technical/threading-model.md) — the seven threads, the crossbeam channels between them, shared state, and real-time constraints.
- [Data flow](technical/data-flow.md) — step-by-step sequences for the three primary flows: play a track, scan a library, resolve cover art.
- [Data model](technical/data-model.md) — the domain entities, `AppState`, the Application Store ports, and the port traits.
- [Dependencies](technical/dependencies.md) — every crate in `Cargo.toml` grouped by concern, with versions and purpose.
- [Persistence](technical/persistence.md) — the Application Store (`riff.sqlite3`): schema, migrations, corruption recovery, save timing, Session Projections, Clear Library, and the in-memory cover-art LRU.
- [Platform support](technical/platform-support.md) — the macOS/Windows/Linux feature matrix, conditional compilation, and why Linux omits the tray.

### Engineering

How to work on riff correctly.

- [Development setup](engineering/development-setup.md) — prerequisites, the command set, and build-profile notes.
- [Coding standards](engineering/coding-standards.md) — layering rules, clippy and formatting configuration, the error-handling pattern, and the implementation gotchas.
- [Contributing](engineering/contributing.md) — how to orient yourself and the pull-request checklist.
- [Testing strategy](engineering/testing-strategy.md) — the current test suite (and the known build issue), plus prioritized recommendations including CI.
- [Golden-image testing](engineering/golden-image-testing.md) — the snapshot-test harness for visual parity: authoring goldens, re-baselining, and reviewing image diffs.
- [Release and packaging](engineering/release-and-packaging.md) — the release profile, the manual release process today, and recommendations for release automation.

**Spikes**
- [Frameless window chrome](engineering/spikes/frameless-window-chrome-spike.md) — Issue 04 spike findings: go/no-go, the validated Windows implementation approach, and macOS/Linux risk with fallback plans.

### Reference

Quick lookup.

- [Glossary](reference/glossary.md) — product and technical terms, alphabetized.
- [Troubleshooting](reference/troubleshooting.md) — common build and runtime issues as symptom / cause / fix.
- [Configuration](reference/configuration.md) — where every piece of state and configuration lives, and how logging is controlled.

## Relationship to the `.lattice/` tree

This `docs/` tree **supersedes** the older `.lattice/` directory (`standards/architecture.md`, `requirements/index.md`, `requirements/features/`, and `context/`). The content from those files has been consolidated and rewritten here, corrected against the actual source tree — the older `.lattice/` documents referenced some module filenames (`playback_engine.rs`, `app_window.rs`, `library_panel.rs`, `control_bar.rs`, `cover_display.rs`) that do not exist in the codebase, and the project's `AGENTS.md` carried stale dependency versions and the incorrect claim that no tests exist. The documents in `docs/` reflect the verified reality instead. `AGENTS.md` now points here for architecture and feature references.

## Conventions

- All documents are plain Markdown with no YAML front-matter.
- Cross-links are relative, so the tree renders correctly anywhere Markdown is supported.
- "Current State" sections describe the repository as verified; "Recommendations" sections are clearly labeled suggestions that are not yet implemented.
- Source filenames in examples are the real ones from `src/`.
