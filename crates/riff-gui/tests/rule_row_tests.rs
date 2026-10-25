//! The Rule row's own suite (Smart Playlists ticket 14).
//!
//! Two families, both at the row's public interface and neither needing egui:
//!
//! - **What the row offers** is the catalog's answer, so it is asserted *by
//!   reading the catalog* for every field — a test that restated the
//!   field/operator matrix would be a second copy of a fact the catalog exists
//!   to answer once. The handful of literal expectations below are the spec's
//!   own worked examples ("year contains" is not a choice; play count is never
//!   absent), which is what keeps the catalog-derived assertions from being
//!   tautologies: they must agree with those, and with `Rule::is_fully_formed`.
//! - **What the row is missing**, and the value it spells — including the
//!   duration merge, which must be whole milliseconds and lossless in both
//!   directions against the store's nanosecond column.
//!
//! Selections that need a real click, and the picker's read cadence, are the
//! workspace-root UI seam's (`tests/ui_tests.rs::rule_row_ui_tests`).

use riff_backend::domain::field_catalog::{ValueWidget, fields, spec};
use riff_backend::domain::{Comparison, MatchableField, Rule, RuleValue};
use riff_gui::ui::rule_row::{
    ComparisonChoice, RuleDraft, RuleGap, RuleIntent, RuleValueDraft, ValueGap,
};

/// A draft with a field and a comparison chosen the way the row chooses them,
/// so no test reaches past `apply` to build state.
fn draft_with(field: MatchableField, comparison: ComparisonChoice) -> RuleDraft {
    let mut draft = RuleDraft::blank();
    draft.apply(&RuleIntent::FieldChosen { field });
    draft.apply(&RuleIntent::ComparisonChosen { choice: comparison });
    draft
}

/// The stored comparisons the row offers for a field, in offer order.
fn stored(field: MatchableField) -> Vec<Comparison> {
    draft_with(field, ComparisonChoice::Stored(Comparison::Is))
        .comparison_choices()
        .iter()
        .filter_map(|choice| match choice {
            ComparisonChoice::Stored(comparison) => Some(*comparison),
            ComparisonChoice::IsNotMissing => None,
        })
        .collect()
}

/// Whether the row offers the negated spelling of `is missing` for a field.
fn offers_not_missing(field: MatchableField) -> bool {
    draft_with(field, ComparisonChoice::Stored(Comparison::Is))
        .comparison_choices()
        .contains(&ComparisonChoice::IsNotMissing)
}

// --- What the row offers is the catalog's answer -------------------------------

#[test]
fn every_field_offers_exactly_the_comparisons_its_catalog_row_permits() {
    for field in MatchableField::ALL {
        let row = spec(field);
        assert_eq!(
            stored(field),
            row.comparisons().to_vec(),
            "{field:?} must be offered precisely its catalog comparisons, in order"
        );
        for comparison in Comparison::ALL {
            assert_eq!(
                stored(field).contains(&comparison),
                row.permits(comparison),
                "{field:?} / {comparison:?}: the row's offer and the catalog's permission disagree"
            );
        }
    }
}

#[test]
fn is_not_missing_is_offered_only_where_the_catalog_says_a_value_can_be_absent() {
    for field in MatchableField::ALL {
        assert_eq!(
            offers_not_missing(field),
            spec(field).can_be_absent(),
            "{field:?}: the negated spelling appeared away from an absent-able field, \
             or vanished from one that can be absent"
        );
    }
    // Two of the sixteen, named for the reason the spec gives: a column that
    // defaults to 0 has no absence to ask about, so neither spelling appears.
    assert!(!offers_not_missing(MatchableField::PlayCount));
    assert!(!offers_not_missing(MatchableField::Favorite));
    assert!(!stored(MatchableField::PlayCount).contains(&Comparison::IsMissing));
    assert!(offers_not_missing(MatchableField::Genre));
}

