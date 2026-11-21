//! The Custom Smart Playlist editor: the rule tree, its name, the live
//! match count, and the save gate.
//!
//! The draft lives in the bridge signals. The tree's structure rebuilds on
//! a revision bump (add/remove/cycle), while a text field's keystrokes only
//! update the draft, so typing never loses its focus. Save asks the draft
//! for a whole tree first: an unreadable stored row, an empty tree or a
//! half-filled row refuses with the egui editor's own sentence before any
//! store write.

use nsh_base::input;
use nsh_core::prelude::*;
use riff_backend::app::state::TrackSort;
use riff_backend::app::store::SmartPlaylistTree;
use riff_backend::domain::Combine;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::screens::smart_playlists::rules::{
    self, BoxDraft, ChildDraft, RuleDraft, SmartDraft, MAX_RULE_BOX_LEVELS,
    UNREADABLE_ROW_WORDS,
};
use crate::theme::icons::{Icon, icon};
use crate::theme::tokens;

/// Start a fresh draft and open the editor.
pub fn create_new(model: Stored<UiModel>, state: UiState) {
    state.smart_draft.set(Some(SmartDraft::new()));
    state.smart_saved.set(None);
    state.smart_editing.set(true);
    state.smart_status.set(None);
    state.smart_confirm_discard.set(false);
    state.smart_draft_rev.update(|rev| *rev += 1);
    let _ = model;
}

/// Load a saved list into the draft and open the editor.
pub fn edit_list(model: Stored<UiModel>, state: UiState, id: riff_backend::domain::PlaylistId) {
    let name = state
        .custom_lists
        .get()
        .iter()
        .find(|row| row.id == id)
        .map_or_else(|| id.0.clone(), |row| row.name.clone());
    let tree = model.with(|m| m.rt.session_views.custom_list_tree(&id).ok().flatten());
    let Some(tree) = tree else {
        state
            .smart_status
            .set(Some("This list could not be read".to_owned()));
        return;
    };
    let draft = SmartDraft::of_list(id, name, &tree);
    state.smart_draft.set(Some(draft.clone()));
    state.smart_saved.set(Some(draft));
    state.smart_editing.set(true);
    state.smart_status.set(None);
    state.smart_confirm_discard.set(false);
    state.smart_draft_rev.update(|rev| *rev += 1);
}

/// The editor screen.
pub fn screen(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let rev = state.smart_draft_rev;
    view()
        .flex_1()
        .flex_col()
        .test_id("smart-editor")
        .bg(move || palette.get().surface_row)
        .child(header(model, state))
        .child(
            scroll()
                .flex_1()
                .flex_col()
                .p(px(tokens::SPACE_LG))
                .gap(px(tokens::SPACE_MD))
                .child(dynamic(move || {
                    let _ = rev.get();
                    let Some(draft) = state.smart_draft.get() else {
                        return view().into_element();
                    };
                    view()
                        .flex_col()
                        .gap(px(tokens::SPACE_MD))
                        .children(render_box(model, state, &draft.root, Vec::new(), 0))
                        .into_element()
                }))
                .child(unreadable(model, state))
                .child(count_line(model, state))
                .child(dynamic(move || status_line(state))),
        )
        .child(discard_band(model, state))
}

/// The editor header: name field, Save, Back.
fn header(model: Stored<UiModel>, state: UiState) -> impl IntoElement {
    let palette = state.palette;
    let name = state
        .smart_draft
        .get()
        .map_or_else(String::new, |draft| draft.name);
    let name_signal = signal(name);
    let write = name_signal;
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_SM))
        .px(px(tokens::SPACE_LG))
        .py(px(tokens::SPACE_MD))
        .test_id("smart-editor-header")
        .child(
            text("SMART LIST")
                .text_xs()
                .text_color(move || palette.get().ink_3),
        )
        .child(
            input()
                .value(name_signal)
                .on_change(move |text: &str| {
                    write.set(text.to_owned());
                    state.smart_draft.update(|draft| {
                        if let Some(draft) = draft {
                            text.clone_into(&mut draft.name);
                        }
                    });
                })
                .placeholder("List name")
                .test_id("smart-name")
                .label("List name")
                .text_sm()
                .w(px(tokens::COLUMN_MIN_W))
                .p(px(tokens::SPACE_SM))
                .rounded(px(tokens::RADIUS_SM))
                .bg(move || palette.get().surface_2)
                .text_color(move || palette.get().ink),
        )
        .child(view().flex_1())
        .child(button_like(palette, "smart-save", "Save list", "Save", move || {
            save(model, state)
        }))
        .child(button_like(
            palette,
            "smart-back",
            "Back to the lists",
            "Back",
            move || back(model, state),
        ))
}

