//! The library's content top bar (design-handoff issue 06).
//!
//! A second content strip — distinct from the frameless window chrome —
//! above the library stage, carrying the orange riff wordmark, the global
//! "Search or jump to…" field, and the list/grid view toggles. The search
//! field edits the caller's query buffer directly (the same
//! `library.search_query` the sidebar search binds to, so the whole library
//! filters while typing); the toggles report [`TopBarAction`]s the caller
//! applies to the persisted session state, and the browser column reads the
//! resulting layout (issue 08).
//!
//! Headless seams (tested in `tests/ui_tests.rs`): the toggle action
//! contract and the search field's query-buffer editing. The pixels are
//! covered by the `top_bar_dark` golden (`tests/golden_tests.rs`).

use super::fonts;
use super::icons::{Icon, IconCache, icon_button};
use super::sidebar::{SEARCH_H, ghost_icon_button, search_ring_stroke};
use super::theme::{self, Palette};
use eframe::egui;
use riff_backend::app::state::BrowserLayout;

/// Everything the top bar needs to render one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TopBarContent {
    /// The browser column layout the toggles reflect; the active layout
    /// carries the brand tint (issue 08 reads the persisted state itself).
    pub layout: BrowserLayout,
}

/// What the user did to the top bar this frame. The app applies these to the
/// library session and the settings store so the choice persists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopBarAction {
    /// Switch the browser column to this render mode.
    SetLayout(BrowserLayout),
}

/// Upper bound on the search field's width so it stays a field, not a
/// second window; it shrinks with the window before the toggles move.
const SEARCH_MAX_W: f32 = 520.0;
/// Right inset of the toggle cluster and left inset of the wordmark.
const EDGE_INSET: f32 = 12.0;
/// Gap between the wordmark and the search field.
const WORDMARK_GAP: f32 = 16.0;
/// Reserved width for the two view-toggle icon buttons plus their gaps.
const TOGGLES_W: f32 = 76.0;
/// The static normalized bar heights of the wordmark's equalizer glyph — a
/// fixed brand mark, not the playing indicator's animation.
const WORDMARK_BARS: [f32; 4] = [0.55, 0.95, 0.7, 0.4];

/// Draw the content top bar inside its panel: wordmark, global search field,
/// and the list/grid view toggles. Runs inside a top panel of exactly
/// [`crate::ui::theme::TOPBAR_H`] height.
///
/// The search field edits `query` in place — pass the session's
/// `search_query` so typing filters the library immediately. Returns the
/// field's response so the caller can keep driving the Ctrl+K
/// request-focus shortcut. Observed [`TopBarAction`]s append to `actions`.
pub fn show_top_bar(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    query: &mut String,
    content: TopBarContent,
    actions: &mut Vec<TopBarAction>,
) -> egui::Response {
    let rect = ui.max_rect();

    // Wordmark: the orange equalizer glyph plus "riff" in Inter Bold, both in
    // the reconciled brand accent (issue 01 tokens). The text is measured so
    // the search field starts clear of it instead of covering it.
    let mark_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + EDGE_INSET + 9.0, rect.center().y),
        egui::vec2(18.0, 20.0),
    );
    paint_equalizer_mark(&ui.painter_at(mark_rect), mark_rect, palette.brand_primary);
    let wordmark_right = paint_wordmark_text(ui, mark_rect, rect.center().y, palette);

    // View toggles at the right edge — their geometry is fixed, but the
    // widgets are created AFTER the search field so Tab order follows the
    // visual left-to-right order (handoff issue 16).
    let toggles_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - EDGE_INSET - TOGGLES_W, rect.min.y),
        egui::pos2(rect.right() - EDGE_INSET, rect.max.y),
    );

    // Global search field: the "Search or jump to…" well between the
    // wordmark and the toggles. Keyboard/screen-reader parity with the
    // sidebar search — focus ring border, clear affordance, real text field.
    let search_left = wordmark_right + WORDMARK_GAP;
    let search_right = toggles_rect.left() - WORDMARK_GAP;
    let search_w = (search_right - search_left).min(SEARCH_MAX_W);
    let search_rect = egui::Rect::from_min_size(
        egui::pos2(search_left, rect.center().y - SEARCH_H / 2.0),
        egui::vec2(search_w, SEARCH_H),
    );

    let id = egui::Id::new("riff_global_search");
    // Read focus BEFORE painting so the ring lands on the same frame the
    // field gains focus (sidebar precedent).
    let focused = ui.memory(|m| m.has_focus(id));
    paint_search_well(ui, palette, search_rect, focused);

    let response = show_search_field(ui, cache, palette, query, search_rect, id);

    handle_search_dismiss(ui, id, focused, query);

    show_view_toggles(ui, cache, palette, content, toggles_rect, actions);

    response
}

/// The wordmark's "riff" text, painted at the equalizer glyph's right edge.
/// Returns the text's right edge so the search field can start clear of it.
fn paint_wordmark_text(
    ui: &mut egui::Ui,
    mark_rect: egui::Rect,
    center_y: f32,
    palette: &Palette,
) -> f32 {
    let wordmark = ui.painter().layout_no_wrap(
        "riff".to_owned(),
        wordmark_font(ui.ctx()),
        palette.brand_primary,
    );
    let pos = egui::pos2(
        mark_rect.right() + WORDMARK_GAP,
        center_y - wordmark.size().y / 2.0,
    );
    ui.painter()
        .galley(pos, wordmark.clone(), palette.brand_primary);
    pos.x + wordmark.size().x
}