#[test]
fn is_not_missing_sits_beside_is_missing_rather_than_at_the_end_of_the_list() {
    for field in MatchableField::ALL {
        let choices =
            draft_with(field, ComparisonChoice::Stored(Comparison::Is)).comparison_choices();
        if !spec(field).can_be_absent() {
            continue;
        }
        let missing = choices
            .iter()
            .position(|choice| *choice == ComparisonChoice::Stored(Comparison::IsMissing))
            .expect("an absent-able field lists is missing");
        assert_eq!(
            choices[missing + 1],
            ComparisonChoice::IsNotMissing,
            "{field:?}: the two absence spellings must read as one pair"
        );
    }
}

#[test]
fn a_text_field_is_never_offered_an_arithmetic_test_and_a_number_never_a_needle() {
    // The spec's own examples, independent of the catalog's tables.
    assert!(!stored(MatchableField::Year).contains(&Comparison::Contains));
    assert!(!stored(MatchableField::Duration).contains(&Comparison::Contains));
    assert!(!stored(MatchableField::Genre).contains(&Comparison::GreaterThan));
    assert!(!stored(MatchableField::PlayCount).contains(&Comparison::Contains));
    assert!(!stored(MatchableField::Favorite).contains(&Comparison::LessThan));
    assert_eq!(
        stored(MatchableField::Favorite),
        vec![Comparison::Is],
        "a fact with one side to test offers one test"
    );
}

#[test]
fn is_and_equal_to_are_one_meaning_under_two_ids_and_each_field_shows_its_own() {
    assert!(stored(MatchableField::Genre).contains(&Comparison::Is));
    assert!(!stored(MatchableField::Genre).contains(&Comparison::EqualTo));
    assert!(stored(MatchableField::Year).contains(&Comparison::EqualTo));
    assert!(!stored(MatchableField::Year).contains(&Comparison::Is));
    assert_eq!(
        Comparison::Is.semantic(),
        Comparison::EqualTo.semantic(),
        "which word a row shows is the catalog's question; the meaning is one"
    );
    assert_eq!(Comparison::Is.stored_id(), "is");
    assert_eq!(Comparison::EqualTo.stored_id(), "equal_to");
    // The two ids are how the store tells a text field's equality from an
    // arithmetic one. The row's words differ too, and neither claims a
    // different test.
    let genre = draft_with(
        MatchableField::Genre,
        ComparisonChoice::Stored(Comparison::Is),
    );
    let year = draft_with(
        MatchableField::Year,
        ComparisonChoice::Stored(Comparison::EqualTo),
    );
    assert_eq!(genre.current_comparison_label(), "is");
    assert_eq!(year.current_comparison_label(), "is equal to");
}

#[test]
fn field_labels_are_the_catalogs_display_names_with_album_artist_included() {
    for field in MatchableField::ALL {
        let mut draft = RuleDraft::blank();
        draft.apply(&RuleIntent::FieldChosen { field });
        assert_eq!(
            draft.field_label(),
            spec(field).display_name(),
            "{field:?}: the row must show the catalog's word, never its own"
        );
    }
    // The words the rest of riff paints. "artist" alone is the spec's prose,
    // not the app's label.
    assert_eq!(
        spec(MatchableField::AlbumArtist).display_name(),
        "Album Artist"
    );
    assert_eq!(spec(MatchableField::Year).display_name(), "Year");
    assert_eq!(spec(MatchableField::PlayCount).display_name(), "Plays");
    assert_eq!(spec(MatchableField::Favorite).display_name(), "Favorite");
}

