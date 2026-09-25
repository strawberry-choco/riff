//! The shared semantic-button mechanics.
//!
//! One owner for what every icon button does the same way: allocate the click
//! target, report its interaction state (hovered / pressed / focused /
//! disabled), paint the focus ring, register the accessible name, show the
//! tooltip, and report activation. The parts that differ between variants —
//! the hover-fill shape (rounded rect, circle, or none), the glyph tint, and
//! the brand-filled primary treatment — stay with each call site's body.
//!
//! Split into [`begin_icon_button`] (allocate + report state, before the body
//! paints) and [`finish_icon_button`] (focus ring + a11y + tooltip + result,
//! after the body paints so the ring sits on top). A variant paints its body
//! from the [`IconButton`] state between the two calls.

use eframe::egui;

use super::icons::{Icon, IconCache};
use super::theme::{self, Palette};

/// Full-texture UV rect for [`egui::Painter::image`] (sidebar precedent).
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

/// An icon button's allocated target and this frame's interaction state,
/// handed to the variant so its body can paint from `hovered` / `pressed` /
/// `active` without each call site re-deriving them.
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent two-state facts about one button"
)]
pub struct IconButton {
    /// The button's response — the click target (or a hover-only target when
    /// disabled).
    pub response: egui::Response,
    /// The full hit-area rect.
    pub rect: egui::Rect,
    /// Whether the pointer is over the button.
    pub hovered: bool,
    /// Whether the pointer is held down on the button this frame.
    pub pressed: bool,
    /// Whether the button holds keyboard focus.
    pub focused: bool,
    /// Whether the button is disabled (no click, no focus ring).
    pub disabled: bool,
}

/// Allocate an icon button's hit area and report its interaction state. A
/// disabled button takes a hover-only sense, so it can neither be clicked nor
/// take keyboard focus.
pub fn begin_icon_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    disabled: bool,
) -> IconButton {
    let sense = if disabled {
        egui::Sense::hover()
    } else {
        egui::Sense::click()
    };
    let response = ui.interact(rect, id, sense);
    let focused = !disabled && ui.memory(|m| m.has_focus(id));
    IconButton {
        hovered: response.hovered(),
        pressed: response.is_pointer_button_down_on(),
        focused,
        disabled,
        response,
        rect,
    }
}

/// Finish an icon button after its body has painted: draw the shared focus
/// ring on top, register the accessible name and tooltip, and report whether
/// the button was activated this frame (pointer click or keyboard). A disabled
/// button never reports activation.
pub fn finish_icon_button(
    ui: &egui::Ui,
    palette: &Palette,
    button: &IconButton,
    label: &str,
) -> bool {
    if let Some(ring) = theme::focus_ring_stroke(palette, button.focused) {
        ui.painter_at(button.rect).rect_stroke(
            button.rect,
            theme::RADIUS_SM,
            ring,
            egui::StrokeKind::Inside,
        );
    }
    button
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    button.response.clone().on_hover_text(label);
    !button.disabled && button.response.clicked()
}

// --- Semantic text buttons -------------------------------------------------------

/// The five semantic button roles the design system speaks. Each maps to one
/// idle paint; hover, focus and disabled states are shared mechanics layered
/// on top, so a `Primary` button and a `Destructive` button behave
/// identically under the pointer and keyboard and differ only in ink and fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Brand-filled affirmative action (Add Library, Done, Play).
    Primary,
    /// Surface-filled secondary action (Scan All, Rescan now, format chips).
    Secondary,
    /// Frameless text action that reveals a hover wash (breadcrumb crumbs).
    Ghost,
    /// Bordered neutral action (the Settings header Back control).
    Caption,
    /// Error-tinted destructive action (Clear Library).
    Destructive,
}

/// A shared semantic text button's inputs. The caller allocates `rect` (so it
/// controls hit-area and layout) and names the button once for both paint and
/// the accessibility tree.
pub struct TextButton<'a> {
    /// Stable widget id (drives focus and the a11y node).
    pub id: egui::Id,
    /// The allocated hit-area / paint rect.
    pub rect: egui::Rect,
    /// The painted label text.
    pub label: &'a str,
    /// The accessibility name (usually the label; per-path controls suffix
    /// their root so names stay unique).
    pub a11y: &'a str,
    /// Optional hover tooltip text, distinct from the accessible name (the
    /// Detail Panel's header actions show a descriptive sentence while their
    /// visible text stays the a11y name).
    pub tooltip: Option<&'a str>,
    /// Optional leading 16px glyph.
    pub icon: Option<Icon>,
    /// Use the `text-xs` button scale instead of `text-sm`.
    pub small: bool,
    /// The semantic role selecting the paint.
    pub variant: Variant,
    /// Whether the button is enabled (a disabled one takes no click and no
    /// focus).
    pub enabled: bool,
}

