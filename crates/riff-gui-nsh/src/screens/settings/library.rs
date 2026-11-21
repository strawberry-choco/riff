//! The Library page: the registered paths with their readiness and watch
//! states, the add path field, rescan, remove, and Clear Library.

use nsh_base::input;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::theme::tokens;

use super::{action_row, heading, status_line};

/// The page.
pub fn page(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    view()
        .flex_col()
        .gap(px(tokens::SPACE_MD))
        .test_id("settings-library")
        .child(heading(state, "Library folders"))
        .child(
            text("riff indexes the audio files under these folders. Add a folder, then rescan.")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(add_row(model, state))
        .child(
            view()
                .flex_col()
                .gap(px(tokens::SPACE_XS))
                .test_id("library-paths")
                .child(dynamic(move || path_rows(model, state))),
        )
        .child(
            view()
                .flex_row()
                .gap(px(tokens::SPACE_SM))
                .child(action_row(
                    state,
                    "library-rescan-all",
                    "Rescan all folders",
                    move || rescan_all(model),
                ))
                .child(action_row(
                    state,
                    "library-clear",
                    "Clear Library",
                    move || state.clear_confirm.set(true),
                )),
        )
        .child(dynamic(move || clear_band(model, state)))
        .child(status_line(state))
        .into_element()
}

/// The typed add field and its Add control (the N14 picker writes the same
/// field).
fn add_row(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let draft = state.library_add_draft;
    let write = draft;
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .test_id("library-add-row")
        .child(
            input()
                .value(draft)
                .on_change(move |text: &str| write.set(text.to_owned()))
                .placeholder("/path/to/music")
                .test_id("library-add-path")
                .label("Folder path")
                .flex_1()
                .text_sm()
                .p(px(tokens::SPACE_SM))
                .rounded(px(tokens::RADIUS_SM))
                .bg(move || palette.get().surface_2)
                .text_color(move || palette.get().ink),
        )
        .child(action_row(state, "library-add", "Add folder", move || {
            add_path(model, state)
        }))
        .child(action_row(
            state,
            "library-browse",
            "Browse\u{2026}",
            move || crate::platform::library_paths::pick_folders(model, state),
        ))
}

/// One row per registered path: path and readiness on the left, the watch
/// toggle and the two actions on the right.
fn path_rows(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let rows = state.library_paths.get();
    if rows.is_empty() {
        return text("No folders registered yet.")
            .text_xs()
            .text_color(move || {
                let palette = palette;
                palette.get().ink_3
            })
            .test_id("library-paths-empty")
            .into_element();
    }
    view()
        .flex_col()
        .gap(px(tokens::SPACE_SM))
        .children(rows.iter().map(|row| {
            let path = row.path.clone();
            let readiness = row.readiness.clone();
            let remove_path = path.clone();
            let rescan_path = path.clone();
            let watch_path = path.clone();
            let watching = row.watching;
            let warning = row.watch_warning.clone();
            view()
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_MD))
                .px(px(tokens::SPACE_SM))
                .py(px(tokens::SPACE_XS))
                .rounded(px(tokens::RADIUS_SM))
                .border(px(tokens::BORDER_HAIRLINE))
                .border_color(move || palette.get().line)
                .test_id("library-path")
                .child(
                    view()
                        .flex_1()
                        .flex_col()
                        .child(
                            text(move || path.display().to_string())
                                .text_sm()
                                .truncate()
                                .text_color(move || palette.get().ink),
                        )
                        .child(
                            text(move || match &warning {
                                Some(reason) => {
                                    format!("{readiness} \u{b7} watch failed: {reason}")
                                }
                                None => readiness.clone(),
                            })
                            .text_xs()
                            .text_color(move || palette.get().ink_3),
                        ),
                )
                .child(
                    view()
                        .on_press(move || set_watch(model, watch_path.clone(), !watching))
                        .px(px(tokens::SPACE_SM))
                        .py(px(tokens::SPACE_XS))
                        .rounded(px(tokens::RADIUS_SM))
                        .bg(move || {
                            if watching {
                                palette.get().brand_primary
                            } else {
                                palette.get().surface_2
                            }
                        })
                        .test_id("library-watch")
                        .label("Watch for changes")
                        .child(text(if watching {
                            "Watching"
                        } else {
                            "Watch for changes"
                        })
                        .text_xs()
                        .text_color(move || {
                            if watching {
                                palette.get().on_brand
                            } else {
                                palette.get().ink_2
                            }
                        })),
                )
                .child(action_row(state, "library-rescan", "Rescan", move || {
                    model.with(|m| m.rt.scans.request(rescan_path.clone()));
                }))
                .child(action_row(
                    state,
                    "library-remove",
                    "Remove",
                    move || remove_path_row(model, remove_path.clone()),
                ))
                .into_element()
        }))
        .into_element()
}

