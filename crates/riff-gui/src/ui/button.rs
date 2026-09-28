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
    /// Whether the pointer is held down on the button this frame — the press
    /// feedback, and the only source of it. Every painter that takes an
    /// [`IconButton`] reads it (via [`active_fill`]); this field spent its first
    /// year computed every frame and read by nobody, which is the whole defect
    /// this note guards against.
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

/// The fill a held button wears, and the one place that decides it.
///
/// Read off the **style**, not off a `palette.*` field, and that is the whole
/// point: `theme::style_from` publishes the palette's pressed surface onto
/// `visuals.widgets.active`, which is the same slot egui's own `Button` paints
/// from. Naming the palette field here instead would re-decide "what a press
/// looks like" in this module, and the two could then drift — a stock egui
/// button in a menu beside a riff button would disagree about what being held
/// down means. The token is still the store's: this reads the value the store
/// put there.
///
/// Every painter asks here rather than naming the value, so a change to the
/// store's pressed surface reaches all of them and none of them can hold a
/// private copy. It is deliberately **not** derived or tweened: press
/// acknowledgement belongs in the same frame as the click, which is rule 4 of
/// the motion rule in [`theme`].
#[must_use]
pub fn active_fill(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().widgets.active.bg_fill
}

/// The suffix the accent tier's hover-wash tween is stored under in egui's
/// animation manager.
const ACCENT_WASH: &str = "accent_hover_wash";

/// The destructive role's, for the same reason: two roles, two washes, two
/// keys — so a button that changes role across a rebuild starts at rest rather
/// than inheriting a wash it never asked for, and neither role can read the
/// other's value.
const DESTRUCTIVE_WASH: &str = "destructive_hover_wash";

