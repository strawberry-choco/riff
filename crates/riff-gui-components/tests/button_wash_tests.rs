//! Button wash probes: the hover washes are pure functions of their tween
//! values, and only the two wash roles ask the frame loop for passes.
//!
//! Moved here from the root `riff-tests` crate by the widgetry split (the
//! guard follows the source it guards): the button primitive lives in this
//! crate, so its probes do too. The repaint-cause filter below reads the
//! component crate's own `button.rs`, so a probe that finds silence is
//! silence in *this* crate's button and not a stale path reading nothing.

// The probes keep the root suite's test scaffolding shapes (twenty-pass
// driven frames, tiny local bindings, closure-heavy harness builders) while
// this crate keeps `[lints] workspace = true`; the file-level opt-out scopes
// the root `riff-tests` crate's documented manifest-level exemption to just
// the files that need it.
#![allow(clippy::pedantic)]

use riff_gui_components::button::{self, TextButton, Variant};
use riff_gui_theme::icons::IconCache;
use riff_gui_theme::theme::{self, MOTION_HOVER, Palette};

/// One step of the harness clock, small enough that the hover token spans
/// several frames — the same choice the row band's frame-loop test makes,
/// and for the same reason: at the default 0.25 s step a 0.10 s tween
/// saturates inside one frame and there is nothing left to observe.
const STEP_DT: f32 = 0.02;
/// Passes sampled per probe, several times the longest a motion token
/// could publish.
const SAMPLED_PASSES: usize = 20;
/// Settled passes the sample must end with, so "stops asking" is asserted
/// over a run and not over one frame.
const SETTLED_PASSES: usize = 4;

/// A `Color32`'s channels as a fraction of its own alpha — the
/// *un-premultiplied* RGB, which is the colour a translucent wash is
/// nominally "of". `Color32` stores premultiplied bytes, so a wash that
/// keeps its hue reads the same here at every coverage; one that had lerped
/// its RGB toward a backdrop would drift. The row-band test carries the
/// same helper privately; this is that helper, for the same reason.
fn nominal_rgb(color: egui::Color32) -> [f32; 3] {
    let a = f32::from(color.a()).max(1.0) / 255.0;
    [
        f32::from(color.r()) / a,
        f32::from(color.g()) / a,
        f32::from(color.b()) / a,
    ]
}

/// The source files that asked for a repaint on the pass that has just
/// finished, or an empty vector if nothing did.
///
/// egui swaps the accumulated causes into a "previous pass" slot at the
/// *start* of a pass (`egui-0.35.0/src/context.rs:101-105`), so a reading
/// taken right after `step` describes the pass that just ran, and no
/// alignment arithmetic is needed. `Context::request_repaint` is
/// `#[track_caller]`, so a cause names riff's call site — the
/// `animate_bool_with_time` line in this crate's `button.rs` — rather than
/// egui's own.
///
/// The separators are normalized to `/`: `file!()` on Windows bakes in
/// backslash paths, so a `contains` against a forward-slash module path
/// never matches there and every ask reads as silence.
fn asked_by(harness: &egui_kittest::Harness<'_, ()>) -> Vec<String> {
    harness
        .ctx
        .repaint_causes()
        .iter()
        .map(|cause| format!("{}:{}", cause.file.replace('\\', "/"), cause.line))
        .collect()
}

/// The causes attributed to this crate's button module, which is where a
/// wash's tween read lives. Filtered by file because egui's own housekeeping
/// (a pointer move, a focus change) also asks for frames, and the claim
/// under test is about the ones a *riff tween* issues.
fn asked_by_buttons(harness: &egui_kittest::Harness<'_, ()>) -> Vec<String> {
    asked_by(harness)
        .into_iter()
        .filter(|cause| cause.contains("crates/riff-gui-components/src/button.rs"))
        .collect()
}

