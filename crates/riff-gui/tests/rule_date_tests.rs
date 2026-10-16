//! The Rule row's date controls, at their own seam (Smart Playlists ticket 20).
//!
//! Two families, both needing no egui:
//!
//! - **What the buffers spell** — [`RuleDateDraft::value`] parses the typed
//!   buffers into the one [`RuleValue`] a date Rule stores: a window of whole
//!   days for `in the last`, a zone-less day plus time for `before`/`after`.
//!   The typed pair is stored exactly as typed, never resolved through a
//!   clock: `before`/`after` are read against the listener's own clock on the
//!   machine that reads the Rule, so a stored zone would make one saved Rule
//!   answer differently on another machine. The comparison side of that
//!   decision is pinned at the store seam
//!   (`riff-infra/tests/store_tests.rs::test_a_zone_less_boundary_is_read_against_the_machines_own_local_midnight`).
//! - **What a half-filled pair reports** — the row's own `gap()`: a partly
//!   typed date is a missing value like any other half-filled Rule, never an
//!   error and never a silently coerced answer.
//!
//! What a real click does (the calendar's options, the typed time) is the
//! workspace-root UI seam's (`tests/ui_tests.rs::rule_row_ui_tests`).

use riff_backend::domain::{Comparison, MatchableField, Rule, RuleValue};
use riff_gui::ui::rule_date::{RuleDateDraft, RuleDateIntent, RuleDateKind};
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

/// The date buffers a draft holds, guaranteed to be the date half.
fn date_buffers(draft: &RuleDraft) -> (&RuleDateKind, &RuleDateDraft) {
    let RuleValueDraft::Date { kind, draft } = &draft.value else {
        panic!("this draft's value half must be the date buffers");
    };
    (kind, draft)
}

// --- the day window: a whole number of days, nothing else ----------------------

#[test]
fn a_day_window_is_a_whole_number_of_days() {
    let days = |text: &str| {
        RuleDateDraft {
            window_days: text.to_string(),
            ..RuleDateDraft::blank()
        }
        .value(RuleDateKind::DateWindow)
    };
    assert_eq!(days("7"), Some(RuleValue::Days(7)));
    assert_eq!(
        days(" 30 "),
        Some(RuleValue::Days(30)),
        "a typed day count is what a number field's own parse accepts"
    );
    // Everything that is not a whole non-negative day count spells no value.
    for text in ["", " ", "-3", "7.5", "seven", "4294967296"] {
        assert_eq!(
            days(text),
            None,
            "{text:?} is not a whole day count and must not become one"
        );
    }
}

#[test]
fn a_day_window_ignores_the_fixed_date_buffers_and_vice_versa() {
    // One field, two widgets: the kind picks the buffers, so half the draft
    // being filled never leaks into the other shape.
    let fixed = RuleDateDraft {
        day: "2024-03-02".to_string(),
        time: "23:30:00".to_string(),
        ..RuleDateDraft::blank()
    };
    assert_eq!(
        fixed.value(RuleDateKind::DateWindow),
        None,
        "the day window reads only the window buffer"
    );
    let window = RuleDateDraft {
        window_days: "7".to_string(),
        ..RuleDateDraft::blank()
    };
    assert_eq!(
        window.value(RuleDateKind::DateTime),
        None,
        "the fixed date reads only the day and time buffers"
    );
}

#[test]
fn the_day_window_lands_in_the_drafts_rule_as_a_whole_day_count() {
    let mut draft = draft_with(
        MatchableField::DateAdded,
        ComparisonChoice::Stored(Comparison::InLast),
    );
    draft.apply(&RuleIntent::Date(RuleDateIntent::WindowDays(
        "7".to_string(),
    )));
    assert_eq!(draft.gap(), None);
    assert_eq!(
        draft.rule(),
        Some(Rule::new(
            MatchableField::DateAdded,
            Comparison::InLast,
            Some(RuleValue::Days(7))
        )),
        "the stored value is a whole number of days — the cutoff is the read's to compute, \
         at read time, which is what keeps the window moving"
    );
}

