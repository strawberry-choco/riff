//! The one Rule row — field, then comparison, then value — assembled from
//! generic presenters and driven entirely by the Matchable Field catalog
//! (Smart Playlists ticket 14, ADR 0018 / ADR 0021).
//!
//! A listener builds a Rule by choosing a field, then a comparison, then a
//! value, and never learns a formula language. Everything about *what may be
//! chosen* is the catalog's answer, asked through
//! [`riff_backend::domain::field_catalog`] and never restated here: the
//! comparisons offered are exactly the selected field's, `is missing` / `is
//! not missing` appear exactly where a value can be absent, and the value
//! control is picked from the catalog's widget *ideas* (`Text`, `Number`,
//! `Duration`, `DateTime`, `DateWindow`, `Yes/No`, `ChooseFromLibrary`) —
//! which name what kind of answer a field wants, never a control. This module
//! is therefore the only place in the product that translates an idea into a
//! widget, and that translation is exactly why the row is a frontend module:
//! the primitives it draws with ([`option_list`], [`text_field`],
//! [`toggle_switch`]) may not know what a Rule means, so a widget that had to
//! would live here instead (ADR 0018).
//!
//! **The row's contract is the crate's standing one: props in, typed intents
//! out.** [`rule_row`] performs no effect — no store read, no service call, no
//! session write — and mutates only the draft's *typed buffers*, which is what
//! [`text_field`] does to every caller it is handed. Every structural change
//! leaves the row as a [`RuleIntent`], and [`RuleDraft::apply`] is the one
//! place an intent lands, so the reset-on-field-change rule exists once and is
//! testable with no egui at all. The returned [`egui::Response`] is not an
//! intent: it is the row's rect, which the host needs to keep a clipped row
//! reachable and to drag it (ticket 19) — the same reason [`text_field`] hands
//! its response back.
//!
//! **What a host owes this row** (tickets 15–19 own these, not the row):
//!
//! - *A picker read per open, never per frame.* The row takes the Library's
//!   values as a prop ([`RulePickerValues`]) and reports
//!   [`RuleIntent::PickerOpened`] on the frame the listener opens it. The host
//!   calls [`riff_backend::app::views::SessionViews::picker_texts`] or
//!   [`riff_backend::app::views::SessionViews::picker_numbers`] once per
//!   intent, keyed on the field it names, and hands the answer back as the
//!   prop. Painting the row a hundred frames reads nothing: the row holds no
//!   port, so it cannot.
//! - *Scrolling an opened list into view.* egui 0.35 does not auto-scroll a
//!   keyboard-focused row inside a clipped pane, so a host in a `ScrollArea`
//!   must call `scroll_to_me` on the outcome's response when `draft.open`
//!   changes.
//! - *Naming a half-formed row.* [`RuleDraft::gap`] says which of field,
//!   comparison or value is missing, so ticket 18's Save refusal can point at
//!   the offending row. The row paints no reason line of its own: the tag
//!   editor's single-slot, self-naming copy is the precedent it defers to.
//!
//! The date widgets are ticket 20's: `DateTime` and `DateWindow` route to
//! [`crate::ui::rule_date`], whose stub is honest about being unrendered, and
//! the seam it plugs into is documented there.

use eframe::egui;
use riff_backend::domain::field_catalog::{ValueWidget, spec};
use riff_backend::domain::{Comparison, MatchableField, Rule, RuleValue};
use riff_gui_components::button::{TextButton, Variant, text_button};
use riff_gui_components::option_list::{OptionChoice, OptionList, option_list};
use riff_gui_components::text_field::{TextField, text_field};
use riff_gui_components::toggle_switch::toggle_switch;
use riff_gui_theme::icons::{Icon, IconCache};
use riff_gui_theme::theme::geometry::rule_row::{
    DURATION_PART_GAP, DURATION_PART_W, HEADER_H, HEADER_LABEL_PAD, LIST_TOP_GAP, NUMBER_W,
    PART_GAP, ROW_BOTTOM_GAP, TEXT_VALUE_W, WELL_H,
};
use riff_gui_theme::theme::{self, Palette};

use crate::ui::rule_date::{RuleDateDraft, RuleDateIntent, RuleDateKind, rule_date_row};

/// How many Library values one open picker offers.
///
/// A screenful and no more. The bound is the store's to enforce
/// ([`riff_backend::app::views::SessionViews::picker_texts`] answers at most
/// this many however large the Library is), and it is asked for once per
/// open — never once per frame.
pub const PICKER_LIMIT: usize = 50;

/// One comparison as the row offers it.
///
/// [`Comparison`] is the stored vocabulary and has nine members. The row
/// offers a tenth *word*: `is not missing` is not a stored operator —
/// [`Comparison::IsMissing`] with [`RuleDraft::negated`] set is what leaves
/// the row, because `negated` already inverts any test and one meaning spelled
/// two ways would make "has this tree changed?" comparisons lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonChoice {
    /// A stored comparison, shown under this module's word for it.
    Stored(Comparison),
    /// The negated spelling of [`Comparison::IsMissing`].
    IsNotMissing,
}

/// Which of the row's inline lists is open. One at a time: a Rule row lives in
/// a scrolling pane, and three open lists at once would push the next row off
/// the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulePanel {
    /// The Matchable Field list.
    Field,
    /// The selected field's comparison list.
    Comparison,
    /// The library-value picker for a pickable field.
    Value,
}

/// Why the value half is not an answer yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueGap {
    /// Nothing has been entered.
    Missing,
    /// Something was entered, and this field's type does not accept it.
    Unparseable,
}