/// The list/grid toggle cluster at the right edge: the active layout carries
/// the brand tint; clicks report [`TopBarAction::SetLayout`] through
/// `actions`.
fn show_view_toggles(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: TopBarContent,
    toggles_rect: egui::Rect,
    actions: &mut Vec<TopBarAction>,
) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(toggles_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.visuals_mut().button_frame = false;

            let list_tint = toggle_tint(palette, BrowserLayout::List, content.layout);
            if icon_button(ui, cache, Icon::List, "List view", 16.0, list_tint)
                .on_hover_text("Show the browser as a list")
                .clicked()
            {
                actions.push(TopBarAction::SetLayout(BrowserLayout::List));
            }

            let grid_tint = toggle_tint(palette, BrowserLayout::Grid, content.layout);
            if icon_button(ui, cache, Icon::LayoutGrid, "Grid view", 16.0, grid_tint)
                .on_hover_text("Show the browser as a grid")
                .clicked()
            {
                actions.push(TopBarAction::SetLayout(BrowserLayout::Grid));
            }
        },
    );
}

/// Tint for a view toggle: the brand accent while its layout is active,
/// muted ink otherwise.
fn toggle_tint(palette: &Palette, own: BrowserLayout, active: BrowserLayout) -> egui::Color32 {
    if own == active {
        palette.brand_primary
    } else {
        palette.ink_2
    }
}

/// The rounded input well behind the search field: surface-2 fill with the
/// sidebar search's ring border — hairline when idle, focus ring when the
/// field has keyboard focus.
fn paint_search_well(ui: &egui::Ui, palette: &Palette, rect: egui::Rect, focused: bool) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_2);
    painter.rect_stroke(
        rect,
        theme::RADIUS_MD,
        search_ring_stroke(palette, focused),
        egui::StrokeKind::Inside,
    );
}

/// The field inside the search well: search glyph, frameless text edit with
/// the "Search or jump to…" hint, and a clear affordance while the query is
/// non-empty. Returns the text edit's response for the caller's focus logic.
fn show_search_field(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    query: &mut String,
    search_rect: egui::Rect,
    id: egui::Id,
) -> egui::Response {
    let inner = search_rect.shrink2(egui::vec2(10.0_f32, 4.0_f32));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let tex_id = cache.texture(ui.ctx(), Icon::Search, 16.0, palette.ink_3);
            let sized = egui::load::SizedTexture::new(tex_id, egui::vec2(16.0, 16.0));
            ui.add(egui::Image::from_texture(sized));

            let response = ui.add(
                egui::TextEdit::singleline(query)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .hint_text("Search or jump to…")
                    .desired_width(ui.available_width() - 20.0),
            );

            if !query.is_empty() {
                let clear_rect = egui::Rect::from_center_size(
                    egui::pos2(inner.right() - 10.0, search_rect.center().y),
                    egui::vec2(20.0, SEARCH_H - 8.0),
                );
                if ghost_icon_button(
                    ui,
                    cache,
                    palette,
                    clear_rect,
                    id.with("clear"),
                    Icon::Close,
                    "Clear search",
                    false,
                ) {
                    query.clear();
                }
            }

            response
        },
    )
    .inner
}

/// Keyboard dismissal (REQ-UI-007 parity): while the field has focus,
/// Escape clears the query and gives the focus back, so a keyboard user can
/// operate — and dismiss — the search entirely from the keyboard.
///
/// The gate is *last frame's* focus, not this frame's: egui itself clears
/// keyboard focus during pass begin when Escape is pressed, so by the time
/// widget code runs on the Escape frame the field no longer reports focus.
fn handle_search_dismiss(ui: &egui::Ui, id: egui::Id, focused: bool, query: &mut String) {
    let focus_key = id.with("had_focus");
    let had_focus = focused || ui.memory(|m| m.data.get_temp::<bool>(focus_key).unwrap_or(false));
    ui.memory_mut(|m| m.data.insert_temp(focus_key, focused));
    if had_focus && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        query.clear();
        ui.memory_mut(|m| m.surrender_focus(id));
    }
}

/// The wordmark font: Inter Bold at 18 px. Resolved through the context's
/// bound families so a bare harness frame (fonts not yet installed — the
/// kittest constructor renders once before a test can configure them) falls
/// back to the default proportional family instead of panicking; the app
/// installs the vendored faces at startup, so the bold face always wins
/// there.
fn wordmark_font(ctx: &egui::Context) -> egui::FontId {
    let family = fonts::family_bold();
    let bound = ctx.fonts(|f| f.definitions().families.contains_key(&family));
    if bound {
        egui::FontId::new(18.0, family)
    } else {
        egui::FontId::proportional(18.0)
    }
}

/// Paint the wordmark's static equalizer glyph: four rounded bars of fixed
/// heights ([`WORDMARK_BARS`]) in one color.
#[expect(clippy::cast_precision_loss)]
fn paint_equalizer_mark(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let n = WORDMARK_BARS.len() as f32;
    let bar_w = rect.width() / (n * 1.6);
    let gap = (rect.width() - bar_w * n) / (n - 1.0);
    for (i, h) in WORDMARK_BARS.iter().enumerate() {
        let x = rect.left() + i as f32 * (bar_w + gap);
        let bar_h = rect.height() * h;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - bar_h / 2.0),
                egui::vec2(bar_w, bar_h),
            ),
            bar_w / 2.0,
            color,
        );
    }
}