#[test]
fn a_stored_day_window_reopens_as_the_same_rule() {
    for days in [0_u32, 7, 365] {
        let value = RuleValue::Days(days);
        let draft = RuleDraft::from_rule(&Rule::new(
            MatchableField::LastPlayed,
            Comparison::InLast,
            Some(value.clone()),
        ));
        let (kind, buffers) = date_buffers(&draft);
        assert_eq!(*kind, RuleDateKind::DateWindow);
        assert_eq!(
            buffers.value(*kind),
            Some(value),
            "reopening a saved window must not rewrite it"
        );
        assert_eq!(draft.gap(), None);
    }
}

#[test]
fn an_empty_day_window_wants_a_value_rather_than_spelling_one() {
    let draft = draft_with(
        MatchableField::LastPlayed,
        ComparisonChoice::Stored(Comparison::InLast),
    );
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
    assert!(draft.rule().is_none());
}

// --- the fixed date: the typed day and time, stored zone-less ------------------

/// The civil parts a `(day, time)` pair spells, through the date parse.
fn civil(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> RuleValue {
    RuleValue::DateTime(riff_backend::domain::CivilDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

#[test]
fn a_fixed_date_stores_the_typed_day_and_time_zone_less() {
    let fixed = |day: &str, time: &str| {
        RuleDateDraft {
            day: day.to_string(),
            time: time.to_string(),
            ..RuleDateDraft::blank()
        }
        .value(RuleDateKind::DateTime)
    };
    assert_eq!(
        fixed("2024-03-02", "23:30:00"),
        Some(civil(2024, 3, 2, 23, 30, 0)),
        "the pair is what the listener typed, to the second, with no clock or zone in between"
    );
    assert_eq!(
        fixed("0001-01-01", "00:00:00"),
        Some(civil(1, 1, 1, 0, 0, 0))
    );
    assert_eq!(
        fixed("9999-12-31", "23:59:59"),
        Some(civil(9999, 12, 31, 23, 59, 59)),
        "the whole four-digit year range the stored text shape can spell"
    );
}

#[test]
fn a_half_typed_fixed_date_spells_no_value() {
    let fixed = |day: &str, time: &str| {
        RuleDateDraft {
            day: day.to_string(),
            time: time.to_string(),
            ..RuleDateDraft::blank()
        }
        .value(RuleDateKind::DateTime)
    };
    for (day, time) in [
        ("", ""),
        ("2024-03-02", ""),
        ("", "23:30:00"),
        ("2024-03-02", "23:30"),
        ("2024-03-02", "23::00"),
        ("2024-03", "23:30:00"),
        ("not a date", "23:30:00"),
    ] {
        assert_eq!(
            fixed(day, time),
            None,
            "{day:?} + {time:?} is not a whole pair and must not become one"
        );
    }
}

#[test]
fn an_impossible_fixed_date_spells_no_value() {
    let fixed = |day: &str, time: &str| {
        RuleDateDraft {
            day: day.to_string(),
            time: time.to_string(),
            ..RuleDateDraft::blank()
        }
        .value(RuleDateKind::DateTime)
    };
    // Calendar parts the stored text shape cannot mean — a month that is not
    // a month, a day that is not a day, a time that is not a time — refuse
    // rather than store text no stamp orders beside.
    for (day, time) in [
        ("2024-13-01", "00:00:00"),
        ("2024-00-10", "00:00:00"),
        ("2024-03-32", "00:00:00"),
        ("2024-03-00", "00:00:00"),
        ("10000-01-01", "00:00:00"),
        ("2024-03-02", "24:00:00"),
        ("2024-03-02", "23:60:00"),
        ("2024-03-02", "23:30:60"),
    ] {
        assert_eq!(
            fixed(day, time),
            None,
            "{day:?} {time:?} is not a real civil instant"
        );
    }
}

#[test]
fn a_partly_typed_date_reports_the_rows_own_gap_not_an_error() {
    let mut draft = draft_with(
        MatchableField::LastPlayed,
        ComparisonChoice::Stored(Comparison::Before),
    );
    assert_eq!(
        draft.gap(),
        Some(RuleGap::Value(ValueGap::Missing)),
        "nothing typed is a missing value"
    );
    // A day from the calendar, no time yet: still the row's own missing-value
    // answer, and the typed day stays in the buffers so the edit continues.
    draft.apply(&RuleIntent::Date(RuleDateIntent::Day(
        "2024-03-02".to_string(),
    )));
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
    assert_eq!(date_buffers(&draft).1.day, "2024-03-02");
    draft.apply(&RuleIntent::Date(RuleDateIntent::Time("23:".to_string())));
    assert_eq!(draft.gap(), Some(RuleGap::Value(ValueGap::Missing)));
    assert_eq!(draft.rule(), None);
    // Nothing a date row can hold ever reports as unparseable: the buffers
    // are typed text, and every non-answer is an absence — a date has no
    // "entered, but not this field's type" state (the number and duration
    // wells' `Unparseable` has no date-side spelling).
    for junk in ["", "x", "1:2:3:4", "99999-99-99"] {
        draft.apply(&RuleIntent::Date(RuleDateIntent::Time(junk.to_string())));
        assert_eq!(
            draft.gap(),
            Some(RuleGap::Value(ValueGap::Missing)),
            "{junk:?} must read as a half-filled Rule, not as an error"
        );
    }
    // The pair completes into the row's whole Rule, gap-free.
    draft.apply(&RuleIntent::Date(RuleDateIntent::Time(
        "23:30:00".to_string(),
    )));
    assert_eq!(draft.gap(), None);
    assert_eq!(
        draft.rule(),
        Some(Rule::new(
            MatchableField::LastPlayed,
            Comparison::Before,
            Some(civil(2024, 3, 2, 23, 30, 0))
        ))
    );
}

#[test]
fn a_stored_fixed_date_reopens_as_the_same_rule() {
    for value in [
        civil(2024, 3, 2, 23, 30, 0),
        civil(2024, 3, 2, 0, 30, 0),
        civil(1, 1, 1, 0, 0, 0),
    ] {
        let draft = RuleDraft::from_rule(&Rule::new(
            MatchableField::DateAdded,
            Comparison::After,
            Some(value.clone()),
        ));
        let (kind, buffers) = date_buffers(&draft);
        assert_eq!(*kind, RuleDateKind::DateTime);
        assert_eq!(
            buffers.value(*kind),
            Some(value),
            "reopening a saved boundary must not rewrite it"
        );
        assert_eq!(draft.gap(), None);
    }
}

#[test]
fn a_stored_fixed_date_shows_padded_buffers_of_the_exact_stored_shape() {
    // The shape the store's text column reads back: zero-padded digits,
    // whole seconds, nothing else — so a saved Rule and a re-typed one
    // cannot come to disagree about what was stored.
    let buffers = RuleDateDraft::from_value(&civil(2024, 3, 2, 3, 4, 5));
    assert_eq!(buffers.day, "2024-03-02");
    assert_eq!(buffers.time, "03:04:05");
    assert_eq!(
        buffers.value(RuleDateKind::DateTime),
        Some(civil(2024, 3, 2, 3, 4, 5))
    );
}

#[test]
fn a_history_date_still_answers_is_missing_with_no_control_at_all() {
    for field in [MatchableField::LastPlayed, MatchableField::DateAdded] {
        let draft = draft_with(field, ComparisonChoice::Stored(Comparison::IsMissing));
        assert_eq!(
            draft.value_widgets(),
            &[],
            "{field:?}: `is missing` draws no date control"
        );
        assert_eq!(draft.gap(), None, "{field:?}: and the row is whole");
        assert_eq!(
            draft.rule(),
            Some(Rule::new(field, Comparison::IsMissing, None))
        );
    }
}