/// One text button of `variant` over the card plane, with a clock small
/// enough to see a hover token fade. No tooltip: a tooltip schedules its own
/// frame, and the probe is counting the tween's.
fn wash_probe(
    variant: Variant,
    label: &'static str,
) -> (egui_kittest::Harness<'static, ()>, egui::Rect) {
    let palette = Palette::dark();
    let rect = egui::Rect::from_min_size(egui::pos2(40.0, 22.0), egui::vec2(120.0, 36.0));
    let id = egui::Id::new(label);
    let mut cache = IconCache::new();
    let harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(200.0, 80.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(STEP_DT)
        .build_ui_state(
            move |ui, _: &mut ()| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                let _ = button::text_button(
                    ui,
                    &mut cache,
                    &palette,
                    false,
                    &TextButton {
                        id,
                        rect,
                        label,
                        a11y: label,
                        tooltip: None,
                        icon: None,
                        small: false,
                        variant,
                        enabled: true,
                    },
                );
            },
            (),
        );
    (harness, rect)
}

/// The last frame's pixels, with the card plane the probe painted behind
/// the button so a comparison reads the button and not the window.
fn render_probe(harness: &mut egui_kittest::Harness<'_, ()>) -> image::RgbaImage {
    harness
        .render()
        .expect("the wash probe must render headlessly")
}

/// Walk the pointer onto the button and hold it there for
/// [`SAMPLED_PASSES`] passes, returning the first hovered frame, the last
/// one, and the repaint causes attributed to `button.rs` on **each** pass.
///
/// The causes come back per pass and un-collapsed, because the *count* on a
/// pass is the claim: a tween that asked twice would show up as a second
/// cause on every in-flight pass while leaving the number of asking passes
/// exactly where it was, so a probe that reduced each pass to a bool could
/// not tell a tween from a tween plus a redundant request.
///
/// The first frame is read *after* the first hover step, and the last after
/// the final one, so both are frames the pointer was on the button for.
fn hover_frames(
    harness: &mut egui_kittest::Harness<'_, ()>,
    rect: egui::Rect,
) -> (image::RgbaImage, image::RgbaImage, Vec<Vec<String>>) {
    // Settling first is what registers the tween at 0.0, and it has to come
    // before the pointer arrives: egui's `animate_bool` returns its target
    // outright on the *first* call for an id it has never seen, so a probe
    // that hovered before the button had ever painted at rest would measure
    // a snap and call it a fade.
    harness.run();
    let mut causes_per_pass = Vec::with_capacity(SAMPLED_PASSES);
    let mut first = None;
    let mut last = None;
    for pass in 0..SAMPLED_PASSES {
        harness.hover_at(rect.center());
        harness.step();
        causes_per_pass.push(asked_by_buttons(harness));
        let frame = render_probe(harness);
        if pass == 0 {
            first = Some(frame.clone());
        }
        last = Some(frame);
    }
    (
        first.expect("one hovered frame"),
        last.expect("one settled frame"),
        causes_per_pass,
    )
}

