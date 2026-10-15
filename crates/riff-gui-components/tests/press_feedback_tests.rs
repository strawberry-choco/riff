//! Press-feedback probes: a pressed button paints a state distinct from
//! hover, and the press is reported through the widget's own `Response`.
//!
//! Moved here from the root `riff-tests` crate by the widgetry split (the
//! guard follows the source it guards): the text button and the ghost icon
//! button primitives live in this crate, so their probes do too.

// The probes keep the root suite's test scaffolding shapes (driven pointer
// frames, tiny local bindings, closure-heavy harness builders) while this
// crate keeps `[lints] workspace = true`; the file-level opt-out scopes the
// root `riff-tests` crate's documented manifest-level exemption to just the
// files that need it.
#![allow(clippy::pedantic)]

use riff_gui_components::button::{self, Variant};
use riff_gui_components::ghost_icon_button;
use riff_gui_theme::icons::{Icon, IconCache};
use riff_gui_theme::theme;

/// What one driven frame reported for the probed button, read back out of
/// the button's own `Response` — egui's verdict, not the painter's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PressProbe {
    /// The pointer is over the button.
    hovered: bool,
    /// The pointer is held down on the button.
    pressed: bool,
}

/// One driven frame: its pixels, and the state the probed button reported
/// while it was being painted.
struct PressFrame {
    image: image::RgbaImage,
    state: PressProbe,
}

/// Is pixel `p` within `tol` of every channel of `color`?
///
/// The per-channel tolerance rule the root suite's `test_utils` shares;
/// copied here (with [`count_near_in`]) rather than raised into a workspace
/// crate for three small functions.
fn near(p: &image::Rgba<u8>, color: egui::Color32, tol: u8) -> bool {
    p.0[0].abs_diff(color.r()) <= tol
        && p.0[1].abs_diff(color.g()) <= tol
        && p.0[2].abs_diff(color.b()) <= tol
}

/// [`count_near`] scoped to `rect`.
///
/// The rect's edges are floored/ceiled onto whole pixels and clamped to
/// the image bounds; the counted range is the half-open
/// `[x0, x1) * [y0, y1)`.
fn count_near_in(
    image: &image::RgbaImage,
    color: egui::Color32,
    rect: egui::Rect,
    tol: u8,
) -> usize {
    let (width, height) = image.dimensions();
    let x0 = rect.min.x.floor().max(0.0) as u32;
    let y0 = rect.min.y.floor().max(0.0) as u32;
    let x1 = (rect.max.x.ceil().max(0.0) as u32).min(width);
    let y1 = (rect.max.y.ceil().max(0.0) as u32).min(height);
    image
        .enumerate_pixels()
        .filter(|(x, y, p)| *x >= x0 && *x < x1 && *y >= y0 && *y < y1 && near(p, color, tol))
        .count()
}

/// Count frame pixels within a hair of `color` inside `rect` — the idiom
/// for asking *which fill a painter chose* over a specific patch.
fn press_count_color(image: &image::RgbaImage, color: egui::Color32, rect: egui::Rect) -> usize {
    count_near_in(image, color, rect, 2)
}

/// The framework's own active fill, as the theme store publishes it: read
/// straight off the style rather than off a `palette.*` field, because the
/// claim under test is that riff's buttons and egui's stock widgets agree
/// about what a press looks like — which is only true if both read the one
/// style.
fn active_fill() -> egui::Color32 {
    theme::style_from(&theme::Palette::dark(), false)
        .visuals
        .widgets
        .active
        .bg_fill
}

/// Stand the app's style up inside the probe. Only the widget-state block is
/// installed, because `style_from` also names riff's vendored font families
/// and nothing binds those in a headless context — a harness limitation, and
/// nothing about it touches the fill a button reads.
fn use_riff_widget_style(ctx: &egui::Context) {
    let active = active_fill();
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.visuals.widgets.active.bg_fill = active;
    });
}

/// Read the probe out of the widget egui registered under `id` this frame.
fn press_probe_of(ctx: &egui::Context, id: egui::Id) -> PressProbe {
    ctx.read_response(id)
        .map_or_else(PressProbe::default, |response| PressProbe {
            hovered: response.hovered(),
            pressed: response.is_pointer_button_down_on(),
        })
}

