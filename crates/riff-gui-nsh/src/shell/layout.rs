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
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::app::preferences::WindowGeometry;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;

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

    let search = element_ref();
    let (focus, nav_focus) = keyboard(model, search);

    view()
        .flex_1()
        .flex_col()
        .test_id("shell")
        .bg(move || palette.get().background)
        .text_color(move || palette.get().ink)
        .child(title_bar::bar(state, &search, focus))
        .child(middle(model, state, nav_focus))
        .child(player_bar::bar(model, state, focus))
}

/// The middle region: the sidebar panel beside the empty stage. `flex_1` is
/// load-bearing: without it the region sizes to its children and the stage
/// collapses to nothing (P0 correction 21).
fn middle(
    model: Stored<UiModel>,
    state: UiState,
    nav_focus: Signal<Option<usize>>,
) -> impl IntoElement {
    let palette = state.palette;
    view()
        .flex_1()
        .flex_row()
        .child(sidebar::panel(model, state, nav_focus))
        .child(
            view()
                .flex_1()
                .test_id("stage")
                .bg(move || palette.get().surface_row),
        )
}

/// The global shortcuts: Space toggles playback exactly like the egui
/// frame's `apply_keyboard` (pause while playing, resume otherwise — a
/// stopped session stays stopped, the play button is the one that starts
/// one) unless a sidebar row holds the traversal focus, where Space and
/// Enter activate the focused row like the egui frame's keyboard did on its
/// focused widget; Ctrl+K focuses the search field; Escape surrenders it
/// and clears both focus segments; the arrows step the traversal — the
/// chrome ring and the sidebar rows as one sequence. Nothing here writes a
/// bridge signal: the transport dispatch reads the session through the
/// model, and the row activation writes the nav signal the selection
/// handlers own.
fn keyboard(
    model: Stored<UiModel>,
    search: ElementRef,
) -> (Signal<Option<usize>>, Signal<Option<usize>>) {
    let state = model.with(|m| m.signals());
    let focus = signal(None::<usize>);
    let nav_focus = signal(None::<usize>);

    // The sidebar's rows, as the traversal counts them: the fold and the
    // playlists move the count, so every step reads it fresh.
    let row_count = move || {
        let flags = state.settings.get().flags;
        sidebar::nav_rows(
            &state.sidebar.get(),
            flags.smart_lists_collapsed,
            flags.advanced_mode,
        )
        .len()
    };

    let step_traversal = move |delta: i32| {
        let at = match (focus.get(), nav_focus.get()) {
            (Some(slot), _) => Some(super::Slot::Chrome(slot)),
            (None, Some(row)) => Some(super::Slot::Sidebar(row)),
            (None, None) => None,
        };
        match super::step(at, delta, row_count()) {
            super::Slot::Chrome(slot) => {
                focus.set(Some(slot));
                nav_focus.set(None);
            }
            super::Slot::Sidebar(row) => {
                focus.set(None);
                nav_focus.set(Some(row));
            }
        }
    };

    on_key("space", move |_| match nav_focus.get() {
        Some(row) => sidebar::activate(model, row),
        None => model.with(|m| {
            let session = m.rt.playback.lock_or_recover();
            if session.playback_state == riff_backend::app::state::PlaybackState::Playing {
                m.rt.ui_transport.pause();
            } else {
                m.rt.ui_transport.resume();
            }
        }),
    });
    on_key("enter", move |_| {
        if let Some(row) = nav_focus.get() {
            sidebar::activate(model, row);
        }
    });
    on_key("mod+k", move |_| {
        focus.set(None);
        nav_focus.set(None);
        search.focus();
    });
    on_key("escape", move |_| {
        search.blur();
        focus.set(None);
        nav_focus.set(None);
    });
    on_key("left", move |_| step_traversal(-1));
    on_key("right", move |_| step_traversal(1));
    on_key("up", move |_| step_traversal(-1));
    on_key("down", move |_| step_traversal(1));
    (focus, nav_focus)
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
