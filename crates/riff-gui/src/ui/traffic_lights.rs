//! Vertically centres the macOS traffic lights in riff's custom-height strip.
//!
//! # What this is
//!
//! riff's window keeps its `AppKit` decorations with a transparent title bar
//! carrying riff's full-size content
//! ([`chrome::ChromeMode::NativeTrafficLights`](crate::ui::chrome::ChromeMode::NativeTrafficLights)),
//! so the system's red/yellow/green buttons are the window controls. But
//! `AppKit` lays them out for ITS OWN titlebar — 28pt, 32pt on macOS 26 Tahoe,
//! with 16pt/14pt button frames — while riff's strip is
//! [`TITLEBAR_H`](crate::ui::theme::TITLEBAR_H), 56pt. So the cluster rides
//! roughly 14pt above the strip's centre, and this module re-centres it once
//! per frame.
//!
//! # Why per-frame and not a window delegate
//!
//! `eframe`'s frame order is `update(..)` -> `App::logic()` -> `App::ui()` ->
//! paint/present -> `post_rendering` -> `handle_viewport_output`, and the last
//! of those is where `setTitle:` triggers `AppKit`'s titlebar relayout, which
//! RESETS the button positions. A reset therefore lands entirely between two
//! presents, and the next `logic()` puts it right before that frame is
//! presented — no visible jump. That matters concretely because riff sends a
//! `ViewportCommand::Title` on every track change (`update_window_title`), and
//! `setTitle:` is a documented reset trigger: with no hook at all the lights
//! would bounce on every song skip.
//!
//! The alternative, an `NSWindow` delegate subclass re-centring from an
//! enumerated list of triggers, is ~200 lines of `define_class!` boilerplate
//! keeping that list in sync with an `AppKit` that gained a trigger on Tahoe.
//! The drift check has no list to maintain: it only asks whether the measured
//! geometry still matches the target, so a trigger nobody has heard of is
//! caught on the frame after it fires.
//!
//! # Why the test suite does not cover this
//!
//! **Neither the golden-image suite nor CI exercises this module at all.** The
//! traffic lights are composited by the window's theme frame ABOVE egui's
//! render surface, so no pixel they move lands in a committed baseline — the
//! top-left region of `shell_chrome_dark` and `titlebar_search_dark` contains
//! no red/yellow/green at all. The `AppKit` body is additionally gated to
//! macOS, so the Linux/Windows legs compile only the no-op `apply` at the
//! bottom of this file and never link the native binding. What IS covered is
//! the portable decision it executes,
//! [`traffic_light_plan`](crate::ui::chrome::traffic_light_plan)
//! and [`needs_reapply`](crate::ui::chrome::needs_reapply), asserted as data in
//! `tests/ui_tests.rs`. Do not read a green suite as evidence that the lights
//! are where they should be; it is evidence the arithmetic is.

#[cfg(target_os = "macos")]
use super::chrome::{needs_reapply, traffic_light_plan};
#[cfg(target_os = "macos")]
use super::theme;

