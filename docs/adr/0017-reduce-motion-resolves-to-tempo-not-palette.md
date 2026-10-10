# Reduce Motion Resolves to Tempo, Not Palette

**Status**: Accepted
**Date**: 2026-10-09

A "Reduce motion" preference joins High contrast in Settings → Appearance.
It arrives the same way — a `UiFlags` bool persisted through `ScalarSettings`, hydrated by the app layer before the frontend exists — and it is read at the same theme boundary, which is where the resemblance ends.
High contrast is a colour variant: it re-picks `ink`, `line`, `border` and `focus_ring`, so it genuinely belongs on `Palette`.
Reduce motion re-picks nothing.
This record fixes where it lives so the resemblance is not mistaken for a pattern.

## Decision

**`reduce_motion` is a temporal policy that resolves at the theme boundary; it never lands on `Palette`.**
`Palette` stays a semantic colour set whose `resolve` is a two-axis function of `(dark, high_contrast)`, exactly as the motion-token rule in `crates/riff-gui/src/ui/theme.rs` already argues: motion is family-invariant, so a per-family tempo would be four things to keep coherent for no gain, and a reducing *mode* is no different from the families on that axis.

The policy has one sentinel and two resolvers:

- `MOTION_REDUCED: f32 = 0.0` is the reduced value of every motion duration.
Zero is safe for a verified reason, not an assumed one: egui's `AnimationManager::animate_bool` computes `last_value + (end - start) * elapsed / animation_time`, and a zero divisor makes the sum non-finite, so the manager falls back to the clamped `end`.
Every riff-authored wash lands exactly on its settled tint on the first frame — no intermediate frames, no repaint loop.
- `theme::animation_time(reduce_motion)` feeds `style.animation_time`; `theme::hover_duration(reduce_motion)` feeds the riff-authored washes, whose duration is deliberately not published on the `egui::Style`.

`style_from` and `install` take the flag beside the palette, `ThemeState::last_applied` grows into a named `ThemeInputs { dark, high_contrast, reduce_motion }`, and the painters that already receive the palette receive the bool too.
Zeroing `style.animation_time` snaps egui's own tweens — menu and popup fades, collapsing headers, scrollbars — so those surfaces change nothing.
The equalizer is the one exception that is riff's: it drives its own 50 ms `request_repaint_after` loop, which no style setting can reach, so it is gated explicitly — the bars rest at a static shape and the loop stops.

## Considered Options

- **A `Palette` field (rejected).** Mechanically the cheapest diff — the painters already hold `&Palette`, so `palette.reduce_motion` would cost zero new parameters, and `high_contrast` is a standing precedent for an accessibility bool on the bus.
But `Palette` is documented as a colour set, and `high_contrast` earns its slot by re-picking colours, which reduce motion cannot borrow.
It would also widen `resolve` into a temporal axis and make two colour-identical palettes compare unequal on a motion-only toggle.
The churn it saves is paid back in a permanently blurrier token concept.
- **A `MotionMode` enum (rejected).** The input is one boolean with a finite consumer set; an enum buys a third state nobody has a requirement for and a match at every call site.

## Consequences

- The painter families (`row.rs`, `button.rs`, and the `browser.rs`/`sidebar.rs` call sites) each gain one `bool` threaded beside the palette — roughly a dozen signatures.
That is the honest price of keeping the palette pure, and it is paid once.
- The equalizer's rest shape is "one glyph's shape data" living in its view module, the same exception class the token sweeps already make; freezing the bars at their held phase was rejected because a freshly-started track would freeze on a lopsided staircase at phase zero.
- Golden images are untouched apart from the one the feature must move: the golden harness already pins `animation_time = 0.0` for its settled captures, so a reduce-motion render is the state those images already assert, and with the preference defaulting to off the default path is byte-unchanged.
The Settings → Appearance golden gains exactly the new row and nothing else.
- The risk this record exists to close: a future reader holding `high_contrast` as their mental model will find a `reduce_motion` bool on `UiFlags`, note the parallel, and "correctly" move it onto `Palette`.
The parallel is real at the input and deliberately broken at the output.