/// Each button wash is a pure function of its tween value, asserted at both
/// endpoints and at the midpoint — the seam the motion spec's coverage table
/// names for this change ("endpoints match the two tokens; the midpoint is
/// strictly between them").
///
/// **The endpoints are the contract that lets this change exist at all.**
/// `t = 0` must be fully transparent and `t = 1` must be *exactly* the
/// wash the instant swap painted, which is asserted against the existing
/// `accent_fill` / `destructive_fill` helpers rather than against a
/// literal: the tweened form is defined in terms of them, so this is the
/// pin that the definition has not quietly changed what "fully hovered"
/// means. Every golden in the suite captures an unhovered or a settled
/// button, and neither may move.
///
/// The midpoint is asserted three ways, because they are three claims. It
/// is *strictly between* the endpoints; its *coverage* is strictly between
/// nothing and the full wash; and its *hue* is the wash's own, because a
/// wash is laid over a backdrop the painter never sees — scaling coverage
/// is the whole operation, and an RGB lerp would be interpolating toward a
/// fiction. That last one is what catches the premultiplied-alpha trap:
/// `egui::Color32` holds premultiplied bytes, so compositing a faded wash
/// through `blend_over` multiplies the coverage by itself and lands on a
/// colour darker than *both* endpoints — a colour belonging to neither
/// state, which is the exact failure the store's rules exist to prevent.
///
/// Assert one wash's endpoints, midpoint and hue, given its
/// `t`-parameterised form and the settled colour it must land on. Shared by
/// both roles because they are the same gesture in two hues, and running
/// the identical assertions over both is what shows that.
fn assert_wash(
    role: &str,
    at: impl Fn(f32) -> egui::Color32,
    settled: egui::Color32,
    nominal_of: egui::Color32,
) {
    let idle = at(0.0);
    assert_eq!(
        idle,
        theme::TRANSPARENT,
        "{role}: t=0 must be fully transparent, so an unhovered button paints what it painted \
         before the wash tweened"
    );
    assert_eq!(
        at(1.0),
        settled,
        "{role}: t=1 must be exactly the wash the instant swap painted, so a settled hover is \
         byte-identical to the behaviour this replaces"
    );

    let mid = at(0.5);
    assert!(
        mid != idle && mid != settled,
        "{role}: the midpoint must be strictly between the two endpoints, not equal to \
         either: {mid:?}"
    );
    assert!(
        mid.a() > 0 && mid.a() < settled.a(),
        "{role}: the midpoint's coverage must be strictly between nothing and the full wash, \
         which is what 'half a wash' means: alpha {} of the wash's {}",
        mid.a(),
        settled.a()
    );
    // The hue is the wash's own at every coverage — the anti-crossfade
    // property, asserted in the value rather than in a frame, and against the
    // *token* the wash is nominally "of" rather than against the settled
    // wash, so the claim is that the fade changes coverage and nothing else.
    //
    // **The bound is half a byte step at the midpoint's own alpha, not a
    // round number.** These targets are *translucent* — a 10% wash — so the
    // stored bytes are premultiplied by 13/255 at the midpoint and one byte
    // step there is worth 255/13 = 19.6 nominal units; rounding the same
    // quantity two ways puts the brand's green at 137.3 and 127.5 around a
    // true 130. A +/-1 tolerance (which is what an *opaque* target like the
    // row band's `row_hover` affords) would fail on quantisation alone, and
    // loosening it to a whole byte step would be toothless. Half a step is
    // exactly what the rounding guarantees, and a crossfade — which is what
    // compositing through `blend_over` would produce — misses it by an
    // order of magnitude, because it also lands opaque.
    let half_step = 127.5 / f32::from(mid.a());
    let (nominal, target) = (nominal_rgb(mid), nominal_rgb(nominal_of));
    for (channel, (got, want)) in nominal.iter().zip(target.iter()).enumerate() {
        assert!(
            (got - want).abs() <= half_step,
            "{role}: channel {channel} of the midpoint drifted from the wash's own hue ({got} \
             vs {want}, within {half_step} of half a byte step at alpha {}): a wash scales \
             coverage over a backdrop the painter cannot see, and never lerps RGB toward a \
             guess at it",
            mid.a()
        );
    }
}

/// Each button wash is a pure function of its tween value, asserted at both
/// endpoints and at the midpoint — the seam the motion spec's coverage table
/// names for this change ("endpoints match the two tokens; the midpoint is
/// strictly between them").
///
/// **The endpoints are the contract that lets this change exist at all.**
/// They are asserted against the existing `accent_fill` /
/// `destructive_fill` helpers rather than against a literal, because the
/// tweened forms are *defined* in terms of them: this is the pin that the
/// definition has not quietly changed what "fully hovered" means. Every
/// golden in the suite captures an unhovered or a settled button, and
/// neither may move.
///
/// The midpoint carries two further claims. Its *coverage* is strictly
/// between nothing and the full wash — that is what "half a wash" means.
/// And its *hue* is the wash's own, because a wash is laid over a backdrop
/// the painter never sees: scaling coverage is the whole operation, and an
/// RGB lerp would be interpolating toward a fiction. That last one is what
/// catches the premultiplied-alpha trap — `egui::Color32` holds
/// premultiplied bytes, so compositing a faded wash through `blend_over`
/// multiplies the coverage by itself and lands on a colour darker than
/// *both* endpoints, which is the exact failure the store's rules exist to
/// prevent.
///
/// **Red, proven by revert:** before these two functions existed the suite
/// did not compile — `error[E0425]: cannot find function 'accent_wash' in
/// module 'theme'`, with egui pointing at `destructive_fill` as the
/// similarly-named function — which is the honest red for a test whose
/// subject is a function. It earned its keep immediately after: the first
/// implementation, `wash_at(accent_fill(palette, true), t)`, failed the
/// endpoint assertion with `left: #18_0D_03_FF` against
/// `right: #18_0D_03_1A` — a 10% wash landing **opaque**, because
/// `wash_at` builds straight-alpha and was handed a colour that was already
/// premultiplied. Fixing the *production* code (feed it the un-premultiplied
/// token and a coverage) is what the endpoint assertion is for.
#[test]
fn test_the_accent_and_destructive_washes_are_pure_functions_of_their_tween_values() {
    let palette = Palette::dark();
    assert_wash(
        "accent",
        |t| theme::accent_wash(&palette, t),
        theme::accent_fill(&palette, true),
        palette.brand_primary,
    );
    assert_wash(
        "destructive",
        |t| theme::destructive_wash(&palette, t),
        theme::destructive_fill(&palette, true),
        palette.error,
    );
}

