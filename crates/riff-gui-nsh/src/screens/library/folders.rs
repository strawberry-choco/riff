//! The Folders browse mode's tree: the egui `render_folder_tree` /
//! `render_folder_node` behavior on the nsh element tree. Roots are the
//! session's registered paths (the pump's mirror — a render block cannot
//! lock the session); a node renders when the store says its folder holds
//! audio, pruned under a query by the same store probe the egui node asked;
//! a click toggles the expansion AND selects the folder, exactly the egui
//! row's gesture set; the children read only when a folder is open — lazy,
//! one level per expansion — with the tracks directly inside listed under
//! them, filtered by the needle the way the egui `folder_tracks_filtered`
//! matched on the track's own search text.
//!
//! Expansion state survives a stage switch (the signal outlives the tree,
//! the egui `CollapsingState`'s persistence). The egui scroll memory kept no
//! Folders slot and neither does this stage: the tree is not a virtual
//! list, so there is no row address to restore — recorded in the trail as
//! the deliberate gap.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nsh_core::prelude::*;
use riff_backend::app::MutexExt;
use riff_backend::domain::Track;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::components::empty_state;
use crate::components::track_row::{TrackRowView, row_of};
use crate::model::reads;
use crate::screens::inspector;
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// The node indent: one spacing step per tree level — the token scale
/// stepped, never a raw literal.
fn indented(depth: usize) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a tree depth is small; the f32 indent loses nothing"
    )]
    let depth = depth as f32;
    depth * tokens::SPACE_MD
}

/// The stage: the tree beside the inspector, or the no-roots empty state
/// (the egui tree's own copy — the fixture store carries no folder roots,
/// so this is its usual live shape until N14's picker).
pub fn stage(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let roots = state.folder_roots;
    view()
        .flex_1()
        .flex_row()
        .test_id("folders-stage")
        .bg(move || palette.get().surface_row)
        .child(
            view()
                .flex_1()
                .flex_col()
                .test_id("folders-body")
                .p(px(tokens::SPACE_LG))
                .child(
                    when(
                        move || roots.get().is_empty(),
                        move || {
                            empty_state::state(
                                palette,
                                "No folders yet",
                                "Add a folder from the sidebar to start scanning your library.",
                            )
                            .into_element()
                        },
                    )
                    .otherwise(move || tree(model, state).into_element()),
                ),
        )
        .child(inspector::panel(model, state))
}

/// The tree: one scrolling region whose rows the dynamic rebuilds when the
/// expansion set, the selection, the needle or the Library generation
/// moves — the egui per-frame node walk, run on moves instead of frames.
fn tree(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    scroll()
        .flex_1()
        .flex_col()
        .test_id("folder-tree")
        .child(dynamic(move || {
            let query = state.search.get();
            let _ = state.library_revision.get();
            let mut rows: Vec<AnyElement> = Vec::new();
            for root in state.folder_roots.get().iter() {
                node_rows(model, &state, root.as_path(), 0, &query, &mut rows);
            }
            view()
                .flex_col()
                .gap(px(tokens::SPACE_XS))
                .children(rows)
                .into_element()
        }))
        .bg(move || palette.get().surface_row)
}

/// One node's rows: the folder row itself, and — only when it is open — its
/// children (read here, at expansion time: the lazy read is what keeps a
/// deep tree cheap) and the tracks directly inside it.
fn node_rows(
    model: Stored<UiModel>,
    state: &UiState,
    path: &Path,
    depth: usize,
    query: &str,
    rows: &mut Vec<AnyElement>,
) {
    let (has_audio, matches) = model.with(|m| {
        (
            reads::folder_has_audio(&mut m.rt.session_views, path),
            query.is_empty() || reads::folder_search_match(&mut m.rt.session_views, path, query),
        )
    });
    if !has_audio || !matches {
        return;
    }
    let open = state.folder_open.get().contains(path);
    rows.push(folder_row(model, *state, path, depth, open).into_element());
    if !open {
        return;
    }
    let children = model.with(|m| reads::folder_children(&mut m.rt.session_views, path));
    for child in children.iter() {
        node_rows(model, state, child, depth + 1, query, rows);
    }
    let tracks = model.with(|m| reads::folder_direct_tracks(&mut m.rt.session_views, path));
    let needle = query.to_lowercase();
    for track in tracks
        .iter()
        .filter(|track| needle.is_empty() || track.search_text.contains(needle.as_str()))
    {
        rows.push(folder_track_row(model, *state, track, depth + 1).into_element());
    }
}