fn button_like(
    palette: Signal<crate::theme::Palette>,
    id: &'static str,
    label: &'static str,
    text_label: &'static str,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .on_press(on_press)
        .test_id(id)
        .label(label)
        .px(px(tokens::SPACE_MD))
        .py(px(tokens::SPACE_XS))
        .rounded(px(tokens::RADIUS_SM))
        .hover(|s| s.bg(move || palette.get().row_hover))
        .child(
            text(text_label)
                .text_xs()
                .text_color(move || palette.get().ink_2),
        )
}

/// One box: the combine toggle, its rows, and its add/remove controls.
fn render_box(
    model: Stored<UiModel>,
    state: UiState,
    draft: &BoxDraft,
    path: Vec<usize>,
    depth: usize,
) -> Vec<AnyElement> {
    let palette = state.palette;
    let mut rows = Vec::new();
    let is_root = path.is_empty();
    rows.push(
        view()
            .flex_row()
            .items_center()
            .gap(px(tokens::SPACE_SM))
            .pl(px(tokens::SPACE_MD * u32::try_from(depth).unwrap_or(0) as f32))
            .test_id("smart-box")
            .child(
                text("MATCH")
                    .text_xs()
                    .text_color(move || palette.get().ink_3),
            )
            .child(button_like(
                palette,
                "smart-combine",
                "Toggle all or any",
                rules::combine_label(draft.combine),
                {
                    let path = path.clone();
                    move || toggle_combine(model, state, &path)
                },
            ))
            .child(button_like(
                palette,
                "smart-add-rule",
                "Add rule",
                "+ Rule",
                {
                    let path = path.clone();
                    move || add_rule(model, state, &path)
                },
            ))
            .children(
                (depth + 1 < MAX_RULE_BOX_LEVELS).then(|| {
                    button_like(palette, "smart-add-group", "Add group", "+ Group", {
                        let path = path.clone();
                        move || add_group(model, state, &path)
                    })
                }),
            )
            .children((!is_root).then(|| {
                icon_button(palette, "smart-remove-group", "Remove group", Icon::Trash, {
                    let path = path.clone();
                    move || remove_child(model, state, &path)
                })
            }))
            .into_element(),
    );
    for (at, child) in draft.children.iter().enumerate() {
        match child {
            ChildDraft::Rule(rule) => {
                rows.push(rule_row(model, state, rule, path.clone(), at));
            }
            ChildDraft::Box(inner) => {
                let mut child_path = path.clone();
                child_path.push(at);
                rows.extend(render_box(model, state, inner, child_path, depth + 1));
            }
        }
    }
    rows
}

/// One rule row: field, comparison, value, negation, remove.
fn rule_row(
    model: Stored<UiModel>,
    state: UiState,
    rule: &RuleDraft,
    box_path: Vec<usize>,
    at: usize,
) -> AnyElement {
    let palette = state.palette;
    let field = rule.field;
    let comparison = rule.comparison;
    let negated = rule.negated;
    let value = signal(rule.value.clone());
    let write = value;
    let path = box_path.clone();
    let negation_path = box_path.clone();
    let hint = rules::value_hint(field, comparison);
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_XS))
        .pl(px(tokens::SPACE_MD))
        .test_id("smart-rule")
        .child(button_like(
            palette,
            "smart-rule-field",
            "Change field",
            field_display(field),
            {
                let path = path.clone();
                move || cycle_field(model, state, &path, at)
            },
        ))
        .child(button_like(
            palette,
            "smart-rule-comparison",
            "Change comparison",
            rules::comparison_label(comparison),
            {
                let path = path.clone();
                move || cycle_comparison(model, state, &path, at)
            },
        ))
        .child(
            input()
                .value(value)
                .on_change({
                    let path = path.clone();
                    move |text: &str| {
                        write.set(text.to_owned());
                        set_rule(model, state, &path, at, |rule| {
                            text.clone_into(&mut rule.value);
                        });
                    }
                })
                .placeholder(hint)
                .test_id("smart-rule-value")
                .label("Rule value")
                .text_sm()
                .w(px(tokens::COLUMN_MIN_W))
                .p(px(tokens::SPACE_XS))
                .rounded(px(tokens::RADIUS_SM))
                .bg(move || palette.get().surface_2)
                .text_color(move || palette.get().ink),
        )
        .child(button_like(
            palette,
            "smart-rule-negate",
            "Invert this rule",
            if negated { "not" } else { "-" },
            move || {
                let path = negation_path.clone();
                set_rule(model, state, &path, at, |rule| rule.negated = !rule.negated);
            },
        ))
        .child(icon_button(
            palette,
            "smart-rule-remove",
            "Remove rule",
            Icon::Close,
            {
                let path = path.clone();
                move || remove_child(model, state, &path)
            },
        ))
        .into_element()
}

