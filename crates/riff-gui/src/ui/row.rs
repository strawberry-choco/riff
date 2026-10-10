//! The neutral row frame shared by every entity and Track row in a Library
//! Section or Drill Column.
//!
//! One owner for the row's interaction treatment — the selected/hover band,
//! the focus ring, and the accessibility registration — so those states cannot
//! drift between the sidebar tree rows ([`crate::ui::sidebar::tree_row`]) and
//! the browser rows ([`crate::ui::browser`]). Row density, leading media (the
//! favorite cell, cover tile, thumbnail), the trailing clusters, and the
//! truncate-vs-wrap strategy stay with each variant; only the shared band and
//! identity live here.
//!
//! **The band is also the app's largest tween.** It fades its hover wash in
//! over [`theme::MOTION_HOVER`] rather than swapping it, because it is the
//! surface a moving pointer crosses fastest, and this module is where the
//! tween lives. The colour it produces is not derived here: [`theme::row_band_fill`]
//! is a pure function of the tween value, which is what lets the fade be
//! asserted at its endpoints and its midpoint without a frame.

use eframe::egui;

use super::theme::{self, Palette};

/// The suffix the row's hover-wash tween is stored under in egui's animation
/// manager, so two rows cannot share one another's wash.
const HOVER_WASH: &str = "row_hover_wash";

/// The row's hover-wash tween value: `0.0` at rest, rising to `1.0` over
/// [`theme::MOTION_HOVER`] and falling back over the same window.
///
/// **The store owns the colour, this owns the clock.** The wash is
/// [`theme::row_band_fill`]'s argument and nothing here derives a colour: the
/// tween is read, and the value goes straight to the store. That split is what
/// lets the fade be asserted without a frame loop.
///
/// **Why `animate_bool_with_time` and not `animate_bool`.** The stock
/// `animate_bool` reads `Style::animation_time`, which the store publishes as
/// [`theme::MOTION_DEFAULT`] — the tempo of egui's own widget tweens. A hover
/// wash riff authors is not a popup, and the hover token is deliberately not
/// published onto the style for exactly this reason, so the duration is passed
/// explicitly here.
///
/// **No frame request of its own, and that is the finding rather than an
/// oversight.** egui 0.35's `animate_bool_with_time_and_easing` ends in
/// `if 0.0 < animated_value && animated_value < 1.0 { self.request_repaint(); }`
/// (`egui-0.35.0/src/context.rs:3145-3148`), so a hovered row is already
/// served every frame of its own tween and a settled one asks for nothing —
/// the bound at both ends included, which matters here because an unhovered row
/// rests at exactly `0.0` and a one-sided `t < 1.0` would ask for a frame on
/// behalf of every idle row in the list, forever. Adding a second request would
/// be a duplicate of the one already issued.
///
/// The frame-loop test in `tests/ui_tests.rs` keeps that finding falsifiable. It
/// samples one request per in-flight pass, attributed (via `#[track_caller]`)
/// to the call below, and none once the wash has settled — so an extra
/// `request_repaint` here shows up as a second cause on every in-flight pass.
fn hover_wash_t(ctx: &egui::Context, id: egui::Id, reduce_motion: bool, hovered: bool) -> f32 {
    ctx.animate_bool_with_time(
        id.with(HOVER_WASH),
        hovered,
        theme::hover_duration(reduce_motion),
    )
}

/// Paint one row's background band: the selected fill with the hover wash
/// blended over it, else the wash alone, and the row's own focus ring.
///
/// `band` is the full row shape the fills cover — the favorite cell included on
/// track rows, so the row reads as one continuous band. `id` is the row's own
/// widget id, which is where the wash's tween lives and what the focus ring
/// keys on, so the two states of a row are stored under one identity.
/// `selected` decides the fill the wash sits on; `hovered` is the tween's
/// target, not a paint decision; `focused` is whether the row's interactive
/// response currently holds keyboard focus (callers pass the memory-focus read,
/// per [`theme::focus_ring_stroke`]).
///
/// **The wash tween, and why the painter needed a `Ui` to do it.** The hover
/// wash fades in over [`theme::MOTION_HOVER`] rather than swapping, because
/// the band is the largest hover surface in the app and the one a moving
/// pointer crosses fastest — a change that large in area reads as flicker
/// rather than as responsiveness (the motion store's threshold rule). Reading
/// and advancing a tween both need a [`egui::Context`], so the painter takes
/// the `Ui` that every call site already holds; the colour math did not follow
/// it, which is why the fill is a store helper rather than something spelled
/// out between the painter and the token.
#[expect(
    clippy::too_many_arguments,
    clippy::fn_params_excessive_bools,
    reason = "the band, its identity and its state; reduce_motion rides the wash alongside the independent state booleans"
)]
pub fn paint_row_band(
    ui: &egui::Ui,
    painter: &egui::Painter,
    palette: &Palette,
    reduce_motion: bool,
    band: egui::Rect,
    id: egui::Id,
    selected: bool,
    hovered: bool,
    focused: bool,
) {
    let fill = theme::row_band_fill(
        palette,
        selected,
        hover_wash_t(ui.ctx(), id, reduce_motion, hovered),
    );
    // A fully transparent fill is not painted at all rather than painted at
    // zero: an idle row's band must emit no primitive, so a list of fifty
    // unhovered rows costs fifty fewer shapes than it used to and a golden
    // captured before this change still matches pixel for pixel.
    if fill != theme::TRANSPARENT {
        painter.rect_filled(band, theme::RADIUS_MD, fill);
    }
    if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
        painter.rect_stroke(band, theme::RADIUS_MD, ring, egui::StrokeKind::Inside);
    }
}

/// Register one row in the accessibility tree as a selectable label carrying
/// its identity and selection state.
///
/// `label` folds in the row's count or detail line so assistive tech reads it
/// rather than only painting it — the same "Name (detail)" shape every list
/// row speaks.
pub fn register_row_a11y(response: &egui::Response, selected: bool, label: impl Into<String>) {
    let label = label.into();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, selected, &label)
    });
}