/// One folder row: the glyph per state (closed/open), the folder's own
/// name, the selection band while it is the selected one. A press toggles
/// the expansion and selects the folder — the egui row's same write set
/// (`selected_folder` through the guard, the signal mirror outside it).
fn folder_row(
    model: Stored<UiModel>,
    state: UiState,
    path: &Path,
    depth: usize,
    open: bool,
) -> impl IntoElement {
    let palette = state.palette;
    let label = path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let path_buf = path.to_path_buf();
    let press_path = path_buf.clone();
    let menu_path = path_buf.clone();
    let selected_path = state.folder_selected;
    let selected_target = path_buf.clone();
    let selected = move || selected_path.get().as_deref() == Some(selected_target.as_path());
    let selected_a11y = selected.clone();
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .pl(px(indented(depth)))
        .pr(px(tokens::SPACE_SM))
        .py(px(tokens::SPACE_XS))
        .rounded(px(tokens::RADIUS_SM))
        .test_id("folder-row")
        .label(label.clone())
        .accessibility_state(move || AccessibilityState {
            selected: selected_a11y(),
            ..AccessibilityState::default()
        })
        .bg(move || {
            if selected() {
                palette.get().surface_3
            } else {
                transparent_black()
            }
        })
        .hover(|s| s.bg(move || palette.get().row_hover))
        .on_press(move || toggle_folder(model, press_path.clone()))
        .on_context_menu(move |event: &ContextMenuEvent| {
            crate::menus::collection::open_collection_menu(
                model,
                crate::menus::CollectionTarget::Folder(menu_path.clone()),
                event.page_position,
            );
        })
        .child(icon(
            if open { Icon::FolderOpen } else { Icon::Folder },
            Icon::ICON_SIZE,
            move || palette.get().ink_2,
        ))
        .child(
            text(label)
                .text_sm()
                .truncate()
                .text_color(move || palette.get().ink_2)
                .test_id("folder-label"),
        )
}

/// One folder's direct track: the shared track row's anatomy at the node's
/// indent, press-selecting like every track row does. The traversal's list
/// segment is the track stages' own, so these rows take a never-set focus
/// handle.
fn folder_track_row(
    model: Stored<UiModel>,
    state: UiState,
    track: &Track,
    depth: usize,
) -> impl IntoElement {
    let track = track.clone();
    let list_focus = signal(None::<usize>);
    view().pl(px(indented(depth))).child(row_of(
        model,
        state,
        move || track_view(&track),
        list_focus,
    ))
}

/// The track row's display facts, straight off the store row — the folder
/// listing carries only scanned-and-present rows, so `missing` fills
/// `false` like the flat listing's.
fn track_view(track: &Track) -> TrackRowView {
    TrackRowView {
        index: 0,
        id: track.id.clone(),
        title: track.metadata.display_title(&track.file_path),
        artist: track.metadata.display_artist(),
        album: track.metadata.display_album().into_owned(),
        duration: track.duration,
        favorite: track.favorite,
        missing: false,
    }
}

/// The folder row's press: toggle the expansion (the set signal is the
/// tree's rebuild trigger) and write the selection — the session field
/// through the guard, the signal mirror outside the borrow.
fn toggle_folder(model: Stored<UiModel>, path: PathBuf) {
    let open = model.with(|m| m.state.folder_open);
    let selected = model.with(|m| m.state.folder_selected);
    model.with(|m| {
        m.rt.library.lock_or_recover().selected_folder = Some(path.clone());
    });
    open.update(|set| {
        let set = Arc::make_mut(set);
        if set.contains(&path) {
            set.remove(&path);
        } else {
            set.insert(path.clone());
        }
    });
    selected.set(Some(path));
}
