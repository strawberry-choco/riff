//! The sidebar: the port of the egui nav panel's behavior —
//! `ui/app.rs`'s `render_library_sidebar` and `ui/frame.rs`'s
//! `apply_sidebar_action` — with the wording `features.md`'s Library
//! Explorer Panel entry gives. LIBRARY, SMART LISTS and PLAYLISTS rows
//! carry their live counts (the pump's [`SidebarView`]) and write their
//! selections: the session's view fields through the guard, the open-list
//! slots through the nav signal, both from one [`apply`]. The active row's
//! band reads the nav signal, so it survives a pump tick without any
//! session lock in a render binding. The footer keeps the Add-folder
//! affordance (the picker is N14's platform integration) and the last full
//! scan's stamp, pinned below the scrolling rows like the egui footer's
//! bottom panel.

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::state::{BrowseMode, LibrarySection, LibrarySession, ViewMode};
use riff_backend::domain::{PlaylistId, SmartPlaylistKind};
use std::time::SystemTime;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::model::{NavView, SidebarView};
use crate::theme::Palette;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

use super::ring_when;

/// The rows' selection identity: what one nav row opens, exactly the egui
/// sidebar's action set (`SidebarAction::Navigate`, `NavigateFolders`,
/// `OpenSmartList`, the playlist row's Open). The rows' create/rename/
/// delete flows are N10's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SidebarTarget {
    /// A LIBRARY row: the browser variant it opens.
    Section(LibrarySection),
    /// The Folders row: the browse mode it switches to.
    Folders,
    /// A SMART LISTS row: the built-in list it opens.
    SmartList(SmartPlaylistKind),
    /// A PLAYLISTS row: the list it opens.
    Playlist(PlaylistId),
}

/// One nav row's facts: what it opens, what it paints. The one model the
/// render and the keyboard traversal share, so both agree on the display
/// order and the row count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavRow {
    pub(crate) target: SidebarTarget,
    pub(crate) label: String,
    pub(crate) glyph: Option<Icon>,
    /// The live count shown right-aligned in muted ink; `None` paints no
    /// count (the playlist rows fold theirs into the label).
    pub(crate) count: Option<usize>,
}

/// The LIBRARY rows, in `features.md`'s order.
const LIBRARY_ROWS: [(LibrarySection, &str, Icon); 4] = [
    (LibrarySection::AllTracks, "All Tracks", Icon::ListMusic),
    (LibrarySection::Artists, "Artists", Icon::Music),
    (LibrarySection::Albums, "Albums", Icon::Disc),
    (LibrarySection::Genres, "Genres", Icon::LayoutGrid),
];

/// The smart lists a library shows, the egui sidebar's gate: the four core
/// lists always, Never Played and Lost Gems behind Advanced mode
/// (`ui/app.rs`'s `smart_list_kinds`).
fn smart_list_kinds(advanced: bool) -> Vec<SmartPlaylistKind> {
    const CORE: [SmartPlaylistKind; 4] = [
        SmartPlaylistKind::RecentlyAdded,
        SmartPlaylistKind::RecentlyPlayed,
        SmartPlaylistKind::MostPlayed,
        SmartPlaylistKind::Favorites,
    ];
    if !advanced {
        return CORE.to_vec();
    }
    let mut kinds = CORE.to_vec();
    kinds.push(SmartPlaylistKind::NeverPlayed);
    kinds.push(SmartPlaylistKind::LostGems);
    kinds
}

/// The smart list's total as the sidebar carries it; a kind the counts read
/// missed reads as `0`, never as absent (the egui sidebar's `smart_count`).
fn smart_count(view: &SidebarView, kind: SmartPlaylistKind) -> usize {
    view.smart_lists
        .iter()
        .find(|(known, _)| known == &kind)
        .map_or(0, |(_, total)| *total)
}

