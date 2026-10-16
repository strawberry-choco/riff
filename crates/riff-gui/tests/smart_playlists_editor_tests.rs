//! The Smart Playlists editor's own suite (Smart Playlists ticket 18): the
//! draft's shape, its derived dirtiness, the one reconcile hook, and every
//! decision Save makes before a store is asked.
//!
//! Everything here is at the editor's public interface and needs no egui at
//! all — the module is pure state and walks, which is why the draft, the
//! Unreadable Rule's stored position, the refusal ladder and the reconcile can
//! be pinned without a rendered frame. The rendered half (Save and Cancel as
//! typed intents, each reason painted under the row that caused it) is the
//! workspace-root UI seam's (`tests/ui_tests.rs::tests`).

use riff_backend::app::store::{CustomSmartPlaylist, SmartPlaylistTree, UnreadableRule};
use riff_backend::domain::{
    Combine, Comparison, MatchableField, PlaylistId, RootBox, Rule, RuleBox, RuleChild, RuleValue,
};
use riff_gui::ui::rule_box::{BoxPath, RuleChildDraft};
use riff_gui::ui::rule_row::{RuleDraft, RuleIntent};
use riff_gui::ui::smart_playlists_page::{SmartPlaylistRefusal, SmartPlaylistsEditor};

// --- Fixtures ------------------------------------------------------------------

/// One saved-list row as the D14 enumeration answers it.
fn list(id: &str, name: &str) -> CustomSmartPlaylist {
    CustomSmartPlaylist {
        id: PlaylistId::new(id),
        name: name.to_string(),
        created_at: std::time::SystemTime::UNIX_EPOCH,
    }
}

/// A whole Rule for `field`, spelled through `RuleDraft::from_rule` so no test
/// reaches past the row's own constructors.
fn whole_rule(field: MatchableField) -> RuleDraft {
    RuleDraft::from_rule(&Rule::new(
        field,
        Comparison::Contains,
        Some(RuleValue::Text("anything".to_string())),
    ))
}

/// A stored tree: one Rule in the Root Box, then a nested any-of Box holding a
/// Rule and a Rule Box holding another Rule. Typed children come back dense
/// from a store read; the helpers below add the Unreadable rows.
fn nested_tree() -> SmartPlaylistTree {
    let root = RootBox::new(RuleBox::new(Combine::All).with_children(vec![
        RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Is,
            Some(RuleValue::Text("midnight".to_string())),
        )),
        RuleChild::Box(RuleBox::new(Combine::Any).with_children(vec![
            RuleChild::Rule(Rule::new(
                MatchableField::Year,
                Comparison::GreaterThan,
                Some(RuleValue::Number(1975)),
            )),
            RuleChild::Box(
                RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(Rule::new(
                    MatchableField::PlayCount,
                    Comparison::GreaterThan,
                    Some(RuleValue::Number(10)),
                ))]),
            ),
        ])),
    ]));
    SmartPlaylistTree::new(root)
}

/// An Unreadable Rule row: a `field_id` no build knows, stored where it hung.
fn unreadable(box_path: Vec<usize>, position: usize) -> UnreadableRule {
    UnreadableRule::new(
        box_path,
        position,
        "mood".to_string(),
        "is".to_string(),
        false,
        Some("calm".to_string()),
    )
}

/// The editor holding `tree`, opened through the production load.
fn editor_holding(tree: &SmartPlaylistTree) -> SmartPlaylistsEditor {
    let mut editor = SmartPlaylistsEditor::default();
    editor.load_saved(&list("spl-late-night-2", "Late night jazz"), Some(tree));
    editor
}

/// The store-boundary type of a draft tree that loaded without unreadable
/// rows, for comparing what the editor holds against what was stored.
fn whole_tree_of(editor: &SmartPlaylistsEditor) -> RootBox {
    editor
        .root
        .to_root_box()
        .expect("a tree loaded from the store spells back")
}

// --- The draft's shape and derived dirtiness ------------------------------------