/// Which half of the row's Rule is not there yet.
///
/// One answer, in reading order — the first part a listener still has to
/// supply — which is what lets ticket 18 name the offending row instead of
/// reporting that "a rule is incomplete".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleGap {
    /// No field has been chosen.
    Field,
    /// No comparison has been chosen, or the chosen one is not one the field
    /// permits — the illegal combination a field change would have reset.
    Comparison,
    /// A comparison that draws a value has no acceptable one.
    Value(ValueGap),
}

/// What one row reports. The row performs no effect: tickets 15–19 dispatch on
/// these, and [`RuleDraft::apply`] is the one place each lands in a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleIntent {
    /// The listener picked a Matchable Field. Applying it is what enforces "a
    /// field change resets a comparison the new field does not permit".
    FieldChosen {
        /// The field chosen.
        field: MatchableField,
    },
    /// The listener picked a comparison for the current field. `is not
    /// missing` arrives here already normalised to [`Comparison::IsMissing`]
    /// plus negation, so nothing downstream has to know the wording existed.
    ComparisonChosen {
        /// The choice clicked.
        choice: ComparisonChoice,
    },
    /// The single Not control changed. It inverts any test.
    NegationToggled {
        /// The state the control was set to.
        negated: bool,
    },
    /// One of the row's two structural lists opened or closed. The library
    /// picker has its own two intents instead, because opening it is a *read*.
    PanelToggled {
        /// Which list.
        panel: RulePanel,
        /// `true` when the list is now open.
        open: bool,
    },
    /// A value was accepted — on Enter, on blur, from the library picker, or
    /// from the yes/no list. The row parsed it against the field's own type,
    /// so an unparseable buffer never reaches this intent: [`RuleDraft::gap`]
    /// reports it as [`ValueGap::Unparseable`] instead.
    ValueCommitted {
        /// The typed answer.
        value: RuleValue,
    },
    /// The listener opened the library picker for `field`. **The host reads
    /// the picker's values in answer to this intent and only to this intent** —
    /// once per open.
    PickerOpened {
        /// The field whose values to read.
        field: MatchableField,
    },
    /// The listener closed the library picker, or chose from it. The host may
    /// drop the values it read for it.
    PickerClosed,
    /// One of ticket 20's date controls reported an edit. Carried through the
    /// row so that completing the date widgets adds no variant here.
    Date(RuleDateIntent),
}

/// The Library values the host read for this row's open picker.
///
/// A prop, not a port: the row cannot reach a store, which is the mechanical
/// reason a picker read cannot happen on a per-frame path.
#[derive(Debug, Clone, Copy)]
pub enum RulePickerValues<'a> {
    /// `picker_texts` answers for a pickable text field.
    Texts(&'a [String]),
    /// `picker_numbers` answers for a pickable numeric field.
    Numbers(&'a [i64]),
}

/// The value half of a draft, held as the buffers the catalog's widget ideas
/// call for — typed text stays text until it is committed, so a half-typed
/// value is still editable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleValueDraft {
    /// The comparison takes no value at all (`is missing`), or nothing has
    /// been chosen yet.
    NoValue,
    /// One needle well: the text fields, and the pickable fields beside their
    /// picker.
    Text(String),
    /// One whole-number well: year, play count, sample rate, channels.
    Number(String),
    /// Three wells merged into one whole-millisecond value. `hours` and
    /// `minutes` are whole numbers; `seconds` may carry a millisecond fraction
    /// to three digits, which is what keeps the merge lossless both ways —
    /// [`RuleValue::DurationMillis`] is whole milliseconds, and the store
    /// compares it against `duration_nanos` at 1 ms = 1e6 ns.
    Duration {
        /// The hours well.
        hours: String,
        /// The minutes well.
        minutes: String,
        /// The seconds well, optionally to millisecond precision.
        seconds: String,
    },
    /// The yes/no answer, chosen rather than typed.
    YesNo(bool),
    /// Ticket 20's controls, entered through [`crate::ui::rule_date`].
    Date {
        /// Which of the catalog's two date ideas this is.
        kind: RuleDateKind,
        /// The typed buffers.
        draft: RuleDateDraft,
    },
}

impl RuleValueDraft {
    /// The empty buffers the catalog's widget ideas for the current
    /// `(field, comparison)` call for. No ideas means no value.
    #[must_use]
    pub fn for_widgets(widgets: &'static [ValueWidget]) -> Self {
        if widgets.is_empty() {
            return Self::NoValue;
        }
        if widgets.contains(&ValueWidget::Duration) {
            return Self::Duration {
                hours: String::new(),
                minutes: String::new(),
                seconds: String::new(),
            };
        }
        if widgets.contains(&ValueWidget::DateWindow) {
            return Self::Date {
                kind: RuleDateKind::DateWindow,
                draft: RuleDateDraft::blank(),
            };
        }
        if widgets.contains(&ValueWidget::DateTime) {
            return Self::Date {
                kind: RuleDateKind::DateTime,
                draft: RuleDateDraft::blank(),
            };
        }
        if widgets.contains(&ValueWidget::YesNo) {
            return Self::YesNo(false);
        }
        if widgets.contains(&ValueWidget::Number) {
            return Self::Number(String::new());
        }
        Self::Text(String::new())
    }