#[test]
fn value_widgets_come_from_the_catalog_per_field_and_per_comparison() {
    for field in MatchableField::ALL {
        for comparison in spec(field).comparisons() {
            let draft = draft_with(field, ComparisonChoice::Stored(*comparison));
            let expected = spec(field).value_widgets(*comparison);
            assert_eq!(
                draft.value_widgets(),
                expected,
                "{field:?} / {comparison:?}: the row drew a widget the catalog did not name"
            );
            assert_eq!(
                draft.takes_library_values(),
                expected.contains(&ValueWidget::ChooseFromLibrary),
                "{field:?} / {comparison:?}: the picker affordance and the catalog disagree"
            );
            if comparison.semantic() == riff_backend::domain::Semantic::Absence {
                assert!(
                    expected.is_empty(),
                    "{field:?}: an absence test must draw no value at all"
                );
            }
        }
    }
}

#[test]
fn exactly_six_fields_are_pickable_and_they_are_the_four_text_and_two_numeric_ones() {
    let pickable: Vec<MatchableField> = fields()
        .iter()
        .filter(|row| row.is_pickable())
        .map(riff_backend::domain::FieldSpec::field)
        .collect();
    assert_eq!(pickable.len(), 6, "the picker list is six fields wide");
    for field in [
        MatchableField::Genre,
        MatchableField::Artist,
        MatchableField::AlbumArtist,
        MatchableField::Album,
    ] {
        assert!(pickable.contains(&field), "{field:?} must be pickable");
        let draft = draft_with(field, ComparisonChoice::Stored(Comparison::Is));
        assert!(draft.takes_library_values());
        // The typed well stays: an affordance, never a constraint.
        assert!(matches!(draft.value, RuleValueDraft::Text(_)));
    }
    for field in [MatchableField::SampleRate, MatchableField::Channels] {
        assert!(pickable.contains(&field), "{field:?} must be pickable");
        let draft = draft_with(field, ComparisonChoice::Stored(Comparison::EqualTo));
        assert!(draft.takes_library_values());
        assert!(matches!(draft.value, RuleValueDraft::Number(_)));
    }
    for field in [
        MatchableField::Title,
        MatchableField::Composer,
        MatchableField::Year,
        MatchableField::Duration,
        MatchableField::PlayCount,
        MatchableField::Favorite,
        MatchableField::Lossless,
    ] {
        assert!(!pickable.contains(&field), "{field:?} must not be pickable");
        for comparison in spec(field).comparisons() {
            let draft = draft_with(field, ComparisonChoice::Stored(*comparison));
            assert!(
                !draft.takes_library_values(),
                "{field:?} / {comparison:?}: a picker appeared where the catalog named none"
            );
        }
    }
}

// --- The Not control and the two absence spellings -----------------------------

#[test]
fn is_not_missing_leaves_the_row_as_is_missing_plus_negation() {
    let draft = draft_with(MatchableField::Genre, ComparisonChoice::IsNotMissing);
    assert_eq!(draft.comparison, Some(Comparison::IsMissing));
    assert!(draft.negated);
    let rule = draft.rule().expect("`is not missing` is a whole test");
    assert_eq!(rule.comparison(), Comparison::IsMissing);
    assert!(rule.is_negated());
    assert_eq!(
        rule.value(),
        None,
        "an absence test stores no value — the `value` column stays NULL"
    );
    // And the negated spelling is a tenth word, never a tenth operator.
    assert_eq!(rule.comparison().stored_id(), "is_missing");
    assert_eq!(Comparison::ALL.len(), 9, "nine operators, not ten");
}

#[test]
fn picking_is_missing_explicitly_clears_a_negation_the_list_already_carried() {
    let mut draft = draft_with(MatchableField::Genre, ComparisonChoice::IsNotMissing);
    assert!(draft.negated);
    draft.apply(&RuleIntent::ComparisonChosen {
        choice: ComparisonChoice::Stored(Comparison::IsMissing),
    });
    assert_eq!(draft.comparison, Some(Comparison::IsMissing));
    assert!(
        !draft.negated,
        "the two absence spellings own the flag: choosing one means exactly that one"
    );
    assert_eq!(
        draft.current_comparison_label(),
        "is missing",
        "the header word and the list mark must agree"
    );
}

