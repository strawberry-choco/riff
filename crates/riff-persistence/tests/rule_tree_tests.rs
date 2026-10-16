// riff-persistence — the Rule tree's shape (ticket 01).
//
// A Rule is a field, a comparison, an optional negation and one value; a Rule
// Box says whether all of it or any of it must match and holds Rules and Boxes
// in one shared sibling order; a Custom Smart Playlist has exactly one Root
// Box. None of them carries an identity: ids are the store's to generate and
// are never handed to the frontend, so a draft can never hold one that goes
// stale — which is also why two trees compare by shape alone.

use riff_persistence::field_catalog::ValueType;
use riff_persistence::rule::Combine;
use riff_persistence::rule::Comparison;
use riff_persistence::rule::MatchableField;
use riff_persistence::rule::RootBox;
use riff_persistence::rule::Rule;
use riff_persistence::rule::RuleBox;
use riff_persistence::rule::RuleChild;
use riff_persistence::rule::RuleValue;

fn text_rule(field: MatchableField, value: &str) -> Rule {
    Rule::new(
        field,
        Comparison::Is,
        Some(RuleValue::Text(value.to_string())),
    )
}

fn count_rule() -> Rule {
    Rule::new(
        MatchableField::Year,
        Comparison::GreaterThan,
        Some(RuleValue::Number(1975)),
    )
}

#[test]
fn a_rule_names_one_field_one_comparison_and_one_value() {
    let rule = count_rule();
    assert_eq!(rule.field(), MatchableField::Year);
    assert_eq!(rule.comparison(), Comparison::GreaterThan);
    assert_eq!(rule.value(), Some(&RuleValue::Number(1975)));
    assert!(
        !rule.is_negated(),
        "a plain Rule asserts the test, not its inverse"
    );

    let inverted = rule.clone().negated();
    assert!(inverted.is_negated());
    assert_eq!(
        inverted.comparison(),
        Comparison::GreaterThan,
        "negating a Rule leaves its stored comparison alone — the inverse is the flag's job"
    );
    assert_eq!(inverted.field(), rule.field());
    assert_eq!(inverted.value(), rule.value());
}

#[test]
fn is_not_missing_is_spelled_as_a_negated_is_missing_and_nothing_else() {
    let missing = Rule::new(MatchableField::Genre, Comparison::IsMissing, None);
    assert!(
        missing.is_fully_formed(),
        "`is missing` needs no value: the stored column is NULL"
    );
    let not_missing = missing.clone().negated();
    assert_eq!(
        not_missing.comparison(),
        Comparison::IsMissing,
        "the editor's \"is not missing\" wording has no operator of its own to land on"
    );
    assert!(
        not_missing.is_negated(),
        "and the negation is what carries the inverse meaning"
    );
    assert_ne!(
        missing, not_missing,
        "the two are different trees, so \"has this tree changed?\" still sees the edit"
    );
}

#[test]
fn a_rule_is_formed_exactly_as_the_catalog_declares() {
    // Completeness is a catalog question, so it is asked of the catalog: which
    // comparisons a field permits and which of them draw a value widget. A
    // later ticket names the offending half of a draft row with this.
    for field in MatchableField::ALL {
        for comparison in Comparison::ALL {
            let row = riff_persistence::field_catalog::spec(field);
            let without_value = Rule::new(field, comparison, None);
            if !row.permits(comparison) {
                assert!(
                    !without_value.is_fully_formed(),
                    "{field:?} against {} is not a legal Rule",
                    comparison.stored_id()
                );
            } else if row.value_widgets(comparison).is_empty() {
                assert!(
                    without_value.is_fully_formed(),
                    "{field:?} against {} needs no value and has none",
                    comparison.stored_id()
                );
            } else {
                assert!(
                    !without_value.is_fully_formed(),
                    "{field:?} against {} draws a value widget, so a Rule without one is \
                     half-filled",
                    comparison.stored_id()
                );
            }
        }
    }

    assert!(text_rule(MatchableField::Artist, "hook").is_fully_formed());
    assert!(
        !Rule::new(
            MatchableField::Artist,
            Comparison::Is,
            Some(RuleValue::Number(1975)),
        )
        .is_fully_formed(),
        "a number is not an artist"
    );
    assert!(
        !Rule::new(
            MatchableField::Year,
            Comparison::Contains,
            Some(RuleValue::Text("7".to_string())),
        )
        .is_fully_formed(),
        "a field that never takes the comparison is never a formed Rule, however well-typed \
         its value looks"
    );
    assert!(
        Rule::new(
            MatchableField::Duration,
            Comparison::GreaterThan,
            Some(RuleValue::DurationMillis(3_600_000)),
        )
        .is_fully_formed()
    );
    assert!(
        Rule::new(
            MatchableField::Favorite,
            Comparison::Is,
            Some(RuleValue::YesNo(true)),
        )
        .is_fully_formed()
    );
}

