//! Ghost icon button probe: the hover-revealed glyph still registers its
//! accessibility label every frame, and hovering shows its tooltip.
//!
//! Moved here from the root `riff-tests` crate by the widgetry split (the
//! guard follows the source it guards): the ghost icon button primitive
//! lives in this crate, so its probe does too.

// The probe keeps the root suite's test scaffolding shapes (closure-heavy
// harness builders) while this crate keeps `[lints] workspace = true`; the
// file-level opt-out scopes the root `riff-tests` crate's documented
// manifest-level exemption to just the files that need it.
#![allow(clippy::pedantic)]

use riff_gui_components::ghost_icon_button;
use riff_gui_theme::icons::{Icon, IconCache};
use riff_gui_theme::theme;

/// Zero out the tooltip hover delay so headless hovers show tooltips on
/// the very next frame instead of after the interactive grace period.
fn make_tooltips_instant(ctx: &egui::Context) {
    ctx.global_style_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
}

/// Hover-reveal hides the glyph until hovered, but the hit target (and its
/// accessibility label) is registered every frame — and hovering it shows
/// the tooltip the label carries.
#[test]
fn test_ghost_icon_buttons_show_tooltips_on_hover() {
    use egui_kittest::kittest::Queryable;

    let palette = theme::Palette::dark();
    let mut cache = IconCache::new();
    let btn_id = egui::Id::new("tooltip_probe");
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(64.0, theme::geometry::sidebar::ROW_H))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, opened: &mut Vec<bool>| {
                make_tooltips_instant(ui.ctx());
                let rect =
                    egui::Rect::from_center_size(ui.max_rect().center(), egui::vec2(24.0, 24.0));
                let _ = ghost_icon_button::ghost_icon_button(
                    ui,
                    &mut cache,
                    &palette,
                    rect,
                    btn_id,
                    Icon::Trash,
                    "Delete playlist",
                    true,
                );
                if ui
                    .ctx()
                    .read_response(btn_id)
                    .is_some_and(|r| r.is_tooltip_open())
                {
                    opened.push(true);
                }
            },
            Vec::new(),
        );
    harness.run();
    assert!(harness.state().is_empty(), "no tooltip before hovering");

    // Hover-reveal hides the glyph until hovered, but the hit target (and
    // its accessibility label) is registered every frame.
    harness.get_by_label("Delete playlist").hover();
    harness.run();
    assert!(
        !harness.state().is_empty(),
        "hovering a ghost icon button shows its tooltip"
    );
}
