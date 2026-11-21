//! The Custom Smart Playlist draft: the editable mirror of
//! [`SmartPlaylistTree`], its conversions, and the save gate the editor runs
//! before one store write.
//!
//! Every refusal sentence is the egui editor's, so a listener who met the
//! old surface reads the same words here: the unreadable-row block, the
//! three field/comparison/value gaps, and the catalog's value-type sentence.

use riff_backend::app::store::{SmartPlaylistTree, UnreadableRule};
use riff_backend::domain::PlaylistId;
use riff_backend::domain::{
    CivilDateTime, Combine, Comparison, MatchableField, RootBox, Rule, RuleBox, RuleChild,
    RuleValue,
};
use riff_backend::domain::{ValueType, field_catalog};

/// The words an unreadable row wears, the egui editor's copy.
pub const UNREADABLE_ROW_WORDS: &str = "riff can't understand this rule";
/// Why Save refuses while one is in the tree.
pub const UNREADABLE_BLOCK_REASON: &str =
    "riff can't understand this rule — delete the row to save the list";

/// The depth cap the editor's group control honors (the egui
/// `MAX_RULE_BOX_LEVELS`).
pub const MAX_RULE_BOX_LEVELS: usize = 7;

/// One rule row's editable state.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleDraft {
    pub field: MatchableField,
    pub comparison: Comparison,
    pub negated: bool,
    pub value: String,
}

/// One child of a draft box.
#[derive(Debug, Clone, PartialEq)]
pub enum ChildDraft {
    Rule(RuleDraft),
    Box(BoxDraft),
}

/// One draft box: its combine word and its one shared sibling order.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxDraft {
    pub combine: Combine,
    pub children: Vec<ChildDraft>,
}

/// The whole draft: the list's identity (none while creating), its name,
/// the tree, and the stored rows this build cannot read.
#[derive(Debug, Clone, PartialEq)]
pub struct SmartDraft {
    pub id: Option<PlaylistId>,
    pub name: String,
    pub root: BoxDraft,
    pub unreadable: Vec<UnreadableRule>,
}

impl SmartDraft {
    /// A fresh draft: one empty rule in an all-box.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: None,
            name: String::new(),
            root: BoxDraft {
                combine: Combine::All,
                children: vec![ChildDraft::Rule(RuleDraft {
                    field: MatchableField::Artist,
                    comparison: Comparison::Is,
                    negated: false,
                    value: String::new(),
                })],
            },
            unreadable: Vec::new(),
        }
    }

    /// The draft a stored list loads into.
    #[must_use]
    pub fn of_list(id: PlaylistId, name: String, tree: &SmartPlaylistTree) -> Self {
        Self {
            id: Some(id),
            name,
            root: box_draft(tree.root().inner()),
            unreadable: tree.unreadable().to_vec(),
        }
    }

    /// The root's rule count, at every depth.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        count_rules(&self.root)
    }

    /// The save gate: the unreadable block, then an empty tree, then each
    /// half-filled row's own sentence. `Ok` carries the storable tree.
    pub fn to_tree(&self) -> Result<RootBox, Vec<String>> {
        let mut reasons = Vec::new();
        if !self.unreadable.is_empty() {
            reasons.push(UNREADABLE_BLOCK_REASON.to_owned());
        }
        if self.rule_count() == 0 {
            reasons.push("Add a rule to save this list.".to_owned());
        }
        let root = build_box(&self.root, &mut reasons);
        if reasons.is_empty() {
            Ok(RootBox::new(root))
        } else {
            Err(reasons)
        }
    }

    /// Whether the draft differs from what was loaded or last saved.
    #[must_use]
    pub fn differs_from(&self, saved: &Self) -> bool {
        self != saved
    }
}

impl Default for SmartDraft {
    fn default() -> Self {
        Self::new()
    }
}

fn box_draft(boxed: &RuleBox) -> BoxDraft {
    BoxDraft {
        combine: boxed.combine(),
        children: boxed
            .children()
            .iter()
            .map(|child| match child {
                RuleChild::Rule(rule) => ChildDraft::Rule(RuleDraft {
                    field: rule.field(),
                    comparison: rule.comparison(),
                    negated: rule.is_negated(),
                    value: rule.value().map(display_value).unwrap_or_default(),
                }),
                RuleChild::Box(inner) => ChildDraft::Box(box_draft(inner)),
            })
            .collect(),
    }
}