#[test]
fn not_inverts_any_test_without_touching_the_stored_comparison() {
    for (field, comparison) in [
        (MatchableField::Genre, Comparison::Contains),
        (MatchableField::Year, Comparison::GreaterThan),
        (MatchableField::Duration, Comparison::LessThan),
        (MatchableField::Favorite, Comparison::Is),
    ] {
        let mut draft = draft_with(field, ComparisonChoice::Stored(comparison));
        draft.apply(&RuleIntent::NegationToggled { negated: true });
        commit_example_value(&mut draft);
        let rule = draft
            .rule()
            .unwrap_or_else(|| panic!("{field:?} / {comparison:?}: a filled row is a whole test"));
        assert!(rule.is_negated());
        assert_eq!(rule.comparison(), comparison);
        draft.apply(&RuleIntent::NegationToggled { negated: false });
        assert!(
            !draft.rule().expect("un-negated").is_negated(),
            "one control inverts and restores any test"
        );
    }
}

#[test]
fn not_survives_a_comparison_change_because_it_is_orthogonal() {
    let mut draft = draft_with(
        MatchableField::Genre,
        ComparisonChoice::Stored(Comparison::Is),
    );
    draft.apply(&RuleIntent::NegationToggled { negated: true });
    draft.apply(&RuleIntent::ComparisonChosen {
        choice: ComparisonChoice::Stored(Comparison::Contains),
    });
    assert_eq!(draft.comparison, Some(Comparison::Contains));
    assert!(draft.negated, "story 24: no hunting for opposite wording");
}

// --- The field change and its reset --------------------------------------------

#[test]
fn choosing_a_field_resets_a_comparison_the_new_field_does_not_permit() {
    let mut draft = draft_with(
        MatchableField::Year,
        ComparisonChoice::Stored(Comparison::GreaterThan),
    );
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Genre,
    });
    assert_eq!(draft.field, Some(MatchableField::Genre));
    assert_eq!(
        draft.comparison, None,
        "`greater than` is not a thing a text field permits, so it cannot be kept"
    );
    assert!(
        draft.value_widgets().is_empty(),
        "a row with no comparison draws no value"
    );
    assert_eq!(draft.gap(), Some(RuleGap::Comparison));
}

#[test]
fn choosing_a_field_keeps_a_comparison_the_new_field_also_permits() {
    let mut draft = draft_with(
        MatchableField::Genre,
        ComparisonChoice::Stored(Comparison::Contains),
    );
    draft.apply(&RuleIntent::ValueCommitted {
        value: RuleValue::Text("rock".to_string()),
    });
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Album,
    });
    assert_eq!(draft.comparison, Some(Comparison::Contains));
    assert_eq!(
        draft.value,
        RuleValueDraft::Text("rock".to_string()),
        "the same widget idea lets the listener's needle survive the field change"
    );
    assert!(draft.rule().is_some());
}

#[test]
fn choosing_a_field_rebuilds_the_value_when_the_widget_idea_changes() {
    let mut draft = draft_with(
        MatchableField::Year,
        ComparisonChoice::Stored(Comparison::EqualTo),
    );
    draft.apply(&RuleIntent::ValueCommitted {
        value: RuleValue::Number(1975),
    });
    assert_eq!(draft.value, RuleValueDraft::Number("1975".to_string()));
    // Duration permits `equal to` too, so the comparison survives — and the
    // value it takes is a different shape entirely.
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Duration,
    });
    assert_eq!(draft.comparison, Some(Comparison::EqualTo));
    assert_eq!(
        draft.value,
        RuleValueDraft::Duration {
            hours: String::new(),
            minutes: String::new(),
            seconds: String::new(),
        },
        "a number cannot survive as a duration"
    );
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
}

