//! Seek-bar affordance probes: the hover thumb, the dragged-track
//! thickening, and the geometry tokens that own both.
//!
//! Moved here from the root `riff-tests` crate by the widgetry split (the
//! guard follows the source it guards): the linear control primitive lives
//! in this crate, so its probes do too.
//!
//! The spec's coverage table is explicit that this change is appearance-only
//! and that no hover is simulated, so nothing here drives a pointer onto a
//! control. What these tests pin instead is everything the hover is *built
//! from*, which is the part that can rot with no pixel ever changing:
//!
//! - the dimensions are tokens in the seek surface's own store, ordered so
//!   the grabbed track is thicker than the idle one and the thumb caps it;
//! - at rest a seek bar paints no thumb and its track is the idle
//!   thickness — and the *same* counter, on a frame that differs only in
//!   which of the control's two thumb slots the diameter comes from, does
//!   find a thumb on a control that wears one unconditionally, so the first
//!   assertion cannot pass on a frame that renders nothing at all;
//! - the thickening is a pure function of the caller's rect and the drag
//!   state, asserted directly, with no frame and no pointer involved.
//!
//! That last one is why the derivation is a `pub fn` rather than an
//! expression inside `paint`: dragging must change the track's thickness
//! and *only* its thickness, or the bar shifts under the pointer at the
//! moment the user can least afford to see it move.

// The probes keep the root suite's test scaffolding shapes (pixel-counting
// helpers, long local fixture functions) while this crate keeps
// `[lints] workspace = true`; the file-level opt-out scopes the root
// `riff-tests` crate's documented manifest-level exemption to just the
// files that need it.
#![allow(clippy::pedantic)]

use riff_gui_components::linear::{LinearControl, active_track, linear_control};
use riff_gui_theme::theme::Palette;
use riff_gui_theme::theme::geometry::now_playing::SEEK_H;
use riff_gui_theme::theme::geometry::playerbar::VOLUME_THUMB;
use riff_gui_theme::theme::geometry::seek::{THUMB_D, TRACK_H, TRACK_H_ACTIVE};
use std::collections::BTreeSet;

/// Is pixel `p` within `tol` of every channel of `color`?
///
/// The per-channel tolerance rule the root suite's `test_utils` shares;
/// copied here (with [`count_near`]) rather than raised into a workspace
/// crate for three small functions.
fn near(p: &image::Rgba<u8>, color: egui::Color32, tol: u8) -> bool {
    p.0[0].abs_diff(color.r()) <= tol
        && p.0[1].abs_diff(color.g()) <= tol
        && p.0[2].abs_diff(color.b()) <= tol
}

/// Count the pixels of `image` within `tol` of every channel of `color`.
fn count_near(image: &image::RgbaImage, color: egui::Color32, tol: u8) -> usize {
    image.pixels().filter(|p| near(p, color, tol)).count()
}

/// How many distinct pixel rows carry `color` anywhere in the frame — the
/// painted *height* of a band, read straight off the pixels rather than off
/// the caller's own rect. A row count is used instead of a pixel count
/// because the band's width is not what these tests claim anything about.
fn rows_carrying(image: &image::RgbaImage, color: egui::Color32) -> usize {
    image
        .enumerate_pixels()
        .filter(|(_, _, p)| near(p, color, 2))
        .map(|(_, y, _)| y)
        .collect::<BTreeSet<_>>()
        .len()
}

/// Render one seek-bar-shaped linear control **at rest** and hand back the
/// frame. No pointer event is ever fed to the harness, so egui has no
/// `latest_pos` at all: nothing is hovered, nothing is dragged, and — the
/// reason this seam is usable at all — `Harness::render` paints no cursor
/// triangle either (it only does so when a pointer is in the window).
///
/// The two flags select which thumb the control wears: a seek bar asks for
/// the hover-driven one and the volume slider brings its own, so holding
/// everything else fixed leaves exactly one thing varying between the pair of
/// frames the resting test compares.
fn render_at_rest(hover_thumb: bool, always_thumb: bool) -> image::RgbaImage {
    let palette = Palette::dark();
    let track = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(160.0, TRACK_H));
    let hit = track.expand2(egui::vec2(0.0, 12.0));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(200.0, 48.0))
        .with_pixels_per_point(1.0)
        .build_ui(move |ui| {
            linear_control(
                ui,
                &palette,
                &LinearControl {
                    id: egui::Id::new("test_seek_rest"),
                    track,
                    hit,
                    value: 0.5,
                    thumb: always_thumb.then_some(VOLUME_THUMB),
                    hover_thumb,
                    interactive: true,
                    label: "Seek",
                },
            );
        });
    harness.run();
    harness
        .render()
        .expect("the linear control renders headlessly")
}