fn field_display(field: riff_backend::domain::MatchableField) -> &'static str {
    riff_backend::domain::spec(field).display_name()
}

/// The unreadable rows: shown with the spec's words, each removable, and a
/// save blocker until gone.
fn unreadable(_model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(draft) = state.smart_draft.get() else {
        return view().into_element();
    };
    if draft.unreadable.is_empty() {
        return view().into_element();
    }
    view()
        .flex_col()
        .gap(px(tokens::SPACE_XS))
        .px(px(tokens::SPACE_LG))
        .test_id("smart-unreadable")
        .child(
            text(UNREADABLE_ROW_WORDS)
                .text_xs()
                .text_color(move || palette.get().error),
        )
        .children((0..draft.unreadable.len()).map(|at| {
            view()
                .flex_row()
                .items_center()
                .gap(px(tokens::SPACE_SM))
                .child(
                    text(move || {
                        state
                            .smart_draft
                            .get()
                            .and_then(|draft| draft.unreadable.get(at).map(|row| {
                                format!("{} {}", row.field_id(), row.operator_id())
                            }))
                            .unwrap_or_default()
                    })
                    .text_xs()
                    .text_color(move || palette.get().ink_3),
                )
                .child(icon_button(
                    palette,
                    "smart-unreadable-remove",
                    "Delete this unreadable rule",
                    Icon::Trash,
                    move || {
                        state.smart_draft.update(|draft| {
                            if let Some(draft) = draft
                                && at < draft.unreadable.len()
                            {
                                draft.unreadable.remove(at);
                            }
                        });
                        state.smart_draft_rev.update(|rev| *rev += 1);
                    },
                ))
                .into_element()
        }))
        .into_element()
}

/// The live count line, or why no count can be shown.
fn count_line(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    let line = model.with(|m| {
        let Some(draft) = state.smart_draft.get() else {
            return "0 songs".to_owned();
        };
        match draft.to_tree() {
            Ok(tree) => match m.rt.session_views.custom_list_match_count(&SmartPlaylistTree::new(tree)) {
                Ok(total) if total == 1 => "1 song".to_owned(),
                Ok(total) => format!("{total} songs"),
                Err(_) => "The count could not be read".to_owned(),
            },
            Err(reasons) => reasons.first().cloned().unwrap_or_default(),
        }
    });
    text(move || line.clone())
        .text_xs()
        .text_color(move || palette.get().ink_3)
        .test_id("smart-count")
        .into_element()
}

fn status_line(state: UiState) -> AnyElement {
    let palette = state.palette;
    let Some(status) = state.smart_status.get() else {
        return view().into_element();
    };
    text(move || status.clone())
        .text_xs()
        .text_color(move || palette.get().error)
        .test_id("smart-status")
        .into_element()
}

