//! The shell root: the window's one layout — titlebar over a sidebar-stage
//! middle over the player bar — plus the two behaviors the egui frame owned
//! as composition: the keyboard map (`ui/frame.rs`'s `apply_keyboard` and
//! the arrows) and the window-geometry loop (the nsh frontend's own: the
//! egui app opens at a fixed size). Geometry restores at mount and reports
//! its settled frame to the `Preferences` handle, whose frame-end
//! diff-commit lands it in the Application Store — the sessions know
//! nothing of the window.

use nsh_core::desktop::{WindowFrame, WindowMode, WindowPlacement};
use nsh_core::element::desktop::current_window;
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::preferences::WindowGeometry;
use riff_backend::app::state::{BrowseMode, LibrarySection, ViewMode};
use riff_backend::domain::{PlaylistId, SmartPlaylistKind};

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::model::{NavView, StageKind};

use super::player_bar;
use super::sidebar;
use super::title_bar;

/// The shell: chrome, geometry, keyboard, and the one tree. Runs once at
/// mount (and again on a return from the debug readout, whose toggles
/// unmount this subtree); every piece is idempotent or owner-scoped.
pub fn install(model: Stored<UiModel>) -> impl IntoElement {
    let state = model.with(|m| m.signals());
    let palette = state.palette;

    title_bar::install_chrome();
    restore_window_geometry(model);
    watch_window_geometry(model);
    crate::platform::install(model, state);

    let search = element_ref();
    let list = element_ref();
    let (focus, nav_focus, list_focus) = keyboard(model, search, list);
    clear_list_focus_on_stage_switch(state, list_focus);
    let nav = state.nav;
    let branch = computed(move || ShellBranch::of(&nav.get()));

    view()
        .flex_1()
        .flex_col()
        .test_id("shell")
        .bg(move || palette.get().background)
        .text_color(move || palette.get().ink)
        .child(title_bar::bar(model, state, &search, focus))
        .child(middle(model, state, nav_focus, list_focus, list, branch))
        .child(dynamic(move || {
            if state.queue_open.get() {
                crate::screens::queue::panel(model, state).into_element()
            } else {
                view().into_element()
            }
        }))
        .child(player_bar::bar(model, state, focus))
        .on_file_drop(move |event: &FileDropEvent| {
            crate::platform::library_paths::accept_drop(model, state, &event.uris);
        })
}

/// Which top-level surface the stage slot mounts. It names only the fields
/// that change the surface itself, so a selection write inside the browse
/// columns leaves the composed tree mounted and the columns' `each` diffs in
/// place instead of remounting every column.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellBranch {
    NowPlaying,
    Settings,
    Playlist(PlaylistId),
    SmartList(SmartPlaylistKind),
    CustomList(PlaylistId),
    Folders,
    Section(LibrarySection),
}

impl ShellBranch {
    fn of(nav: &NavView) -> Self {
        if nav.view_mode == ViewMode::NowPlaying {
            return Self::NowPlaying;
        }
        if nav.view_mode == ViewMode::Settings {
            return Self::Settings;
        }
        if let Some(playlist) = &nav.playlist {
            return Self::Playlist(playlist.clone());
        }
        if let Some(kind) = nav.smart_list {
            return Self::SmartList(kind);
        }
        if let Some(list) = &nav.custom_list {
            return Self::CustomList(list.clone());
        }
        if nav.browse_mode == BrowseMode::Folders {
            return Self::Folders;
        }
        Self::Section(nav.library_section)
    }
}