#[test]
fn a_value_answers_the_kind_its_field_holds() {
    for (value, expected) in [
        (RuleValue::Text("trip-hop".to_string()), ValueType::Text),
        (RuleValue::Number(1975), ValueType::Number),
        (RuleValue::DurationMillis(3_600_000), ValueType::Duration),
        (RuleValue::Days(30), ValueType::Date),
        (
            RuleValue::DateTime(riff_persistence::rule::CivilDateTime {
                year: 1975,
                month: 9,
                day: 6,
                hour: 18,
                minute: 30,
                second: 0,
            }),
            ValueType::Date,
        ),
        (RuleValue::YesNo(false), ValueType::YesNo),
    ] {
        assert_eq!(
            value.value_type(),
            expected,
            "the stored value says what kind of answer it is, so a Rule can be checked without \
             re-reading the catalog's table"
        );
    }
}

#[test]
fn a_box_holds_rules_and_boxes_in_one_shared_sibling_order() {
    // The editor draws one interleaved list and the store numbers siblings
    // together, so the order is one sequence over both kinds — not two lists
    // that a later reader has to re-interleave.
    let leaf = RuleBox::new(Combine::Any).with_children(vec![RuleChild::Rule(text_rule(
        MatchableField::Genre,
        "trip-hop",
    ))]);
    let root = RuleBox::new(Combine::All).with_children(vec![
        RuleChild::Rule(count_rule()),
        RuleChild::Box(leaf.clone()),
        RuleChild::Rule(text_rule(MatchableField::Artist, "hook")),
    ]);

    assert_eq!(root.combine(), Combine::All);
    assert_eq!(root.children().len(), 3, "one shared sibling order");
    assert!(matches!(root.children()[0], RuleChild::Rule(_)));
    assert!(
        matches!(&root.children()[1], RuleChild::Box(boxed) if boxed.combine() == Combine::Any)
    );
    assert_eq!(
        root.children()
            .iter()
            .filter(|child| matches!(child, RuleChild::Box(_)))
            .count(),
        1,
        "a box is a sibling of the rules around it, never a separate list"
    );
    assert_eq!(
        root.rule_count(),
        3,
        "a saved tree always holds at least one Rule, so the save path counts them through the \
         nesting: two siblings plus the one inside the nested box"
    );

    let reordered = RuleBox::new(Combine::All).with_children(vec![
        RuleChild::Rule(text_rule(MatchableField::Artist, "hook")),
        RuleChild::Box(leaf),
        RuleChild::Rule(count_rule()),
    ]);
    assert_ne!(
        root, reordered,
        "sibling order is part of the tree's shape, because it is part of what the store \
         renumbers"
    );
}

#[test]
fn trees_compare_by_shape_because_nothing_in_one_carries_an_id() {
    let build = || {
        RootBox::new(RuleBox::new(Combine::Any).with_children(vec![
            RuleChild::Rule(count_rule()),
            RuleChild::Box(
                RuleBox::new(Combine::All).with_children(vec![RuleChild::Rule(text_rule(
                    MatchableField::Album,
                    "unknown pleasures",
                ))]),
            ),
        ]))
    };
    assert_eq!(
        build(),
        build(),
        "the same tree written twice is one tree — what a store-generated id would break"
    );

    let root = build();
    assert_eq!(root.combine(), Combine::Any);
    assert_eq!(root.children().len(), 2);
    assert_eq!(
        root.inner().children(),
        root.children(),
        "the Root Box is a Rule Box like any other, and the list has exactly one"
    );
    assert_eq!(
        root.rule_count(),
        2,
        "rules count through the nesting, however deep"
    );
    assert!(
        !root.is_empty(),
        "a tree with a Rule somewhere inside it is storable"
    );
    assert!(
        RootBox::new(RuleBox::new(Combine::All)).is_empty(),
        "an empty tree is not storable, and that is a question the shape answers"
    );
    assert!(
        RuleBox::new(Combine::All)
            .with_children(vec![RuleChild::Box(RuleBox::new(Combine::Any))])
            .is_empty(),
        "so is any empty box, a nested one included — nothing riff would have to interpret \
         as \"empty\" is ever stored"
    );
}

#[test]
fn the_two_combine_words_are_the_stored_ids_the_boxes_column_enforces() {
    let ids: Vec<&str> = Combine::ALL
        .iter()
        .map(|combine| combine.stored_id())
        .collect();
    assert_eq!(
        ids,
        vec!["all", "any"],
        "`combine IN ('all', 'any')` and this enumeration are the same list"
    );
    for combine in Combine::ALL {
        assert_eq!(Combine::from_stored_id(combine.stored_id()), Some(combine));
    }
    assert_eq!(Combine::from_stored_id("ALL"), None);
    assert_eq!(Combine::from_stored_id("xor"), None);
}