#[test]
fn choosing_a_field_that_cannot_be_absent_drops_a_negated_absence_test() {
    let mut draft = draft_with(MatchableField::Genre, ComparisonChoice::IsNotMissing);
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::PlayCount,
    });
    assert_eq!(draft.comparison, None);
    assert!(
        !draft.negated,
        "a leftover Not would silently invert the next test the listener picks"
    );
    assert!(
        !draft
            .comparison_choices()
            .contains(&ComparisonChoice::IsNotMissing)
    );
}

// --- Which half is missing -----------------------------------------------------

#[test]
fn a_blank_row_names_the_field_as_the_missing_half() {
    let draft = RuleDraft::blank();
    assert_eq!(draft.gap(), Some(RuleGap::Field));
    assert!(draft.rule().is_none());
    assert_eq!(draft.field_label(), "Choose a field");
    assert_eq!(draft.current_comparison_label(), "Choose a comparison");
    assert!(
        draft.comparison_choices().is_empty(),
        "no field, no comparisons to offer"
    );
}

#[test]
fn a_field_with_no_comparison_names_the_comparison() {
    let mut draft = RuleDraft::blank();
    draft.apply(&RuleIntent::FieldChosen {
        field: MatchableField::Album,
    });
    assert_eq!(draft.gap(), Some(RuleGap::Comparison));
}

#[test]
fn a_test_that_draws_a_value_names_the_value_when_nothing_is_typed() {
    let draft = draft_with(
        MatchableField::Artist,
        ComparisonChoice::Stored(Comparison::Is),
    );
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
    let draft = draft_with(
        MatchableField::Favorite,
        ComparisonChoice::Stored(Comparison::Is),
    );
    assert_eq!(
        draft.gap(),
        None,
        "yes/no has no third state, so the first tap is not required for a whole test"
    );
}

#[test]
fn an_absence_test_is_a_whole_rule_with_no_value_at_all() {
    for field in MatchableField::ALL {
        if !spec(field).can_be_absent() {
            continue;
        }
        let draft = draft_with(field, ComparisonChoice::Stored(Comparison::IsMissing));
        assert_eq!(
            draft.gap(),
            None,
            "{field:?}: `is missing` needs nothing else"
        );
        assert!(draft.rule().expect("whole").is_fully_formed());
    }
}

#[test]
fn a_value_stray_on_an_absence_test_is_refused_by_the_row_and_by_the_rule() {
    // The state `apply` never produces, reached by hand so the row's answer is
    // compared against `Rule::is_fully_formed` rather than assumed to match it.
    let mut draft = draft_with(
        MatchableField::Genre,
        ComparisonChoice::Stored(Comparison::IsMissing),
    );
    draft.value = RuleValueDraft::Text("rock".to_string());
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Unparseable)));
    assert!(draft.rule().is_none());
    assert!(
        !Rule::new(
            MatchableField::Genre,
            Comparison::IsMissing,
            Some(RuleValue::Text("rock".to_string())),
        )
        .is_fully_formed(),
        "the catalog's own check agrees the stray value makes this unstorable"
    );
}

#[test]
fn gap_and_is_fully_formed_never_disagree_across_the_catalog() {
    for field in MatchableField::ALL {
        for comparison in Comparison::ALL {
            for value in &example_values(field) {
                let mut draft = draft_with(field, ComparisonChoice::Stored(comparison));
                if let Some(value) = value {
                    draft.value = RuleValueDraft::from_value(value);
                }
                let rule = draft.rule();
                assert_eq!(
                    rule.as_ref().is_some_and(Rule::is_fully_formed),
                    rule.is_some(),
                    "{field:?} / {comparison:?} / {value:?}: a draft that thinks it is whole \
                     produced a Rule the catalog calls incomplete"
                );
                assert_eq!(
                    draft.gap().is_none(),
                    rule.is_some(),
                    "{field:?} / {comparison:?} / {value:?}: gap() and rule() disagree"
                );
            }
        }
    }
}