/// The unsaved guard: Back with a dirty draft asks before discarding.
fn discard_band(model: Stored<UiModel>, state: UiState) -> AnyElement {
    let palette = state.palette;
    if !state.smart_confirm_discard.get() {
        return view().into_element();
    }
    view()
        .flex_row()
        .items_center()
        .gap(px(tokens::SPACE_MD))
        .px(px(tokens::SPACE_LG))
        .py(px(tokens::SPACE_MD))
        .test_id("smart-discard-band")
        .child(
            text("Discard unsaved changes?")
                .flex_1()
                .text_sm()
                .text_color(move || palette.get().ink_2),
        )
        .child(button_like(
            palette,
            "smart-discard",
            "Discard changes",
            "Discard",
            move || close(model, state),
        ))
        .child(button_like(
            palette,
            "smart-keep-editing",
            "Keep editing",
            "Keep editing",
            move || state.smart_confirm_discard.set(false),
        ))
        .into_element()
}

fn icon_button(
    palette: Signal<crate::theme::Palette>,
    id: &'static str,
    label: &'static str,
    glyph: Icon,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .on_press(on_press)
        .test_id(id)
        .label(label)
        .p(px(tokens::SPACE_XS))
        .rounded(px(tokens::RADIUS_SM))
        .hover(|s| s.bg(move || palette.get().row_hover))
        .child(icon(glyph, Icon::ICON_SIZE, move || palette.get().ink_3))
}

// --- Draft mutation helpers ------------------------------------------------

fn box_mut<'a>(root: &'a mut BoxDraft, path: &[usize]) -> Option<&'a mut BoxDraft> {
    let mut current = root;
    for at in path {
        match current.children.get_mut(*at)? {
            ChildDraft::Box(inner) => current = inner,
            ChildDraft::Rule(_) => return None,
        }
    }
    Some(current)
}

fn add_rule(model: Stored<UiModel>, state: UiState, path: &[usize]) {
    state.smart_draft.update(|draft| {
        if let Some(draft) = draft
            && let Some(boxed) = box_mut(&mut draft.root, path)
        {
            boxed.children.push(ChildDraft::Rule(RuleDraft {
                field: riff_backend::domain::MatchableField::Artist,
                comparison: riff_backend::domain::Comparison::Is,
                negated: false,
                value: String::new(),
            }));
        }
    });
    state.smart_draft_rev.update(|rev| *rev += 1);
    let _ = model;
}

fn add_group(model: Stored<UiModel>, state: UiState, path: &[usize]) {
    state.smart_draft.update(|draft| {
        if let Some(draft) = draft
            && let Some(boxed) = box_mut(&mut draft.root, path)
        {
            boxed.children.push(ChildDraft::Box(BoxDraft {
                combine: Combine::All,
                children: vec![ChildDraft::Rule(RuleDraft {
                    field: riff_backend::domain::MatchableField::Artist,
                    comparison: riff_backend::domain::Comparison::Is,
                    negated: false,
                    value: String::new(),
                })],
            }));
        }
    });
    state.smart_draft_rev.update(|rev| *rev += 1);
    let _ = model;
}

/// Remove the child at `path` (the last index addresses the child in its
/// parent box).
fn remove_child(model: Stored<UiModel>, state: UiState, path: &[usize]) {
    let Some((&at, parent)) = path.split_last() else {
        return;
    };
    state.smart_draft.update(|draft| {
        if let Some(draft) = draft
            && let Some(boxed) = box_mut(&mut draft.root, parent)
            && at < boxed.children.len()
        {
            boxed.children.remove(at);
        }
    });
    state.smart_draft_rev.update(|rev| *rev += 1);
    let _ = model;
}

fn toggle_combine(model: Stored<UiModel>, state: UiState, path: &[usize]) {
    state.smart_draft.update(|draft| {
        if let Some(draft) = draft
            && let Some(boxed) = box_mut(&mut draft.root, path)
        {
            boxed.combine = match boxed.combine {
                Combine::All => Combine::Any,
                Combine::Any => Combine::All,
            };
        }
    });
    state.smart_draft_rev.update(|rev| *rev += 1);
    let _ = model;
}

fn set_rule(
    model: Stored<UiModel>,
    state: UiState,
    box_path: &[usize],
    at: usize,
    edit: impl FnOnce(&mut RuleDraft),
) {
    state.smart_draft.update(|draft| {
        if let Some(draft) = draft
            && let Some(boxed) = box_mut(&mut draft.root, box_path)
            && let Some(ChildDraft::Rule(rule)) = boxed.children.get_mut(at)
        {
            edit(rule);
        }
    });
    let _ = model;
}