/// A list opened from the store is not dirty by construction, and each kind of
/// edit makes it dirty — while putting the edit back makes it clean again,
/// which is what "derived by comparing" means and what a `dirty` flag could
/// not promise.
#[test]
fn a_freshly_opened_list_is_clean_and_every_edit_is_derived() {
    let tree = nested_tree();
    let mut editor = editor_holding(&tree);
    assert!(
        !editor.is_dirty(),
        "nothing is dirty by construction when a list is opened"
    );

    editor.name = "Late Night".to_string();
    assert!(editor.is_dirty(), "the name buffer changed");
    editor.name = "Late night jazz".to_string();
    assert!(!editor.is_dirty(), "and putting it back is clean again");

    let mut root = editor.root.clone();
    root.set_combine(&BoxPath::root(), Combine::Any);
    editor.root = root;
    assert!(
        editor.is_dirty(),
        "the Root Box's combine is part of the tree"
    );
}

/// The default draft is a new, empty, unsaved list: one blank Rule row, no
/// name, nothing selected — and it is clean until the listener changes
/// something.
#[test]
fn a_new_draft_starts_clean_with_one_blank_rule() {
    let mut editor = SmartPlaylistsEditor::default();
    assert!(editor.is_new());
    assert_eq!(
        editor.root.children.len(),
        1,
        "one blank Rule row is ready to write"
    );
    assert!(
        editor.root.children.iter().all(|child| matches!(
            child,
            RuleChildDraft::Rule(draft) if draft.gap().is_some()
        )),
        "the row is blank, not a Rule: it has nothing spellable yet"
    );
    assert!(!editor.is_dirty());
    editor.add_rule(&BoxPath::root());
    assert!(
        editor.is_dirty(),
        "the listener put a second row in, so the draft has something to save"
    );
}

/// A loaded tree spells back exactly what was stored — the round-trip the
/// store's own read and this draft's Save share.
#[test]
fn a_loaded_tree_spells_back_the_tree_that_was_stored() {
    let tree = nested_tree();
    let editor = editor_holding(&tree);
    assert_eq!(
        &whole_tree_of(&editor),
        tree.root(),
        "nesting, order and every value survive the load"
    );
}

// --- Switching lists is a whole-draft replacement -------------------------------

/// Picking another list replaces the draft — the previous draft's edits are
/// gone, nothing selected survives, and the new one is clean.
#[test]
fn opening_another_list_replaces_the_draft_wholesale() {
    let tree = nested_tree();
    let mut editor = editor_holding(&tree);
    editor.name = "half-typed".to_string();
    editor.remove_child(&BoxPath::root(), 0);
    assert!(editor.is_dirty());

    let next = list("spl-morning-1", "Morning drive");
    editor.load_saved(&next, Some(&tree));
    assert_eq!(editor.name, "Morning drive");
    assert_eq!(editor.selected.as_ref(), Some(&next.id));
    assert!(!editor.is_dirty(), "the new list opens clean");
    assert_eq!(editor.root.children.len(), 2, "with its own whole tree");
}

// --- The Unreadable Rule: shown, preserved, at its stored position --------------

/// An Unreadable Rule lands where it was stored — and producing that place is
/// **not** an index match: `box_path` counts box children only, while the
/// editor's siblings interleave Rules among the boxes. Two Rules ahead of the
/// first Box means the stored `box_path[0]` is the editor's interleaved
/// index 2.
#[test]
fn an_unreadable_rule_lands_inside_the_box_a_box_only_path_names() {
    // The Root Box: Rule (stored 0), Rule (1), Box (2). The Unreadable Rule
    // sits in the Box at box-only path [0].
    let root = RootBox::new(RuleBox::new(Combine::All).with_children(vec![
        RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Is,
            Some(RuleValue::Text("a".to_string())),
        )),
        RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Is,
            Some(RuleValue::Text("b".to_string())),
        )),
        RuleChild::Box(
            RuleBox::new(Combine::Any).with_children(vec![RuleChild::Rule(Rule::new(
                MatchableField::Year,
                Comparison::GreaterThan,
                Some(RuleValue::Number(1975)),
            ))]),
        ),
    ]));
    let tree = SmartPlaylistTree::new(root).with_unreadable(vec![unreadable(vec![0], 0)]);
    let editor = editor_holding(&tree);

    assert!(
        matches!(editor.root.children[0], RuleChildDraft::Rule(_)),
        "the root's own children are untouched"
    );
    let RuleChildDraft::Box(nested) = &editor.root.children[2] else {
        panic!("the box is the root's third child");
    };
    assert!(
        matches!(nested.children[0], RuleChildDraft::Unreadable(_)),
        "the unreadable row sits inside the box the box-only path names, \
         not at the root's index 0"
    );
    assert!(
        matches!(nested.children[1], RuleChildDraft::Rule(_)),
        "and the box's own Rule follows it in the stored order"
    );
}