/// Every nav row in display order: LIBRARY, then SMART LISTS (folded away
/// when the persisted flag says so), then PLAYLISTS. The keyboard
/// traversal's row count is this list's length.
pub(crate) fn nav_rows(view: &SidebarView, collapsed: bool, advanced: bool) -> Vec<NavRow> {
    let mut rows: Vec<NavRow> = LIBRARY_ROWS
        .iter()
        .map(|(section, label, glyph)| NavRow {
            target: SidebarTarget::Section(*section),
            label: (*label).to_owned(),
            glyph: Some(*glyph),
            count: Some(count_of(view, *section)),
        })
        .collect();
    rows.push(NavRow {
        target: SidebarTarget::Folders,
        label: "Folders".to_owned(),
        glyph: Some(Icon::Folder),
        count: Some(view.folder_roots),
    });
    if !collapsed {
        rows.extend(smart_list_kinds(advanced).into_iter().map(|kind| NavRow {
            target: SidebarTarget::SmartList(kind),
            label: kind.display_name().to_owned(),
            glyph: Some(Icon::Sparkles),
            count: Some(smart_count(view, kind)),
        }));
    }
    rows.extend(view.playlists.iter().map(|playlist| NavRow {
        target: SidebarTarget::Playlist(playlist.id.clone()),
        label: format!("{} ({})", playlist.name, playlist.tracks),
        glyph: None,
        count: None,
    }));
    rows
}

/// The LIBRARY row's count: the sidebar's own field per section.
fn count_of(view: &SidebarView, section: LibrarySection) -> usize {
    match section {
        LibrarySection::AllTracks => view.tracks,
        LibrarySection::Artists => view.artists,
        LibrarySection::Albums => view.albums,
        LibrarySection::Genres => view.genres,
    }
}

/// The panel: sections and rows over the scrolling region, the footer
/// pinned below it. The egui footer was a bottom panel for exactly this —
/// the rows may outgrow the window, the footer may not.
pub fn panel(
    model: Stored<UiModel>,
    state: UiState,
    nav_focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    view()
        .w(px(tokens::SIDEBAR_W))
        .flex_col()
        .test_id("sidebar")
        .bg(move || palette.get().surface)
        .border_r(px(tokens::BORDER_HAIRLINE))
        .border_color(move || palette.get().line)
        .p(px(tokens::SPACE_LG))
        .gap(px(tokens::SPACE_LG))
        .child(
            scroll()
                .flex_1()
                .flex_col()
                .gap(px(tokens::SPACE_LG))
                .test_id("sidebar-rows")
                .child(header(palette, "LIBRARY"))
                .child(dynamic(move || {
                    section_rows(model, state, nav_focus, |target| {
                        matches!(target, SidebarTarget::Section(_) | SidebarTarget::Folders)
                    })
                }))
                .child(smart_lists_header(model, state))
                .child(dynamic(move || {
                    section_rows(model, state, nav_focus, |target| {
                        matches!(target, SidebarTarget::SmartList(_))
                    })
                }))
                .child(header(palette, "PLAYLISTS"))
                .child(dynamic(move || {
                    section_rows(model, state, nav_focus, |target| {
                        matches!(target, SidebarTarget::Playlist(_))
                    })
                })),
        )
        .child(footer(state))
}

/// The rows of one section, cut out of the display order by `keep` — the
/// region's rows keep their global traversal index, which is what the key
/// map's activation reads. Rebuilt when the counts or the fold move; the
/// selection and the traversal ride the rows' own bindings.
fn section_rows(
    model: Stored<UiModel>,
    state: UiState,
    nav_focus: Signal<Option<usize>>,
    keep: impl Fn(&SidebarTarget) -> bool,
) -> Vec<impl IntoElement> {
    let view = state.sidebar.get();
    let flags = state.settings.get().flags;
    nav_rows(&view, flags.smart_lists_collapsed, flags.advanced_mode)
        .into_iter()
        .enumerate()
        .filter(|(_, row)| keep(&row.target))
        .map(|(at, row)| nav_row(model, state, nav_focus, at, row))
        .collect()
}

/// A section header: the small caps label the egui sidebar led its
/// sections with.
fn header(palette: Signal<Palette>, label: &'static str) -> impl IntoElement {
    text(label)
        .text_xs()
        .text_color(move || palette.get().ink_3)
        .test_id("sidebar-section")
}

/// The SMART LISTS header with the fold chevron the egui header pinned at
/// its right edge. The fold is a persisted flag: the handler flips the
/// session's, and the pump's settings diff lands it back in the snapshot
/// this header and the rows read.
fn smart_lists_header(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let collapsed = move || state.settings.get().flags.smart_lists_collapsed;
    view()
        .flex_row()
        .items_center()
        .test_id("sidebar-section")
        .child(
            text("SMART LISTS")
                .flex_1()
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            view()
                .on_press(move || toggle_smart_lists(model))
                .test_id("smart-lists-collapse")
                .label(move || {
                    if collapsed() {
                        "Expand Smart Lists"
                    } else {
                        "Collapse Smart Lists"
                    }
                })
                .p(px(tokens::SPACE_XS))
                .rounded(px(tokens::RADIUS_SM))
                .hover(|s| s.bg(move || palette.get().row_hover))
                .child(dynamic(move || {
                    let glyph = if collapsed() {
                        Icon::ChevronRight
                    } else {
                        Icon::ChevronDown
                    };
                    icon(glyph, Icon::ICON_SIZE, move || palette.get().ink_2)
                })),
        )
}