    /// Buffers that re-show a stored or committed value, so reopening an
    /// editor never silently rewrites the Rule it displays.
    #[must_use]
    pub fn from_value(value: &RuleValue) -> Self {
        match value {
            RuleValue::Text(text) => Self::Text(text.clone()),
            RuleValue::Number(number) => Self::Number(number.to_string()),
            RuleValue::DurationMillis(millis) => {
                let (hours, minutes, seconds) = split_millis(*millis);
                Self::Duration {
                    hours,
                    minutes,
                    seconds,
                }
            }
            RuleValue::YesNo(yes) => Self::YesNo(*yes),
            RuleValue::Days(_) | RuleValue::DateTime(_) => Self::Date {
                kind: if matches!(value, RuleValue::Days(_)) {
                    RuleDateKind::DateWindow
                } else {
                    RuleDateKind::DateTime
                },
                draft: RuleDateDraft::from_value(value),
            },
        }
    }

    /// Handles into the typed buffers this draft was built for, so the row can
    /// hand [`text_field`] a mutable buffer without reaching through the draft
    /// twice. A chosen answer (yes/no) and ticket 20's date buffers have none:
    /// the first is a list, the second is drawn by [`rule_date_row`].
    fn buffers_mut(&mut self) -> Buffers<'_> {
        match self {
            Self::NoValue => Buffers::None,
            Self::Text(text) | Self::Number(text) => Buffers::One(text),
            Self::Duration {
                hours,
                minutes,
                seconds,
            } => Buffers::Duration {
                hours,
                minutes,
                seconds,
            },
            // Yes/no is a list and the date half draws its own controls, so
            // neither hands the row a text buffer.
            Self::YesNo(_) | Self::Date { .. } => Buffers::Choice,
        }
    }

    /// The answer these buffers spell for the widget ideas they were built
    /// for. See [`ParsedValue`].
    fn parse(&self, widgets: &'static [ValueWidget]) -> ParsedValue {
        if widgets.is_empty() {
            return match self {
                Self::NoValue => ParsedValue::NoValueWanted,
                _ => ParsedValue::Invalid,
            };
        }
        match self {
            Self::NoValue => ParsedValue::Missing,
            Self::Text(text) => {
                // Absent text and empty text are the same absence everywhere
                // else in riff, and a needle of only whitespace matches nothing
                // worth matching. The text itself is never trimmed: a Rule
                // tests the tag, and the tag is what the listener typed.
                if text.is_empty() || text.chars().all(char::is_whitespace) {
                    ParsedValue::Missing
                } else {
                    ParsedValue::Value(RuleValue::Text(text.clone()))
                }
            }
            Self::Number(text) => match text.trim().parse::<i64>() {
                Ok(number) => ParsedValue::Value(RuleValue::Number(number)),
                Err(_) if text.trim().is_empty() => ParsedValue::Missing,
                Err(_) => ParsedValue::Invalid,
            },
            Self::Duration {
                hours,
                minutes,
                seconds,
            } => match merge_millis(hours, minutes, seconds) {
                Boxed::Parsed(millis) => ParsedValue::Value(RuleValue::DurationMillis(millis)),
                Boxed::Invalid => ParsedValue::Invalid,
                Boxed::Empty => ParsedValue::Missing,
            },
            Self::YesNo(yes) => ParsedValue::Value(RuleValue::YesNo(*yes)),
            Self::Date { kind, draft } => draft
                .value(*kind)
                .map_or(ParsedValue::Missing, ParsedValue::Value),
        }
    }
}

/// The outcome of parsing a value draft's buffers.
enum ParsedValue {
    /// A whole answer.
    Value(RuleValue),
    /// Nothing entered yet.
    Missing,
    /// Entered, but not this field's type.
    Invalid,
    /// This comparison draws no value and holds none.
    NoValueWanted,
}

/// One duration box: empty, a whole number of milliseconds, or not a number.
enum Boxed {
    Empty,
    Parsed(u64),
    Invalid,
}

/// Borrowed handles into a value draft's typed buffers.
enum Buffers<'a> {
    /// Nothing to type into.
    None,
    /// One well, for a text or numeric field.
    One(&'a mut String),
    /// The duration's three wells.
    Duration {
        /// The hours well.
        hours: &'a mut String,
        /// The minutes well.
        minutes: &'a mut String,
        /// The seconds well.
        seconds: &'a mut String,
    },
    /// A chosen answer, with no buffer.
    Choice,
}

/// The row's editable state: what a Rule is becoming. The host owns one per
/// row slot — `Vec<RuleDraft>` is an editor's draft, exactly as the tag editor
/// owns one optional draft — and applies every reported intent through
/// [`RuleDraft::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDraft {
    /// The chosen Matchable Field, if any.
    pub field: Option<MatchableField>,
    /// The chosen comparison, if any. Always one the chosen field permits.
    pub comparison: Option<Comparison>,
    /// The single Not control. `is_not_missing` is not a stored operator: it
    /// is this flag beside [`Comparison::IsMissing`].
    pub negated: bool,
    /// The value half, as the catalog's widget ideas shape it.
    pub value: RuleValueDraft,
    /// Which inline list is open, if any. Presentation state that travels with
    /// the row it belongs to, so a host needs only one draft per row.
    pub open: Option<RulePanel>,
    /// The value the draft last landed, maintained by [`RuleDraft::apply`].
    ///
    /// Held so the row can keep its half of the selector's deduplication rule:
    /// `option_list` reports every click including the row already current, and
    /// a text field keeps reporting Enter while it holds focus. A second report
    /// of the same answer is a no-op for every host (tickets 15–18), so the row
    /// does not emit one — which is what stops one Enter becoming two live-count
    /// reads.
    committed: Option<RuleValue>,
}

impl RuleDraft {
    /// The empty row a new Rule starts from.
    #[must_use]
    pub fn blank() -> Self {
        Self {
            field: None,
            comparison: None,
            negated: false,
            value: RuleValueDraft::NoValue,
            open: None,
            committed: None,
        }
    }