/// The middle region: the sidebar panel beside the stage slot. The branch
/// memo picks the surface; the browse stage builds its columns from the nav
/// signal it reads itself. `flex_1` is load-bearing: without it the region
/// sizes to its children and the stage collapses to nothing (P0 correction
/// 21).
fn middle(
    model: Stored<UiModel>,
    state: UiState,
    nav_focus: Signal<Option<usize>>,
    list_focus: Signal<Option<usize>>,
    list: ElementRef,
    branch: Memo<ShellBranch>,
) -> impl IntoElement {
    view()
        .flex_1()
        .flex_row()
        .child(sidebar::panel(model, state, nav_focus))
        .child(dynamic(move || match branch.get() {
            ShellBranch::NowPlaying => {
                crate::screens::now_playing::screen(model, state).into_element()
            }
            ShellBranch::Settings => crate::screens::settings::screen(model, state).into_element(),
            ShellBranch::Playlist(_) => {
                crate::screens::playlists::stage(model, state).into_element()
            }
            ShellBranch::SmartList(_) | ShellBranch::CustomList(_) => {
                crate::screens::smart_playlists::stage(model, state).into_element()
            }
            ShellBranch::Folders => {
                crate::screens::library::folders::stage(model, state).into_element()
            }
            ShellBranch::Section(section) => match section {
                LibrarySection::AllTracks => {
                    crate::screens::library::list::stage(model, state, list, list_focus)
                        .into_element()
                }
                _ => {
                    crate::screens::library::browse::stage(model, state, section, list, list_focus)
                        .into_element()
                }
            },
        }))
}
/// The phantom-segment cleanup (N6's carried-forward flag): a stage switch
/// unmounts the rows the traversal focused, and `list_focus` naming a row
/// of a dead stage left the arrows walking an invisible segment. Every nav
/// move clears the segment, not just Escape. The search retarget keeps the
/// listing (its rows move under an unchanged focus, which the egui frame
/// also kept) — the cleanup reads only the nav.
fn clear_list_focus_on_stage_switch(state: UiState, list_focus: Signal<Option<usize>>) {
    let nav = state.nav;
    effect(move || {
        let _ = crate::model::stage_of(&nav.get());
        list_focus.set(None);
    });
}

/// The global shortcuts: Space toggles playback exactly like the egui
/// frame's `apply_keyboard` (pause while playing, resume otherwise — a
/// stopped session stays stopped, the play button is the one that starts
/// one) unless a traversal-focused row holds it, where Space and Enter
/// activate the focused row like the egui frame's keyboard did on its
/// focused widget; Ctrl+K focuses the search field; Escape surrenders it,
/// clears every focus segment and restores the unfiltered listing (the
/// search flow's write, `When::Always` so it reaches a focused field); the
/// arrows step the traversal — the chrome ring, the sidebar rows and the
/// track list's rows as one sequence (the egui rows took no arrow
/// traversal; the list segment is the nsh frame's own, recorded in the N6
/// trail). Nothing here writes a bridge signal except the traversal's own
/// handles and the search flow's (the handlers own those signals the way
/// the sidebar's selection writes own theirs): the transport dispatch
/// reads the session through the model, and the activations write the
/// signals their handlers own.
#[allow(clippy::type_complexity)]
type FocusSegments = (
    Signal<Option<usize>>,
    Signal<Option<usize>>,
    Signal<Option<usize>>,
);

fn keyboard(model: Stored<UiModel>, search: ElementRef, list: ElementRef) -> FocusSegments {
    let state = model.with(|m| m.signals());
    let focus = signal(None::<usize>);
    let nav_focus = signal(None::<usize>);
    let list_focus = signal(None::<usize>);

    // The sidebar's rows, as the traversal counts them: the fold and the
    // playlists move the count, so every step reads it fresh.
    let row_count = move || {
        let flags = state.settings.get().flags;
        sidebar::nav_rows(
            &state.sidebar.get(),
            flags.smart_lists_collapsed,
            flags.advanced_mode,
            &state.custom_lists.get(),
        )
        .len()
    };

    // The list's rows: the listing's total, the same signal the virtual
    // list sizes itself with.
    let list_count = move || state.list_total.get();

    let reveal = list;
    let step_traversal = move |delta: i32| {
        let at = match (focus.get(), nav_focus.get(), list_focus.get()) {
            (Some(slot), _, _) => Some(super::Slot::Chrome(slot)),
            (None, Some(row), _) => Some(super::Slot::Sidebar(row)),
            (None, None, Some(row)) => Some(super::Slot::List(row)),
            (None, None, None) => None,
        };
        match super::step(at, delta, row_count(), list_count()) {
            super::Slot::Chrome(slot) => {
                focus.set(Some(slot));
                nav_focus.set(None);
                list_focus.set(None);
            }
            super::Slot::Sidebar(row) => {
                focus.set(None);
                nav_focus.set(Some(row));
                list_focus.set(None);
            }
            super::Slot::List(row) => {
                focus.set(None);
                nav_focus.set(None);
                list_focus.set(Some(row));
                reveal.scroll_to_index(
                    u32::try_from(row).unwrap_or(u32::MAX),
                    percentage(0.5),
                    px(0.0),
                    false,
                );
            }
        }
    };

    on_key("space", move |_| {
        match (nav_focus.get(), list_focus.get()) {
            (Some(row), _) => sidebar::activate(model, row),
            (None, Some(_)) => select_focused(model, state, list_focus),
            (None, None) => model.with(|m| {
                let session = m.rt.playback.lock_or_recover();
                if session.playback_state == riff_backend::app::state::PlaybackState::Playing {
                    m.rt.ui_transport.pause();
                } else {
                    m.rt.ui_transport.resume();
                }
            }),
        }
    });
    on_key("enter", move |_| {
        match (nav_focus.get(), list_focus.get()) {
            (Some(row), _) => sidebar::activate(model, row),
            (None, Some(_)) => select_focused(model, state, list_focus),
            (None, None) => {}
        }
    });
    on_key("mod+k", move |_| {
        focus.set(None);
        nav_focus.set(None);
        list_focus.set(None);
        search.focus();
    });
    // Escape, wherever the focus is (`When::Always`): the field is a
    // native input and the raw key path serves custom views only, so the
    // egui field's `dismiss_on_escape` lands here — the query clears and
    // the full list restores through the same retarget a keystroke takes.
    let search_ref = search;
    on_key_with(
        "escape",
        nsh_core::element::KeyOptions {
            when: When::Always,
            ..nsh_core::element::KeyOptions::default()
        },
        move |_| {
            crate::screens::library::search::escape(model, state.search_text, search_ref);
            focus.set(None);
            nav_focus.set(None);
            list_focus.set(None);
        },
    );
    on_key("left", move |_| step_traversal(-1));
    on_key("right", move |_| step_traversal(1));
    on_key("up", move |_| step_traversal(-1));
    on_key("down", move |_| step_traversal(1));
    #[cfg(debug_assertions)]
    debug_keys(model, state, list);
    #[cfg(not(debug_assertions))]
    let _ = list;
    (focus, nav_focus, list_focus)
}

