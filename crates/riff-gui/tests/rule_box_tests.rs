//! The Rule Box draft model (Smart Playlists ticket 16): the tree around
//! ticket 14's rows, asserted with no egui at all.
//!
//! The draft is frontend-local state, so these tests pin it at its own
//! interface — the same placement ticket 14's pure half uses: a box holds its
//! children in one interleaved order; a path names a box; the store
//! conversion is faithful in both directions and refuses rather than drops.
//!
//! What is deliberately NOT asserted here: matching, storage and the store's
//! position renumbering (tickets 08/09 pin those against a real SQLite file),
//! and the paint (the pane's own harness and the golden do that).

use riff_backend::domain::{
    Combine, Comparison, MatchableField, RootBox, Rule, RuleBox, RuleChild, RuleValue,
};
use riff_gui::ui::rule_box::{BoxPath, ChildSlot, RuleBoxDraft, RuleChildDraft};
use riff_gui::ui::rule_row::{RuleDraft, RulePanel};

/// One whole Rule, as a draft: a field, a comparison and a value, in the one
/// shape [`RuleDraft::from_rule`] reads back.
fn filled_rule(field: MatchableField, text: &str) -> RuleDraft {
    RuleDraft::from_rule(&Rule::new(
        field,
        Comparison::Is,
        Some(RuleValue::Text(text.to_string())),
    ))
}

/// The field and value text of a fully-formed draft row, for order assertions.
fn spelled(draft: &RuleDraft) -> (MatchableField, String) {
    let rule = draft.rule().expect("a filled draft spells a whole Rule");
    let text = match rule.value() {
        Some(RuleValue::Text(text)) => text.clone(),
        other => panic!("expected a text value, got {other:?}"),
    };
    (rule.field(), text)
}

/// The children of the box a path names, panicking when the path is wrong.
fn children_at<'a>(root: &'a RuleBoxDraft, path: &[usize]) -> &'a Vec<RuleChildDraft> {
    path.iter()
        .fold(root, |boxed, index| match &boxed.children[*index] {
            RuleChildDraft::Box(nested) => nested,
            RuleChildDraft::Rule(_) => panic!("path index {index} names a Rule, not a Box"),
            RuleChildDraft::Unreadable(_) => {
                panic!("path index {index} names an Unreadable Rule, not a Box")
            }
        })
        .children
        .as_ref()
}

/// The box a slice of indices names, mutably — the test-side walk.
fn box_at_mut<'a>(root: &'a mut RuleBoxDraft, path: &[usize]) -> &'a mut RuleBoxDraft {
    path.iter()
        .fold(root, |boxed, index| match &mut boxed.children[*index] {
            RuleChildDraft::Box(nested) => nested,
            RuleChildDraft::Rule(_) => panic!("path index {index} names a Rule, not a Box"),
            RuleChildDraft::Unreadable(_) => {
                panic!("path index {index} names an Unreadable Rule, not a Box")
            }
        })
}

#[test]
fn rules_and_boxes_interleave_in_one_sibling_order() {
    let mut root = RuleBoxDraft::empty(Combine::All);
    root.add_rule(&BoxPath::root());
    root.add_box(&BoxPath::root());
    root.add_rule(&BoxPath::root());

    // One vector, in the order they were added — a Rule, a Box, a Rule.
    assert_eq!(root.children.len(), 3);
    assert!(matches!(root.children[0], RuleChildDraft::Rule(_)));
    assert!(matches!(root.children[1], RuleChildDraft::Box(_)));
    assert!(matches!(root.children[2], RuleChildDraft::Rule(_)));

    // A new Box holds one blank Rule, so adding a Box is a usable group and
    // not an empty husk the listener has to find the control to fill.
    let nested = children_at(&root, &[1]);
    assert_eq!(nested.len(), 1);
    assert!(matches!(nested[0], RuleChildDraft::Rule(_)));
}

