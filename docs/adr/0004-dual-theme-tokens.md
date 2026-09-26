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

- **Some dark tokens are no longer the mockup's hexes.** Several dark tokens were
  lifted off the sheet so muted text and status colors clear WCAG 2.1 AA on the
  fills they paint; the exact ratios are asserted by the contrast test rather than
  recorded here. `warning` stopped aliasing `brand_primary`, and the focus ring became its
  own token instead of the brand's (design-handoff review P1-9 and P2-17). "Dark tokens
  straight from the sheet" now means: the sheet, minus the values a contrast floor
  overrode.
- **The light mirror does not hold for text.** Light's muted ink rungs and its
  `warning`/`error`/`focus_ring` are chosen against the light surfaces, not channel-wise
  flipped from dark. This narrows the "derived by rule" claim to surfaces, lines and `ink`.
- **`error` is the one token that had to be re-picked in both families, because
  no single red serves both.** Every other overridden value needed one family
  lifted; this one needed two, pulled opposite ways. The shared mockup red does not clear
  the floor on a light panel, so dark wears the bright end of the red ramp and light the
  deep end — the same two-ended arrangement `warning` already makes for amber, arrived at
  from the other direction. The destructive ghost button's hover wash is that same token at
  the mockup's 10%, so it moved with it. The light palette's own doc comment, which
  claimed status colors were untouched, now says what is true instead: the two status
  slots that carry text are chosen per family, and the two that are only ever an 8 px
  readiness dot (`success`, `info`) still inherit unchanged.
- **Contrast is now checked, not assumed.** A computed WCAG 2.1 contrast test in
  `tests/ui_tests.rs` holds every text token at 4.5:1 on the fills it paints on
  and every ring at 3:1.
