//! The shared empty-state composition.
//!
//! One owner for the labelled "nothing here yet, and here is the one thing to
//! do" block that every listing surface draws instead of an unexplained hole:
//! Sections, Drill Columns, the Detail Panel, Playlists, and the Playback
//! Queue. Two entry points share one copy-and-type treatment so the surfaces
//! agree on ink and hierarchy while each keeps its own layout model:
//! [`empty_state`] flows into an [`egui::Ui`] (the explorer columns), and
//! [`empty_state_in_rect`] paints into a caller-owned rect via a [`Painter`]
//! (the hand-laid queue panel and Now Playing list).
//!
//! Surface-appropriate copy stays at each call site — this owner fixes only
//! *how* a labelled empty state looks, never *what* it says. Pure paint, reads
//! its colors from [`Palette`] tokens (ADR 0004).

use eframe::egui;

use super::theme::Palette;

/// The friendly empty state in a flowing [`egui::Ui`]: what the section is plus
/// the one hint that moves the listener forward — never a raw error.
pub fn empty_state(ui: &mut egui::Ui, palette: &Palette, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(
            egui::RichText::new(title)
                .text_style(egui::TextStyle::Heading)
                .color(palette.ink_2),
        );
        ui.label(
            egui::RichText::new(hint)
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
        );
    });
}

/// The same labelled composition painted into a caller-owned `rect` (centered):
/// for the surfaces that hand-position their content — the Playback Queue panel
/// and the Now Playing Up Next list — where a flowing `Ui` is not the layout
/// model. The whole title/hint block is centered in `rect`; when `hint` is
/// empty only the title line paints.
pub fn empty_state_in_rect(
    painter: &egui::Painter,
    palette: &Palette,
    rect: egui::Rect,
    title: &str,
    hint: &str,
) {
    let heading_font = egui::FontId::proportional(super::theme::TEXT_XL);
    let small_font = egui::FontId::proportional(super::theme::TEXT_XS);
    let title_galley = painter.layout_no_wrap(title.to_owned(), heading_font, palette.ink_2);
    let hint_galley = if hint.is_empty() {
        None
    } else {
        Some(painter.layout_no_wrap(hint.to_owned(), small_font, palette.ink_3))
    };

    let hint_h = hint_galley.as_ref().map_or(0.0, |h| 6.0 + h.size().y);
    let block_h = title_galley.size().y + hint_h;
    let top = rect.center().y - block_h / 2.0;

    painter.galley(
        egui::pos2(rect.center().x - title_galley.size().x / 2.0, top),
        title_galley.clone(),
        palette.ink_2,
    );
    if let Some(hint_galley) = hint_galley {
        painter.galley(
            egui::pos2(
                rect.center().x - hint_galley.size().x / 2.0,
                top + title_galley.size().y + 6.0,
            ),
            hint_galley,
            palette.ink_3,
        );
    }
}