    /// The draft that re-shows a stored Rule, so an editor can open a saved
    /// tree and hand back the same tree if nothing was changed.
    #[must_use]
    pub fn from_rule(rule: &Rule) -> Self {
        let widgets = widgets_for(Some(rule.field()), Some(rule.comparison()));
        let value = match rule.value() {
            Some(value) => RuleValueDraft::from_value(value),
            None => RuleValueDraft::for_widgets(widgets),
        };
        Self {
            field: Some(rule.field()),
            comparison: Some(rule.comparison()),
            negated: rule.is_negated(),
            value,
            open: None,
            committed: rule.value().cloned(),
        }
    }

    /// The catalog's display name for the chosen field — the words riff paints
    /// everywhere else, `Album Artist` included — or the row's own invitation
    /// while nothing is chosen.
    #[must_use]
    pub fn field_label(&self) -> &'static str {
        match self.field {
            Some(field) => spec(field).display_name(),
            None => "Choose a field",
        }
    }

    /// The comparisons this row offers, read straight from the catalog: the
    /// selected field's own list, plus `is not missing` exactly where the
    /// catalog says a value can be absent. Never a restated matrix.
    #[must_use]
    pub fn comparison_choices(&self) -> Vec<ComparisonChoice> {
        let Some(field) = self.field else {
            return Vec::new();
        };
        let row = spec(field);
        let mut choices: Vec<ComparisonChoice> = row
            .comparisons()
            .iter()
            .copied()
            .map(ComparisonChoice::Stored)
            .collect();
        if row.can_be_absent() {
            // The catalog lists `is missing` last, so the negated spelling
            // lands beside it rather than at the far end of the list.
            choices.push(ComparisonChoice::IsNotMissing);
        }
        choices
    }

    /// The choice the row currently shows, or `None` while no comparison is
    /// chosen. The negated spelling of `is missing` is one choice rather than
    /// two flags, so the header word and the list mark cannot disagree.
    #[must_use]
    pub fn comparison_choice(&self) -> Option<ComparisonChoice> {
        let comparison = self.comparison?;
        if comparison == Comparison::IsMissing && self.negated {
            Some(ComparisonChoice::IsNotMissing)
        } else {
            Some(ComparisonChoice::Stored(comparison))
        }
    }

    /// The word the row shows for its current comparison, or its invitation
    /// while none is chosen.
    #[must_use]
    pub fn current_comparison_label(&self) -> &'static str {
        self.comparison_choice()
            .map_or("Choose a comparison", ComparisonChoice::label)
    }
}

impl ComparisonChoice {
    /// The word the row shows for this choice. The catalog answers *which*
    /// comparison id a field uses; spelling the word is this module's, which
    /// is how `is` and `equal to` stay one meaning under two ids.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ComparisonChoice::IsNotMissing => "is not missing",
            ComparisonChoice::Stored(comparison) => match comparison {
                Comparison::Is => "is",
                Comparison::Contains => "contains",
                Comparison::EqualTo => "is equal to",
                Comparison::GreaterThan => "is greater than",
                Comparison::LessThan => "is less than",
                Comparison::InLast => "in the last",
                Comparison::Before => "is before",
                Comparison::After => "is after",
                Comparison::IsMissing => "is missing",
            },
        }
    }
}