/// A design-scale [`egui::FontId`] at `size`, riding the family the installed
/// token style mapped onto `Button` (so weight families resolve even before
/// the vendored fonts are installed).
fn button_font(ui: &egui::Ui, size: f32) -> egui::FontId {
    let family = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .map_or(egui::FontFamily::Proportional, |font| font.family.clone());
    egui::FontId::new(size, family)
}

/// The size a [`paint_text_button`] block needs for `label`: the measured
/// galley in the same font the button paints with, plus the style's own button
/// padding, at the style's button height. For a call site that flows its
/// buttons inside an [`egui::Ui`] and must allocate the rect itself.
#[must_use]
pub fn text_button_size(ui: &egui::Ui, palette: &Palette, label: &str, small: bool) -> egui::Vec2 {
    let size = if small {
        theme::TEXT_XS
    } else {
        theme::TEXT_SM
    };
    // The ink is irrelevant to the measurement; it is the font that matters.
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), button_font(ui, size), palette.ink);
    egui::vec2(
        galley.size().x + 2.0 * ui.spacing().button_padding.x,
        ui.spacing().interact_size.y,
    )
}

/// Render one shared semantic text button and report whether it was activated
/// this frame (pointer click or keyboard Enter/Space) AND enabled. A disabled
/// button takes a hover-only sense, so it can neither be clicked nor focused.
pub fn text_button(
    ui: &egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    btn: &TextButton,
) -> bool {
    let sense = if btn.enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let response = ui.interact(btn.rect, btn.id, sense);
    let focused = btn.enabled && ui.memory(|m| m.has_focus(btn.id));
    paint_text_button(
        ui,
        cache,
        palette,
        btn.rect,
        btn.label,
        btn.icon,
        btn.small,
        btn.variant,
        btn.enabled,
        response.hovered(),
        focused,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, btn.enabled, btn.a11y));
    if let Some(tooltip) = btn.tooltip {
        response.clone().on_hover_text(tooltip);
    }
    btn.enabled && response.clicked()
}

/// The single paint authority for a semantic text button: variant fill/ink,
/// the hover stroke, the optional leading glyph, and the one focus ring on top.
/// [`text_button`] calls this; a call site that already owns an interaction
/// response (so it cannot let [`text_button`] allocate one) paints through here
/// too, keeping every text button's pixels defined in exactly one place.
#[allow(
    clippy::too_many_arguments,
    clippy::fn_params_excessive_bools,
    clippy::match_same_arms,
    reason = "the variant's paint is a pure function of these independent axes; some states \
              intentionally share a body"
)]
pub fn paint_text_button(
    ui: &egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    label: &str,
    icon: Option<Icon>,
    small: bool,
    variant: Variant,
    enabled: bool,
    hovered: bool,
    focused: bool,
) {
    let painter = ui.painter_at(rect);
    let (fill, ink, hover_stroke) = match (enabled, variant, hovered) {
        (false, _, _) => (palette.surface, palette.ink_3, false),
        (true, Variant::Primary, _) => (palette.brand_primary, palette.on_brand, false),
        (true, Variant::Secondary, false) => (palette.surface_2, palette.ink, false),
        (true, Variant::Secondary, true) => (palette.surface_3, palette.ink, true),
        (true, Variant::Caption, false) => (palette.surface, palette.ink_2, false),
        (true, Variant::Ghost, false) => (theme::TRANSPARENT, palette.ink_2, false),
        (true, Variant::Ghost | Variant::Caption, true) => (palette.surface_2, palette.ink, false),
        (true, Variant::Destructive, h) => {
            (theme::destructive_fill(palette, h), palette.error, false)
        }
    };

    painter.rect_filled(rect, theme::RADIUS_MD, fill);
    if matches!(variant, Variant::Caption) {
        painter.rect_stroke(
            rect,
            theme::RADIUS_MD,
            egui::Stroke::new(1.0_f32, palette.border),
            egui::StrokeKind::Inside,
        );
    }
    if hover_stroke {
        painter.rect_stroke(
            rect,
            theme::RADIUS_MD,
            egui::Stroke::new(1.0_f32, palette.focus_ring),
            egui::StrokeKind::Inside,
        );
    }

    let size = if small {
        theme::TEXT_XS
    } else {
        theme::TEXT_SM
    };
    let galley = painter.layout_no_wrap(label.to_owned(), button_font(ui, size), ink);
    let icon_w: f32 = if icon.is_some() { 16.0 + 8.0 } else { 0.0 };
    let mut x = rect.center().x - icon_w.midpoint(galley.size().x);
    if let Some(icon) = icon {
        let tex_id = cache.texture(ui.ctx(), icon, 16.0, ink);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(x + 8.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        );
        painter.image(tex_id, icon_rect, UV_FULL, ink);
        x += icon_w;
    }
    painter.galley(
        egui::pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );

    if let Some(ring) = theme::focus_ring_stroke(palette, focused) {
        painter.rect_stroke(rect, theme::RADIUS_MD, ring, egui::StrokeKind::Inside);
    }
}