/// One missing answer and one typed answer per value type, so the sweep above
/// covers the whole `(field, comparison)` grid rather than only its empty half.
fn example_values(field: MatchableField) -> Vec<Option<RuleValue>> {
    let typed = match spec(field).value_type() {
        riff_backend::domain::ValueType::Text => RuleValue::Text("trip-hop".to_string()),
        riff_backend::domain::ValueType::Number => RuleValue::Number(1975),
        riff_backend::domain::ValueType::Duration => RuleValue::DurationMillis(3_723_000),
        riff_backend::domain::ValueType::Date => RuleValue::Days(30),
        riff_backend::domain::ValueType::YesNo => RuleValue::YesNo(true),
    };
    vec![None, Some(typed)]
}

// --- The duration's three boxes ------------------------------------------------

#[test]
fn three_boxes_merge_into_one_whole_millisecond_value() {
    let mut draft = draft_with(
        MatchableField::Duration,
        ComparisonChoice::Stored(Comparison::GreaterThan),
    );
    draft.value = RuleValueDraft::Duration {
        hours: "1".to_string(),
        minutes: "2".to_string(),
        seconds: "3".to_string(),
    };
    assert_eq!(
        draft.rule().expect("whole").value(),
        Some(&RuleValue::DurationMillis(3_723_000)),
        "1 h 2 m 3 s = 3_723_000 ms — and 3_723_000 x 1e6 ns is exactly the store's column"
    );
}

#[test]
fn an_empty_duration_box_reads_as_zero_and_three_empty_ones_as_no_value() {
    let mut draft = draft_with(
        MatchableField::Duration,
        ComparisonChoice::Stored(Comparison::EqualTo),
    );
    draft.value = RuleValueDraft::Duration {
        hours: String::new(),
        minutes: "30".to_string(),
        seconds: String::new(),
    };
    assert_eq!(
        draft.rule().expect("half an hour").value(),
        Some(&RuleValue::DurationMillis(1_800_000))
    );
    draft.value = RuleValueDraft::Duration {
        hours: String::new(),
        minutes: String::new(),
        seconds: String::new(),
    };
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
}

#[test]
fn seconds_carry_milliseconds_and_the_split_round_trips_exactly() {
    // The stored unit is whole milliseconds, so the merge may not round, and a
    // reopened rule must spell the same three boxes back.
    for millis in [
        1_u64, 5, 999, 1_000, 1_500, 3_723_500, 3_600_000, 86_399_999, 10_800_000,
    ] {
        let mut draft = draft_with(
            MatchableField::Duration,
            ComparisonChoice::Stored(Comparison::EqualTo),
        );
        draft.value = RuleValueDraft::from_value(&RuleValue::DurationMillis(millis));
        let rule = draft
            .rule()
            .unwrap_or_else(|| panic!("{millis} ms must survive its own three boxes"));
        assert_eq!(rule.value(), Some(&RuleValue::DurationMillis(millis)));
    }
    let mut draft = draft_with(
        MatchableField::Duration,
        ComparisonChoice::Stored(Comparison::EqualTo),
    );
    draft.value = RuleValueDraft::Duration {
        hours: "0".to_string(),
        minutes: "0".to_string(),
        seconds: "1.005".to_string(),
    };
    assert_eq!(
        draft.rule().expect("whole").value(),
        Some(&RuleValue::DurationMillis(1_005))
    );
}

#[test]
fn a_duration_too_precise_or_too_huge_to_be_whole_milliseconds_is_refused() {
    for (hours, minutes, seconds) in [
        ("0", "0", "1.0005"),
        ("0", "0", "1."),
        ("0", "0", "-1"),
        ("0", "-1", "0"),
        ("18446744073709", "0", "0"),
        ("0", "0", "abc"),
    ] {
        let mut draft = draft_with(
            MatchableField::Duration,
            ComparisonChoice::Stored(Comparison::EqualTo),
        );
        draft.value = RuleValueDraft::Duration {
            hours: hours.to_string(),
            minutes: minutes.to_string(),
            seconds: seconds.to_string(),
        };
        assert_eq!(
            draft.gap(),
            Some(RuleGap::Value(ValueGap::Unparseable)),
            "{hours}:{minutes}:{seconds} cannot be a whole-millisecond duration"
        );
        assert!(draft.rule().is_none());
    }
}

