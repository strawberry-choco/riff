# 05 — Land progressive disclosure + accessibility (REQ-UI-006, REQ-UI-007)

**What to build:** An app that starts simple and stays usable for everyone: first launch shows only the essential controls, power features are one explicit toggle away, and the whole interface is operable by keyboard with a high-contrast theme for users who need it. Both features already exist in the working tree; verify them together (disclosure wraps tag editing and smart playlists, so those must land first), fix any gaps, and commit.

**Blocked by:** 03 — Land metadata tag writing (REQ-ML-008); 04 — Land offline discovery playlists (REQ-ML-009).

**Status:** done

- [ ] Default UI shows only minimal controls (play/pause, library, search); tag editing, smart playlists, and stop are hidden until an explicit advanced-mode toggle reveals them
- [ ] Settings are organized by frequency of use, with no empty placeholder states for unconfigured features
- [ ] Advanced controls carry contextual hover tooltips explaining what they do
- [ ] The entire UI is navigable by keyboard (Tab, arrow keys, Enter, Escape) with a clearly visible focus indicator on the focused element
- [ ] The high-contrast theme toggle (near-black background, white text, bright focus outlines) overrides light/dark, persists across restarts, and switching away from it fully restores the normal theme
- [ ] Text remains readable at 150% UI scaling; no rapid animations or flashing elements anywhere
- [ ] Feature committed (advanced-mode gating, keyboard nav, high-contrast theme)

