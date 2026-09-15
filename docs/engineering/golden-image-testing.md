# Golden-Image Snapshot Testing

How riff pins its rendered UI pixel-for-pixel, and the workflow for authoring,
re-baselining, and reviewing golden images. Established by Issue 05; every
later visual-parity ticket builds on it.

## What exists

- **Harness**: [`egui_kittest`](https://docs.rs/egui_kittest) (dev-dependency,
  features `wgpu` + `snapshot`) renders real egui frames offscreen through
  wgpu — no window, no display server. Tests live in `tests/golden_tests.rs`
  inside the single integration crate (`tests/mod.rs`) and run under plain
  `cargo test` on a normal Windows dev box.
- **Baselines**: committed PNGs under `tests/snapshots/<name>.png` — 71 of
  them, all produced through the `snapshot` / `snapshot_animating` helpers.
- **Palettes**: the base set is **dark** ([ADR 0004](../adr/0004-dual-theme-tokens.md));
  eight structural components are mirrored in **light**, and two pin the
  **High Contrast** token set. `snapshot` takes the palette as a parameter.
- **First component**: `play_card_dark.png` — a primary "Play" button on a
  surface card, styled entirely from the token constants in
  `riff-gui/src/ui/theme.rs`.

See [`golden-image-gaps.md`](golden-image-gaps.md) for what each baseline
pins, the defects the coverage audit turned up, and the states that are
deliberately **not** pinned (hover, OS-level surfaces, animation phases).

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
     Two fixed frames are rendered; the harness never advances `input.time`,
     so the result is still deterministic.

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
   smell — it usually means the widget rendered nothing at all, which is how
   the detail column's missing empty state was found.

## Re-baselining

When a visual change is intentional:

```powershell
# Rewrite only the snapshots that currently fail:
$env:UPDATE_SNAPSHOTS = "true"; cargo test; Remove-Item Env:UPDATE_SNAPSHOTS

# Rewrite every snapshot, even ones passing within tolerance:
$env:UPDATE_SNAPSHOTS = "force"; cargo test; Remove-Item Env:UPDATE_SNAPSHOTS
```

`true`/`1` means "update the failing ones", so a change **smaller than
`failed_pixel_count_threshold`** (8 000 pixels) leaves the test green and the
stale baseline in place: the suite reports success while the committed PNG no
longer matches what the widget renders. After landing a change in an
already-pinned component — or when a golden's own composition changed — use
`force`, then confirm with a plain `cargo test golden_tests` run that the set
is genuinely green.

Commit the regenerated `tests/snapshots/*.png` together with the change that
caused them, so reviewers see the code diff and image diff in one place.
Never re-baseline to make an unexplained failure go away — a golden diff is a
review signal, not noise.

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
- **Harness concurrency is capped.** `tests/golden_tests.rs` runs at most
  `MAX_CONCURRENT_HARNESSES` (4) golden harnesses at a time, because every
  harness brings up its own wgpu device: with all seventy-one arriving
  together, `cargo test --all-targets` twice died with
  `STATUS_ACCESS_VIOLATION` (0xc0000005) part-way through the golden block —
  no failing test, no image diff, green on the next run. The cap costs a few
  seconds of suite time and removed the crash. If it ever recurs, lower the
  cap rather than chasing pixels.
  **It has recurred** (measured 2026-09-16, three golden-only runs crashed 3/3
  at the cap of 4 — and those runs spawn no runtime workers, so the crash is
  the harness block and not application teardown). See
  [./access-violation-flake.md](./access-violation-flake.md) for the run matrix
  and the open questions; lowering the cap is the next thing to try.
- **Baselines are machine-local.** wgpu picks different adapters/backends on
  different machines, and tiny driver-level differences can exceed the
  default per-pixel tolerance. Treat committed baselines as authored *on your
  machine*: if goldens fail everywhere after switching hardware, re-baseline
  once with `UPDATE_SNAPSHOTS=true` rather than chasing individual pixels.
