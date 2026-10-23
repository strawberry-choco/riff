//! Shared text-field probes: one well, one clear affordance, one focus
//! treatment for every surface that drives the primitive.
//!
//! Moved here from the root `riff-tests` crate's `browser_column_ui_tests`
//! by the widgetry split (the guard follows the source it guards): the
//! text-field primitive lives in this crate, so its probes do too. The
//! focus ring is the theme store's `search_ring_stroke`, read from
//! `riff_gui_theme` — the same token production's titlebar search hands the
//! primitive.

// The probes keep the root suite's test scaffolding shapes (long local
// fixture functions, closure-heavy harness builders) while this crate keeps
// `[lints] workspace = true`; the file-level opt-out scopes the root
// `riff-tests` crate's documented manifest-level exemption to just the
// files that need it.
#![allow(clippy::pedantic)]

use riff_gui_components::text_field::{TextField, text_field};
use riff_gui_theme::icons::{Icon, IconCache};
use riff_gui_theme::theme::{self, Palette};

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

/// Render the shared text-field seam over an owned value buffer and return
/// the frame, so a test can see the focus ring the well paints.
fn run_text_field(focused: bool) -> image::RgbaImage {
    let palette = Palette::dark();
    let id = egui::Id::new("test_text_field");
    let mut cache = IconCache::new();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(240.0, 40.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            |ui, value| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                if focused {
                    ui.memory_mut(|m| m.request_focus(id));
                }
                let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(200.0, 24.0));
                text_field(
                    ui,
                    &mut cache,
                    &palette,
                    value,
                    &TextField {
                        id,
                        rect,
                        hint: "Search or jump to…",
                        leading_icon: Some(Icon::Search),
                        clear_label: Some("Clear search"),
                        trailing: None,
                        dismiss_on_escape: true,
                    },
                );
            },
            String::new(),
        );
    harness.run();
    harness
        .render()
        .expect("the text field must render headlessly")
}

/// The shared text-field contract gives every surface the same well, clear
/// affordance and focus treatment: the clear button appears only while the
/// value is non-empty and empties it when clicked, and a focused field
/// paints the search ring while an idle one paints the hairline instead.
/// Titlebar search drives the exact same primitive end to end.
#[test]
fn test_text_field_shares_clear_affordance_and_focus_ring() {
    use egui_kittest::kittest::Queryable;

    let palette = Palette::dark();
    let id = egui::Id::new("tf_clear");

    // A non-empty field exposes a clear affordance that empties the value.
    let mut cache = IconCache::new();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(240.0, 40.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            |ui, value| {
                let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(200.0, 24.0));
                text_field(
                    ui,
                    &mut cache,
                    &palette,
                    value,
                    &TextField {
                        id,
                        rect,
                        hint: "Search or jump to…",
                        leading_icon: Some(Icon::Search),
                        clear_label: Some("Clear search"),
                        trailing: None,
                        dismiss_on_escape: true,
                    },
                );
            },
            "boards".to_string(),
        );
    harness.run();
    harness.get_by_label("Clear search").click();
    harness.run();
    assert!(
        harness.state().is_empty(),
        "the clear affordance empties the value"
    );

    // An empty field offers no clear affordance.
    let mut cache2 = IconCache::new();
    let mut idle = egui_kittest::Harness::builder()
        .with_size(egui::vec2(240.0, 40.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            |ui, value| {
                let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(200.0, 24.0));
                text_field(
                    ui,
                    &mut cache2,
                    &palette,
                    value,
                    &TextField {
                        id,
                        rect,
                        hint: "Search or jump to…",
                        leading_icon: Some(Icon::Search),
                        clear_label: Some("Clear search"),
                        trailing: None,
                        dismiss_on_escape: true,
                    },
                );
            },
            String::new(),
        );
    idle.run();
    assert!(
        idle.query_by_label("Clear search").is_none(),
        "an empty field shows no clear affordance"
    );

    // Focus drives the well's ring, not a fixed stroke.
    let ring_color = theme::search_ring_stroke(&palette, true).color;
    let focused_img = run_text_field(true);
    assert!(
        count_near(&focused_img, ring_color, 2) > 0,
        "a focused text field paints the search ring"
    );
    let idle_img = run_text_field(false);
    assert_eq!(
        count_near(&idle_img, ring_color, 2),
        0,
        "an idle text field paints no focus ring"
    );
}
