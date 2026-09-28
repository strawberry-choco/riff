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

## Amendment — 2026-09-28 (native macOS title bar, custom chrome on Windows and Linux)

The decision above stands for Windows and Linux: those two keep the frameless
window and riff's own caption controls. One of its three platforms no longer
matches it, so the split, the price of it, and the consequence it retires are
recorded here.

- **macOS is the exception: an AppKit-decorated window with a transparent title
  bar.** The window keeps its native decorations, takes a full-size content
  view, and hides the title bar, so the 56px strip renders behind the system
  title bar and the system's traffic lights are the window controls — riff draws
  none of them there. The strip's left cluster (wordmark, scan status) starts
  clear of the lights: its inset is the measured width of the traffic-light
  cluster, read from the window through eframe's window-chrome metrics and
  divided by the zoom factor into egui points, floored at the fixed
  `theme::geometry::titlebar::TRAFFIC_LIGHT_CLEARANCE` wherever eframe cannot
  measure. The two branches therefore differ in two places — the decoration and
  the left inset — and nowhere else.
- **The per-OS decision is one pure function, consumed twice.** `chrome_mode()`
  answers which convention this platform runs under, and both the launch viewport
  configuration and the titlebar renderer read it rather than branching on their
  own `#[cfg]`. That is what keeps the macOS half assertable as plain data on the
  Linux and Windows CI machines, which can never compile or run it. The remaining
  `#[cfg]`s exist to compile a branch's code out where it cannot run; they decide
  nothing.
- **The reversal cost far less than the record feared.** The stated worry was that
  reversing after layout shipped "would mean re-adopting native decorations
  across a chrome built around custom controls." Almost none of the strip is
  custom-control chrome. Shared by all three platforms: the wordmark and its
  equalizer glyph, the scan-status line, the global search field, the nav
  toggles, the strip's height, the palette, and the minimum-window logic. What
  differs is the decoration and the left inset. The strip was never rebuilt per
  platform — only its window-controls cluster and its left inset were branched —
  and the integrated wordmark survives on all three.
- **The spike gate stands, and the macOS leg passed it on a windowed run.** The
  gate has always been a per-platform spike before the shell commits; this
  change's macOS leg is a **windowed** spike, because what it must establish is
  precisely what no headless renderer can see: (a) hover and clicks reach the
  strip's interactive content under the invisible title-bar overlay, (b) the
  wordmark clears the traffic lights, (c) native dragging works. It cannot be
  automated — nothing in the suite drives AppKit — so it takes a human launching
  the built app on a Mac. **Outcome: passed, 2026-09-28.** All four facts are
  verified on a windowed run: (a) the search field and the nav toggles take
  hover and clicks under the overlay, (b) the wordmark and scan status clear the
  lights, (c) dragging the strip moves the window, and (d) below — tray → Quit
  quits — was confirmed on the same pass. **The gate is closed and the documented
  fallback is not needed:** the native path is the shipping path on macOS. The
  fallback stays recorded rather than deleted, because it is what the gate
  bought: keeping the frameless window and drawing mac-style buttons in the
  top-left, which is cheap precisely because every action behind those buttons
  already exists behind the Windows-convention ones. A gate that has never
  fired is not evidence the fallback was unnecessary.
- **Fact (d), verified windowed: the tray's Quit quits, and the reason it
  nearly did not is the sharpest edge in this amendment.** Adopting a native
  close request created a bug no test could catch. The tray's Quit and the red
  traffic light both reach eframe as `ViewportEvent::Close` — one variant, no
  payload, bit-for-bit identical at the point riff decides. So the new
  Quit-on-close resolution, asked to answer a *window* close, answered the
  *quit* too: with the preference off (the default) the Quit was cancelled, the
  window hid, and the app was left running with playback stopped and its tray
  menu dead — reachable only by force-quit. It failed silently and only when
  the window was visible, which is why the windowed run is what caught it.
  The fix makes the intent explicit: `close_resolution(CloseIntent, …)` takes
  who asked, and a `CloseIntent::Quit` passes through in *both* preference
  states. Provenance cannot be recovered from the event, so the app consults the
  `quit_flag` its own tray stores before enqueueing the close. Confirmed by a
  windowed run: tray → Quit quits. The order matters — the flag must be stored
  before the close is enqueued, or a frame can read it unset and cancel the quit
  anyway.