/// Two Unreadable Rules in one box, with a stored hole between them: the
/// read's typed children come back dense, so each stored position is shifted
/// by the unreadable rows already put back before it.
#[test]
fn unreadable_rules_keep_the_stored_gaps_around_the_typed_children() {
    // Stored children 0..3: typed, unreadable, typed, unreadable.
    let root = RootBox::new(RuleBox::new(Combine::All).with_children(vec![
        RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Is,
            Some(RuleValue::Text("first".to_string())),
        )),
        RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Is,
            Some(RuleValue::Text("second".to_string())),
        )),
    ]));
    let tree = SmartPlaylistTree::new(root)
        .with_unreadable(vec![unreadable(Vec::new(), 3), unreadable(Vec::new(), 1)]);
    let editor = editor_holding(&tree);

    let shape: Vec<&str> = editor
        .root
        .children
        .iter()
        .map(|child| match child {
            RuleChildDraft::Rule(_) => "rule",
            RuleChildDraft::Box(_) => "box",
            RuleChildDraft::Unreadable(_) => "unreadable",
        })
        .collect();
    assert_eq!(
        shape,
        vec!["rule", "unreadable", "rule", "unreadable"],
        "the stored numbering 0..3 is rebuilt exactly, whatever order the \
         side list arrived in"
    );
}

/// An Unreadable Rule is never counted as a Rule the way the store's own count
/// is not, and it blocks the tree from spelling at all — which is what makes
/// Save refuse instead of the store's `CHECK` list failing opaquely.
#[test]
fn an_unreadable_rule_is_not_a_rule_and_spells_no_tree() {
    let root = RootBox::new(
        RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Contains,
            Some(RuleValue::Text("x".to_string())),
        ))]),
    );
    let tree = SmartPlaylistTree::new(root).with_unreadable(vec![unreadable(Vec::new(), 1)]);
    let editor = editor_holding(&tree);
    assert_eq!(editor.root.rule_count(), 1);
    assert!(
        editor.root.to_root_box().is_none(),
        "a tree holding one spells no tree for the store"
    );
}

// --- The one reconcile hook -----------------------------------------------------

/// The reconcile decides the draft's right to exist in one place: the page
/// closing discards it, a saved list that no longer exists discards it, and
/// while the page is open with the list still there it stays untouched.
#[test]
fn the_reconcile_drops_the_draft_only_where_it_no_longer_belongs() {
    let tree = nested_tree();
    let saved = list("spl-late-night-2", "Late night jazz");

    let mut editor = editor_holding(&tree);
    editor.name = "half-typed".to_string();
    editor.reconcile(true, std::slice::from_ref(&saved));
    assert_eq!(
        editor.name, "half-typed",
        "an open page and a list that still exists keep the draft, dirt and all"
    );

    editor.reconcile(false, &[]);
    assert!(editor.is_new(), "leaving the page discards the whole draft");
    assert_eq!(
        editor,
        SmartPlaylistsEditor::default(),
        "what it leaves behind is the fresh new-list draft, not a half-state"
    );

    let mut editor = editor_holding(&tree);
    editor.reconcile(true, &[]);
    assert!(
        editor.is_new(),
        "a draft whose list was deleted elsewhere cannot outlive it"
    );
    assert_eq!(editor.name, "");
}

/// A new, unsaved draft belongs to the page: the reconcile does not touch it
/// while the page is open, however empty it is.
#[test]
fn a_new_draft_survives_the_reconcile_while_the_page_is_open() {
    let mut editor = SmartPlaylistsEditor {
        name: "Unfinished".to_string(),
        ..SmartPlaylistsEditor::default()
    };
    editor.reconcile(true, &[]);
    assert_eq!(editor.name, "Unfinished");
    assert!(editor.is_new(), "still nothing saved");
}