impl RuleDraft {
    /// The catalog's widget ideas for the current `(field, comparison)` — empty
    /// where no value is entered, which is `is missing`'s case.
    #[must_use]
    pub fn value_widgets(&self) -> &'static [ValueWidget] {
        widgets_for(self.field, self.comparison)
    }

    /// Whether the current field and comparison want the Library's own values
    /// beside the typed input. The catalog's `ChooseFromLibrary` answer and
    /// nothing else: an affordance, never a constraint.
    #[must_use]
    pub fn takes_library_values(&self) -> bool {
        self.value_widgets()
            .contains(&ValueWidget::ChooseFromLibrary)
    }

    /// Which half is missing, in reading order, or `None` when the row is a
    /// whole Rule. Ticket 18 names the offending row with this.
    #[must_use]
    pub fn gap(&self) -> Option<RuleGap> {
        let Some(field) = self.field else {
            return Some(RuleGap::Field);
        };
        let Some(comparison) = self.comparison else {
            return Some(RuleGap::Comparison);
        };
        if !spec(field).permits(comparison) {
            return Some(RuleGap::Comparison);
        }
        match self.value.parse(widgets_for(Some(field), Some(comparison))) {
            ParsedValue::Value(_) | ParsedValue::NoValueWanted => None,
            ParsedValue::Missing => Some(RuleGap::Value(ValueGap::Missing)),
            ParsedValue::Invalid => Some(RuleGap::Value(ValueGap::Unparseable)),
        }
    }

    /// The Rule this draft spells, or `None` while it is half-filled — exactly
    /// when [`RuleDraft::gap`] answers.
    ///
    /// The catalog gets the last word: [`Rule::is_fully_formed`] asks the same
    /// two questions this row does, so a draft and a Rule can never disagree
    /// about what a whole Rule is.
    #[must_use]
    pub fn rule(&self) -> Option<Rule> {
        let field = self.field?;
        let comparison = self.comparison?;
        let value = match self.value.parse(widgets_for(Some(field), Some(comparison))) {
            ParsedValue::Value(value) => Some(value),
            ParsedValue::NoValueWanted => None,
            ParsedValue::Missing | ParsedValue::Invalid => return None,
        };
        let rule = if self.negated {
            Rule::new(field, comparison, value).negated()
        } else {
            Rule::new(field, comparison, value)
        };
        Some(rule).filter(Rule::is_fully_formed)
    }

    /// Apply one reported intent, landing it on the draft. Idempotent for
    /// every setter, so a host that re-applies an intent it already landed
    /// cannot corrupt a draft.
    pub fn apply(&mut self, intent: &RuleIntent) {
        match intent {
            RuleIntent::FieldChosen { field } => self.choose_field(*field),
            RuleIntent::ComparisonChosen { choice } => self.choose_comparison(*choice),
            RuleIntent::NegationToggled { negated } => self.negated = *negated,
            RuleIntent::PanelToggled { panel, open } => {
                if *panel != RulePanel::Value {
                    self.open = if *open { Some(*panel) } else { None };
                }
            }
            RuleIntent::PickerOpened { .. } => self.open = Some(RulePanel::Value),
            RuleIntent::PickerClosed => {
                if self.open == Some(RulePanel::Value) {
                    self.open = None;
                }
            }
            RuleIntent::ValueCommitted { value } => {
                let widgets = self.value_widgets();
                if widgets.is_empty() {
                    // A value cannot be drawn here, so it cannot be stored
                    // either: `is missing` keeps its value column NULL.
                    self.value = RuleValueDraft::NoValue;
                    self.committed = None;
                } else {
                    self.value = RuleValueDraft::from_value(value);
                    self.committed = Some(value.clone());
                }
            }
            RuleIntent::Date(date_intent) => {
                if let RuleValueDraft::Date { draft, .. } = &mut self.value {
                    match date_intent {
                        RuleDateIntent::Day(text) => draft.day.clone_from(text),
                        RuleDateIntent::Time(text) => draft.time.clone_from(text),
                        RuleDateIntent::WindowDays(text) => draft.window_days.clone_from(text),
                    }
                }
            }
        }
    }

    /// The last value this row **committed** — accepted on Enter, on blur,
    /// from the picker or from the yes/no list ([`Self::value`] is the live
    /// typing buffer beside it). `None` until one was committed, or while
    /// the buffers were rebuilt for a different widget.
    ///
    /// The two are deliberately readable apart: ticket 17's live match count
    /// derives its tree from the committed answers, because a shape derived
    /// from buffers would move on every keystroke and re-query with it. A
    /// host that must know what the listener has finished — rather than what
    /// they are in the middle of typing — reads this.
    #[must_use]
    pub fn committed_value(&self) -> Option<&RuleValue> {
        self.committed.as_ref()
    }

    /// Choose a field, and **reset a comparison the new field does not permit
    /// rather than keep an illegal combination**. The value buffers follow the
    /// widget ideas: the same ideas let the typed answer survive (a listener
    /// moving from Artist to Album keeps their needle), different ideas rebuild
    /// it as the new field wants it.
    fn choose_field(&mut self, field: MatchableField) {
        if self.field == Some(field) {
            return;
        }
        let before = self.value_widgets();
        self.field = Some(field);
        self.open = None;
        if !self
            .comparison
            .is_some_and(|comparison| spec(field).permits(comparison))
        {
            self.comparison = None;
            self.negated = false;
        }
        let after = self.value_widgets();
        if before != after {
            self.value = RuleValueDraft::for_widgets(after);
            self.committed = None;
        }
    }

    /// Choose a comparison, normalising `is not missing` on the way in to the
    /// one stored form it has — [`Comparison::IsMissing`] plus negation — and
    /// rebuilding the value buffers whenever the widget answer changes: `is
    /// missing` draws none, and `in the last` draws a window where `before`
    /// draws a day and a time.
    ///
    /// The two absence spellings own the negation flag outright, because they
    /// *are* the negation spelled in the list. For every other comparison the
    /// Not control is orthogonal and stays where the listener left it (story
    /// 24: invert any test without hunting for the opposite wording).
    fn choose_comparison(&mut self, choice: ComparisonChoice) {
        let (comparison, negated) = match choice {
            ComparisonChoice::Stored(comparison) => (comparison, false),
            ComparisonChoice::IsNotMissing => (Comparison::IsMissing, true),
        };
        let before = self.value_widgets();
        self.comparison = Some(comparison);
        if comparison == Comparison::IsMissing {
            self.negated = negated;
        }
        self.open = None;
        let after = self.value_widgets();
        if before != after {
            self.value = RuleValueDraft::for_widgets(after);
            self.committed = None;
        }
    }
}

/// The catalog's widget ideas for a `(field, comparison)` pair, asking the
/// catalog rather than answering for it.
fn widgets_for(
    field: Option<MatchableField>,
    comparison: Option<Comparison>,
) -> &'static [ValueWidget] {
    match (field, comparison) {
        (Some(field), Some(comparison)) => spec(field).value_widgets(comparison),
        _ => &[],
    }
}

/// What [`rule_row`] produced this frame: the row's typed intents, and its
/// rect for the host's own scroll and drag handling.
#[must_use]
pub struct RuleRowOutcome {
    /// Every intent the row reported, in the order the parts were drawn.
    pub intents: Vec<RuleIntent>,
    /// The row's own response.
    pub response: egui::Response,
}