fn build_box(draft: &BoxDraft, reasons: &mut Vec<String>) -> RuleBox {
    let children = draft
        .children
        .iter()
        .filter_map(|child| match child {
            ChildDraft::Rule(rule) => build_rule(rule, reasons).map(RuleChild::Rule),
            ChildDraft::Box(inner) => Some(RuleChild::Box(build_box(inner, reasons))),
        })
        .collect();
    RuleBox::new(draft.combine).with_children(children)
}

fn build_rule(draft: &RuleDraft, reasons: &mut Vec<String>) -> Option<Rule> {
    let spec = field_catalog::spec(draft.field);
    if !spec.permits(draft.comparison) {
        reasons.push("this rule needs a comparison".to_owned());
        return None;
    }
    match parse_value(draft.field, draft.comparison, &draft.value) {
        Ok(value) => {
            let rule = Rule::new(draft.field, draft.comparison, value);
            Some(if draft.negated { rule.negated() } else { rule })
        }
        Err(reason) => {
            reasons.push(reason);
            None
        }
    }
}

fn count_rules(draft: &BoxDraft) -> usize {
    draft
        .children
        .iter()
        .map(|child| match child {
            ChildDraft::Rule(_) => 1,
            ChildDraft::Box(inner) => count_rules(inner),
        })
        .sum()
}

/// One rule's value, typed by its field's catalog row.
pub fn parse_value(
    field: MatchableField,
    comparison: Comparison,
    text: &str,
) -> Result<Option<RuleValue>, String> {
    let spec = field_catalog::spec(field);
    if comparison.semantic() == riff_backend::domain::Semantic::Absence {
        return Ok(None);
    }
    let text = text.trim();
    let value = match spec.value_type() {
        ValueType::Text => RuleValue::Text(text.to_owned()),
        ValueType::Number => RuleValue::Number(
            text.parse()
                .map_err(|_| "this rule's value can't be read as this field's kind".to_owned())?,
        ),
        ValueType::Duration => RuleValue::DurationMillis(
            crate::screens::smart_playlists::date_picker::parse_duration(text)?,
        ),
        ValueType::Date => {
            if comparison == Comparison::InLast {
                RuleValue::Days(text.parse().map_err(|_| {
                    "this rule's value can't be read as this field's kind".to_owned()
                })?)
            } else {
                RuleValue::DateTime(crate::screens::smart_playlists::date_picker::parse_civil(
                    text,
                )?)
            }
        }
        ValueType::YesNo => RuleValue::YesNo(parse_yes_no(text)?),
    };
    Ok(Some(value))
}

fn parse_yes_no(text: &str) -> Result<bool, String> {
    match text.trim().to_lowercase().as_str() {
        "yes" | "true" | "1" => Ok(true),
        "no" | "false" | "0" => Ok(false),
        _ => Err("this rule's value can't be read as this field's kind".to_owned()),
    }
}

/// A value as its field's text form, for loading a stored rule into a row.
#[must_use]
pub fn display_value(value: &RuleValue) -> String {
    match value {
        RuleValue::Text(text) => text.clone(),
        RuleValue::Number(number) => number.to_string(),
        RuleValue::DurationMillis(millis) => format!(
            "{}:{:02}:{:02}",
            millis / 3_600_000,
            (millis / 60_000) % 60,
            (millis / 1000) % 60
        ),
        RuleValue::Days(days) => days.to_string(),
        RuleValue::DateTime(at) => crate::screens::smart_playlists::date_picker::format_civil(*at),
        RuleValue::YesNo(yes) => {
            if *yes {
                "yes".to_owned()
            } else {
                "no".to_owned()
            }
        }
    }
}

/// The next field in catalog order.
#[must_use]
pub fn next_field(field: MatchableField) -> MatchableField {
    let all = MatchableField::ALL;
    let at = all
        .iter()
        .position(|candidate| *candidate == field)
        .unwrap_or(0);
    all[(at + 1) % all.len()]
}