/// Re-centres the window's standard traffic lights in riff's strip, and does
/// nothing at all when they are already there. Called once per frame from
/// `RiffApp::logic`.
///
/// Every early return fails safe: an unresolvable handle, a window without the
/// three standard buttons, or a titlebar hierarchy not shaped the way we assume
/// leaves `AppKit` in charge of its own default placement. A titlebar 14pt high
/// is the correct failure for cosmetic chrome; a misplaced cluster is not.
#[cfg(target_os = "macos")]
pub fn apply(frame: &mut eframe::Frame) {
    use objc2_app_kit::{NSWindowButton, NSWindowStyleMask};

    let Some(window) = ns_window(frame) else {
        return;
    };

    // Fullscreen is a transition, not a steady state: the window's whole frame
    // is animating, and writing frames into the middle of that animation
    // fights it. Leaving fullscreen is itself a relayout, i.e. a drift event,
    // so the next frame re-centres — there is nothing to restore and no state
    // to unwind.
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }

    // A window without the standard buttons (borderless, or a style that hides
    // the cluster) is not an error to report: it means the platform convention
    // being emulated is not in play, and AppKit's default is already correct.
    let (Some(close), Some(miniaturize), Some(zoom)) = (
        window.standardWindowButton(NSWindowButton::CloseButton),
        window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        window.standardWindowButton(NSWindowButton::ZoomButton),
    ) else {
        return;
    };

    // `superview` twice: button -> titlebar container -> the titlebar view
    // itself. That is an ASSUMPTION about AppKit's private titlebar hierarchy,
    // and it is the one assumption in this module. It is also why the
    // container is resized alongside the buttons: the box the cluster is laid
    // out in is what AppKit's cached hover group is most plausibly keyed to.
    // (winit PR #4466 moved the buttons alone and shipped with hover stuck at
    // the stale position; Zed moves container + buttons and does not.) If a
    // future macOS reshuffles the hierarchy this chain resolves to `None` and
    // we simply do not interfere.
    //
    // SAFETY: live AppKit views owned by the window for its lifetime, and both
    // results are `Retained`, so the returned objects stay alive across the
    // reads and writes below and nothing dangles. `superview` is unsafe because
    // objc2 cannot prove the superview outlives the call — AppKit's titlebar
    // hierarchy does.
    let Some(container) =
        (unsafe { close.superview() }).and_then(|titlebar| unsafe { titlebar.superview() })
    else {
        return;
    };

    let strip = f64::from(theme::TITLEBAR_H);
    // MEASURED, never hardcoded: 16pt before Tahoe, 14pt on it. A hardcoded
    // height would centre against the wrong target the day Apple changes it.
    let button_h = close.frame().size.height;
    let (inset_y, target_h) = traffic_light_plan(strip, button_h);

    // Read first, write only on drift. The steady state is two pointer chases
    // and two float compares with zero AppKit writes — no layout pass, no
    // tracking-area churn.
    if !needs_reapply(
        container.frame().size.height,
        close.frame().origin.y,
        target_h,
        inset_y,
    ) {
        return;
    }

    // Container first: the buttons are placed relative to it, and its
    // tracking areas are what the cluster's hover group is computed from.
    // AppKit caches each view's tracking areas, so moving one without this
    // leaves the hover rects where the buttons used to be — the highlight
    // would track the pointer's stale position while the buttons moved.
    resize_container(&container, target_h);
    container.updateTrackingAreas();

    // Buttons: Y ONLY. x is left exactly as AppKit put it, and that is
    // load-bearing rather than an oversight. eframe derives
    // `traffic_lights_size.x` from the close button's `origin.x`, which feeds
    // `measured_traffic_lights_width` -> `traffic_light_clearance` ->
    // `titlebar_left_inset` -> the wordmark's position inside the strip.
    // Moving x would shift the wordmark and churn every golden baseline;
    // writing `origin.y` alone leaves the measured clearance bit-identical, so
    // the change is invisible to the snapshots.
    for button in [close, miniaturize, zoom] {
        let mut origin = button.frame().origin;
        origin.y = inset_y;
        button.setFrameOrigin(origin);
        button.updateTrackingAreas();
    }
}

/// The `NSWindow` behind an eframe frame, resolved through the raw window
/// handle. `AppKitWindowHandle` names only the `ns_view`, so the window comes
/// from `NSView::window()` — the same hop eframe makes in its own chrome
/// metrics.
///
/// `eframe::Frame` implements `HasWindowHandle` directly, so this takes the
/// short path rather than the `winit_window().window_handle()` hop
/// [`measured_traffic_lights_width`](crate::ui::chrome::measured_traffic_lights_width)
/// takes for its own metric: one fewer hop, and no reason to reach into winit's
/// window when it is the same handle either way.
///
/// Returned as `impl Deref` because the concrete `Retained` type lives in
/// `objc2`, which this crate does not depend on directly — everything needed
/// here is reachable through `NSWindow`.
#[cfg(target_os = "macos")]
fn ns_window(
    frame: &mut eframe::Frame,
) -> Option<impl std::ops::Deref<Target = objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    let ns_view_ptr = appkit.ns_view.as_ptr().cast::<NSView>();
    if ns_view_ptr.is_null() {
        return None;
    }
    // SAFETY: the pointer is non-null (checked just above) and comes from the
    // windowing system, so it names a live `NSView` that AppKit owns for the
    // window's lifetime; the reference is read only before the borrow ends and
    // `window()` hands back a retained window, so nothing outlives it. This is
    // eframe's own `ns_view_from_handle`, restated inline because its
    // `#[expect(unsafe_code)]` is crate-local and does not carry across.
    let ns_view = unsafe { &*ns_view_ptr };
    ns_view.window()
}

/// Grows the titlebar container to the strip's full height, growing it AWAY
/// from the window's top edge.
///
/// `setFrameSize` alone would resize about the centre point and leave half the
/// new box hanging above the window, so the origin is re-pinned alongside the
/// height. Which edge is pinned is the ONLY place `AppKit`'s coordinate
/// orientation leaks into this module: a flipped container grows downward from
/// its origin, an unflipped one grows upward. The buttons need no such branch,
/// because the plan's inset is both the top and the bottom gap by construction
/// — the placement is flip-agnostic even though the container's anchor is not.
#[cfg(target_os = "macos")]
fn resize_container(container: &objc2_app_kit::NSView, target_h: f64) {
    let old = container.frame();
    let flipped = container.isFlipped();
    let pinned = if flipped {
        old.origin.y
    } else {
        old.origin.y + old.size.height
    };
    let mut next = old;
    next.size.height = target_h;
    next.origin.y = if flipped { pinned } else { pinned - target_h };
    container.setFrame(next);
}

/// No-op off macOS, so the call site is one ungated line. (On Linux the
/// caller's enclosing function does not exist at all, so this is never reached
/// there; on Windows it is called and does nothing.)
#[cfg(not(target_os = "macos"))]
pub fn apply(_frame: &mut eframe::Frame) {}