- **One consequence is retired; the others stand.** "Drag regions, resize
  handles, and maximize snapping become our responsibility on every target" now
  holds only on Windows and Linux, which keep the manual drag region and the
  double-click-to-maximize. On macOS the system owns dragging,
  double-click-to-zoom, resizing and snapping, and the user's own
  double-click-titlebar preference governs that band — riff hardcodes no
  maximize toggle there, and overrides the preference in neither direction. Linux
  remaining a third surface where frameless behavior must be explicitly verified
  is unchanged, and so is the chosen option: one chrome implementation, with a
  per-OS branch naming which decoration the window gets.
- **A native close request now resolves through the Quit-on-close preference.**
  The red traffic light is a close *request* where the custom X was a gesture, so
  it is answered by the persisted preference: off (the default) the close is
  cancelled and the window hides to the tray, which is the same gesture the
  custom X performs on Windows; on, the close proceeds and the app quits. OS-level
  Cmd+Q still always quits. Close behavior on Windows and Linux is unchanged.

## Amendment — 2026-09-28 (the macOS traffic lights are re-centred in riff's strip)

The decision above, and the amendment above it, both stand: macOS is the
AppKit-decorated exception, and its window controls are the system's own. What
the first amendment left unsaid was where those buttons *sit* — it inventoried
the platform split as the decoration and the left inset, "and nowhere else" —
and silence about placement is exactly what a defect in it will hide inside. The
placement is recorded here now, including the parts of it no test can see.

- **The cluster was about 14pt too high, and the first amendment's "and nowhere
  else" no longer holds.** `AppKit` lays the standard buttons out for *its own*
  titlebar — 28pt, 32pt on macOS 26 Tahoe, with 16pt/14pt button frames — while
  riff's strip is 56pt, so the lights rode roughly 14pt above the strip's centre.
  That was read as "native, therefore correct"; in a strip twice the system's
  own titlebar it is not, and a custom-height strip that strands the system's
  buttons high-left reads as broken to a mac user rather than as native. The
  first amendment's inventory of the platform split — "the two branches
  therefore differ in two places — the decoration and the left inset — and
  nowhere else" — is therefore corrected rather than left to contradict this:
  they differ in **three** places, the third being the lights' vertical
  placement. Nothing else about the two branches changed, and the strip itself
  remains one piece of shared chrome.
- **Y only, never x, and that is load-bearing.** The re-centring writes
  `origin.y` and leaves `origin.x` exactly as `AppKit` put it, because eframe
  derives `traffic_lights_size.x` from the close button's `origin.x`, which feeds
  `measured_traffic_lights_width` → `traffic_light_clearance` →
  `titlebar_left_inset` → the wordmark's position in the strip. Writing `y` alone
  leaves the measured clearance bit-identical, which is why the change is
  invisible to the committed goldens; moving `x` would shift the wordmark and
  churn every baseline. This is a standing constraint on any future work in
  this area, not a detail of this change: if a future change needs to move the
  cluster horizontally, it re-records the goldens deliberately or it does not
  move `x`.
- **The approach is public AppKit only, with no private API and no window
  delegate.** `NSWindow.standardWindowButton` and `NSView.setFrameOrigin` are
  public and honoured, and the titlebar container is reached by walking
  `superview` twice — button → titlebar container → the titlebar view — so no
  private class is ever *named*. There is no API that declares a traffic-light
  position and no public titlebar-height setter, which is why the container's
  frame, resized to the strip's height, is what actually gives the strip its
  height. Doing that needed a native binding, so `riff-gui` gained
  `objc2-app-kit` 0.3.2 under a `[target.'cfg(target_os = "macos")'.dependencies]`
  section — pinned to the version eframe 0.35 already resolves, so `NSWindow`
  unifies with eframe's own and the build cost is zero. Linux and Windows CI
  never compile it.
- **The correction runs per frame, in `logic()`, not from an `NSWindow` delegate.**
  eframe's frame order is `update` → `App::logic()` → `App::ui()` → paint/present
  → `post_rendering` → `handle_viewport_output`, and it is the last of those
  where `setTitle:` triggers AppKit's titlebar relayout, which *resets* the
  button positions. A reset therefore lands entirely between two presents, and
  the next `logic()` puts it right before that frame is presented — no visible
  jump. That is not a theoretical concern here: riff sends
  `ViewportCommand::Title` on every track change (`update_window_title`), and
  `setTitle:` is a documented reset trigger, so with no hook at all the lights
  would bounce on every song skip. The delegate alternative is ~200 lines of
  `define_class!` boilerplate maintaining an enumerated list of triggers against
  an AppKit that gained one on Tahoe. The per-frame check has no list to
  maintain: it asks only whether the measured geometry still matches the target,
  so a trigger nobody has heard of is caught on the frame after it fires. The
  call site is one ungated line in `RiffApp::logic` — ahead of `ui()`, so the
  lights are in place before the titlebar measures the clearance they feed, and
  outside `ui()`, which eframe skips entirely while the window is hidden. A
  settled window performs zero AppKit writes: the steady state is two pointer
  chases and two float compares.
