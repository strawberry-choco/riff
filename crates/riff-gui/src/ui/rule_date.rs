//! The date half of a Rule row — **ticket 20's seam, stubbed here by design**.
//!
//! The catalog answers `ValueWidget::DateTime` for `before`/`after` and
//! `ValueWidget::DateWindow` for `in the last` (ADR 0021's zone-less dates);
//! the row that assembles the parts routes both ideas to this module and
//! nothing else. Two items are the whole contract, and ticket 20 replaces
//! their bodies without touching [`crate::ui::rule_row`] again:
//!
//! - [`RuleDateDraft::value`], the parse from typed buffers to the
//!   [`RuleValue`] a date Rule stores (`Days` for a window, `DateTime` for a
//!   fixed moment) — the place `YYYY-MM-DD HH:MM:SS` and a whole day count
//!   get their shapes.
//! - [`rule_date_row`], the paint-and-report half.
//!
//! Until then the stub is honest about being unrendered: it paints one label
//! naming what is missing, takes no input, and reports no intent, so
//! [`RuleDateDraft::value`] answers `None` and a date Rule stays
//! half-formed — which [`crate::ui::rule_row::RuleDraft::gap`] reports as a
//! missing **value**, pointing at this module rather than pretending a
//! calendar exists. No calendar is built here, and no date library is
//! pulled in: `CivilDateTime` carries its parts precisely so this crate
//! never has to name one.

use eframe::egui;
use riff_backend::domain::{CivilDateTime, RuleValue};
use riff_gui_theme::icons::IconCache;
use riff_gui_theme::theme::{self, Palette};

/// Which of the catalog's two date ideas this row is drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDateKind {
    /// A day plus a typed time, zone-less: the listener's own clock on the
    /// machine that reads the Rule. `before` and `after`.
    DateTime,
    /// A window of whole days counted back from the read. `in the last`.
    DateWindow,
}

/// The typed buffers behind the date controls.
///
/// Strings, not parts: a half-typed date is still a buffer a listener can
/// keep editing, and the parse to [`RuleValue`] happens once, in
/// [`RuleDateDraft::value`], exactly like the duration's three boxes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDateDraft {
    /// The calendar day as `YYYY-MM-DD`, typed into the fixed-date control.
    pub day: String,
    /// The time of day as `HH:MM:SS`, typed into the fixed-date control.
    pub time: String,
    /// The window as a whole number of days, typed into the day-window control.
    pub window_days: String,
}

impl RuleDateDraft {
    /// The empty buffers a fresh date row starts with.
    #[must_use]
    pub fn blank() -> Self {
        Self {
            day: String::new(),
            time: String::new(),
            window_days: String::new(),
        }
    }

    /// The buffers that re-show a stored date value, so reopening an editor
    /// does not silently rewrite the Rule it displays.
    #[must_use]
    pub fn from_value(value: &RuleValue) -> Self {
        let mut draft = Self::blank();
        match value {
            RuleValue::Days(days) => draft.window_days = days.to_string(),
            RuleValue::DateTime(moment) => {
                draft.day = format!("{:04}-{:02}-{:02}", moment.year, moment.month, moment.day);
                draft.time = format!(
                    "{:02}:{:02}:{:02}",
                    moment.hour, moment.minute, moment.second
                );
            }
            _ => {}
        }
        draft
    }

    /// **The seam ticket 20 implements.** The stored value these buffers
    /// spell, or `None` while they do not spell one.
    ///
    /// The row treats `None` as a missing value, so an incomplete or invalid
    /// date is reported by [`crate::ui::rule_row::RuleDraft::gap`] and the
    /// editor refuses to save it — the behaviour is already wired, only the
    /// parse is not. Today this always answers `None`: the controls that
    /// would fill the buffers land in ticket 20.
    #[must_use]
    #[allow(
        clippy::unused_self,
        reason = "the stub reads no buffer; ticket 20's parse reads all three"
    )]
    pub fn value(&self, _kind: RuleDateKind) -> Option<RuleValue> {
        None
    }

    /// The parts of a `YYYY-MM-DD HH:MM:SS` pair, once ticket 20 has a
    /// calendar to fill them. Kept here so the shape a stored
    /// [`CivilDateTime`] round-trips through is fixed by this module and not
    /// invented later: two digits of month or day, six digits of year, and
    /// whole seconds only.
    #[must_use]
    pub fn parts(&self) -> Option<CivilDateTime> {
        let (year, month, day) = split_day(&self.day)?;
        let (hour, minute, second) = split_time(&self.time)?;
        Some(CivilDateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }
}

/// `YYYY-MM-DD` as parts, or `None` for anything else.
fn split_day(text: &str) -> Option<(i32, u8, u8)> {
    let mut pieces = text.split('-');
    let year = pieces.next()?.parse().ok()?;
    let month = pieces.next()?.parse().ok()?;
    let day = pieces.next()?.parse().ok()?;
    if pieces.next().is_some() {
        return None;
    }
    Some((year, month, day))
}

/// `HH:MM:SS` as parts, or `None` for anything else.
fn split_time(text: &str) -> Option<(u8, u8, u8)> {
    let mut pieces = text.split(':');
    let hour = pieces.next()?.parse().ok()?;
    let minute = pieces.next()?.parse().ok()?;
    let second = pieces.next()?.parse().ok()?;
    if pieces.next().is_some() {
        return None;
    }
    Some((hour, minute, second))
}

/// What the date controls report: a buffer the listener edited.
///
/// The row maps these into
/// [`crate::ui::rule_row::RuleIntent::Date`] unchanged, so ticket 20 adds no
/// intent variant when it replaces [`rule_date_row`]'s body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDateIntent {
    /// The new contents of the day buffer.
    Day(String),
    /// The new contents of the time buffer.
    Time(String),
    /// The new contents of the window-of-days buffer.
    WindowDays(String),
}

/// **The seam ticket 20 implements.** Draw one date control and report what
/// the listener typed into it. `id` is the row's widget identity for this
/// control — the caller derives it, so two rows in one pane cannot collide.
///
/// The stub paints one honest label and reports nothing; a real
/// implementation keeps this module's contract — props in, typed intents
/// out, every design value read from the theme store, the caller's buffers
/// the only thing mutated (here through the intents, applied by
/// [`crate::ui::rule_row::RuleDraft::apply`]).
#[must_use]
pub fn rule_date_row(
    ui: &mut egui::Ui,
    _cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    kind: RuleDateKind,
    _draft: &mut RuleDateDraft,
) -> Vec<RuleDateIntent> {
    let _ = id;
    let word = match kind {
        RuleDateKind::DateTime => "a date and time",
        RuleDateKind::DateWindow => "a window of days",
    };
    ui.label(
        egui::RichText::new(format!(
            "date entry for {word} lands in ticket 20 — not rendered by this build"
        ))
        .text_style(egui::TextStyle::Small)
        .color(palette.ink_3),
    );
    ui.add_space(theme::SPACE_XS);
    Vec::new()
}