#[test]
fn a_box_can_be_nested_in_a_box_and_each_offers_its_own_combine() {
    let mut root = RuleBoxDraft::empty(Combine::All);
    root.add_box(&BoxPath::root());
    let outer = BoxPath::root().child(0);
    // The new box appends after the blank Rule its own creation put first.
    let inner = outer.child(root.box_at(&outer).expect("the box exists").children.len());
    root.add_box(&outer);
    assert_eq!(inner, outer.child(1));

    root.set_combine(&outer, Combine::Any);
    // The deepest box is where a Rule added through a path lands.
    root.add_rule(&inner);

    assert_eq!(
        root.box_at(&BoxPath::root()).map(|b| b.combine),
        Some(Combine::All)
    );
    assert_eq!(root.box_at(&outer).map(|b| b.combine), Some(Combine::Any));
    assert_eq!(root.box_at(&inner).map(|b| b.combine), Some(Combine::All));
    // The inner box: one blank Rule from its own creation, one added here.
    assert_eq!(root.box_at(&inner).map(RuleBoxDraft::rule_count), Some(2));
    assert_eq!(
        root.rule_count(),
        3,
        "the whole tree counts every box's Rules"
    );
}

#[test]
fn removing_a_box_removes_everything_inside_it() {
    let mut root = RuleBoxDraft::empty(Combine::All);
    root.add_rule(&BoxPath::root());
    root.add_box(&BoxPath::root());
    let outer = BoxPath::root().child(1);
    let inner = outer.child(1);
    root.add_box(&outer);
    root.add_rule(&inner);
    root.add_rule(&inner);
    root.add_rule(&BoxPath::root());

    // The root's own Rule, the outer box's blank starting Rule, the inner
    // box's three Rules, and the last Rule added at the root.
    assert_eq!(root.rule_count(), 1 + 1 + 3 + 1);

    // Remove the outer box by its slot: (parent, index) — no emptying first.
    root.remove_child(&BoxPath::root(), 1);

    assert_eq!(
        root.children.len(),
        2,
        "the box and its whole subtree left the sibling list"
    );
    assert_eq!(root.rule_count(), 2);
    assert!(
        root.box_at(&outer).is_none(),
        "the removed subtree is gone, not orphaned"
    );
    assert!(matches!(root.children[0], RuleChildDraft::Rule(_)));
    assert!(matches!(root.children[1], RuleChildDraft::Rule(_)));
}

#[test]
fn removing_a_rule_leaves_its_siblings_positions_intact() {
    let mut root = RuleBoxDraft::empty(Combine::All);
    root.children = vec![
        RuleChildDraft::Rule(filled_rule(MatchableField::Artist, "a")),
        RuleChildDraft::Rule(filled_rule(MatchableField::Artist, "b")),
        RuleChildDraft::Rule(filled_rule(MatchableField::Artist, "c")),
    ];

    root.remove_child(&BoxPath::root(), 1);

    let texts: Vec<String> = match &root.children[..] {
        [RuleChildDraft::Rule(first), RuleChildDraft::Rule(second)] => {
            vec![spelled(first).1, spelled(second).1]
        }
        other => panic!("expected the two surviving Rules, got {other:?}"),
    };
    assert_eq!(texts, vec!["a".to_string(), "c".to_string()]);
}

