# Dual-Theme Tokens Despite a Dark-Only Design Source

**Status**: Accepted
**Date**: 2026-08-22

The redesign mockups define only a dark palette, but riff already persists a theme and the redesign's own Settings page promises a High contrast toggle. We keep light + dark: Phase 0 builds the token system around two palettes — dark tokens straight from `colors_and_type.css`, light derived by rule (surfaces invert, ink flips, brand amber unchanged) — with High Contrast as a token-set variant over each base, not a third design. Deciding this before Phase 0 closes matters because retrofitting a second palette onto single-palette token constants would touch every themed surface.

## Considered Options

- **Dark-only, drop theme switching**: matches the design source but silently deletes an existing persisted preference and strands the High contrast toggle.
- **Author a fresh light palette against the mockups**: best fidelity, but blocks Phase 0 on new design work.
- **Two palettes with a derived light set (chosen)**: keeps the existing feature; light-theme fidelity is consciously approximate until someone designs it properly.

## Consequences

- Every color in view code must come from the active palette's tokens, not from a flat constant list; "zero hardcoded colors" in the Phase 0 acceptance applies per palette.
- The derived light palette is known-imperfect; visual-parity golden images are authored against the dark palette.
- High Contrast ships as variant token sets over each base, so its cost scales with palette count.