/// Debug-only evidence levers. Each performs its control's own write, and
/// release builds carry none of them.
#[cfg(debug_assertions)]
fn debug_keys(model: Stored<UiModel>, state: UiState, list: ElementRef) {
    // The perf drive's jump (N6 evidence): the production UI has no deep
    // jump affordance, so the drive script presses this debug key to land
    // the list on a deep row before timing its scroll steps.
    #[cfg(debug_assertions)]
    on_key("mod+alt+j", move |_| {
        list.scroll_to_index(40000, percentage(0.5), px(0.0), false);
    });
    // The evidence drive's navigation levers (N15/N16). Each one is the
    // control's own write, so the screenshots record the production state,
    // not a special path; release builds carry none of them.
    #[cfg(debug_assertions)]
    on_key("mod+alt+n", move |_| {
        if state.nav.get().view_mode == riff_backend::app::state::ViewMode::NowPlaying {
            crate::screens::now_playing::close(model, state);
        } else {
            crate::screens::now_playing::open(model, state);
        }
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+q", move |_| {
        state.queue_open.update(|open| *open = !*open);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+s", move |_| {
        crate::screens::settings::open(model, state);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+p", move |_| {
        state.playlist_creating.set(true);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+m", move |_| {
        crate::screens::smart_playlists::editor::create_new(model, state);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+shift+p", move |_| {
        state.playlist_creating.set(true);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+shift+m", move |_| {
        crate::screens::smart_playlists::editor::create_new(model, state);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+shift+1", move |_| {
        crate::screens::settings::open(model, state);
        state.settings_page.set(crate::model::SettingsPage::Library);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+shift+2", move |_| {
        state
            .settings_page
            .set(crate::model::SettingsPage::Playback);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+shift+3", move |_| {
        state
            .settings_page
            .set(crate::model::SettingsPage::Appearance);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+shift+4", move |_| {
        state
            .settings_page
            .set(crate::model::SettingsPage::Advanced);
    });
    #[cfg(debug_assertions)]
    on_key("mod+alt+shift+5", move |_| {
        state.settings_page.set(crate::model::SettingsPage::About);
    });
    // The drive's reliable set: Windows folds Ctrl+Alt into AltGr, so the
    // Alt-bearing forms above are not reachable from SendKeys; these are.
    #[cfg(debug_assertions)]
    on_key("mod+shift+p", move |_| {
        state.playlist_creating.set(true);
    });
    #[cfg(debug_assertions)]
    on_key("mod+shift+m", move |_| {
        crate::screens::smart_playlists::editor::create_new(model, state);
    });
    #[cfg(debug_assertions)]
    on_key("mod+shift+1", move |_| {
        crate::screens::settings::open(model, state);
        state.settings_page.set(crate::model::SettingsPage::Library);
    });
    #[cfg(debug_assertions)]
    on_key("mod+shift+2", move |_| {
        state
            .settings_page
            .set(crate::model::SettingsPage::Playback);
    });
    #[cfg(debug_assertions)]
    on_key("mod+shift+3", move |_| {
        state
            .settings_page
            .set(crate::model::SettingsPage::Appearance);
    });
    #[cfg(debug_assertions)]
    on_key("mod+shift+4", move |_| {
        state
            .settings_page
            .set(crate::model::SettingsPage::Advanced);
    });
    #[cfg(debug_assertions)]
    on_key("mod+shift+5", move |_| {
        state.settings_page.set(crate::model::SettingsPage::About);
    });
}