/// Clear Library's confirmation band.
fn clear_band(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    if !state.clear_confirm.get() {
        return view().into_element();
    }
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .p(px(tokens::SPACE_MD))
        .rounded(px(tokens::RADIUS_SM))
        .border(px(tokens::BORDER_HAIRLINE))
        .border_color(move || palette.get().error)
        .test_id("library-clear-band")
        .child(
            text("Clear the library? Tracks are removed; your playlists and settings stay.")
                .flex_1()
                .text_sm()
                .text_color(move || palette.get().ink_2),
        )
        .child(action_row(
            state,
            "library-clear-confirm",
            "Clear Library",
            move || clear_library(model, state),
        ))
        .child(action_row(
            state,
            "library-clear-cancel",
            "Cancel",
            move || state.clear_confirm.set(false),
        ))
        .into_element()
}

fn add_path(model: Stored<UiModel>, state: UiState) {
    let text = state.library_add_draft.get().trim().to_owned();
    if text.is_empty() {
        state
            .settings_status
            .set(Some("Enter a folder path".to_owned()));
        return;
    }
    let path = std::path::PathBuf::from(&text);
    if !path.is_dir() {
        state
            .settings_status
            .set(Some(format!("{} is not a folder", path.display())));
        return;
    }
    model.with(|m| {
        let mut library = m.rt.library.lock_or_recover();
        library
            .library_paths
            .register(path.clone(), m.rt.settings.as_mut());
    });
    state.library_add_draft.set(String::new());
    state.settings_status.set(Some(format!(
        "Added {}. Rescan it to index its tracks.",
        path.display()
    )));
}

fn remove_path_row(model: Stored<UiModel>, path: std::path::PathBuf) {
    model.with(|m| {
        let mut watcher = m.rt.watcher_manager.lock_or_recover();
        let mut library = m.rt.library.lock_or_recover();
        library.library_paths.retire(
            &path,
            &mut watcher,
            m.rt.settings.as_mut(),
            m.rt.library_mutations.as_mut(),
        );
    });
}

fn set_watch(model: Stored<UiModel>, path: std::path::PathBuf, watching: bool) {
    model.with(|m| {
        let mut watcher = m.rt.watcher_manager.lock_or_recover();
        let mut library = m.rt.library.lock_or_recover();
        library.library_paths.set_watch(
            &path,
            watching,
            &mut watcher,
            m.rt.settings.as_mut(),
        );
    });
}

fn rescan_all(model: Stored<UiModel>) {
    model.with(|m| {
        let paths = m
            .rt
            .library
            .lock_or_recover()
            .library_paths
            .paths()
            .to_vec();
        for path in paths {
            m.rt.scans.request(path);
        }
    });
}

fn clear_library(model: Stored<UiModel>, state: UiState) {
    let result = model.with(|m| {
        let removed = m.rt.library_mutations.clear_library();
        m.rt.library
            .lock_or_recover()
            .library_paths
            .clear_collection_data();
        removed
    });
    match result {
        Ok(removed) => {
            state
                .settings_status
                .set(Some(format!("Cleared the library: {removed} tracks removed")));
        }
        Err(e) => state.settings_status.set(Some(e.to_string())),
    }
    state.clear_confirm.set(false);
}