- **The container is resized as well as the buttons, and its tracking areas are
  updated — the mitigation for the largest risk here.** AppKit caches each view's
  tracking areas, and a button moved without that leaves the hover highlight
  tracking the pointer's *stale* position. The evidence is not theoretical on
  either side: winit PR #4466 moved the buttons alone and shipped with hover
  stuck; Zed moves the container and the buttons together and does not. The
  container is grown away from the window's top edge, which is the one place
  AppKit's coordinate orientation leaks into the change — only the *anchor* is
  orientation-sensitive, because the plan's inset is both the top and the bottom
  gap by construction, so the buttons' placement is flip-agnostic. Every failure
  path fails safe into AppKit's own default placement: a window without the three
  standard buttons, a fullscreen window whose whole frame is animating, or a
  titlebar hierarchy a future macOS reshuffles (the `superview`-twice chain
  resolves to `None`) all simply stop riff interfering.
- **The button height is measured, never hardcoded, and there is no OS-version
  branch.** Apple publishes none of these numbers, and they have already changed
  twice — the titlebar 22pt→28pt at Big Sur and 28pt→32pt at Tahoe, the button
  frames 16pt→14pt. A hardcoded height would centre against the wrong target on
  somebody else's machine, in the one defect that only ever reproduces where you
  are not. Measuring means Apple's next change costs nothing. The strip height
  comes from `theme::TITLEBAR_H` read, never a literal, and the strip stays 56pt
  on every platform: this adds no design token, and the centring arithmetic is
  structural (like scroll and paging math) rather than design, which is why the
  drift tolerance lives with the algorithm instead of in the token store.
  Windows and Linux are byte-identical.
- **The blind spot, stated plainly: nothing in the suite exercises the AppKit
  half of this.** The traffic lights are composited by the window's theme frame
  *above* egui's render surface, so no pixel they move lands in a committed
  baseline — the top-left region of `shell_chrome_dark` and `titlebar_search_dark`
  contains no red, yellow, or green at all — and the module's OS binding is
  compiled out of the Linux and Windows CI legs entirely. Do not read a green
  suite as evidence that the lights are where they should be; it is evidence
  that the *arithmetic* is. What the suite does cover is the portable decision
  the shell executes: `traffic_light_plan` (target inset and container height)
  and `needs_reapply` (the drift threshold that separates "a layout pass with no
  effect" from "AppKit re-centred the cluster again") are plain `f64` functions
  asserted as data in `tests/ui_tests.rs`, ungated, so the Linux and Windows CI
  legs execute the geometry that positions the lights. The AppKit shell around
  them is a thin wrapper the suite does not test at all — which is why the next
  bullet is a human's observation rather than a green run.
- **Two facts only a human could establish, and both are now established on a
  windowed run (2026-09-28).** (a) The hover highlight tracks the *new* button
  position rather than the stale one. This was the largest risk in the change:
  AppKit caches each view's tracking areas, and a button moved without that
  leaves the hover rects where the buttons used to be, so the highlight would
  have tracked the pointer's stale position while the buttons moved. The
  mitigation was to resize the titlebar container as well as the buttons and to
  call `updateTrackingAreas()`, and **the mitigation works — the highlight
  follows the new position.** (b) Enlarging the titlebar container also shifts
  the window's top corner radius and horizontal padding, so the corner treatment
  changes; observed, it looks right. (b) was always going to be a judgement rather
  than a measurement — the argument that the new treatment is the *correct* one,
  on the grounds that the radius should track the 56pt strip rather than a 28pt
  one, is a design opinion — and a person has now made that call rather than the
  record merely flagging it as owed. What remains true after the run is the part
  that matters to anyone reading this later: **nothing in the suite covers
  either fact, and nothing ever will.** The lights are composited above egui's
  render surface, so a green run is silent about all of it; these two were
  answered by a human looking at a window, which is the only instrument that
  exists for them. Both were checked in the same run that closed the first
  amendment's spike gate above, so the whole macOS verification story is one
  windowed pass.