/// A nav row: the egui `tree_row`'s anatomy — leading glyph, label, the
/// live count right-aligned in muted ink — with the selection writes and
/// the two highlights. The selection band paints the egui row's selected
/// fill (`surface_3` at rest) while [`is_active`] names the row; the
/// traversal ring paints while the key map's focus names it. The
/// accessibility state carries the selection, the band's same fact.
fn nav_row(
    model: Stored<UiModel>,
    state: UiState,
    nav_focus: Signal<Option<usize>>,
    at: usize,
    row: NavRow,
) -> impl IntoElement {
    let palette = state.palette;
    let id = row_id(&row);
    let label = row.label.clone();
    let active_target = row.target.clone();
    let active = move || is_active(state.nav.get(), &active_target);
    let selected_a11y = active.clone();
    let press_target = row.target.clone();
    let leading = row
        .glyph
        .map(|glyph| icon(glyph, Icon::ICON_SIZE, move || palette.get().ink_2).into_element());
    ring_when(
        view()
            .on_press(move || apply(model, press_target.clone()))
            .test_id(&id)
            .label(label)
            .accessibility_state(move || AccessibilityState {
                selected: selected_a11y(),
                ..AccessibilityState::default()
            })
            .flex_row()
            .items_center()
            .gap(px(tokens::SPACE_SM))
            .py(px(tokens::SPACE_XS))
            .px(px(tokens::SPACE_SM))
            .rounded(px(tokens::RADIUS_SM))
            .bg(move || {
                if active() {
                    palette.get().surface_3
                } else {
                    transparent_black()
                }
            })
            .hover(|s| s.bg(move || palette.get().row_hover))
            .children(leading)
            .child(
                text(move || row.label.clone())
                    .text_sm()
                    .flex_1()
                    .truncate()
                    .text_color(move || palette.get().ink_2),
            )
            .children(row.count.map(|count| {
                text(move || count.to_string())
                    .text_xs()
                    .text_color(move || palette.get().ink_3)
            })),
        move || nav_focus.get() == Some(at),
        palette,
    )
}

/// Whether the row's selection is live, the egui sidebar's highlight
/// contract: a LIBRARY row only while the Library view stands in Library
/// browse mode with no list opened over it, the Folders row while the
/// Folders mode does, a list row while that list is the one opened.
fn is_active(nav: NavView, target: &SidebarTarget) -> bool {
    match target {
        SidebarTarget::Section(section) => {
            nav.view_mode == ViewMode::Library
                && nav.browse_mode == BrowseMode::Library
                && nav.smart_list.is_none()
                && nav.playlist.is_none()
                && nav.library_section == *section
        }
        SidebarTarget::Folders => {
            nav.view_mode == ViewMode::Library && nav.browse_mode == BrowseMode::Folders
        }
        SidebarTarget::SmartList(kind) => nav.smart_list == Some(*kind),
        SidebarTarget::Playlist(id) => nav.playlist.as_ref() == Some(id),
    }
}

/// The focused row's activation, the key map's Enter and Space on a
/// sidebar row: the same write the row's press makes.
pub(crate) fn activate(model: Stored<UiModel>, at: usize) {
    let state = model.with(|m| m.signals());
    let flags = state.settings.get().flags;
    let rows = nav_rows(
        &state.sidebar.get(),
        flags.smart_lists_collapsed,
        flags.advanced_mode,
    );
    if let Some(row) = rows.into_iter().nth(at) {
        apply(model, row.target);
    }
}

