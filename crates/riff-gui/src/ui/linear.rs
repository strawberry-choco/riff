//! The shared linear (bar) control: one contract for the Playerbar seek row,
//! the Playerbar volume slider, and the Now Playing seek row.
//!
//! All three are the same shape — a 4px rounded track on `surface_3` with a
//! brand-tinted fill to a `0..=1` value, driven by pointer click/drag over a
//! taller hit area and by the keyboard when focused — so they share one owner.
//! The variants differ only in presentation and effect. The volume slider wears
//! a permanent round thumb ([`LinearControl::thumb`]); a seek bar grows one the
//! frame the pointer is over it and loses it the frame the pointer leaves
//! ([`LinearControl::hover_thumb`]), and its track thickens while it is being
//! dragged ([`active_track`]) — so the two stay distinguishable by purpose
//! instead of looking as though one of them had lost its thumb. A seek whose
//! track total is unknown or zero is non-interactive (nothing to seek against),
//! while volume is always live. The primitive returns the NEW fraction it was
//! asked to move to (a pointer position or a keyboard step), and the caller maps
//! that to its own typed intent (`Seek(Duration)`, `SetVolume`) against whatever
//! range it owns.
//!
//! Pure widget seam: paints from [`Palette`] tokens, mutates nothing, and
//! reports its change to the caller — no state, no store, no Transport.

use eframe::egui;

use super::theme::geometry::seek::{KEY_STEP, THUMB_D, TRACK_H_ACTIVE};
use super::theme::{self, Palette};

/// One linear control's inputs. `track` is the painted bar at its **idle**
/// thickness; `hit` is the (vertically taller) interactive rect that shares
/// `track`'s horizontal span.
pub struct LinearControl<'a> {
    /// Stable widget id — the focus target and interaction id.
    pub id: egui::Id,
    /// The idle-thickness bar rect; [`active_track`] grows it while grabbed.
    pub track: egui::Rect,
    /// The taller interaction rect; its `x` span equals `track`'s.
    pub hit: egui::Rect,
    /// The current fill fraction, clamped into `0..=1` when painted.
    pub value: f32,
    /// `Some(diameter)` draws a round thumb at `value` on every frame — the
    /// volume slider's, whose diameter is its own surface's token. `None` draws
    /// only the fill.
    pub thumb: Option<f32>,
    /// `true` makes this a grabbable seek: a round thumb of the *seek* surface's
    /// [`THUMB_D`] is drawn at `value` while the pointer is on the control or
    /// has hold of it, and the track is painted at [`TRACK_H_ACTIVE`] while it
    /// is being dragged. `false` leaves the control's thumbs to `thumb` alone.
    ///
    /// A second field rather than a mode on `thumb`, because the two are
    /// different claims about the control: `thumb` is a permanent statement that
    /// this control has a position indicator, while this one is the transient
    /// statement that *this* bar can be grabbed. Folding them together would
    /// leave the volume slider with a bar that grows a thumb on hover, which
    /// would make the two bars indistinguishable — as if one of them were
    /// broken rather than a different kind of control.
    ///
    /// The diameter is deliberately not passed in: it is a property of the seek
    /// surface, not of the call site, so a view names the affordance it wants and
    /// never a dimension of its own.
    pub hover_thumb: bool,
    /// Whether the control responds to pointer/keyboard. A seek with an
    /// unknown or zero duration passes `false` — it is not seekable.
    pub interactive: bool,
    /// The accessible name (`"Seek"` / `"Volume"`).
    pub label: &'a str,
}

/// Render one linear control and return the fraction a pointer click/drag or a
/// keyboard step asked to move to this frame, `None` when nothing changed or the
/// control is non-interactive. The caller maps the returned fraction onto its
/// own range and emits its own typed action.
pub fn linear_control(ui: &egui::Ui, palette: &Palette, control: &LinearControl) -> Option<f32> {
    let response = if control.interactive {
        ui.interact(control.hit, control.id, egui::Sense::click_and_drag())
    } else {
        ui.interact(control.hit, control.id, egui::Sense::hover())
    };

    // egui keeps reporting a widget as hovered for as long as it is dragged, even
    // once the pointer has left its rect, so `dragged` is named rather than
    // leaned on: the seek thumb is the grab *handle*, and a handle that blinked
    // out mid-drag would contradict a drag that is still in progress. A
    // non-interactive control is not seekable and wears no handle at all —
    // promising a drag the control cannot honour is worse than promising nothing.
    let grabbed = response.dragged();
    let pointer_on = control.interactive && (response.hovered() || grabbed);

    paint(ui, palette, control, pointer_on, grabbed);

    if !control.interactive {
        return None;
    }

    // Pointer: map the press/drag position along the track's horizontal span to
    // a clamped fraction.
    let pointer = if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        Some(fraction_at(control.track, pos))
    } else {
        None
    };

    // Keyboard: a focused control nudges by one step per arrow press.
    let keyboard = if response.has_focus() {
        let delta = ui.input(|i| {
            let mut d = 0.0_f32;
            if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp) {
                d += KEY_STEP;
            }
            if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown) {
                d -= KEY_STEP;
            }
            d
        });
        (delta.abs() > f32::EPSILON).then(|| (control.value + delta).clamp(0.0, 1.0))
    } else {
        None
    };

    response
        .widget_info(|| egui::WidgetInfo::slider(true, f64::from(control.value), control.label));

    pointer.or(keyboard)
}

