//! The shared linear (bar) control: one contract for the Playerbar seek row,
//! the Playerbar volume slider, and the Now Playing seek row.
//!
//! All three are the same shape — a 4px rounded track on `surface_3` with a
//! brand-tinted fill to a `0..=1` value, driven by pointer click/drag over a
//! taller hit area and by the keyboard when focused — so they share one owner.
//! The variants differ only in presentation and effect: the volume slider draws
//! a round thumb, the seek bars do not; a seek whose track total is unknown or
//! zero is non-interactive (nothing to seek against), while volume is always
//! live. The primitive returns the NEW fraction it was asked to move to (a
//! pointer position or a keyboard step), and the caller maps that to its own
//! typed intent (`Seek(Duration)`, `SetVolume`) against whatever range it owns.
//!
//! Pure widget seam: paints from [`Palette`] tokens, mutates nothing, and
//! reports its change to the caller — no state, no store, no Transport.

use eframe::egui;

use super::theme::geometry::seek::{KEY_STEP, TRACK_H};
use super::theme::{self, Palette};

/// One linear control's inputs. `track` is the painted bar; `hit` is the
/// (vertically taller) interactive rect that shares `track`'s horizontal span.
pub struct LinearControl<'a> {
    /// Stable widget id — the focus target and interaction id.
    pub id: egui::Id,
    /// The 4px painted bar rect.
    pub track: egui::Rect,
    /// The taller interaction rect; its `x` span equals `track`'s.
    pub hit: egui::Rect,
    /// The current fill fraction, clamped into `0..=1` when painted.
    pub value: f32,
    /// `Some(diameter)` draws a round thumb at `value` (the volume slider);
    /// `None` draws only the fill (a seek bar).
    pub thumb: Option<f32>,
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

    paint(ui, palette, control);

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

/// Paint the track, the fill to `value`, and the optional thumb. The fill has no
/// minimum stub, so a zero value paints zero fill.
fn paint(ui: &egui::Ui, palette: &Palette, control: &LinearControl) {
    let painter = ui.painter_at(control.hit);
    let radius = TRACK_H / 2.0;
    let value = control.value.clamp(0.0, 1.0);
    painter.rect_filled(control.track, radius, palette.surface_3);
    let fill_w = control.track.width() * value;
    if fill_w > 0.0 {
        painter.rect_filled(
            egui::Rect::from_min_size(control.track.min, egui::vec2(fill_w, TRACK_H)),
            radius,
            palette.brand_primary,
        );
    }
    if let Some(diameter) = control.thumb {
        let thumb_x = control.track.left() + control.track.width() * value;
        painter.circle_filled(
            egui::pos2(thumb_x, control.track.center().y),
            diameter / 2.0,
            palette.ink,
        );
    }
    // The shared keyboard-focus ring on the hit area while focused.
    if control.interactive {
        let focused = ui.memory(|m| m.has_focus(control.id));
        if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
            painter.rect_stroke(control.track, radius, ring, egui::StrokeKind::Inside);
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