/// A row renamed elsewhere (the sidebar's own Rename) is followed by the
/// draft: the stored name is adopted — the draft is clean again against it,
/// and the Rules in hand are untouched, because a rename writes no rule row —
/// while a name buffer the listener has typed into wins, being the name the
/// page shows and the one Save writes.
#[test]
fn the_reconcile_adopts_a_renamed_rows_name_only_over_an_untouched_buffer() {
    let tree = nested_tree();
    let renamed = list("spl-late-night-2", "Late night jazz deep");

    // Untouched: the draft follows the store's name and re-stamps its own
    // baseline with it, and the tree is the one it was opened from.
    let mut editor = editor_holding(&tree);
    assert!(!editor.is_dirty());
    editor.reconcile(true, std::slice::from_ref(&renamed));
    assert_eq!(editor.name, "Late night jazz deep");
    assert_eq!(editor.original.name, "Late night jazz deep");
    assert!(
        !editor.is_dirty(),
        "adopting the stored name re-stamps the baseline, so nothing is pending"
    );
    assert_eq!(
        &whole_tree_of(&editor),
        tree.root(),
        "the listener's Rules are not re-read: a rename touches no rule row"
    );

    // Typed: their own name is left alone — it is what the page shows, and
    // overwriting it here would silently edit the draft under their hands.
    let mut editor = editor_holding(&tree);
    editor.name = "Typed by the listener".to_string();
    editor.reconcile(true, std::slice::from_ref(&renamed));
    assert_eq!(editor.name, "Typed by the listener");

    // A rename over an edited tree: the name is adopted and the work in
    // progress stays where it was.
    let mut editor = editor_holding(&tree);
    editor.add_rule(&BoxPath::root());
    editor.reconcile(true, std::slice::from_ref(&renamed));
    assert_eq!(editor.name, "Late night jazz deep");
    assert!(editor.is_dirty(), "the tree edit is still unsaved");
    assert_eq!(
        editor.root.children.len(),
        3,
        "the Rules written so far are still in the draft"
    );
}

// --- Save: the refusals, each naming its cause ----------------------------------

/// Save on a draft the listener has not finished refuses, and the refusal
/// names the row and what it is missing — never "a rule is incomplete".
#[test]
fn save_names_the_half_filled_row_and_what_is_missing() {
    let mut editor = SmartPlaylistsEditor {
        name: "Morning".to_string(),
        ..SmartPlaylistsEditor::default()
    };
    // The default draft's one row is blank: no field at all.
    assert!(editor.prepare_save().is_none());
    assert_eq!(
        editor.refusal,
        Some(SmartPlaylistRefusal::Row {
            slot: riff_gui::ui::rule_box::ChildSlot::new(BoxPath::root(), 0),
            reason: "this rule needs a field".to_string(),
        })
    );

    // A field, no comparison.
    editor.root.children[0] = RuleChildDraft::Rule({
        let mut draft = RuleDraft::blank();
        draft.apply(&RuleIntent::FieldChosen {
            field: MatchableField::Genre,
        });
        draft
    });
    assert!(editor.prepare_save().is_none());
    assert_eq!(
        editor.refusal,
        Some(SmartPlaylistRefusal::Row {
            slot: riff_gui::ui::rule_box::ChildSlot::new(BoxPath::root(), 0),
            reason: "this rule needs a comparison".to_string(),
        })
    );

    // A whole field/comparison waiting on its value.
    let mut draft = RuleDraft::blank();
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Genre,
    });
    draft.apply(&RuleIntent::ComparisonChosen {
        choice: riff_gui::ui::rule_row::ComparisonChoice::Stored(Comparison::Is),
    });
    editor.root.children[0] = RuleChildDraft::Rule(draft);
    assert!(editor.prepare_save().is_none());
    assert_eq!(
        editor.refusal,
        Some(SmartPlaylistRefusal::Row {
            slot: riff_gui::ui::rule_box::ChildSlot::new(BoxPath::root(), 0),
            reason: "this rule needs a value".to_string(),
        })
    );
}