/// One selection: the egui frame's write set, applied through the library
/// guard, mirrored to the nav signal outside the borrow. A selection write
/// mutates no store, so the pump's sidebar rebuild cannot see it — the
/// signal is the handler's own record of what it wrote, and the pump never
/// touches it.
pub(crate) fn apply(model: Stored<UiModel>, target: SidebarTarget) {
    let nav_signal = model.with(|m| m.state.nav);
    let mut nav = nav_signal.get();
    nav = model.with(|m| {
        let mut library = m.rt.library.lock_or_recover();
        match &target {
            SidebarTarget::Section(section) => {
                land_on_library(&mut library);
                library.library_section = *section;
                nav.smart_list = None;
                nav.playlist = None;
            }
            SidebarTarget::Folders => {
                land_on_library(&mut library);
                library.browse_mode = BrowseMode::Folders;
                nav.smart_list = None;
                nav.playlist = None;
            }
            SidebarTarget::SmartList(kind) => {
                land_on_library(&mut library);
                nav.smart_list = Some(*kind);
                nav.playlist = None;
            }
            SidebarTarget::Playlist(id) => {
                // The egui row's Open is slots only: no view field moves,
                // the list draws over whatever the Library view stands in.
                nav.playlist = Some(id.clone());
                nav.smart_list = None;
            }
        }
        nav.view_mode = library.view_mode;
        nav.browse_mode = library.browse_mode;
        nav.library_section = library.library_section;
        library.browser_path.clone_into(&mut nav.browser_path);
        nav
    });
    nav_signal.set(nav);
}

/// The shared "land on the Library view" reset, the egui frame's
/// `land_on_library`: the Library view in Library browse mode, drill-down
/// path and search filter cleared.
fn land_on_library(library: &mut LibrarySession) {
    library.view_mode = ViewMode::Library;
    library.browse_mode = BrowseMode::Library;
    library.reset_browser_path();
    library.search_query.clear();
}

/// The fold chevron's write, the egui `ToggleSmartListsCollapsed`: the
/// persisted flag flips through the guard and any opened smart list closes
/// with it — with the rows gone there is no other way back to that view.
/// The playlist slot is the egui frame's, left alone.
fn toggle_smart_lists(model: Stored<UiModel>) {
    let nav_signal = model.with(|m| m.state.nav);
    let mut nav = nav_signal.get();
    nav.smart_list = None;
    nav = model.with(|m| {
        let mut library = m.rt.library.lock_or_recover();
        library.ui_flags.smart_lists_collapsed = !library.ui_flags.smart_lists_collapsed;
        nav.view_mode = library.view_mode;
        nav.browse_mode = library.browse_mode;
        nav.library_section = library.library_section;
        library.browser_path.clone_into(&mut nav.browser_path);
        nav
    });
    nav_signal.set(nav);
}

/// The footer: the Add-folder affordance (inert — the egui contract routes
/// it to the native folder picker, N14's platform integration here) and the
/// last full scan's stamp, the two facts the egui footer carried.
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
                .border(px(tokens::BORDER_HAIRLINE))
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
/// one place: "All Tracks" rows as `nav-all-tracks`. A playlist row keys
/// on its id, whose slug the name's label would only re-slug.
fn row_id(row: &NavRow) -> String {
    match &row.target {
        SidebarTarget::Playlist(id) => format!("nav-playlist-{}", id.0),
        _ => nav_id(&row.label),
    }
}

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

    #[test]
    fn advanced_mode_gates_the_two_relocated_lists() {
        let core = smart_list_kinds(false);
        assert_eq!(core.len(), 4);
        assert!(!core.contains(&SmartPlaylistKind::NeverPlayed));
        let all = smart_list_kinds(true);
        assert_eq!(all.len(), 6);
    }

    #[test]
    fn nav_rows_carry_every_sections_counts() {
        let view = SidebarView {
            tracks: 50_000,
            artists: 120,
            albums: 300,
            genres: 9,
            folder_roots: 2,
            smart_lists: [(SmartPlaylistKind::Favorites, 41)].into(),
            playlists: [crate::model::PlaylistRow {
                id: PlaylistId("night-drive".to_owned()),
                name: "Night Drive".to_owned(),
                tracks: 12,
            }]
            .into(),
            last_scan: None,
        };
        let rows = nav_rows(&view, false, true);
        assert_eq!(rows.len(), 5 + 6 + 1);
        assert_eq!(rows[0].count, Some(50_000));
        assert_eq!(rows[3].count, Some(9));
        assert_eq!(rows[4].count, Some(2));
        assert_eq!(
            rows[5],
            NavRow {
                target: SidebarTarget::SmartList(SmartPlaylistKind::RecentlyAdded),
                label: "Recently Added".to_owned(),
                glyph: Some(Icon::Sparkles),
                count: Some(0),
            }
        );
        assert_eq!(rows[11].label, "Night Drive (12)");
    }

    #[test]
    fn the_fold_removes_the_smart_rows_only() {
        let view = SidebarView::default();
        let open = nav_rows(&view, false, false);
        let folded = nav_rows(&view, true, false);
        assert_eq!(open.len(), 9);
        assert_eq!(folded.len(), 5);
    }
}
