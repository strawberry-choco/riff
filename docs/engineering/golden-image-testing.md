# Golden-Image Snapshot Testing

How riff pins its rendered UI pixel-for-pixel, and the workflow for authoring,
re-baselining, and reviewing golden images.

## What exists

- **Harness**: [`egui_kittest`](https://docs.rs/egui_kittest) (dev-dependency,
  features `wgpu` + `snapshot`) renders real egui frames offscreen through
  wgpu — no window, no display server. Tests live in `tests/golden_tests.rs`
  inside the single integration crate (`tests/mod.rs`) and run under plain
  `cargo test` on a normal Windows dev box.
- **Baselines**: committed PNGs under `tests/snapshots/<name>.png`, all
  produced through the `snapshot` / `snapshot_animating` helpers.
- **Palettes**: the base set is **dark** ([ADR 0004](../adr/0004-dual-theme-tokens.md));
  structural components are mirrored in **light**, and the **High Contrast**
  token set is pinned as a variant. `snapshot` takes the palette as a
  parameter.

## Running

```bash
cargo test                      # everything, including golden tests
cargo test golden_tests         # just the golden suite
```

A mismatch fails the test and prints the absolute path of the diff image.

## Authoring a new golden

1. Draw the component in a function taking `(&mut egui::Ui, &Palette)`, styling
   every color from the `Palette` it is handed (never hardcoded values, never
   a token read from a global).
