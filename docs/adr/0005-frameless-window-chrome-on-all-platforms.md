# Custom Window Chrome (Frameless) on All Platforms

**Status**: Accepted
**Date**: 2026-08-22

The redesign specifies custom minimize/close buttons in a 56px titlebar, which requires a frameless window (`decorated(false)` + drag region). We ship this on Windows, macOS, and Linux — not Windows alone — so the Phase 1 spike must validate drag regions, resizing, and native behaviors (e.g., macOS traffic-light conventions) on each platform before the shell phase commits. Reversing this after layout ships would mean re-adopting native decorations across a chrome built around custom controls.

## Considered Options

- **Windows-only custom chrome, native elsewhere**: cheapest spike, but three divergent titlebar implementations to maintain.
- **Native decorations everywhere**: drops the mockup's integrated wordmark + window controls entirely.
- **Frameless on all platforms, gated by a per-platform spike (chosen)**: one chrome implementation matching the design; platform risk is paid up front, and a failed spike on any platform escalates before Phase 1 commits rather than after.

## Consequences

- Phase 1 cannot start until the spike passes on all three platforms; a failure forces a fallback decision (native decorations) while only token work has landed.
- riff already treats Linux specially (no tray icon, no native folder picker); frameless windows add a third surface where Linux behavior must be explicitly verified.
- Drag regions, resize handles, and maximize snapping become our responsibility on every target.
