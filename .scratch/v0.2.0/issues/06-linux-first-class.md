# 06 — Land Linux first-class support (REQ-SI-002)

**What to build:** Linux is an equal citizen: adding a library folder on Linux is a polished, validated experience rather than a bare text field, platform limitations are documented in-app, and every core flow — scan, play, cover art, search — works reliably. The improved picker already exists in the working tree; verify it, fix any gaps, and commit.

**Blocked by:** 01 — Baseline green gate.

**Status:** ready-for-agent

- [ ] The Linux folder picker validates input and shows a clear, actionable error for paths that don't exist or aren't directories; valid paths are added normally
- [ ] Platform-specific limitations (no system tray, no native folder dialog) are documented in the app's settings/about surface on Linux
- [ ] Core flows verified on a Linux build: library scan, playback, cover art display, search
- [ ] The Linux CI runner is green and platform-conditional code introduces no regression on Windows/macOS builds
- [ ] Feature committed