2. Render it through a shared helper — never a bare harness:

   - `snapshot(name, size, palette, draw)` for a normal composition. It
     installs the palette's style plus the vendored Inter faces
     (`inter_only_font_definitions`) and runs the frames to stability.
   - `snapshot_animating(name, size, palette, draw)` for a composition where
     one `run()` is wrong or not enough: an always-repainting widget (the
     playing row's equalizer) blows the step budget, and focus requested
     through a widget's own `Response` only lands in the **next** frame.
     Two fixed frames are rendered. The harness never *sets* `input.time` — it
     only pins `input.predicted_dt` to its fixed 0.25 s step, and egui derives
     `time = prev + predicted_dt` — so two frames land on the same clock value
     on every machine. Deterministic, but the clock is **moving**: a golden
     whose content reads `input.time` (the playing row's equalizer bars) is
     pinned to a phase set by the frame count, so retuning that animation's
     tempo legitimately moves that one baseline.

   Both go through `with_golden_style`, which keeps the ui closure inert until
   the style and fonts are installed. That matters: `HarnessBuilder::build_ui`
   draws — and `run_ok`s — frames from inside its own constructor, *before*
   the test body can touch `harness.ctx`, so a draw fn that names an Inter
   family outright (as `draw_type_scale` does) would panic with
   `FontFamily::Name("riff-inter-medium") is not bound to any fonts` on the
   default font set.
3. Paint the full canvas background first (see the determinism rules) and give
   the harness a fixed size with `pixels_per_point(1.0)`.
4. Run `cargo test golden_tests`. The first run **fails** because no baseline
   exists — that is the expected red step.
5. Generate the baseline:

   ```powershell
   $env:UPDATE_SNAPSHOTS = "true"; cargo test golden_tests; Remove-Item Env:UPDATE_SNAPSHOTS
   ```

6. **Open the PNG and review it** before committing. A golden that renders
   blank, clipped, or with harness artifacts (see determinism rules below) is
   worse than no golden. A single-colour baseline is the strongest possible
   smell — it usually means the widget rendered nothing at all.

## Re-baselining

When a visual change is intentional:

```powershell
# Rewrite only the snapshots that currently fail:
$env:UPDATE_SNAPSHOTS = "true"; cargo test; Remove-Item Env:UPDATE_SNAPSHOTS

# Rewrite every snapshot, even ones passing within tolerance:
$env:UPDATE_SNAPSHOTS = "force"; cargo test; Remove-Item Env:UPDATE_SNAPSHOTS
```

`true`/`1` means "update the failing ones", so a change **smaller than
`failed_pixel_count_threshold`** leaves the test green and the stale baseline in
place: the suite reports success while the committed PNG no longer matches what
the widget renders. After landing a change in an already-pinned component — or
when a golden's own composition changed — use `force`.

**Then prove byte-identity with `git diff`, not with a green test run.** A plain
`cargo test golden_tests` is *not* confirmation of anything: at a non-zero
threshold the suite reports success for a deliberately recoloured label exactly
as it does for an unchanged tree. Force the render and ask git instead:

```bash
# Byte-identity is proved by comparing two FORCED RENDERS TO EACH OTHER, not by
# `git diff` against HEAD: a re-baseline is *supposed* to differ from HEAD before
# it is committed, so `git diff` is non-empty by design at that point.
#
# The `-name` filters matter. A failing run leaves `<name>.new.png`,
# `<name>.diff.png` and `<name>.old.png` beside the baselines (they are
# gitignored, not deleted), and they match `-name '*.png'`. Without the filters a
# byte-identical pair looks different, and the baseline count is wrong.
find tests/snapshots -name '*.png' ! -name '*.new.png' ! -name '*.diff.png' \
  ! -name '*.old.png' | sort | xargs shasum -a 256 > /tmp/snapshots_a
UPDATE_SNAPSHOTS=force cargo test --test integration golden_tests
find tests/snapshots -name '*.png' ! -name '*.new.png' ! -name '*.diff.png' \
  ! -name '*.old.png' | sort | xargs shasum -a 256 > /tmp/snapshots_b
diff /tmp/snapshots_a /tmp/snapshots_b    # no output == byte-identical
```

On macOS `[mac] failed_pixel_count_threshold` is `0` and repeated renders are
bit-exact, so this check is exact and total there. This is the evidence a ticket
should cite when it claims a golden is byte-identical or visually neutral.

Commit the regenerated `tests/snapshots/*.png` together with the change that
caused them, so reviewers see the code diff and image diff in one place.
Never re-baseline to make an unexplained failure go away — a golden diff is a
review signal, not noise. In particular, **classify every changed PNG before
accepting it**: a re-baseline mixes an intentional change with machine drift
and with any stale baseline the previous author forgot to rewrite, and only a
per-file classification tells those three apart.

## Reviewing image diffs

On a mismatch, kittest writes files next to the baseline (all gitignored):

| File | Meaning |
|---|---|
| `<name>.new.png` | What the test just rendered |
| `<name>.diff.png` | Highlighted pixel differences |
| `<name>.old.png` | Backup of the previous baseline (written during updates) |

Open `.diff.png` (or flip between `.old.png` / `.new.png`) to judge whether
the change is intended. For triaging many failures at once,
[`kitdiff`](https://github.com/rerun-io/kitdiff) (`cargo install --git
https://github.com/rerun-io/kitdiff`; then `kitdiff files .`) collects all
`.new.png` / `.diff.png` files under a directory.

## Determinism rules

Golden images are only useful if the same input renders identically on every
run. The harness enforces several rules; keep them when adding goldens:

- **Vendored Inter fonts only.** Goldens build font definitions from
  `fonts::INTER_FACES` directly. Never use `fonts::font_definitions()` here:
  it appends a system CJK fallback font, which differs per machine and would
  make baselines non-portable.
- **Fixed geometry.** Every harness pins its window size and
  `pixels_per_point(1.0)` so host DPI scaling cannot change output dimensions.
- **Settled state, re-asserted *after* the palette install.** kittest's
  `Harness::new` zeroes `Style::animation_time`, `Style::scroll_animation` and
  `Style::visuals.text_cursor.blink` on **both** theme slots before the first
  frame. riff then installs the palette, and `theme::install` replaces that
  theme's whole `Arc<Style>` with a `Style::default()`-derived build — which
  hands all three straight back to egui's defaults, undoing the harness's own
  determinism. `with_golden_style` therefore re-asserts the three fields after
  `theme::install` (as `pin_settled_time`), and the composed-`RiffApp` harness
  re-asserts them after construction because it installs its own palette inside
  its first `update` and never passes through `with_golden_style`. So "baselines
  capture settled state" is a decision the suite makes, not a coincidence of
  call order — which is what lets a tween be introduced on a surface without
  silently re-baselining every affected image. **The app's own theme
  installation path is untouched**: the windowed app is *supposed* to animate,
  and this is a suite concern.
- **Time is moving, not frozen — so the frame count is the contract.** The
  harness never *sets* `input.time`; it pins `input.predicted_dt` to a fixed
  0.25 s step and egui derives `time = prev + predicted_dt`. A fixed frame
  count therefore lands on a fixed clock value on every machine, which is why
  `run_steps(2)` is deterministic. The corollary to remember when a golden
  moves for no visible reason: a surface that reads `input.time` (the playing
  row's equalizer bars) is pinned to a *phase* set by the frame count, so
  retuning its tempo legitimately moves that one baseline.
  `snapshot_animating`'s doc comment in `tests/golden_tests.rs` records the
  same fact at the call site.
- **Full-canvas background.** Under kittest the root UI is inset from the
  true screen rect; paint the palette background through
  `ctx.layer_painter(LayerId::background())` over `ctx.screen_rect()` (as
  `draw_play_card` does). A panel fill alone leaves an unpainted clear-color
  ring around the image.
- **No interaction state.** Snapshots capture the last rendered frame; avoid
  hover/cursor-dependent rendering (call `harness.remove_cursor()` after
  simulated clicks if a future golden needs them). **Focus** is allowed and
  deterministic: request it through `ui.memory_mut(|m| m.request_focus(id))`
  before the widget draws (the search-well goldens), or let
  `snapshot_animating` render the second frame that a
  `Response::request_focus()` needs.
- **Platform-conditional copy is platform-conditional pixels.** The Advanced
  Settings pane renders different info lines under
  `#[cfg(not(target_os = "linux"))]`, so `settings_advanced_dark` is authored
  against the Windows/Linux split and the Linux leg leans on the wider
  `[linux] failed_pixel_count_threshold`. Anything behind a `cfg` belongs here
  in the doc as well as in the test.
- **The traffic lights are OS-composited pixels the harness can never capture.**
  The strip goldens (`shell_chrome_*`, `titlebar_search_*`, `composed_shell_*`)
  call `chrome_mode()` directly instead of branching on a `cfg`, so they render
  whichever window-chrome convention the *host* OS ships. A macOS re-baseline of
  the strip therefore legitimately differs from the same goldens rendered on
  Linux or Windows, while the Linux and Windows CI legs stay green at their
  existing thresholds — the same class of platform difference the bullet above
  records, arriving through a data value rather than through a `cfg`. What those
  goldens *can* show on macOS is the **clearance**: the left cluster starting
  past where the lights are, pinned by
  `theme::geometry::titlebar::TRAFFIC_LIGHT_CLEARANCE` as the floor and by
  `chrome_mode()` as the branch that selects it. What they can never show is the
  **lights** themselves — AppKit draws them, compositing them over the frame
  egui renders, so they are not in the surface the harness reads back. A passing
  strip golden is therefore *not* evidence that the traffic lights render, that
  they sit where they should, or that they are clickable; that evidence is a
  windowed run, full stop. This is a permanent property of the harness, not a
  caveat some future kittest release lifts: the decorations live outside every
  renderer kittest can drive.

  That windowed run has now happened — 2026-09-28, and it passed, so the strip
  goldens sit next to a verified composite rather than an unexamined one. **The
  blind spot is unchanged by that, and that is the point of recording it here.**
  The verification is evidence about the app; it is not evidence the harness
  gained. Everything the run established about the lights — that they are
  vertically centred in the 56pt strip, that the hover highlight follows the new
  position, that the enlarged titlebar container's corner treatment looks right
  — it established because a person looked at a window, and a future run of this
  suite will be just as silent on all three. A green suite before that run was
  not evidence the lights were misplaced, and a green suite after it is not
  evidence they are correct; it is evidence the clearance arithmetic held. Read
  the composite claims in ADR 0005 and the pixel claims here as two different
  kinds of evidence, because they are.

  **And that is now a demonstrated property of a change, not only a limitation.**
  Re-centring the traffic lights in the strip — see
  [ADR 0005](../adr/0005-frameless-window-chrome-on-all-platforms.md)'s second
  amendment — moved **zero** golden bytes, and the reason is structural rather
  than lucky, in two independent parts. The correction writes `origin.y` alone
  and leaves `origin.x` as AppKit put it, and the one traffic-light measurement
  that can reach the render is the close button's `origin.x` — eframe's
  `traffic_lights_size.x` — which is the only thing that could move the wordmark;
  the goldens do not even consult it, passing
  `traffic_light_clearance(None, 1.0)` so the strip's clearance is the fixed
  floor constant. And the buttons themselves are not in the surface, per the
  paragraph above, so there is nothing there for them to change. The committed
  baselines therefore still carry timestamps from before the re-centring landed,
  and the suite is green with them. **Do not "fix" a moved strip baseline by
  re-recording it after this work without an independent reason** — a re-record
  here means something else moved, which is exactly the signal the byte-identity
  check is for.

  What the suite *does* cover for that change is the portable half, asserted
  ungated in `tests/ui_tests.rs` rather than here: `traffic_light_plan` (the
  target inset and container height) and `needs_reapply` (the drift threshold)
  are plain functions over `f64`, so the Linux and Windows CI legs execute the
  arithmetic that positions the lights even though they never compile the AppKit
  code that applies it. Keep the two kinds of evidence distinct: the arithmetic
  is asserted as unit data on every CI leg, and the placement of the buttons
  themselves is asserted by no test at all — only by a human looking at a window,
  which is the same instrument that closed ADR 0005's spike gate.
- **Harness concurrency is capped.** `tests/golden_tests.rs` runs at most
  `MAX_CONCURRENT_HARNESSES` harnesses at a time, because every harness brings
  up its own wgpu device. A wgpu `STATUS_ACCESS_VIOLATION` flake in the golden
  block is open — see
  [./access-violation-flake.md](./access-violation-flake.md). If it recurs,
  lower the cap rather than chasing pixels.
- **Baselines are machine-local, and macOS owns them.** wgpu picks different
  adapters/backends per machine, and driver-level differences can exceed the
  default per-pixel tolerance. Note the asymmetry: `egui` rasterizes **glyphs on
  the CPU** into a font atlas, so text is backend-independent and every
  text-only golden is byte-identical across machines. **Shapes are not** — 1px
  strokes, rounded-rect edges, dividers and image sampling are rasterized by
  the GPU, and those are what drift.

  The committed set is authored on, and exact for, **macOS**
  (`[mac] failed_pixel_count_threshold = 0`). CI runs only
  `ubuntu-24.04` and `windows-latest` (`.github/workflows/ci.yml`), so the
  per-platform thresholds in [`kittest.toml`](../../kittest.toml) absorb genuine
  cross-rasterizer drift and are **smoke-only**: they catch layout shifts and
  large colour changes, and they cannot catch a sub-threshold regression. That
  is a deliberate, accepted tradeoff, not an oversight.

  Consequences to keep in mind:
  - Any ticket claiming byte-identity must demonstrate it on macOS with the
    `git diff --exit-code tests/snapshots/` check above. A green CI run is not
    evidence.
  - If this machine's adapter or GPU changes, every golden fails at 0. That
    wall of failures is a feature: it is one environmental change, easy to
    diagnose, and far better than a suite that quietly stopped checking.
  - On any other machine, expect to re-baseline once before the suite is
    meaningful.