/// Draw one Rule row — field, comparison, value, and the single Not control —
/// into the pane's layout flow, editing the draft's typed buffers in place and
/// reporting every other change as an intent.
///
/// `id` is this row's identity and every widget id derives from it, so a pane
/// of sibling rows cannot collide even when two rows show the same field or the
/// same Library value. `picker` is what the host read for this row's open
/// picker, `None` while none is open or the read has not come back.
///
/// The three lists are the component library's inline selector, one instance
/// per part with its own derived id; every header is a real button, so each
/// part is reachable by a click, by the keyboard and by the a11y tree.
#[must_use = "a row's output is what the listener just decided; dropping it                 loses the choice"]
pub fn rule_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    picker: Option<&RulePickerValues<'_>>,
) -> RuleRowOutcome {
    let mut intents = Vec::new();
    let outer = ui.horizontal(|ui| {
        draw_field_header(ui, cache, palette, id, draft, &mut intents);
        ui.add_space(PART_GAP);
        if draft.field.is_some() {
            draw_comparison_header(ui, cache, palette, id, draft, &mut intents);
            ui.add_space(PART_GAP);
        }
        // The switch's own accessible name is "Not", but a bare pill says
        // nothing to a listener reading the row as a sentence — and the golden
        // of this row proved it. The word is painted beside the control.
        ui.label(
            egui::RichText::new("Not")
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_2),
        );
        ui.add_space(theme::SPACE_XS);
        if toggle_switch(ui, palette, id.with("not"), "Not", draft.negated) {
            intents.push(RuleIntent::NegationToggled {
                negated: !draft.negated,
            });
        }
        ui.add_space(PART_GAP);
        draw_value(ui, cache, palette, id, draft, &mut intents);
    });
    draw_open_list(ui, palette, id, draft, picker, &mut intents);
    ui.add_space(ROW_BOTTOM_GAP);
    RuleRowOutcome {
        intents,
        response: outer.response,
    }
}

/// The field header: the catalog's display name, and the list it opens.
fn draw_field_header(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    intents: &mut Vec<RuleIntent>,
) {
    let open = draft.open == Some(RulePanel::Field);
    let pressed = header(
        ui,
        cache,
        palette,
        id.with(("field", "header")),
        draft.field_label(),
        open,
    );
    if pressed {
        intents.push(RuleIntent::PanelToggled {
            panel: RulePanel::Field,
            open: !open,
        });
    }
}

/// The comparison header: this module's word for the catalog's chosen id.
fn draw_comparison_header(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    intents: &mut Vec<RuleIntent>,
) {
    let open = draft.open == Some(RulePanel::Comparison);
    let pressed = header(
        ui,
        cache,
        palette,
        id.with(("comparison", "header")),
        draft.current_comparison_label(),
        open,
    );
    if pressed {
        intents.push(RuleIntent::PanelToggled {
            panel: RulePanel::Comparison,
            open: !open,
        });
    }
}

/// One disclosure header, and whether it was pressed this frame. A real
/// button, because a painted rectangle senses nothing.
fn header(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    label: &str,
    open: bool,
) -> bool {
    let width = header_width(ui, palette, label);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEADER_H), egui::Sense::hover());
    text_button(
        ui,
        cache,
        palette,
        false,
        &TextButton {
            id,
            rect,
            label,
            a11y: label,
            tooltip: None,
            icon: Some(if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            }),
            small: true,
            variant: Variant::Secondary,
            enabled: true,
        },
    )
}

/// A header is as wide as its label plus its glyph, bounded by the pane, so
/// the four parts still sit on one line at the stage's minimum width.
fn header_width(ui: &egui::Ui, palette: &Palette, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        theme::styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS),
        palette.ink,
    );
    (galley.size().x + HEADER_LABEL_PAD).min(ui.available_width().max(0.0))
}

/// The value part: whatever control the catalog's widget idea names, plus the
/// picker affordance beside a pickable field's typed well.
fn draw_value(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    intents: &mut Vec<RuleIntent>,
) {
    let widgets = draft.value_widgets();
    if widgets.is_empty() {
        return;
    }
    if widgets.contains(&ValueWidget::YesNo) {
        draw_yes_no(ui, palette, id, draft, intents);
    } else if widgets.contains(&ValueWidget::DateWindow) || widgets.contains(&ValueWidget::DateTime)
    {
        draw_date(ui, cache, palette, id, draft, intents);
    } else {
        let numeric = widgets.contains(&ValueWidget::Number);
        let hint = if numeric { "0" } else { "value" };
        let width = if numeric { NUMBER_W } else { TEXT_VALUE_W };
        let mut commit = false;
        match draft.value.buffers_mut() {
            Buffers::One(buffer) => {
                commit = typed_well(
                    ui,
                    cache,
                    palette,
                    id.with(("value", 0)),
                    buffer,
                    hint,
                    width,
                );
            }
            Buffers::Duration {
                hours,
                minutes,
                seconds,
            } => {
                for (slot, (buffer, box_hint)) in
                    [(hours, "hh"), (minutes, "mm"), (seconds, "ss.mmm")]
                        .into_iter()
                        .enumerate()
                {
                    if slot > 0 {
                        ui.add_space(DURATION_PART_GAP);
                    }
                    commit |= typed_well(
                        ui,
                        cache,
                        palette,
                        id.with(("value", slot)),
                        buffer,
                        box_hint,
                        DURATION_PART_W,
                    );
                }
            }
            Buffers::None | Buffers::Choice => {}
        }
        if commit {
            commit_value(draft, intents);
        }
    }
    if draft.takes_library_values() {
        ui.add_space(PART_GAP);
        let open = draft.open == Some(RulePanel::Value);
        let pressed = header(
            ui,
            cache,
            palette,
            id.with(("picker", "header")),
            if open { "Hide values" } else { "Choose…" },
            open,
        );
        if pressed {
            intents.push(if open {
                RuleIntent::PickerClosed
            } else {
                match draft.field {
                    Some(field) => RuleIntent::PickerOpened { field },
                    None => RuleIntent::PickerClosed,
                }
            });
        }
    }
}