/// The band of a button's face a pixel count is read from: an interior
/// strip along the left edge, clear of the centred glyph, of the rounded
/// corners, and of the mouse cursor `Harness::render` synthesises at the
/// pointer. A colour counted here is the fill the painter chose and nothing
/// else — the cursor is white on black and its antialiased edge walks the
/// neutral surface ramp, which is exactly what an "this fill must be
/// absent" assertion would otherwise trip over.
fn press_face_patch(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 8.0, rect.min.y + 8.0),
        egui::pos2(rect.min.x + 14.0, rect.max.y - 8.0),
    )
}

/// The strip a hover *stroke* is read from: the button's top edge, where an
/// inside stroke actually lands, held clear of the corner arcs (a stroke
/// follows the rounded outline, so a count over the corners would measure
/// the radius rather than the stroke).
fn press_stroke_band(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 14.0, rect.min.y),
        egui::pos2(rect.max.x - 14.0, rect.min.y + 2.0),
    )
}

/// The last frame's pixels. The probe paints the card plane behind the
/// button first, so every count below reads a fill and never the window.
fn render_probe(harness: &mut egui_kittest::Harness<'_, PressProbe>) -> image::RgbaImage {
    harness
        .render()
        .expect("the press probe must render headlessly")
}

/// Settle the probe, walk the pointer onto it for one hover frame, then hold
/// the button down for one press frame, returning both frames with the state
/// each reported.
fn drive_press(
    harness: &mut egui_kittest::Harness<'_, PressProbe>,
    rect: egui::Rect,
) -> (PressFrame, PressFrame) {
    harness.run();
    harness.hover_at(rect.center());
    harness.step();
    let hovered = PressFrame {
        image: render_probe(harness),
        state: *harness.state(),
    };
    // The press gets a frame of its own and no further pointer event, so
    // the frame is exactly "over the button, button held down".
    harness.drag_at(rect.center());
    harness.step();
    let pressed = PressFrame {
        image: render_probe(harness),
        state: *harness.state(),
    };
    (hovered, pressed)
}

/// One text button of `variant` over the card plane, recording the state it
/// reports. Driven through `text_button` — the real entry point — so a
/// probe cannot pass by a painter production does not take.
fn press_text_button(
    variant: Variant,
    label: &'static str,
) -> (egui_kittest::Harness<'static, PressProbe>, egui::Rect) {
    let palette = theme::Palette::dark();
    let rect = egui::Rect::from_min_size(egui::pos2(40.0, 22.0), egui::vec2(120.0, 36.0));
    let id = egui::Id::new(label);
    let mut cache = IconCache::new();
    let harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(200.0, 80.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, probe: &mut PressProbe| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                let _ = button::text_button(
                    ui,
                    &mut cache,
                    &palette,
                    false,
                    &button::TextButton {
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
                *probe = press_probe_of(ui.ctx(), id);
            },
            PressProbe::default(),
        );
    use_riff_widget_style(&harness.ctx);
    (harness, rect)
}

/// The same, for the hand-painted ghost icon button — one of the painters
/// that holds an `IconButton` and read only its `hovered` field.
fn press_icon_button() -> (egui_kittest::Harness<'static, PressProbe>, egui::Rect) {
    let palette = theme::Palette::dark();
    let rect = egui::Rect::from_min_size(egui::pos2(84.0, 24.0), egui::vec2(32.0, 32.0));
    let id = egui::Id::new("press_icon_probe");
    let mut cache = IconCache::new();
    let harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(200.0, 80.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, probe: &mut PressProbe| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                let _ = ghost_icon_button::ghost_icon_button(
                    ui,
                    &mut cache,
                    &palette,
                    rect,
                    id,
                    Icon::Trash,
                    "Delete playlist",
                    true,
                );
                *probe = press_probe_of(ui.ctx(), id);
            },
            PressProbe::default(),
        );
    use_riff_widget_style(&harness.ctx);
    (harness, rect)
}