/// The next comparison this field permits.
#[must_use]
pub fn next_comparison(field: MatchableField, comparison: Comparison) -> Comparison {
    let permitted = field_catalog::spec(field).comparisons();
    if permitted.is_empty() {
        return comparison;
    }
    let at = permitted
        .iter()
        .position(|candidate| *candidate == comparison)
        .unwrap_or(0);
    permitted[(at + 1) % permitted.len()]
}

/// A comparison's display word.
#[must_use]
pub fn comparison_label(comparison: Comparison) -> &'static str {
    match comparison {
        Comparison::Is => "is",
        Comparison::Contains => "contains",
        Comparison::GreaterThan => "is greater than",
        Comparison::LessThan => "is less than",
        Comparison::EqualTo => "equals",
        Comparison::InLast => "in the last (days)",
        Comparison::Before => "is before",
        Comparison::After => "is after",
        Comparison::IsMissing => "is missing",
    }
}

/// A combine word's label.
#[must_use]
pub fn combine_label(combine: Combine) -> &'static str {
    match combine {
        Combine::All => "all of",
        Combine::Any => "any of",
    }
}

/// The value widget's hint sentence for a row, from the catalog.
#[must_use]
pub fn value_hint(field: MatchableField, comparison: Comparison) -> &'static str {
    if comparison.semantic() == riff_backend::domain::Semantic::Absence {
        return "no value";
    }
    let widgets = field_catalog::spec(field).value_widgets(comparison);
    widgets.first().map_or("no value", |widget| match widget {
        field_catalog::ValueWidget::Number => "number",
        field_catalog::ValueWidget::Duration => "h:mm:ss",
        field_catalog::ValueWidget::DateTime => "YYYY-MM-DD HH:MM",
        field_catalog::ValueWidget::DateWindow => "days",
        field_catalog::ValueWidget::YesNo => "yes or no",
        field_catalog::ValueWidget::Text | field_catalog::ValueWidget::ChooseFromLibrary => "text",
    })
}

/// The zone-less day a `CivilDateTime` names, for the "Today" shortcut.
pub use crate::screens::smart_playlists::date_picker::today;

#[allow(unused_imports)]
use CivilDateTime as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_draft_round_trips_through_its_tree() {
        let mut draft = SmartDraft::new();
        draft.root.children = vec![ChildDraft::Rule(RuleDraft {
            field: MatchableField::Year,
            comparison: Comparison::GreaterThan,
            negated: false,
            value: "1999".to_owned(),
        })];
        let tree = SmartPlaylistTree::new(draft.to_tree().expect("a whole rule saves"));
        let loaded = SmartDraft::of_list(PlaylistId("x".to_owned()), "X".to_owned(), &tree);
        assert_eq!(loaded.root, draft.root);
    }

    #[test]
    fn a_half_filled_row_blocks_the_save_with_its_own_sentence() {
        let mut draft = SmartDraft::new();
        draft.root.children = vec![ChildDraft::Rule(RuleDraft {
            field: MatchableField::Year,
            comparison: Comparison::GreaterThan,
            negated: false,
            value: "not a year".to_owned(),
        })];
        let reasons = draft.to_tree().expect_err("an unreadable value blocks");
        assert!(
            reasons.contains(&"this rule's value can't be read as this field's kind".to_owned())
        );
    }

    #[test]
    fn the_unreadable_block_wins_over_everything() {
        let mut draft = SmartDraft::new();
        draft.unreadable.push(UnreadableRule::new(
            Vec::new(),
            0,
            "future_field".to_owned(),
            "is".to_owned(),
            false,
            None,
        ));
        let reasons = draft.to_tree().expect_err("unreadable blocks");
        assert_eq!(reasons[0], UNREADABLE_BLOCK_REASON);
    }

    #[test]
    fn the_date_value_round_trips_through_the_picker() {
        let value = parse_value(
            MatchableField::LastPlayed,
            Comparison::Before,
            "2024-03-05 07:08",
        )
        .expect("a civil time parses")
        .expect("a date rule carries a value");
        assert_eq!(display_value(&value), "2024-03-05 07:08");
    }
}