/// A value accepted on Enter or on blur: re-parse the buffers the shared text
/// fields just edited, and report only what this field's type can read.
fn commit_value(draft: &RuleDraft, intents: &mut Vec<RuleIntent>) {
    if let ParsedValue::Value(value) = draft.value.parse(draft.value_widgets()) {
        report_value(draft, value, intents);
    }
}

/// Report a chosen or typed answer, once. Clicking the option already current,
/// or pressing Enter twice over an unchanged buffer, is the host's no-op — the
/// selector's deduplication rule, kept by the row that uses it.
fn report_value(draft: &RuleDraft, value: RuleValue, intents: &mut Vec<RuleIntent>) {
    if draft.committed.as_ref() == Some(&value) {
        return;
    }
    intents.push(RuleIntent::ValueCommitted { value });
}

/// One typed well, and whether the listener asked it to commit this frame.
///
/// Committing is Enter while the well holds focus, or the frame the well loses
/// focus — a listener who tabs away from a typed value means it. The buffer
/// itself is edited in place by the shared field, which is that widget's
/// standing contract; the *answer* is the row's intent.
fn typed_well(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    buffer: &mut String,
    hint: &str,
    width: f32,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, WELL_H), egui::Sense::hover());
    let response = text_field(
        ui,
        cache,
        palette,
        buffer,
        &TextField {
            id,
            rect,
            hint,
            leading_icon: None,
            clear_label: None,
            // Escape belongs to the editor that hosts the row, never to the
            // well the cursor happens to sit in (the tag editor's rule).
            dismiss_on_escape: false,
        },
    );
    let focused = ui.memory(|mem| mem.has_focus(id));
    let enter = focused && ui.input(|input| input.key_pressed(egui::Key::Enter));
    enter || response.lost_focus()
}

/// The yes/no answer: the catalog's `Yes/No` idea drawn as the selector's two
/// options. A toggle would hide one half of the answer behind a word that is
/// not the answer.
fn draw_yes_no(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    intents: &mut Vec<RuleIntent>,
) {
    let current = match &draft.value {
        RuleValueDraft::YesNo(yes) => Some(RuleValue::YesNo(*yes)),
        _ => None,
    };
    let options = [yes_no_choice(true), yes_no_choice(false)];
    if let Some(value) = option_list(
        ui,
        palette,
        &OptionList {
            id: id.with("yes-no"),
            options: &options,
            current: current.as_ref(),
        },
    ) {
        report_value(draft, value, intents);
    }
}

/// One option of the yes/no control.
fn yes_no_choice(yes: bool) -> OptionChoice<'static, RuleValue> {
    OptionChoice {
        key: RuleValue::YesNo(yes),
        label: if yes { "Yes" } else { "No" },
    }
}

/// Ticket 20's date control, through the seam in [`crate::ui::rule_date`].
fn draw_date(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    intents: &mut Vec<RuleIntent>,
) {
    if let RuleValueDraft::Date { kind, draft: date } = &mut draft.value {
        let kind = *kind;
        for intent in rule_date_row(ui, cache, palette, id.with("date"), kind, date) {
            intents.push(RuleIntent::Date(intent));
        }
    }
}

/// The one list this row has open, drawn inline at the pane's full width —
/// which is what keeps every option reachable: nothing here is a popup, so no
/// parent clip rect can silently exclude a choice.
fn draw_open_list(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    picker: Option<&RulePickerValues<'_>>,
    intents: &mut Vec<RuleIntent>,
) {
    let Some(panel) = draft.open else {
        return;
    };
    ui.add_space(LIST_TOP_GAP);
    match panel {
        RulePanel::Field => {
            let options: Vec<OptionChoice<'_, MatchableField>> = MatchableField::ALL
                .iter()
                .map(|field| OptionChoice {
                    key: *field,
                    label: spec(*field).display_name(),
                })
                .collect();
            if let Some(field) = option_list(
                ui,
                palette,
                &OptionList {
                    id: id.with("field-list"),
                    options: &options,
                    current: draft.field.as_ref(),
                },
            ) {
                intents.push(RuleIntent::FieldChosen { field });
            }
        }
        RulePanel::Comparison => {
            let choices = draft.comparison_choices();
            let current = draft.comparison_choice();
            let options: Vec<OptionChoice<'_, ComparisonChoice>> = choices
                .iter()
                .map(|choice| OptionChoice {
                    key: *choice,
                    label: choice.label(),
                })
                .collect();
            if let Some(choice) = option_list(
                ui,
                palette,
                &OptionList {
                    id: id.with("comparison-list"),
                    options: &options,
                    current: current.as_ref(),
                },
            ) {
                intents.push(RuleIntent::ComparisonChosen { choice });
            }
        }
        RulePanel::Value => draw_picker_list(ui, palette, id, draft, picker, intents),
    }
}