/// One wash role's tween value: `0.0` at rest, rising to `1.0` over
/// [`theme::MOTION_HOVER`] and falling back over the same window.
///
/// **The store owns the colour, this owns the clock.** The tween is read here
/// and the value goes straight to [`theme::accent_wash`] /
/// [`theme::destructive_wash`]; nothing in this module derives a colour, which
/// is what lets the fade be asserted without a frame loop.
///
/// **Why `animate_bool_with_time` and not `animate_bool`.** The stock
/// `animate_bool` reads `Style::animation_time`, which the store publishes as
/// [`theme::MOTION_DEFAULT`] — the tempo of egui's *own* widget tweens, and
/// nine passes at this suite's step where the hover token is five. The hover
/// token is deliberately not published onto the style, so the duration is
/// passed explicitly.
///
/// **No frame request of its own**, and that is the finding rather than an
/// oversight: egui 0.35's `animate_bool_with_time_and_easing` ends in `if 0.0
/// < animated_value && animated_value < 1.0 { self.request_repaint(); }`
/// (`egui-0.35.0/src/context.rs:3145-3148`), so a fading wash is already served
/// every frame of its own tween and a settled one asks for nothing — the bound
/// at both ends included, which matters here because an unhovered button rests
/// at exactly `0.0` and a one-sided `t < 1.0` would ask for a frame on behalf
/// of every button in the app, forever. Adding a second request would be a
/// duplicate of the one already issued.
///
/// **Press does not participate, and the tween is not retargeted for it.** The
/// wash keeps aiming at `hovered` while the button is down and simply stops
/// being *painted* (see the paint below). Retargeting at `hovered && !pressed`
/// would be tidier and wrong: it would tween the wash out over the hover token
/// during a press, which is a tween on press feedback, and rule 4 of the motion
/// rule puts press outside the allow-list entirely.
fn wash_tween(ui: &egui::Ui, id: egui::Id, suffix: &str, hovered: bool) -> f32 {
    ui.ctx()
        .animate_bool_with_time(id.with(suffix), hovered, theme::MOTION_HOVER)
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
    /// Neutral-filled action that takes a translucent **brand** wash on hover:
    /// the mockup's second tier, which is tinted rather than neutral, and the
    /// same gesture as [`Variant::Destructive`] in the brand hue instead of the
    /// error hue. Distinct from [`Variant::Secondary`] because not every
    /// secondary action is tinted — the neutral ones say so by being Secondary.
    Accent,
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

/// Paint the primary action's face: the accent bloom behind it, then the
/// lit-from-above brand gradient over it. **The one authority for that paint** —
/// [`Variant::Primary`] and the player bar's play FAB both come through here, so
/// a primary action cannot come to read differently from its neighbour because
/// one of them rolled its own. The inspector is no longer one of its callers:
/// that panel is a readout, its action row is retired, and the only actions it
/// still surfaces are the inline editor's own Save and Cancel — so if a third
/// call site appears, it must route through here for the same reason.
///
/// `radius` is the shape's corner radius ([`theme::RADIUS_MD`] for a text
/// button, half the width for a circle). Paint-only: both passes are bounded by
/// `rect`, so no hit target moves, and the bloom deliberately reaches past it.
pub fn paint_primary_face(ui: &egui::Ui, palette: &Palette, rect: egui::Rect, radius: f32) {
    theme::paint_accent_glow(ui.painter(), palette, rect);
    theme::paint_gradient_shape(
        &ui.painter_at(rect),
        rect,
        radius,
        theme::brand_gradient_stops(),
    );
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
        btn.id,
        btn.rect,
        btn.label,
        btn.icon,
        btn.small,
        btn.variant,
        btn.enabled,
        response.hovered(),
        response.is_pointer_button_down_on(),
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
/// the pressed fill, the hover stroke, the optional leading glyph, and the one
/// focus ring on top. [`text_button`] calls this; a call site that already owns
/// an interaction response (so it cannot let [`text_button`] allocate one)
/// paints through here too, keeping every text button's pixels defined in
/// exactly one place.
///
/// `hovered` and `pressed` are the two facts the caller reads off its response
/// (`Response::hovered` and `Response::is_pointer_button_down_on`) and hands
/// over rather than re-deriving: a press state this module computed for itself
/// would be a second opinion about the same pointer.
///
/// `id` is the button's own widget id, and it is here because the two wash
/// roles keep their tween under it: a tween needs somewhere to live between
/// frames, and the button's identity is the only key that is already unique,
/// already stable, and already what a reader of the animation manager would
/// expect to find.
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
    id: egui::Id,
    rect: egui::Rect,
    label: &str,
    icon: Option<Icon>,
    small: bool,
    variant: Variant,
    enabled: bool,
    hovered: bool,
    pressed: bool,
    focused: bool,
) {
    // **Only these two roles tween, and the shape of this statement is the
    // guarantee.** A tween is not free: it allocates an animation id in egui's
    // manager and asks the frame loop for a pass on every frame it is in
    // flight. A variant that tweened for tidiness would pay that on every
    // button on screen, so the allow-list is not a comment on the way in — it
    // is a `match` on the variant with one arm per allowed role and a
    // fallthrough for everything else. The fallthrough returns a literal and
    // never reaches `animate_bool_with_time`, so a `Secondary` button does not
    // so much as reserve an id. `test_only_the_accent_and_destructive_washes_
    // tween` in `tests/ui_tests.rs` pins that from the outside, by repaint
    // cause.
    //
    // The threshold that selects these two and not the others is the motion
    // rule's, recorded beside the tokens: both washes run from fully
    // transparent to a low-alpha tint, so the step is large in alpha, and they
    // sit on the two affordances where a missed click costs most.
    let wash_t = match variant {
        Variant::Accent => wash_tween(ui, id, ACCENT_WASH, hovered),
        Variant::Destructive => wash_tween(ui, id, DESTRUCTIVE_WASH, hovered),
        _ => 0.0,
    };
    let painter = ui.painter_at(rect);
    let (fill, ink, hover_stroke) = match (enabled, variant, hovered) {
        (false, _, _) => (palette.surface, palette.ink_3, false),
        // **Press replaces hover, it does not compose with it.** A held button
        // is a different state, not a stronger version of the same one, and a
        // face carrying both treatments at once reads as a glitch rather than
        // as a press. So the press arm sits above every variant arm and no
        // variant arm below it is ever reached while the pointer is down: the
        // fill becomes the framework's active fill and the hover stroke goes
        // away with the hover fill, rather than sitting on top of it.
        //
        // Deliberately instant — no tween, no duration token, no frame request
        // of its own. A press tween delays the acknowledgement of a gesture the
        // user has already made, and at the app's frame cadence a tween shorter
        // than one frame renders as a flicker rather than as a press. That is
        // rule 4 of the motion rule in [`theme`], and this is the surface it is
        // written for.
        //
        // `Primary` is the one variant that keeps its own face here: its paint
        // is the brand gradient (below), not this `fill`, and its hover is
        // likewise unchanged — so a held primary is indistinguishable from a
        // hovered one, which is consistent rather than an oversight.
        (true, _, _) if pressed => (active_fill(ui), palette.ink, false),
        (true, Variant::Primary, _) => (palette.brand_primary, palette.on_brand, false),
        (true, Variant::Secondary, false) => (palette.surface_2, palette.ink, false),
        (true, Variant::Secondary, true) => (palette.surface_3, palette.ink, true),
        (true, Variant::Accent, _) => (palette.surface_2, palette.ink, hovered),
        (true, Variant::Caption, false) => (palette.surface, palette.ink_2, false),
        (true, Variant::Ghost, false) => (theme::TRANSPARENT, palette.ink_2, false),
        (true, Variant::Ghost | Variant::Caption, true) => (palette.surface_2, palette.ink, false),
        (true, Variant::Destructive, _) => (
            theme::destructive_wash(palette, wash_t),
            palette.error,
            false,
        ),
    };

    if enabled && variant == Variant::Primary {
        // The mockup's primary action is the design's one flourish: a face lit
        // from above and a soft accent bloom behind it, painted by the one
        // authority every primary shares.
        paint_primary_face(ui, palette, rect, theme::RADIUS_MD);
    } else {
        painter.rect_filled(rect, theme::RADIUS_MD, fill);
    }
    // The accent tier's hover: the neutral base stays and a translucent brand
    // wash goes over it, which is what the mockup's `hover:bg-primary/10` does.
    // Composited over the base rather than replacing it — so the resting fill is
    // untouched and an idle button is byte-identical — and the same gesture
    // `destructive_fill` makes one role down, in the brand hue. A press skips
    // it, for the same reason it skips the hover stroke: the accent wash *is*
    // this variant's hover treatment, and a press has already replaced it.
    //
    // The wash is gated on `wash_t > 0.0` rather than on `hovered`, which is
    // what makes the fade's two endpoints land: at rest the tween rests at
    // exactly `0.0` and nothing is painted, and a settled hover reaches `1.0`
    // and paints the wash the instant swap painted. A *fully transparent* fill
    // is not painted at all rather than painted at zero, the same call the row
    // band makes and for the same reason — an idle button on a screen of them
    // should not emit a primitive per button for a colour nobody can see.
    if enabled && variant == Variant::Accent && !pressed && wash_t > 0.0 {
        painter.rect_filled(rect, theme::RADIUS_MD, theme::accent_wash(palette, wash_t));
    }
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