/// The reason names the **offending** row, not the first row: a complete rule
/// ahead of the blank one leaves the refusal on the blank one's slot.
#[test]
fn the_refusal_points_at_the_row_that_is_missing_something() {
    let mut editor = SmartPlaylistsEditor {
        name: "Morning".to_string(),
        ..SmartPlaylistsEditor::default()
    };
    editor.root.children[0] = RuleChildDraft::Rule(whole_rule(MatchableField::Title));
    editor.add_rule(&BoxPath::root()); // a second, blank row

    assert!(editor.prepare_save().is_none());
    assert_eq!(
        editor.refusal,
        Some(SmartPlaylistRefusal::Row {
            slot: riff_gui::ui::rule_box::ChildSlot::new(BoxPath::root(), 1),
            reason: "this rule needs a field".to_string(),
        })
    );

    // Removing the offending row lets the same Save through.
    editor.remove_child(&BoxPath::root(), 1);
    let plan = editor.prepare_save().expect("every row is whole now");
    assert_eq!(plan.name, "Morning");
    assert!(plan.id.is_none(), "a new list creates its row");
    assert_eq!(plan.tree.rule_count(), 1);
    assert!(
        editor.refusal.is_none(),
        "the reason is cleared by the pass"
    );
}

/// An Unreadable Rule blocks Save before any other check, points at its own
/// row, and survives: the draft keeps the row until the listener deletes it,
/// and then the same Save succeeds.
#[test]
fn an_unreadable_rule_blocks_save_is_pointed_at_and_survives_deletion() {
    let root = RootBox::new(
        RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Contains,
            Some(RuleValue::Text("x".to_string())),
        ))]),
    );
    let tree = SmartPlaylistTree::new(root).with_unreadable(vec![unreadable(Vec::new(), 1)]);
    let mut editor = editor_holding(&tree);

    assert!(editor.prepare_save().is_none());
    assert_eq!(
        editor.refusal,
        Some(SmartPlaylistRefusal::Row {
            slot: riff_gui::ui::rule_box::ChildSlot::new(BoxPath::root(), 1),
            reason: "riff can't understand this rule — delete the row to save the list".to_string(),
        }),
        "the reason names the rule the way its own row does"
    );
    assert!(
        matches!(editor.root.children[1], RuleChildDraft::Unreadable(_)),
        "and the row is still there, preserved rather than dropped"
    );

    // A half-filled Rule elsewhere does not take the blame.
    editor.add_rule(&BoxPath::root());
    assert!(editor.prepare_save().is_none());
    assert!(
        matches!(
            editor.refusal,
            Some(SmartPlaylistRefusal::Row { slot: _, reason: _ })
        ),
        "still a row refusal"
    );

    // The row's own delete control is the way out.
    editor.remove_child(&BoxPath::root(), 1);
    editor.remove_child(&BoxPath::root(), 1); // the blank row added above
    let plan = editor.prepare_save().expect("the unreadable row is gone");
    assert_eq!(plan.tree.rule_count(), 1);
    assert_eq!(
        plan.tree.root(),
        &RootBox::new(
            RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(Rule::new(
                MatchableField::Title,
                Comparison::Contains,
                Some(RuleValue::Text("x".to_string())),
            ),)])
        ),
        "the rest of the tree is what gets written"
    );
}

/// The name is refused before the store is ever asked: empty after trimming,
/// or one of the six Built-in Smart Playlists' names, folded the way the
/// store's own rule folds them.
#[test]
fn save_refuses_a_missing_or_builtin_name() {
    let mut editor = SmartPlaylistsEditor::default();
    editor.root.children[0] = RuleChildDraft::Rule(whole_rule(MatchableField::Title));

    editor.name = "   ".to_string();
    assert!(editor.prepare_save().is_none());
    assert_eq!(
        editor.refusal,
        Some(SmartPlaylistRefusal::Name(
            "this list needs a name".to_string()
        ))
    );

    for (typed, builtin) in [
        ("most played", "Most Played"),
        ("  RECENTLY ADDED ", "Recently Added"),
        ("favorites", "Favorites"),
        ("lost gems", "Lost Gems"),
        ("never played", "Never Played"),
        ("recently played", "Recently Played"),
    ] {
        editor.name = typed.to_string();
        assert!(
            editor.prepare_save().is_none(),
            "{typed:?} must not be given to a sidebar row"
        );
        assert_eq!(
            editor.refusal,
            Some(SmartPlaylistRefusal::Name(format!(
                "a Built-in Smart Playlist is already named {builtin:?}"
            ))),
            "the refusal quotes the built-in's own spelling"
        );
    }

    // A name of the listener's own passes all of that.
    editor.name = "  Morning drive  ".to_string();
    let plan = editor.prepare_save().expect("a free name saves");
    assert_eq!(
        plan.name, "Morning drive",
        "trimmed on the way to the store"
    );
}