/// Only the two wash roles tween, and they ask for frames only while they
/// are in flight.
///
/// This is the property the pure function cannot see, and it is the one
/// that keeps the feature affordable: a tween allocates an animation id and
/// asks the frame loop for a pass, so a variant that tweens for tidiness
/// multiplies that cost by every button on screen. So the probe pins both
/// halves from the outside:
///
/// - a hovered **Secondary** — a role with a real hover treatment (a
///   `surface_3` face and the hover stroke) that must nonetheless be
///   **instant** — asks for no frame from `button.rs` on any pass, and its
///   first hovered frame is already **pixel-identical to its own settled
///   frame**. That is the byte-identity claim, stated against the only
///   comparison that can mean it: a hovered button legitimately repaints,
///   so "identical to unhovered" would be a claim about nothing. Identical
///   to *settled* is the claim that no tween was allocated.
/// - a hovered **Accent** asks for a frame across the whole tween and then
///   stops, and its first hovered frame differs from its settled one —
///   which is what a fade looks like from the outside.
///
/// The asking run's length is the claim that the wash runs on the *hover*
/// token rather than the published global default: at this step
/// `MOTION_HOVER` (0.10 s) is five passes and `MOTION_DEFAULT` (0.18 s) is
/// nine, so a wash that had picked up `Style::animation_time` instead of
/// passing the token explicitly is caught here.
///
/// **Red, proven by revert, twice.** Before the tween was wired the Accent
/// probe reported `asked for a frame on 0 of 20` with
/// `asked=[false, false, ... x20]` — the instant swap's signature, which is
/// also what an *un*-tweened implementation looks like from the outside.
/// Reverting only the wiring in `paint_text_button` (leaving both
/// `theme::accent_wash` and `theme::destructive_wash` in place) reproduces
/// that identical red while the pure-function test above stays green. That
/// split is the point of having two tests: the value and the clock are
/// separate claims, and only one of them can be wrong at a time.
///
/// **The per-pass count, proven by duplication.** The asking-pass assertions
/// above cannot see a redundant `request_repaint`, because asking twice does
/// not make a pass any more *asked-for* than asking once. So the redundant
/// request was added to `wash_tween` in the form rule 3 warns about —
/// `if 0.0 < t && t < 1.0 { ui.ctx().request_repaint(); }`, which is how the
/// Folders tree writes its own — and this failed with:
///
/// ```text
/// pass 1 made ["crates/riff-gui-components/src/button.rs:166",
///              "crates/riff-gui-components/src/button.rs:170"] — the accent wash must cost
/// exactly one repaint request per in-flight frame …
///   left: 2
///  right: 1
/// ```
///
/// Two things are worth keeping from that output. The two causes name
/// **two different lines of `button.rs`** — `:166` the
/// `animate_bool_with_time` and `:170` the redundant call — so
/// `#[track_caller]` attributes per call site and a duplicate is legible in
/// the failure message rather than merely counted. And the row band's
/// frame-loop test **passed with the duplicate in place** (measured, not
/// assumed): it drives a different animation id in a different module, so
/// nothing outside this module's own frame loop can stand in for it. That is
/// why the store's rule-3 claim needed pinning *here* and not just there.
#[test]
fn test_only_the_accent_and_destructive_washes_tween() {
    let hover_passes = (MOTION_HOVER / STEP_DT).ceil() as usize;

    // --- Secondary: a real hover treatment, deliberately instant. --------
    let (mut harness, rect) = wash_probe(Variant::Secondary, "Rescan now");
    harness.run();
    assert!(
        asked_by_buttons(&harness).is_empty(),
        "an idle button must ask the frame loop for nothing"
    );
    let (first_secondary, settled_secondary, causes_secondary) = hover_frames(&mut harness, rect);
    assert!(
        causes_secondary.iter().all(Vec::is_empty),
        "a hovered secondary asked for frames {causes_secondary:?} — only the accent and \
         destructive washes are on the allow-list, and a per-variant fill is not one of them"
    );
    assert_eq!(
        first_secondary, settled_secondary,
        "a hovered secondary's first frame must be pixel-identical to its settled one: the \
         hover swap is instant, so there is no tween sample to find part-applied"
    );

    // --- Accent: the same gesture, and this one fades. --------------------
    let (mut harness, rect) = wash_probe(Variant::Accent, "Rescan now");
    harness.run();
    let (first_accent, settled_accent, causes_accent) = hover_frames(&mut harness, rect);

    let asked_accent: Vec<bool> = causes_accent
        .iter()
        .map(|causes| !causes.is_empty())
        .collect();
    let asking = asked_accent.iter().filter(|asked| **asked).count();
    let longest_run = asked_accent
        .iter()
        .fold((0usize, 0usize), |(best, current), asked| {
            let current = if *asked { current + 1 } else { 0 };
            (best.max(current), current)
        })
        .0;
    assert_eq!(
        asking, longest_run,
        "frames must be asked for across the whole tween, not in bursts: {asked_accent:?}"
    );
    assert!(
        (2..=hover_passes).contains(&asking),
        "the accent wash must tween for roughly MOTION_HOVER ({MOTION_HOVER}s ≈ \
         {hover_passes} passes at {STEP_DT}s) and asked for a frame on {asking} of \
         {SAMPLED_PASSES}: an instant swap never asks, a tween that never settles asks \
         forever. asked={asked_accent:?}"
    );
    assert!(
        SAMPLED_PASSES - asking >= SETTLED_PASSES,
        "the sample holds fewer than {SETTLED_PASSES} settled passes, so 'a settled hover \
         asks for nothing' is unproven: {asked_accent:?}"
    );
    assert_ne!(
        first_accent, settled_accent,
        "the accent wash must actually fade: its first hovered frame is identical to its \
         settled one, so nothing moved between them"
    );

    // **Exactly one request per in-flight pass, and none once settled.** This
    // is the half the asking-pass count above cannot see, and it is what
    // makes the store's rule-3 claim true for *this* surface: the wash relies
    // on egui's bound for its frames, so a redundant
    // `ui.ctx().request_repaint()` beside the tween read would be riff
    // saying what egui has already said, once per frame of every fade in the
    // app. It leaves the number of asking passes untouched, so only the
    // per-pass count catches it.
    //
    // `Context::request_repaint` is `#[track_caller]`, so a `RepaintCause`
    // names riff's *call site* — the `animate_bool_with_time` line in
    // `button.rs` — and not egui's own `request_repaint`. Attribution by file
    // therefore cannot by itself tell "egui's wrapper asked on riff's
    // behalf" from "riff asked": both are `button.rs`. The count can.
    for (pass, causes) in causes_accent.iter().enumerate() {
        match asked_accent[pass] {
            true => assert_eq!(
                causes.len(),
                1,
                "pass {pass} made {causes:?} — the accent wash must cost exactly one repaint \
                 request per in-flight frame, the one egui's animate_bool_with_time already \
                 issues while the value is strictly between 0 and 1. A second cause is riff \
                 asking again for a frame it has already been given."
            ),
            false => assert!(
                causes.is_empty(),
                "pass {pass} asked for a frame after the wash had settled: {causes:?} — a \
                 settled tween asks for nothing, which is the bound at the far end"
            ),
        }
    }
}