/// A ghost text button's press is a state of its own. Idle is transparent,
/// hover is the hover fill, press is the active fill — three different
/// faces, so a press that still looked like a hover would be visible on the
/// face alone and no stroke has to be read to tell it.
///
/// The pressed face is also *complete* on that one frame. Press is instant
/// by decision (`theme::MOTION_*` is a hover-family allow-list that press
/// deliberately sits outside), so the first pressed frame is already the
/// whole active fill rather than a tween's first sample on its way there.
///
/// **Red, proven by revert:** with `pressed` unthreaded from `text_button`
/// back out of `paint_text_button`, the press frame paints the hover fill
/// and this fails with `left: 0, right: 120` on the active-fill count — the
/// state assertion above it still passes, because egui was reporting the
/// press all along and no painter was listening.
#[test]
fn test_a_pressed_text_button_paints_a_state_distinct_from_hover() {
    let palette = theme::Palette::dark();
    // The pressed fill is egui's own active fill, which the theme store
    // publishes from the palette's interact styles — so riff's buttons and
    // egui's stock widgets cannot disagree about what a press looks like.
    assert_eq!(
        active_fill(),
        palette.surface_3,
        "the framework's active fill is the palette's own pressed surface"
    );

    let (mut harness, rect) = press_text_button(Variant::Ghost, "Delete all");
    let patch = press_face_patch(rect);
    let area = (patch.width() * patch.height()) as usize;
    let (hovered, pressed) = drive_press(&mut harness, rect);

    assert_eq!(
        hovered.state,
        PressProbe {
            hovered: true,
            pressed: false
        },
        "a pointer over the button is a hover and nothing more"
    );
    assert_eq!(
        pressed.state,
        PressProbe {
            hovered: true,
            pressed: true
        },
        "a held button reports a press — the state no painter used to read"
    );
    let hover_hovered = press_count_color(&hovered.image, palette.surface_2, patch);
    let active_hovered = press_count_color(&hovered.image, palette.surface_3, patch);
    let active_pressed = press_count_color(&pressed.image, palette.surface_3, patch);
    let hover_pressed = press_count_color(&pressed.image, palette.surface_2, patch);
    assert_eq!(
        (hover_hovered, active_hovered),
        (area, 0),
        "a hovered ghost button wears the hover fill, whole"
    );
    assert_eq!(
        active_pressed, area,
        "the first pressed frame is already the whole active fill — press is \
         instant, so there is no tween sample to find part-filled"
    );
    assert_eq!(
        hover_pressed, 0,
        "press REPLACES the hover fill rather than compositing with it"
    );
}

/// A secondary button's face cannot carry the difference — its hover fill
/// *is* the active fill — so what tells a press from a hover there is the
/// hover stroke dropping away. A held button must never look both hovered
/// and pressed.
///
/// **Red, proven by revert:** with `pressed` unthreaded, a held button is
/// told only that it is hovered, so the stroke stays and this fails with
/// `left: 92, right: 0` — 92 px of hover stroke on a button that is down.
#[test]
fn test_a_press_drops_the_text_button_hover_stroke() {
    let palette = theme::Palette::dark();

    let (mut harness, rect) = press_text_button(Variant::Secondary, "Rescan now");
    let band = press_stroke_band(rect);
    let (hovered, pressed) = drive_press(&mut harness, rect);

    assert!(pressed.state.pressed, "the press is reported here too");
    let stroke_hovered = press_count_color(&hovered.image, palette.focus_ring, band);
    let stroke_pressed = press_count_color(&pressed.image, palette.focus_ring, band);
    assert!(
        stroke_hovered > 0,
        "a hovered secondary button carries the hover stroke, else the next \
         assertion would pass on a frame that painted none"
    );
    assert_eq!(
        stroke_pressed, 0,
        "a press drops the hover stroke: a held button must never look both \
         hovered and pressed"
    );
}

/// The other half of the write-only field: the painters that already hold an
/// `IconButton` now read `pressed` too. A ghost icon button has no hover
/// fill to swap out — hovering only reveals the glyph — so its press is the
/// first frame that paints a face at all, which is why the hover frame
/// counts zero.
///
/// **Red, proven by revert:** with the `button.pressed` arm removed from
/// `ghost_icon_button`, this fails on a zero pixel count — "found 0 px of
/// the active fill #28_25_21_FF in the face", because nothing paints a fill
/// behind the glyph while the button is held down.
#[test]
fn test_a_pressed_ghost_icon_button_paints_the_active_fill() {
    let active = active_fill();

    let (mut harness, rect) = press_icon_button();
    let patch = press_face_patch(rect);
    let (hovered, pressed) = drive_press(&mut harness, rect);

    assert_eq!(
        pressed.state,
        PressProbe {
            hovered: true,
            pressed: true
        },
        "the icon button reports the press it used to compute and discard"
    );
    let fill_hovered = press_count_color(&hovered.image, active, patch);
    let fill_pressed = press_count_color(&pressed.image, active, patch);
    assert_eq!(
        fill_hovered, 0,
        "hover alone paints no fill on a ghost icon button"
    );
    assert!(
        fill_pressed > 0,
        "a held ghost icon button paints the framework's active fill, found \
         {fill_pressed} px of the active fill {active:?} in the face"
    );
}