/// Save hands the store the whole tree, and the name the listener typed:
/// nothing reaches a store before this call, and this call carries everything
/// at once (D12).
#[test]
fn save_prepares_the_name_and_the_whole_tree_in_one_plan() {
    let tree = nested_tree();
    let saved = list("spl-late-night-2", "Late night jazz");
    let mut editor = SmartPlaylistsEditor::default();
    editor.load_saved(&saved, Some(&tree));
    editor.name = "Late night".to_string();
    let plan = editor.prepare_save().expect("a loaded tree is whole");
    assert_eq!(plan.id.as_ref(), Some(&saved.id));
    assert_eq!(plan.name, "Late night");
    assert_eq!(plan.tree.root(), tree.root(), "the whole tree, unchanged");
}

/// A committed Save re-stamps the draft: the saved list stays open under its
/// own id, the name buffer is what the store now holds, and nothing is dirty.
#[test]
fn a_committed_save_keeps_the_draft_open_and_clean() {
    let mut editor = SmartPlaylistsEditor {
        name: "  Morning drive  ".to_string(),
        ..SmartPlaylistsEditor::default()
    };
    editor.root.children[0] = RuleChildDraft::Rule(whole_rule(MatchableField::Title));
    let plan = editor.prepare_save().expect("the draft is whole");
    assert!(plan.id.is_none());

    let id = PlaylistId::new("spl-morning-1");
    editor.mark_saved(id.clone());
    assert_eq!(editor.selected, Some(id), "the listener stays on this list");
    assert_eq!(
        editor.name, "Morning drive",
        "trimmed, as the store holds it"
    );
    assert!(!editor.is_dirty(), "a saved draft has nothing unsaved left");
    assert!(editor.refusal.is_none());
}

/// Cancel discards whatever was typed, and so does the reconcile's leaving
/// case: one end state, reached by one call.
#[test]
fn cancel_discards_the_whole_draft() {
    let tree = nested_tree();
    let mut editor = editor_holding(&tree);
    editor.name = "half-typed".to_string();
    editor.remove_child(&BoxPath::root(), 0);
    editor.cancel();
    assert!(editor.is_new());
    assert_eq!(editor.name, "");
    assert_eq!(
        editor,
        SmartPlaylistsEditor::default(),
        "Cancel and leaving the page reach the same end state"
    );
}

/// The store's refusal is rendered exactly as the store phrased it — the
/// double prefix is pre-existing shape and is not reworded here.
#[test]
fn the_stores_refusal_is_carried_verbatim() {
    let mut editor = SmartPlaylistsEditor::default();
    let text = "Invalid operation: store query failed: a Playlist or Custom Smart \
                Playlist named \"Gym\" already exists"
        .to_string();
    editor.refuse_save(text.clone());
    assert_eq!(editor.refusal, Some(SmartPlaylistRefusal::Save(text)));
}

// --- The live match line's committed tree (ticket 17) ---------------------------
//
// What the line counts is the draft's **committed** tree: every Rule row
// spelled from the answers a commit landed, never from the live buffers. The
// distinction is the whole cadence — a keystroke edits a buffer, so a shape
// derived from buffers would move (and re-query) on every keystroke. The
// blocks reuse ticket 18's own ladder and sentences, so the line and a
// refused Save teach the same words.

