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
- **Baselines**: committed PNGs under `tests/snapshots/<name>.png`.
- **Palette**: baselines are authored against the **dark** palette per
  [ADR 0004](../adr/0004-dual-theme-tokens.md).
- **First component**: `play_card_dark.png` — a primary "Play" button on a
  surface card, styled entirely from the token constants in
  `riff-gui/src/ui/theme.rs`.

## Running

```bash
cargo test                      # everything, including golden tests
cargo test golden_tests         # just the golden suite
```

A mismatch fails the test and prints the absolute path of the diff image.

## Authoring a new golden

1. Draw the component in a function taking `&mut egui::Ui`, styling every
   color from `riff_gui::ui::theme` tokens (never hardcoded values).
2. Render it through the shared helper (`snapshot_dark_play_card` shows the
   pattern): fixed harness size, fixed `pixels_per_point(1.0)`, dark palette
   installed, then `harness.snapshot("<name>")`.
3. Run `cargo test golden_tests`. The first run **fails** because no baseline
   exists — that is the expected red step.
4. Generate the baseline:

   ```powershell
   $env:UPDATE_SNAPSHOTS = "true"; cargo test golden_tests; Remove-Item Env:UPDATE_SNAPSHOTS
   ```

5. **Open the PNG and review it** before committing. A golden that renders
   blank, clipped, or with harness artifacts (see determinism rules below) is
   worse than no golden.

## Re-baselining

When a visual change is intentional:

```powershell
# Rewrite only the snapshots that currently fail:
$env:UPDATE_SNAPSHOTS = "true"; cargo test; Remove-Item Env:UPDATE_SNAPSHOTS

# Rewrite every snapshot, even ones passing within tolerance:
$env:UPDATE_SNAPSHOTS = "force"; cargo test; Remove-Item Env:UPDATE_SNAPSHOTS
```

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
  simulated clicks if a future golden needs them).
- **Baselines are machine-local.** wgpu picks different adapters/backends on
  different machines, and tiny driver-level differences can exceed the
  default per-pixel tolerance. Treat committed baselines as authored *on your
  machine*: if goldens fail everywhere after switching hardware, re-baseline
  once with `UPDATE_SNAPSHOTS=true` rather than chasing individual pixels.