#[test]
fn a_saved_tree_loads_into_the_draft_and_spells_back_identically() {
    // An interleaved stored tree: Rule, Box(Rule, Box(Rule), Rule), Rule.
    let stored = RootBox::new(RuleBox::new(Combine::Any).with_children(vec![
        RuleChild::Rule(Rule::new(
            MatchableField::Artist,
            Comparison::Is,
            Some(RuleValue::Text("first".into())),
        )),
        RuleChild::Box(RuleBox::new(Combine::All).with_children(vec![
            RuleChild::Rule(Rule::new(
                MatchableField::Album,
                Comparison::Contains,
                Some(RuleValue::Text("inner".into())),
            )),
            RuleChild::Box(
                RuleBox::new(Combine::Any).with_children(vec![RuleChild::Rule(Rule::new(
                    MatchableField::Favorite,
                    Comparison::Is,
                    Some(RuleValue::YesNo(true)),
                ))]),
            ),
            RuleChild::Rule(Rule::new(
                MatchableField::Year,
                Comparison::GreaterThan,
                Some(RuleValue::Number(1975)),
            )),
        ])),
        RuleChild::Rule(Rule::new(
            MatchableField::Title,
            Comparison::Contains,
            Some(RuleValue::Text("last".into())),
        )),
    ]));

    let draft = RuleBoxDraft::from_root_box(&stored);

    // The draft spells the same tree back: order, nesting and combine all
    // round-trip, which is what "a saved tree reads back the same way" means
    // on this side of the store seam.
    assert_eq!(draft.to_root_box().as_ref(), Some(&stored));
    assert_eq!(draft.combine, Combine::Any);
    assert_eq!(
        draft.rule_count(),
        5,
        "artist, album, favourite, year, title"
    );
    let inner = BoxPath::root().child(1).child(1);
    assert_eq!(draft.box_at(&inner).map(|b| b.combine), Some(Combine::Any));
}

#[test]
fn a_half_filled_row_makes_the_conversion_refuse_rather_than_drop() {
    let mut root = RuleBoxDraft::empty(Combine::All);
    root.add_rule(&BoxPath::root());
    root.add_box(&BoxPath::root());

    // Both the blank Rule and the new Box's blank Rule are half-filled, so
    // there is no honest tree to spell — and a silent skip would rewrite the
    // listener's work on the next Save.
    assert_eq!(root.to_root_box(), None);

    // Filling the root's row is not enough while the box's stays blank.
    root.children[0] = RuleChildDraft::Rule(filled_rule(MatchableField::Artist, "rock"));
    assert_eq!(root.to_root_box(), None);

    // Filling both rows spells the tree.
    box_at_mut(&mut root, &[1]).children[0] =
        RuleChildDraft::Rule(filled_rule(MatchableField::Artist, "metal"));
    assert!(root.to_root_box().is_some());
}

#[test]
fn an_empty_box_converts_as_empty_and_is_not_dropped() {
    // "A box with nothing in it blocks the save" is ticket 18's refusal to
    // make; the model's duty is to stay honest and keep the box visible so
    // that refusal can name it.
    let mut root = RuleBoxDraft::empty(Combine::Any);
    root.children
        .push(RuleChildDraft::Box(RuleBoxDraft::empty(Combine::Any)));

    let tree = root
        .to_root_box()
        .expect("an empty box is a representable tree");
    assert_eq!(tree.children().len(), 1);
    let RuleChild::Box(empty) = &tree.children()[0] else {
        panic!("the empty box survives conversion as a Box");
    };
    assert!(empty.is_empty());
}

#[test]
fn the_editor_imposes_no_limit_on_the_number_of_rules() {
    let mut root = RuleBoxDraft::empty(Combine::All);
    for index in 0..60 {
        root.children.push(RuleChildDraft::Rule(filled_rule(
            MatchableField::Artist,
            &format!("artist {index}"),
        )));
    }

    let tree = root
        .to_root_box()
        .expect("sixty whole Rules spell a whole tree");
    assert_eq!(tree.rule_count(), 60);
    assert_eq!(tree.children().len(), 60, "no row cap, no folding away");
}