#[test]
fn a_year_and_a_play_count_are_whole_numbers_and_nothing_else() {
    for field in [MatchableField::Year, MatchableField::PlayCount] {
        let mut draft = draft_with(field, ComparisonChoice::Stored(Comparison::GreaterThan));
        assert!(matches!(draft.value, RuleValueDraft::Number(_)));
        draft.value = RuleValueDraft::Number("  1975 ".to_string());
        assert_eq!(
            draft.rule().expect("whole").value(),
            Some(&RuleValue::Number(1975)),
            "{field:?}: a number well trims its input"
        );
        draft.value = RuleValueDraft::Number("1975a".to_string());
        assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Unparseable)));
        draft.value = RuleValueDraft::Number("   ".to_string());
        assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
    }
}

#[test]
fn a_needle_is_the_text_typed_but_whitespace_alone_is_no_answer() {
    let mut draft = draft_with(
        MatchableField::Title,
        ComparisonChoice::Stored(Comparison::Contains),
    );
    draft.value = RuleValueDraft::Text("  ".to_string());
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
    draft.value = RuleValueDraft::Text(" Live ".to_string());
    assert_eq!(
        draft.rule().expect("whole").value(),
        Some(&RuleValue::Text(" Live ".to_string())),
        "a Rule tests the tag, so the typed needle is not silently trimmed"
    );
}

// --- Stored rules and the values they hold -------------------------------------

#[test]
fn a_stored_rule_round_trips_through_the_draft_unchanged() {
    let rules = [
        Rule::new(
            MatchableField::Genre,
            Comparison::Contains,
            Some(RuleValue::Text("art rock".to_string())),
        ),
        Rule::new(
            MatchableField::AlbumArtist,
            Comparison::Is,
            Some(RuleValue::Text("Unknown Artist".to_string())),
        ),
        Rule::new(
            MatchableField::Year,
            Comparison::LessThan,
            Some(RuleValue::Number(-44)),
        )
        .negated(),
        Rule::new(
            MatchableField::Duration,
            Comparison::GreaterThan,
            Some(RuleValue::DurationMillis(3_723_500)),
        ),
        Rule::new(
            MatchableField::Favorite,
            Comparison::Is,
            Some(RuleValue::YesNo(true)),
        ),
        Rule::new(
            MatchableField::PlayCount,
            Comparison::EqualTo,
            Some(RuleValue::Number(0)),
        ),
        Rule::new(MatchableField::Genre, Comparison::IsMissing, None).negated(),
        Rule::new(
            MatchableField::SampleRate,
            Comparison::EqualTo,
            Some(RuleValue::Number(44100)),
        ),
    ];
    for rule in &rules {
        let draft = RuleDraft::from_rule(rule);
        assert_eq!(
            draft.rule().as_ref(),
            Some(rule),
            "reopening a saved rule and changing nothing must not rewrite it"
        );
        assert_eq!(draft.gap(), None);
        assert!(!draft.picker_open);
    }
}

