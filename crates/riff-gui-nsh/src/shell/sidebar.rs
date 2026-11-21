//! The sidebar skeleton: the egui nav panel's sections (`ui/app.rs`'s
//! `render_library_sidebar`) with the wording `features.md`'s Library
//! Explorer Panel entry gives — LIBRARY, SMART LISTS, PLAYLISTS — and the
//! footer's Add-folder action and "Last scan" stamp. The rows are the
//! skeleton's placeholders: no counts and no selection writes until N5
//! wires them, so they carry test ids but take no presses.

use nsh_core::prelude::*;
use riff_backend::domain::SmartPlaylistKind;
use std::time::SystemTime;

use crate::theme::Palette;

use crate::bridge::state::UiState;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// The LIBRARY rows, in `features.md`'s order.
const LIBRARY_ROWS: [(&str, Icon); 5] = [
    ("All Tracks", Icon::ListMusic),
    ("Artists", Icon::Music),
    ("Albums", Icon::Disc),
    ("Genres", Icon::LayoutGrid),
    ("Folders", Icon::Folder),
];

/// The panel: sections, the fold of the smart lists, and the footer pinned
/// at the bottom by a spacer that takes the leftover height.
pub fn panel(state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .w(px(tokens::SIDEBAR_W))
        .flex_col()
        .test_id("sidebar")
        .bg(move || palette.get().surface)
        .border_r(px(1.0))
        .border_color(move || palette.get().line)
        .p(px(tokens::SPACE_LG))
        .gap(px(tokens::SPACE_LG))
        .child(header(palette, "LIBRARY"))
        .children(
            LIBRARY_ROWS
                .iter()
                .map(|(label, glyph)| row(palette, label, *glyph)),
        )
        .child(header(palette, "SMART LISTS"))
        .child(smart_lists(state))
        .child(header(palette, "PLAYLISTS"))
        .child(view().flex_1())
        .child(footer(state))
}

/// A section header: the small caps label the egui sidebar led its
/// sections with.
fn header(palette: Signal<Palette>, label: &'static str) -> impl IntoElement {
    text(label)
        .text_xs()
        .text_color(move || palette.get().ink_3)
        .test_id("sidebar-section")
}

/// A smart lists block: the six Built-ins' rows in display order, folded
/// when the persisted flag says so. The flag rides the settings snapshot
/// signal, so a later collapse toggle re-renders this without any wiring
/// here.
fn smart_lists(state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let expanded = move || !state.settings.get().flags.smart_lists_collapsed;
    when(expanded, move || {
        view().flex_col().gap(px(tokens::SPACE_XS)).children(
            SmartPlaylistKind::ALL
                .iter()
                .map(|kind| row(palette, kind.display_name(), Icon::Sparkles)),
        )
    })
}

/// A nav row: the skeleton's placeholder. Hover reads through the palette
/// wash the egui rows painted; the press and the count arrive with N5.
fn row(palette: Signal<Palette>, label: &'static str, glyph: Icon) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .py(px(tokens::SPACE_XS))
        .px(px(tokens::SPACE_SM))
        .rounded(px(tokens::RADIUS_SM))
        .hover(|s| s.bg(move || palette.get().row_hover))
        .test_id(&nav_id(label))
        .child(icon(glyph, Icon::ICON_SIZE, move || palette.get().ink_2))
        .child(
            text(label)
                .text_sm()
                .text_color(move || palette.get().ink_2),
        )
}

/// The footer: the Add-folder affordance (inert — the picker is N14's
/// platform integration) and the last full scan's stamp, the two facts the
/// egui footer carried.
fn footer(state: UiState) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_col()
        .gap(px(tokens::SPACE_SM))
        .test_id("sidebar-footer")
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_SM))
                .p(px(tokens::SPACE_SM))
                .rounded(px(tokens::RADIUS_SM))
                .border(px(1.0))
                .border_color(move || palette.get().border)
                .test_id("add-folder")
                .label("Add folder")
                .child(icon(Icon::Plus, Icon::ICON_SIZE, move || {
                    palette.get().ink_2
                }))
                .child(
                    text("Add folder")
                        .text_sm()
                        .text_color(move || palette.get().ink_2),
                ),
        )
        .child(
            text(move || last_scan_line(state.sidebar.get().last_scan))
                .text_xs()
                .text_color(move || palette.get().ink_3)
                .test_id("last-scan"),
        )
}

/// The "Last scan X ago" stamp, `features.md`'s footer wording: never
/// scanned reads as never, a fresh stamp as just now.
pub fn last_scan_line(last_scan: Option<SystemTime>) -> String {
    let Some(at) = last_scan else {
        return "Last scan never".to_owned();
    };
    match at.elapsed() {
        Ok(ago) if ago.as_secs() < 60 => "Last scan just now".to_owned(),
        Ok(ago) if ago.as_secs() < 3600 => format!("Last scan {}m ago", ago.as_secs() / 60),
        Ok(ago) if ago.as_secs() < 86400 => format!("Last scan {}h ago", ago.as_secs() / 3600),
        Ok(ago) => format!("Last scan {}d ago", ago.as_secs() / 86400),
        Err(_) => "Last scan never".to_owned(),
    }
}

/// A row's test id, derived from its label so the sweep of row ids stays
/// one place: "All Tracks" rows as `nav-all-tracks`.
fn nav_id(label: &str) -> String {
    format!("nav-{}", label.to_lowercase().replace(' ', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_stamp_reads_ago_words_and_never() {
        assert_eq!(last_scan_line(None), "Last scan never");
        assert_eq!(
            last_scan_line(Some(SystemTime::now() - Duration::from_secs(90))),
            "Last scan 1m ago"
        );
    }

    #[test]
    fn row_ids_slug_their_labels() {
        assert_eq!(nav_id("All Tracks"), "nav-all-tracks");
    }
}