/// A row the listener has committed through the row's own intents, so its
/// buffers and its committed answer agree exactly as a real Enter leaves
/// them.
fn committed_rule(field: MatchableField, comparison: Comparison, value: RuleValue) -> RuleDraft {
    let mut draft = RuleDraft::blank();
    draft.apply(&RuleIntent::FieldChosen { field });
    draft.apply(&RuleIntent::ComparisonChosen {
        choice: riff_gui::ui::rule_row::ComparisonChoice::Stored(comparison),
    });
    draft.apply(&RuleIntent::ValueCommitted { value });
    draft
}

/// The `Title is "alpha"` tree one committed root row spells.
fn one_title_rule_tree(value: &str) -> SmartPlaylistTree {
    SmartPlaylistTree::new(RootBox::new(RuleBox::new(Combine::All).with_children(
        vec![RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Is,
            Some(RuleValue::Text(value.to_string())),
        ))],
    )))
}

/// A loaded list is wholly committed, so the tree the line counts is the
/// tree that was stored — boxes, order and values included.
#[test]
fn the_committed_tree_of_a_loaded_draft_is_the_stored_tree() {
    let tree = nested_tree();
    let editor = editor_holding(&tree);
    assert_eq!(
        editor.committed_tree().expect("a loaded draft is whole"),
        tree,
        "the loaded values are committed answers, so the derived tree is the stored one"
    );
}

/// A half-filled row is not counted: the line reports the block, naming what
/// the row is still missing — the same sentence Save refuses with, in Save's
/// reading order.
#[test]
fn a_half_filled_row_blocks_the_count_and_names_what_is_missing() {
    let mut editor = SmartPlaylistsEditor::default();
    assert_eq!(
        editor
            .committed_tree()
            .expect_err("the blank row has no field"),
        "this rule needs a field"
    );

    let mut draft = RuleDraft::blank();
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Genre,
    });
    editor.root.children[0] = RuleChildDraft::Rule(draft);
    assert_eq!(
        editor.committed_tree().expect_err("no comparison chosen"),
        "this rule needs a comparison"
    );

    let mut draft = RuleDraft::blank();
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Genre,
    });
    draft.apply(&RuleIntent::ComparisonChosen {
        choice: riff_gui::ui::rule_row::ComparisonChoice::Stored(Comparison::Is),
    });
    editor.root.children[0] = RuleChildDraft::Rule(draft);
    assert_eq!(
        editor.committed_tree().expect_err("no value committed"),
        "this rule needs a value"
    );
}

/// An Unreadable Rule blocks the count ahead of a half-filled one — ticket
/// 18's ladder order, and the same sentence Save uses — and the draft cannot
/// be counted at all while it holds one.
#[test]
fn an_unreadable_row_blocks_the_count_ahead_of_a_half_filled_one() {
    let root = RootBox::new(
        RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Is,
            Some(RuleValue::Text("kept".to_string())),
        ))]),
    );
    let tree = SmartPlaylistTree::new(root).with_unreadable(vec![unreadable(Vec::new(), 1)]);
    let mut editor = editor_holding(&tree);
    editor.add_rule(&BoxPath::root()); // a blank, half-filled row too

    assert_eq!(
        editor
            .committed_tree()
            .expect_err("the unreadable row blocks"),
        "riff can't understand this rule — delete the row to save the list",
        "the block names the unreadable row first, as Save does"
    );

    // The row's own delete control is the way out, and the count follows.
    editor.remove_child(&BoxPath::root(), 1);
    editor.remove_child(&BoxPath::root(), 1);
    assert_eq!(
        editor.committed_tree().expect("whole again"),
        one_title_rule_tree("kept")
    );
}