#[test]
fn an_empty_date_row_has_a_missing_value_half() {
    // A date Rule starts as a row whose value half is missing, so the
    // editor's Save refusal names it rather than storing a wrong test; the
    // date controls themselves are ticket 20's own suite
    // (`rule_date_tests.rs`).
    for (field, comparison, idea) in [
        (
            MatchableField::LastPlayed,
            Comparison::Before,
            ValueWidget::DateTime,
        ),
        (
            MatchableField::DateAdded,
            Comparison::After,
            ValueWidget::DateTime,
        ),
        (
            MatchableField::DateAdded,
            Comparison::InLast,
            ValueWidget::DateWindow,
        ),
    ] {
        let draft = draft_with(field, ComparisonChoice::Stored(comparison));
        assert_eq!(draft.value_widgets(), &[idea]);
        let RuleValueDraft::Date { kind, .. } = &draft.value else {
            panic!("{field:?} / {comparison:?}: the date idea must route to the date buffers");
        };
        assert_eq!(
            *kind,
            match idea {
                ValueWidget::DateWindow => riff_gui::ui::rule_date::RuleDateKind::DateWindow,
                _ => riff_gui::ui::rule_date::RuleDateKind::DateTime,
            }
        );
        assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
        assert!(draft.rule().is_none());
    }
}

#[test]
fn choosing_a_comparison_rebuilds_the_value_when_the_date_idea_switches() {
    let mut draft = draft_with(
        MatchableField::DateAdded,
        ComparisonChoice::Stored(Comparison::InLast),
    );
    assert!(matches!(
        draft.value,
        RuleValueDraft::Date {
            kind: riff_gui::ui::rule_date::RuleDateKind::DateWindow,
            ..
        }
    ));
    draft.apply(&RuleIntent::ComparisonChosen {
        choice: ComparisonChoice::Stored(Comparison::Before),
    });
    assert!(matches!(
        draft.value,
        RuleValueDraft::Date {
            kind: riff_gui::ui::rule_date::RuleDateKind::DateTime,
            ..
        }
    ));
}

// --- The row's picker-open memo --------------------------------------------------

#[test]
fn a_picker_open_and_close_move_the_rows_memo() {
    let mut draft = draft_with(
        MatchableField::Genre,
        ComparisonChoice::Stored(Comparison::Is),
    );
    draft.apply(&RuleIntent::PickerOpened {
        field: MatchableField::Genre,
    });
    assert!(draft.picker_open);
    draft.apply(&RuleIntent::ValueCommitted {
        value: RuleValue::Text("ambient".to_string()),
    });
    assert_eq!(draft.value, RuleValueDraft::Text("ambient".to_string()));
    assert!(
        draft.picker_open,
        "a choice commits the value; the popup's own close moves the memo"
    );
    draft.apply(&RuleIntent::PickerClosed);
    assert!(!draft.picker_open);
    assert_eq!(
        draft.gap(),
        None,
        "typing was never the point of a picker, and choosing is a whole answer"
    );
}

#[test]
fn applying_the_same_intent_twice_changes_nothing() {
    let mut draft = RuleDraft::blank();
    let intents = [
        RuleIntent::FieldChosen {
            field: MatchableField::AlbumArtist,
        },
        RuleIntent::ComparisonChosen {
            choice: ComparisonChoice::Stored(Comparison::Contains),
        },
        RuleIntent::ValueCommitted {
            value: RuleValue::Text("windy".to_string()),
        },
    ];
    for intent in &intents {
        draft.apply(intent);
    }
    let settled = draft.clone();
    for intent in &intents {
        draft.apply(intent);
    }
    assert_eq!(
        draft, settled,
        "a host may re-land an intent it already took"
    );
}

/// Fill a draft with the first answer its own widget idea accepts, so the
/// negation tests do not each re-derive what a valid value is.
fn commit_example_value(draft: &mut RuleDraft) {
    let widgets = draft.value_widgets();
    if widgets.is_empty() {
        return;
    }
    let value = if widgets.contains(&ValueWidget::YesNo) {
        RuleValue::YesNo(true)
    } else if widgets.contains(&ValueWidget::Duration) {
        RuleValue::DurationMillis(60_000)
    } else if widgets.contains(&ValueWidget::Number) {
        RuleValue::Number(1975)
    } else {
        RuleValue::Text("rock".to_string())
    };
    draft.apply(&RuleIntent::ValueCommitted { value });
}