/// Enter or Space on a traversal-focused list row: the row's press write
/// (the egui click's same selection), read at the focused index. The read
/// follows the stage the nav names — an album's tracks stage selects out
/// of its display copy, the flat stage out of the seam.
fn select_focused(model: Stored<UiModel>, state: UiState, list_focus: Signal<Option<usize>>) {
    if let Some(index) = list_focus.get() {
        let kind = match crate::model::stage_of(&state.nav.get()) {
            Some(StageKind::Tracks { .. }) => crate::screens::library::list::Listing::Album,
            _ => crate::screens::library::list::Listing::Flat,
        };
        let track = model.with(|m| crate::screens::library::list::track_at(m, index, kind));
        if let Some(track) = track {
            crate::components::track_row::select_track(model, track.id.clone());
        }
    }
}

/// Launch: give the window the frame the store carries. A full position
/// and size restore as a placement (`set_placement`, which fits the frame
/// onto the displays there are now and re-maximizes when the row says
/// so); a size alone resizes in place; a store the frontend never saved
/// leaves the window as `AppOptions` opened it. Nothing happens off
/// desktops — headless there is no window to size.
fn restore_window_geometry(model: Stored<UiModel>) {
    let geometry = model.with(|m| m.rt.preferences.window_geometry());
    let window = current_window();
    if window.state().is_none() {
        return;
    }
    if let Some(placement) = placement_of(geometry) {
        window.set_placement(placement);
    } else if let (Some(width), Some(height)) = (geometry.width, geometry.height) {
        window.set_bounds(WindowFrame {
            width: Some(px(width)),
            height: Some(px(height)),
            ..WindowFrame::default()
        });
    }
}

/// Running: report the window's settled frame — a move or a resize is
/// reported once the window stops for a moment — to the `Preferences`
/// handle. No signal write and no store write here: the next pump tick's
/// diff-commit sees the row's geometry columns differ and saves, exactly
/// like a session mutation. Off desktops `state()` is `None` and the
/// effect waits.
fn watch_window_geometry(model: Stored<UiModel>) {
    let window = current_window();
    let last = stored(None::<WindowGeometry>);
    effect(move || {
        let Some(state) = window.state() else { return };
        let geometry = geometry_of(state.placement);
        if last.replace(Some(geometry)) != Some(geometry) {
            model.with(|m| m.rt.preferences.set_window_geometry(geometry));
        }
    });
}

/// The persisted row's placement: a mode plus the normal bounds. `None`
/// while any part of the frame is unknown, so a partial row never
/// invents an origin.
#[must_use]
pub fn placement_of(geometry: WindowGeometry) -> Option<WindowPlacement> {
    let (Some(x), Some(y), Some(width), Some(height)) =
        (geometry.x, geometry.y, geometry.width, geometry.height)
    else {
        return None;
    };
    let mode = if geometry.maximized {
        WindowMode::Maximized
    } else {
        WindowMode::Normal
    };
    Some(WindowPlacement::new(
        mode,
        Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
    ))
}

/// The row's geometry, out of the window's own placement record — the
/// normal-mode bounds (what a maximized window returns to) and the mode.
#[must_use]
pub fn geometry_of(placement: WindowPlacement) -> WindowGeometry {
    WindowGeometry {
        x: Some(placement.bounds.origin.x.0),
        y: Some(placement.bounds.origin.y.0),
        width: Some(placement.bounds.size.width.0),
        height: Some(placement.bounds.size.height.0),
        maximized: placement.mode == WindowMode::Maximized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_geometry_round_trips_through_a_placement() {
        let geometry = WindowGeometry {
            x: Some(64.0),
            y: Some(32.0),
            width: Some(1200.0),
            height: Some(800.0),
            maximized: true,
        };
        let placement = placement_of(geometry).expect("a full geometry restores");
        assert_eq!(geometry_of(placement), geometry);
        assert_eq!(placement.mode, WindowMode::Maximized);
    }

    #[test]
    fn a_partial_geometry_has_no_placement() {
        let geometry = WindowGeometry {
            x: Some(64.0),
            ..WindowGeometry::default()
        };
        assert_eq!(placement_of(geometry), None);
    }
}
