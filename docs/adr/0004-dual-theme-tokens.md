# Dual-Theme Tokens Despite a Dark-Only Design Source

**Status**: Accepted
**Date**: 2026-08-22

The redesign mockups define only a dark palette, but riff already persists a theme and the redesign's own Settings page promises a High contrast toggle. We keep light + dark: Phase 0 builds the token system around two palettes — dark tokens straight from the design's `colors_and_type.css` token sheet, light derived by rule (surfaces invert, ink flips, brand amber unchanged) — with High Contrast as a token-set variant over each base, not a third design. That CSS sheet is an external mockup artifact: it was never vendored into this repository, so the extracted hexes live only in the token module named below, and the sheet itself cannot be read back as evidence. Deciding this before Phase 0 closes matters because retrofitting a second palette onto single-palette token constants would touch every themed surface.

## Considered Options

- **Dark-only, drop theme switching**: matches the design source but silently deletes an existing persisted preference and strands the High contrast toggle.
- **Author a fresh light palette against the mockups**: best fidelity, but blocks Phase 0 on new design work.
- **Two palettes with a derived light set (chosen)**: keeps the existing feature; light-theme fidelity is consciously approximate until someone designs it properly.

## Consequences

- The token store is `crates/riff-gui/src/ui/theme.rs`, and it is both the store and the read source: every color, radius, type size, spacing step and component dimension ships there, and view code reads it from there. The rule is enforced, not just written down — see Design tokens in `docs/engineering/coding-standards.md`.
- Every color in view code must come from the active palette's tokens, not from a flat constant list; "zero hardcoded colors" in the Phase 0 acceptance applies per palette.
- The derived light palette is known-imperfect; visual-parity golden images are authored against the dark palette.
- High Contrast ships as variant token sets over each base, so its cost scales with palette count.

## Amendment — 2026-09-19 (token authority and AA contrast)

The decision above stands: two palettes, High Contrast as a variant, dark as the
designed family. Three of its details no longer describe the code, so they are
recorded here rather than edited out of the record:

- **Some dark tokens are no longer the mockup's hexes.** `ink_2`/`ink_3` were
  lifted until muted text clears WCAG 2.1 AA on every fill it paints on (it read
  3.40:1 on a panel while carrying required text), `warning` stopped aliasing
  `brand_primary`, and the focus ring became its own token instead of the brand's
  (design-handoff review P1-9 and P2-17). "Dark tokens straight from the sheet"
  now means: the sheet, minus the four values a contrast floor overrode.
- **The light mirror does not hold for text.** Light's muted ink rungs and its
  `warning`/`focus_ring` are chosen against the light surfaces, not channel-wise
  flipped from dark — a flip of an AA-compliant dark gray lands at 3.48:1 on a
  light panel, and its gold High-Contrast ring landed at 1.01:1. This narrows the
  "derived by rule" claim to surfaces, lines and `ink`.
- **Contrast is now checked, not assumed.** A computed WCAG 2.1 contrast test in
  `tests/ui_tests.rs` holds every text token at 4.5:1 on the fills it paints on
  and every ring at 3:1, for all four palette combinations.
