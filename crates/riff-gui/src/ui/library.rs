//! The Library stage widgets (Issue 09).
//!
//! The mockup's Library Explorer main stage is an empty-state hero: a 160px
//! disc circle carrying an 80px disc glyph, wrapped in an amber glow, above a
//! semibold title and a muted subtitle — all centered in the stage.
//!
//! The glow stands in for the design's `.riff-disc-glow` box-shadow
//! (`0 0 60px -20px brand@15%`): egui cannot blur, so
//! [`theme::geometry::glow::LAYERS`] stacks concentric translucent brand
//! fills whose alphas fall off with distance, painted largest-first so they
//! read as one soft halo.
//!
//! Pure widget seam like `sidebar`: paints from [`Palette`] tokens (ADR 0004),
//! mutates nothing, and renders headlessly in `tests/ui_tests.rs` /
//! `tests/golden_tests.rs`.

use eframe::egui;

use super::icons::{Icon, IconCache};
use super::theme::geometry::{glow, hero};
use super::theme::{self, Palette};

// --- Mockup copy -----------------------------------------------------------------

/// Hero title, verbatim from the mockup's index.html stage (`text-xl`
/// semibold ink).
pub const HERO_TITLE: &str = "Select a track to view details";

/// Hero subtitle, verbatim from the mockup's index.html stage (`text-sm`
/// secondary ink).
pub const HERO_SUBTITLE: &str = "Your library is ready. Choose something from the sidebar.";

/// Full-texture UV rect for [`egui::Painter::image`].
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

/// Draw the empty-state hero into the Library stage: the glowing disc circle,
/// title, and subtitle, centered horizontally and vertically in the available
/// space with the mockup's `p-8` inset.
///
/// Text renders through the installed style's Heading/Body entries (resolved
/// with proportional fallbacks, like the sidebar's segmented control) so the
/// hero picks up Inter Semibold `text-xl` / regular `text-sm` wherever the
/// token style is active.
pub fn empty_state_hero(ui: &mut egui::Ui, cache: &mut IconCache, palette: &Palette) {
    // Resolve the design type scale through the CURRENT style: the golden
    // harness renders one eager frame before fonts are installed, and naming
    // a weight family directly would panic there (see `segmented_nav`).
    let heading_font = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Heading)
        .cloned()
        .unwrap_or_else(|| egui::FontId::new(theme::TEXT_XL, egui::FontFamily::Proportional));
    let body_font = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .cloned()
        .unwrap_or_else(|| egui::FontId::new(theme::TEXT_SM, egui::FontFamily::Proportional));

    // Measure the copy block first so the whole group centers as one unit.
    let painter = ui.painter();
    let title_galley = painter.layout_no_wrap(HERO_TITLE.to_owned(), heading_font, palette.ink);
    let sub_galley = painter.layout_no_wrap(HERO_SUBTITLE.to_owned(), body_font, palette.ink_2);
    let title_h = title_galley.size().y;
    let sub_h = sub_galley.size().y;
    let total_h = hero::DISC_SIZE + hero::TITLE_GAP + title_h + hero::SUBTITLE_GAP + sub_h;

    let area = ui.max_rect().shrink(hero::STAGE_INSET);
    let center = area.center();
    let group_top = center.y - total_h / 2.0;
    let disc_center = egui::pos2(center.x, group_top + hero::DISC_SIZE / 2.0);
    let disc_radius = hero::DISC_SIZE / 2.0;

    // Amber glow: layered translucent fills standing in for the CSS blur,
    // painted largest-first so the steps stack into a halo.
    for layer in &glow::LAYERS {
        painter.circle_filled(
            disc_center,
            disc_radius + layer.spread,
            theme::glow(palette, layer.alpha),
        );
    }

    // Disc circle: surface fill inside a hairline border, fully rounded.
    let disc_rect =
        egui::Rect::from_center_size(disc_center, egui::vec2(hero::DISC_SIZE, hero::DISC_SIZE));
    painter.rect_filled(disc_rect, disc_radius, palette.surface);
    painter.rect_stroke(
        disc_rect,
        disc_radius,
        egui::Stroke::new(1.0_f32, palette.border),
        egui::StrokeKind::Inside,
    );

    // Disc glyph at the mockup's muted-foreground/40 strength. The texture is
    // rasterized with the tint baked in, so it is drawn at its own colors
    // ([`theme::TEXTURE_TINT`]).
    let glyph_tint = theme::hero_glyph(palette);
    let tex_id = cache.texture(ui.ctx(), Icon::Disc, hero::DISC_ICON_SIZE, glyph_tint);
    let icon_rect = egui::Rect::from_center_size(
        disc_center,
        egui::vec2(hero::DISC_ICON_SIZE, hero::DISC_ICON_SIZE),
    );
    painter.image(tex_id, icon_rect, UV_FULL, theme::TEXTURE_TINT);

    // Copy block under the circle, each line centered on the stage axis.
    let title_top = group_top + hero::DISC_SIZE + hero::TITLE_GAP;
    painter.galley(
        egui::pos2(center.x - title_galley.size().x / 2.0, title_top),
        title_galley,
        palette.ink,
    );
    let sub_top = title_top + title_h + hero::SUBTITLE_GAP;
    painter.galley(
        egui::pos2(center.x - sub_galley.size().x / 2.0, sub_top),
        sub_galley,
        palette.ink_2,
    );
}