/// The rect the bar is painted into: the caller's own idle-thickness `track` at
/// rest, and that same horizontal span and vertical centre grown to
/// [`TRACK_H_ACTIVE`] while the bar is grabbed.
///
/// A `pub fn` rather than a line inside [`paint`] so the geometry it decides can
/// be asserted directly, with no frame and no pointer involved. The contract it
/// exists to protect: grabbing changes the *thickness* and nothing else. The
/// span is the fraction every pointer position maps to and the centre is what a
/// pointer is already sitting on, so letting either drift would move the bar
/// under the user at the moment they can least afford to see it move.
///
/// The active thickness comes from the token, never from the caller's height, so
/// a caller that ever passes something other than
/// [`TRACK_H`](super::theme::geometry::seek::TRACK_H) still gets the same grabbed
/// bar — and a caller that does pass exactly that idle thickness gets, at rest,
/// its own rect back unchanged, which is what keeps every idle frame identical to
/// a golden's.
pub fn active_track(track: egui::Rect, grabbed: bool) -> egui::Rect {
    if !grabbed {
        return track;
    }
    egui::Rect::from_center_size(track.center(), egui::vec2(track.width(), TRACK_H_ACTIVE))
}

/// Paint the track, the fill to `value`, and whichever thumb this control wears.
/// The fill has no minimum stub, so a zero value paints zero fill.
///
/// Nothing in here is tweened, and that is the point rather than an omission: the
/// thumb is the pointer's verdict on where the control is, so it appears and
/// disappears in the same frame the pointer arrives and leaves, and it sits at
/// exactly the position being set rather than easing toward it. `theme`'s motion
/// rule ("a control's value indicator never tweens toward the pointer") is the
/// single copy of that reasoning and is cited rather than restated, so there is
/// no second version to drift. Accordingly there is no animation id and no
/// `animate_*` call anywhere near a thumb: the two pointer-driven changes here
/// are an added circle and a thicker track, both read off this frame's state.
fn paint(
    ui: &egui::Ui,
    palette: &Palette,
    control: &LinearControl,
    pointer_on: bool,
    grabbed: bool,
) {
    let painter = ui.painter_at(control.hit);
    // The caller's `track` is the idle rect; grabbed, the painted one is the same
    // span and centre at the active thickness, and everything below — the track,
    // the fill, the thumb, the focus ring — is measured against *that*, so the
    // bar thickens as one object instead of leaving a 4px sliver of track behind.
    let track = active_track(control.track, grabbed);
    let radius = track.height() / 2.0;
    let value = control.value.clamp(0.0, 1.0);
    painter.rect_filled(track, radius, palette.surface_3);
    let fill_w = track.width() * value;
    if fill_w > 0.0 {
        painter.rect_filled(
            egui::Rect::from_min_size(track.min, egui::vec2(fill_w, track.height())),
            radius,
            palette.brand_primary,
        );
    }
    // The permanent thumb, or the seek thumb once the pointer is on the control.
    let thumb = if pointer_on && control.hover_thumb {
        Some(THUMB_D)
    } else {
        control.thumb
    };
    if let Some(diameter) = thumb {
        let thumb_x = track.left() + track.width() * value;
        painter.circle_filled(
            egui::pos2(thumb_x, track.center().y),
            diameter / 2.0,
            palette.ink,
        );
    }
    // The shared keyboard-focus ring on the hit area while focused.
    if control.interactive {
        let focused = ui.memory(|m| m.has_focus(control.id));
        if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
            painter.rect_stroke(track, radius, ring, egui::StrokeKind::Inside);
        }
    }
}

/// Pointer position along `track` as a clamped `0..=1` fraction.
fn fraction_at(track: egui::Rect, pos: egui::Pos2) -> f32 {
    if track.width() <= 0.0 {
        return 0.0;
    }
    ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0)
}