/// Typing edits a row's buffer and nothing else: the committed tree — the
/// shape the count is keyed on — does not move until the value is committed
/// on Enter or blur, which is what keeps a keystroke from re-querying.
#[test]
fn typing_a_value_changes_no_committed_answer() {
    let mut editor = SmartPlaylistsEditor::default();
    editor.root.children[0] = RuleChildDraft::Rule(committed_rule(
        MatchableField::Title,
        Comparison::Is,
        RuleValue::Text("rock".to_string()),
    ));
    let counted = editor.committed_tree().expect("the row is whole");
    assert_eq!(counted, one_title_rule_tree("rock"));

    // What typing does to the row: the buffer changes, the answer does not.
    let RuleChildDraft::Rule(draft) = &mut editor.root.children[0] else {
        panic!("the row is a Rule");
    };
    draft.value = riff_gui::ui::rule_row::RuleValueDraft::Text("jazz".to_string());
    assert_eq!(
        editor.committed_tree().expect("still whole"),
        counted,
        "a keystroke lands in the buffer only, so the counted shape is unchanged"
    );

    // Committing the typed value is what moves the shape.
    let RuleChildDraft::Rule(draft) = &mut editor.root.children[0] else {
        panic!("the row is a Rule");
    };
    draft.apply(&RuleIntent::ValueCommitted {
        value: RuleValue::Text("jazz".to_string()),
    });
    assert_eq!(
        editor.committed_tree().expect("the commit lands it"),
        one_title_rule_tree("jazz"),
        "Enter or blur is what the line counts"
    );
}

/// Committing one value, then another, then the first one back spells the
/// first tree again — byte for byte, which is what lets the seam serve the
/// held count instead of querying a third time.
#[test]
fn committing_the_previous_value_back_spells_the_previous_tree() {
    let mut editor = SmartPlaylistsEditor::default();
    let RuleChildDraft::Rule(draft) = &mut editor.root.children[0] else {
        panic!("the default draft's row is a Rule");
    };
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Title,
    });
    draft.apply(&RuleIntent::ComparisonChosen {
        choice: riff_gui::ui::rule_row::ComparisonChoice::Stored(Comparison::Is),
    });

    let spell = |editor: &mut SmartPlaylistsEditor, value: &str| {
        let RuleChildDraft::Rule(draft) = &mut editor.root.children[0] else {
            panic!("the row is a Rule");
        };
        draft.apply(&RuleIntent::ValueCommitted {
            value: RuleValue::Text(value.to_string()),
        });
        editor.committed_tree().expect("every row is whole")
    };

    let rock = spell(&mut editor, "rock");
    let jazz = spell(&mut editor, "jazz");
    let back = spell(&mut editor, "rock");
    assert_ne!(rock, jazz);
    assert_eq!(
        back, rock,
        "the earlier value's tree is spelled again, so the seam's shape key hits"
    );
}

/// The line's own sentences: the count folds into a sentence, and a block or
/// a failure is shown exactly as it was given — never as a number, and never
/// as "0 songs".
#[test]
fn the_match_line_says_the_count_or_the_reason_but_never_zero() {
    use riff_gui::ui::smart_playlists_page::SmartPlaylistMatchLine;

    assert_eq!(
        SmartPlaylistMatchLine::Matches(0).text(),
        "Matches 0 songs",
        "a counted empty list is an honest zero"
    );
    assert_eq!(SmartPlaylistMatchLine::Matches(1).text(), "Matches 1 song");
    assert_eq!(
        SmartPlaylistMatchLine::Matches(42).text(),
        "Matches 42 songs"
    );
    assert_eq!(
        SmartPlaylistMatchLine::Uncountable("this rule needs a value".to_string()).text(),
        "this rule needs a value",
        "the block is reported verbatim"
    );
    assert_eq!(
        SmartPlaylistMatchLine::Failed(
            "Invalid operation: cannot match this Custom Smart \
            Playlist: a Rule Box holds no Rule, so there is nothing to match"
                .to_string()
        )
        .text(),
        "Invalid operation: cannot match this Custom Smart Playlist: a Rule Box holds no Rule, \
         so there is nothing to match",
        "a failure is the store's own words — a reason, never a number"
    );
}

/// A replaced draft carries no answer from the draft it replaced: the line
/// is whatever the next refresh derives from the draft in hand.
#[test]
fn a_reloaded_draft_starts_with_no_held_match_line() {
    use riff_gui::ui::smart_playlists_page::SmartPlaylistMatchLine;

    let tree = nested_tree();
    let mut editor = editor_holding(&tree);
    editor.match_line = Some(SmartPlaylistMatchLine::Matches(3));
    editor.load_saved(&list("spl-other-1", "Other"), Some(&tree));
    assert_eq!(
        editor.match_line, None,
        "switching lists drops the previous draft's answer with the draft"
    );
}