#[test]
fn the_seven_level_cap_gates_the_offer_and_nothing_else() {
    // A level is a Rule Box; the Root Box is level 1, so a path holds at
    // most seven boxes and adding a Box further down is never offered.
    assert_eq!(BoxPath::root().level(), 1);
    assert!(BoxPath::root().accepts_a_box());

    let mut root = RuleBoxDraft::empty(Combine::All);
    let mut path = BoxPath::root();
    for level in 2..=7 {
        assert!(
            path.accepts_a_box(),
            "a Box at level {} still offers the add-box control",
            level - 1
        );
        // The appended Box lands after the blank Rule every new Box starts
        // with, so the run follows the sibling list's own length.
        let next_index = root
            .box_at(&path)
            .expect("the path the loop built resolves")
            .children
            .len();
        root.add_box(&path);
        path = path.child(next_index);
        // Fill the blank Rule the new Box starts with, so depth is the only
        // thing under test.
        match &mut root.box_at_mut(&path).expect("the new box").children[0] {
            RuleChildDraft::Rule(draft) => {
                *draft = filled_rule(MatchableField::Artist, &format!("level {level}"));
            }
            RuleChildDraft::Box(_) | RuleChildDraft::Unreadable(_) => {
                panic!("a new Box starts with a Rule")
            }
        }
        assert_eq!(path.level(), level);
    }
    assert!(
        !path.accepts_a_box(),
        "the seventh level holds the deepest Box: the add-box control is absent"
    );
    // The cap gates the offer only — the tree it built is seven levels deep
    // and still spellable, and Rules are still addable at the deepest level.
    root.box_at_mut(&path)
        .expect("the deepest box resolves")
        .children
        .push(RuleChildDraft::Rule(filled_rule(
            MatchableField::Artist,
            "deep",
        )));
    assert_eq!(
        root.rule_count(),
        7,
        "one whole Rule in each of the six nested boxes plus the deepest"
    );
    assert!(root.to_root_box().is_some());
}

#[test]
fn a_tree_deeper_than_the_cap_loads_and_spells_back_untouched() {
    // The cap is an editor rule, not a save or compile rule: a tree a newer
    // riff wrote reads back and answers, so loading it must not flatten,
    // refuse or renumber anything.
    let mut boxed = RuleBox::new(Combine::Any).with_children(vec![RuleChild::Rule(Rule::new(
        MatchableField::Artist,
        Comparison::Is,
        Some(RuleValue::Text("deepest".into())),
    ))]);
    let mut depth = 1;
    while depth < 9 {
        boxed = RuleBox::new(Combine::All).with_children(vec![RuleChild::Box(boxed)]);
        depth += 1;
    }
    let stored = RootBox::new(boxed);

    let draft = RuleBoxDraft::from_root_box(&stored);
    let mut path = BoxPath::root();
    for _ in 0..8 {
        path = path.child(0);
    }
    assert_eq!(path.level(), 9, "nine boxes on the deepest path");
    assert_eq!(draft.box_at(&path).map(RuleBoxDraft::rule_count), Some(1));
    assert_eq!(draft.to_root_box().as_ref(), Some(&stored));
}

#[test]
fn the_picker_tidy_up_closes_every_other_rows_value_panel() {
    let mut root = RuleBoxDraft::empty(Combine::All);
    root.add_rule(&BoxPath::root());
    root.add_box(&BoxPath::root());
    let nested = BoxPath::root().child(1);
    root.add_rule(&nested);

    // Two rows (one at the root, one inside the box) hold open library
    // pickers; the pane can only show one list's values, so opening the
    // nested row's list must close the root row's.
    match &mut root.children[0] {
        RuleChildDraft::Rule(draft) => draft.open = Some(RulePanel::Value),
        RuleChildDraft::Box(_) | RuleChildDraft::Unreadable(_) => {
            panic!("the root's first child is a Rule")
        }
    }
    match &mut box_at_mut(&mut root, &[1]).children[0] {
        RuleChildDraft::Rule(draft) => draft.open = Some(RulePanel::Value),
        RuleChildDraft::Box(_) | RuleChildDraft::Unreadable(_) => {
            panic!("the box's first child is a Rule")
        }
    }

    let keep = ChildSlot::new(nested, 0);
    root.close_value_panels_except(&keep);

    let RuleChildDraft::Rule(first) = &root.children[0] else {
        panic!("the root's first child is a Rule");
    };
    assert_eq!(first.open, None, "the root row's picker closed");
    let RuleChildDraft::Rule(kept) = &box_at_mut(&mut root, &[1]).children[0] else {
        panic!("the box's first child is a Rule");
    };
    assert_eq!(
        kept.open,
        Some(RulePanel::Value),
        "the row that reported the picker keeps its list open"
    );
}