/// The three dimensions this affordance is built from are tokens in the seek
/// surface's own store, and they are ordered the only way that reads:
/// dragging thickens the track, and the thumb is big enough to cap the
/// thickest state of the track it rides on.
///
/// The upper bounds are the real constraint: both seek surfaces build a hit
/// area out of their own row heights, Now Playing's being the tighter of the
/// two at 24px, so a thumb or a grabbed track taller than that would spill
/// onto the meta text above the row.
#[test]
fn test_seek_thickness_and_thumb_are_ordered_tokens_in_the_seek_store() {
    // Read through locals: the tokens are `const`s, and an assertion the
    // compiler can fold to `true` is a build error under this workspace's
    // clippy configuration. The player-bar token test writes its pins as
    // `(token - value).abs()` for the same reason.
    let (idle, active, thumb, hit_h, volume) =
        (TRACK_H, TRACK_H_ACTIVE, THUMB_D, SEEK_H, VOLUME_THUMB);

    assert!(idle > 0.0, "the idle track is a real thickness: {idle}");

    assert!(
        active > idle,
        "the grabbed track must be thicker than the idle one — that difference \
         is the whole grab signal: idle {idle}, active {active}"
    );
    assert!(
        active <= hit_h,
        "the grabbed track has to stay inside the tightest seek hit area \
         ({hit_h}px in Now Playing), or it bleeds onto the row above it"
    );

    assert!(
        thumb >= active,
        "the thumb caps the thickest state of the track, so it cannot be \
         smaller than it: thumb {thumb}, active track {active}"
    );
    assert!(
        thumb <= hit_h,
        "the thumb has to stay inside the tightest seek hit area ({hit_h}px), \
         or it spills over the time readouts flanking the bar"
    );

    // The volume slider's thumb is a *different* dimension on a *different*
    // surface, and it is unchanged: the volume control keeps its own 10px
    // always-visible thumb, which is what keeps the two bars distinguishable
    // by purpose instead of looking like one of them lost its thumb.
    assert!(
        (volume - 10.0).abs() < f32::EPSILON,
        "the volume slider keeps its mockup thumb diameter: {volume}"
    );
}

/// At rest a seek bar paints no thumb, and its track is the idle thickness.
///
/// The two frames share their drawing code, value, position and hit area, and
/// differ only in which of the control's two thumb slots the diameter comes
/// from — the second frame being the volume slider's real shape, at the
/// volume slider's own diameter. So it is the control that keeps the first
/// assertion honest: without it, "no ink pixels" would also be what a control
/// that painted nothing at all would report, and the band-height count would
/// be carrying the whole test.
#[test]
fn test_seek_bar_paints_no_thumb_at_rest_and_keeps_the_idle_thickness() {
    let palette = Palette::dark();

    let seek = render_at_rest(true, false);
    assert_eq!(
        count_near(&seek, palette.ink, 2),
        0,
        "a seek bar the pointer is not over paints no thumb — the affordance \
         that says where the pointer is must be absent, not dimmed"
    );
    assert_eq!(
        rows_carrying(&seek, palette.surface_3),
        TRACK_H as usize,
        "an ungrabbed track is painted at the idle thickness, so the only \
         hover-driven change to the bar's pixels is an added thumb"
    );

    // The control: same code, same value, same ink — but a thumb the control
    // wears unconditionally, exactly like the volume slider's.
    let always = render_at_rest(false, true);
    assert!(
        count_near(&always, palette.ink, 2) > 0,
        "the same counter finds the volume slider's always-visible thumb, so \
         the resting assertion above is about the hover gate and not about a \
         frame that renders nothing"
    );
}

/// The grabbed track is the caller's own span and centre at the *token's*
/// thickness: the thickening is a thickness, not a delta on whatever height
/// the caller happened to ask for, and at rest it is the caller's rect
/// byte-for-byte so an idle frame cannot drift by a rounding step.
#[test]
fn test_dragged_track_thickens_on_the_same_span_and_centre() {
    let idle = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(160.0, TRACK_H));

    assert_eq!(
        active_track(idle, false),
        idle,
        "at rest the painted track *is* the caller's rect — no re-derivation, so \
         an idle frame is identical to a golden's"
    );

    let grabbed = active_track(idle, true);
    assert_eq!(
        (grabbed.left(), grabbed.right()),
        (idle.left(), idle.right()),
        "grabbing changes the thickness only: the horizontal span — and with it \
         the fraction any pointer position maps to — must not move"
    );
    assert!(
        (grabbed.center().y - idle.center().y).abs() < f32::EPSILON,
        "the grabbed track keeps the idle track's centre, so the bar does not \
         shift under a pointer that is already sitting on it"
    );
    assert!(
        (grabbed.height() - TRACK_H_ACTIVE).abs() < f32::EPSILON,
        "the grabbed track is the active token: {} vs {TRACK_H_ACTIVE}",
        grabbed.height()
    );
    assert!(grabbed.height() > idle.height());

    // A caller that ever passes a taller rect than `TRACK_H` still gets the
    // same grabbed bar, measured from the token and not from its own height.
    let tall = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(160.0, SEEK_H));
    assert!(
        (active_track(tall, true).height() - TRACK_H_ACTIVE).abs() < f32::EPSILON,
        "the active thickness is the token, never the caller's height plus a \
         delta"
    );
}