fn cycle_field(model: Stored<UiModel>, state: UiState, box_path: &[usize], at: usize) {
    set_rule(model, state, box_path, at, |rule| {
        rule.field = rules::next_field(rule.field);
        rule.comparison = rules::next_comparison(rule.field, rule.comparison);
    });
    state.smart_draft_rev.update(|rev| *rev += 1);
}

fn cycle_comparison(model: Stored<UiModel>, state: UiState, box_path: &[usize], at: usize) {
    set_rule(model, state, box_path, at, |rule| {
        rule.comparison = rules::next_comparison(rule.field, rule.comparison);
    });
    state.smart_draft_rev.update(|rev| *rev += 1);
}

// --- Save and navigation ----------------------------------------------------

fn save(model: Stored<UiModel>, state: UiState) {
    let Some(draft) = state.smart_draft.get() else {
        return;
    };
    let name = draft.name.trim().to_owned();
    if name.is_empty() {
        state
            .smart_status
            .set(Some("Enter a list name".to_owned()));
        return;
    }
    if let Some(reason) =
        riff_backend::app::playlist_name::builtin_smart_list_name_conflict(&name)
    {
        state.smart_status.set(Some(reason));
        return;
    }
    let tree = match draft.to_tree() {
        Ok(tree) => SmartPlaylistTree::new(tree),
        Err(reasons) => {
            state.smart_status.set(Some(reasons.join("; ")));
            return;
        }
    };
    let saved = model.with(|m| {
        m.rt.library_mutations
            .save_smart_playlist(draft.id.as_ref(), &name, &tree)
    });
    match saved {
        Ok(id) => {
            state.smart_status.set(None);
            state.smart_editing.set(false);
            state.smart_confirm_discard.set(false);
            open_list(model, state, id);
        }
        Err(e) => state.smart_status.set(Some(e.to_string())),
    }
}

fn back(model: Stored<UiModel>, state: UiState) {
    let dirty = state
        .smart_draft
        .get()
        .zip(state.smart_saved.get())
        .is_some_and(|(draft, saved)| draft.differs_from(&saved));
    if dirty || state.smart_saved.get().is_none() {
        state.smart_confirm_discard.set(true);
        return;
    }
    close(model, state);
}

fn close(model: Stored<UiModel>, state: UiState) {
    state.smart_editing.set(false);
    state.smart_confirm_discard.set(false);
    state.smart_status.set(None);
    state.smart_draft.set(None);
    let _ = model;
}

/// Navigate to a saved list: the nav slots and the pump's open custom list.
pub fn open_list(
    model: Stored<UiModel>,
    state: UiState,
    id: riff_backend::domain::PlaylistId,
) {
    let nav = state.nav;
    let mut view = nav.get();
    view.custom_list = Some(id.clone());
    view.playlist = None;
    view.smart_list = None;
    nav.set(view);
    model.with(|m| {
        m.open_custom_list = Some(id.clone());
        m.open_playlist = None;
        m.open_smart_list = None;
    });
}

/// Delete the open custom list after its confirmation.
pub fn delete_open(model: Stored<UiModel>, state: UiState) {
    let Some(id) = state.nav.get().custom_list else {
        return;
    };
    match model.with(|m| m.rt.library_mutations.delete_smart_playlist(&id)) {
        Ok(_) => {
            let nav = state.nav;
            let mut view = nav.get();
            view.custom_list = None;
            nav.set(view);
            model.with(|m| m.open_custom_list = None);
            state.smart_delete_confirm.set(false);
            state.custom_list_total.set(0);
        }
        Err(e) => state.smart_status.set(Some(e.to_string())),
    }
}

/// The ids of the open custom list, for its Play/Shuffle actions.
#[must_use]
pub fn open_list_ids(model: &mut UiModel, id: &riff_backend::domain::PlaylistId) -> Vec<riff_backend::domain::TrackId> {
    let total = model
        .rt
        .session_views
        .custom_list_count(id, TrackSort::NumberAsc, None)
        .unwrap_or_default();
    (0..total)
        .filter_map(|at| {
            model
                .rt
                .session_views
                .custom_list_row(id, TrackSort::NumberAsc, None, at)
                .ok()
                .flatten()
        })
        .map(|track| track.id.clone())
        .collect()
}