/// The open library picker. Absent values are the Library's to exclude
/// (`picker_texts` drops a NULL or all-whitespace tag), so what is listed is
/// what the Library actually holds — and typing remains open either way.
fn draw_picker_list(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: egui::Id,
    draft: &mut RuleDraft,
    picker: Option<&RulePickerValues<'_>>,
    intents: &mut Vec<RuleIntent>,
) {
    let Some(picker) = picker else {
        return;
    };
    let (values, labels) = picker_payload(picker);
    let options: Vec<OptionChoice<'_, RuleValue>> = labels
        .iter()
        .zip(&values)
        .map(|(label, value)| OptionChoice {
            key: value.clone(),
            label: label.as_str(),
        })
        .collect();
    let current = match &draft.value {
        RuleValueDraft::Text(text) if !text.is_empty() => Some(RuleValue::Text(text.clone())),
        RuleValueDraft::Number(text) => text.trim().parse::<i64>().ok().map(RuleValue::Number),
        _ => None,
    };
    if let Some(value) = option_list(
        ui,
        palette,
        &OptionList {
            id: id.with("picker-list"),
            options: &options,
            current: current.as_ref(),
        },
    ) {
        report_value(draft, value, intents);
        intents.push(RuleIntent::PickerClosed);
    }
}

/// The picker's options as `(value, label)` pairs. The value is what a click
/// commits — a Library number and a Library string go through the same report
/// — and the labels are owned here so the options can borrow them for the
/// frame they are drawn in.
fn picker_payload(picker: &RulePickerValues<'_>) -> (Vec<RuleValue>, Vec<String>) {
    match picker {
        RulePickerValues::Texts(values) => {
            let labels = values.to_vec();
            (
                values.iter().cloned().map(RuleValue::Text).collect(),
                labels,
            )
        }
        RulePickerValues::Numbers(values) => {
            let labels = values.iter().map(ToString::to_string).collect();
            (
                values.iter().copied().map(RuleValue::Number).collect(),
                labels,
            )
        }
    }
}

/// The duration's three boxes merged into one whole-millisecond value.
///
/// Integer arithmetic only, and only ever checked: 1 hour is `3_600_000` ms, the
/// store's column is nanoseconds at 1 ms = 1e6 ns, so a merged value that
/// survives `checked_mul` is exactly representable — and a huge one refuses
/// through [`ValueGap::Unparseable`] rather than wrapping into a wrong Rule. An
/// empty box reads as zero (a listener typing `30` into minutes means half an
/// hour); three empty boxes are no value at all.
fn merge_millis(hours: &str, minutes: &str, seconds: &str) -> Boxed {
    let (hours, minutes, seconds) = (
        parse_whole(hours),
        parse_whole(minutes),
        parse_seconds(seconds),
    );
    if matches!(hours, Boxed::Invalid)
        || matches!(minutes, Boxed::Invalid)
        || matches!(seconds, Boxed::Invalid)
    {
        return Boxed::Invalid;
    }
    if matches!(hours, Boxed::Empty)
        && matches!(minutes, Boxed::Empty)
        && matches!(seconds, Boxed::Empty)
    {
        return Boxed::Empty;
    }
    let mut total = 0_u64;
    if let Boxed::Parsed(value) = hours {
        let Some(next) = value
            .checked_mul(3_600_000)
            .and_then(|base| base.checked_add(total))
        else {
            return Boxed::Invalid;
        };
        total = next;
    }
    if let Boxed::Parsed(value) = minutes {
        let Some(next) = value
            .checked_mul(60_000)
            .and_then(|base| base.checked_add(total))
        else {
            return Boxed::Invalid;
        };
        total = next;
    }
    // `parse_seconds` already answered in milliseconds, fraction included.
    if let Boxed::Parsed(value) = seconds {
        let Some(next) = total.checked_add(value) else {
            return Boxed::Invalid;
        };
        total = next;
    }
    Boxed::Parsed(total)
}

/// One whole-number duration box.
fn parse_whole(text: &str) -> Boxed {
    let text = text.trim();
    if text.is_empty() {
        return Boxed::Empty;
    }
    text.parse::<u64>().map_or(Boxed::Invalid, Boxed::Parsed)
}

/// One seconds box, optionally carrying a millisecond fraction, answered in
/// milliseconds. More than three fraction digits is refused rather than
/// rounded: the stored value is whole milliseconds, and quietly rounding a
/// listener's `1.0005` would make the merge lossy.
fn parse_seconds(text: &str) -> Boxed {
    let text = text.trim();
    if text.is_empty() {
        return Boxed::Empty;
    }
    let Some((whole, fraction)) = text.split_once('.') else {
        return match parse_whole(text) {
            Boxed::Parsed(value) => value
                .checked_mul(1_000)
                .map_or(Boxed::Invalid, Boxed::Parsed),
            other => other,
        };
    };
    if fraction.is_empty() || fraction.len() > 3 {
        return Boxed::Invalid;
    }
    let units = if whole.is_empty() {
        0_u64
    } else {
        match whole.parse::<u64>() {
            Ok(value) => value,
            Err(_) => return Boxed::Invalid,
        }
    };
    let padded = format!("{fraction:0<3}");
    match padded.parse::<u64>() {
        Ok(millis) => units
            .checked_mul(1_000)
            .and_then(|base| base.checked_add(millis))
            .map_or(Boxed::Invalid, Boxed::Parsed),
        Err(_) => Boxed::Invalid,
    }
}

/// The three boxes a stored millisecond count spells — the exact inverse of
/// [`merge_millis`], pinned by a round-trip test rather than asserted here.
fn split_millis(millis: u64) -> (String, String, String) {
    let hours = millis / 3_600_000;
    let rest = millis % 3_600_000;
    let minutes = rest / 60_000;
    let rest = rest % 60_000;
    let seconds = rest / 1_000;
    let fraction = rest % 1_000;
    let seconds = if fraction == 0 {
        seconds.to_string()
    } else {
        let padded = format!("{fraction:0>3}");
        format!("{seconds}.{}", padded.trim_end_matches('0'))
    };
    (hours.to_string(), minutes.to_string(), seconds)
}
