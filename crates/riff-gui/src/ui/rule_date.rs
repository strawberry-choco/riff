//! The date half of a Rule row — a day window, or a fixed date with a time.
//!
//! The catalog answers `ValueWidget::DateTime` for `before`/`after` and
//! `ValueWidget::DateWindow` for `in the last` (ADR 0021's zone-less dates);
//! the row that assembles the parts routes both ideas to this module and
//! nothing else. Two items are the whole contract:
//!
//! - [`RuleDateDraft::value`], the parse from typed buffers to the
//!   [`RuleValue`] a date Rule stores (`Days` for a window, `DateTime` for a
//!   fixed moment) — the place `YYYY-MM-DD HH:MM:SS` and a whole day count
//!   get their shapes.
//! - [`rule_date_row`], the paint-and-report half.
//!
//! **The row's contract is the crate's standing one: props in, typed intents
//! out.** [`rule_date_row`] performs no effect, and it does not write the
//! [`RuleDateDraft`] it is handed either: every edit leaves as a
//! [`RuleDateIntent`], and [`crate::ui::rule_row::RuleDraft::apply`] is the
//! one place a buffer changes. A half-typed pair is therefore never an
//! error — [`RuleDateDraft::value`] answers `None`, which
//! [`crate::ui::rule_row::RuleDraft::gap`] reports like any other half-filled
//! Rule, and the buffers keep what the listener typed so the edit can
//! continue.
//!
//! The calendar is the one third-party widget this feature accepts: the
//! `datepicker` on the frontend's existing `egui_extras`, whose `jiff` cost
//! ticket 07 priced. The typed time stays three plain number wells — a full
//! date-and-time control is not worth a second dependency. Storage is the
//! decision, not the widget: [`RuleDateDraft::value`] records why a window
//! is a whole day count and a fixed boundary is zone-less.

use eframe::egui;
use riff_backend::domain::{CivilDateTime, RuleValue};
use riff_gui_components::text_field::{TextField, text_field};
use riff_gui_theme::icons::IconCache;
use riff_gui_theme::theme::geometry::rule_row::{NUMBER_W, WELL_H};
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
    /// `None` is the whole failure surface: the row treats it as a missing
    /// value, so a partly typed pair holds the draft open and
    /// [`crate::ui::rule_row::RuleDraft::gap`] reports it like any other
    /// half-filled Rule — never an error dialog, and never a silently
    /// coerced answer.
    ///
    /// The two kinds store differently on purpose, and that difference is
    /// the feature's decision, not the widget's convenience:
    ///
    /// - a window is a **whole number of days** ([`RuleValue::Days`]): a day
    ///   count is the finest resolution a Rule gives, and the store turns it
    ///   into a cutoff computed **at read time** from the same epoch clock
    ///   the history facts were written with, so the same saved Rule keeps
    ///   meaning "recently" tomorrow;
    /// - a fixed boundary is the **zone-less** day and time exactly as typed
    ///   ([`RuleValue::DateTime`] carrying a [`CivilDateTime`]): `before` and
    ///   `after` are read against the listener's own clock on the machine
    ///   doing the reading, so nothing here resolves the pair through a
    ///   clock or a zone — storing a zone would make one saved Rule answer
    ///   differently on a machine in another zone, the worse surprise for an
    ///   offline desktop player.
    #[must_use]
    pub fn value(&self, kind: RuleDateKind) -> Option<RuleValue> {
        match kind {
            RuleDateKind::DateWindow => whole_days(&self.window_days).map(RuleValue::Days),
            RuleDateKind::DateTime => None,
        }
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

/// A whole day count, or `None` for anything else — including the empty
/// buffer, which is an absence rather than a window of zero days.
fn whole_days(text: &str) -> Option<u32> {
    text.trim().parse().ok()
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
/// Every edit leaves as a [`RuleDateIntent`]; `draft` is read, never written
/// — the caller lands what is reported through
/// [`crate::ui::rule_row::RuleDraft::apply`], which is the one writer of the
/// buffers. A single intent per part is the report: the well hands its edited
/// contents, the calendar its saved day, and a repaint that changed nothing
/// reports nothing.
#[must_use]
pub fn rule_date_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    kind: RuleDateKind,
    draft: &mut RuleDateDraft,
) -> Vec<RuleDateIntent> {
    let mut intents = Vec::new();
    match kind {
        RuleDateKind::DateWindow => draw_window(ui, cache, palette, id, draft, &mut intents),
        RuleDateKind::DateTime => {
            // The calendar-and-time half is the next slice of ticket 20 and
            // stays honest about being unrendered until it lands.
            let _ = (cache, draft);
            ui.label(
                egui::RichText::new(
                    "date entry for a date and time lands in ticket 20 — not rendered by this build",
                )
                .text_style(egui::TextStyle::Small)
                .color(palette.ink_3),
            );
            ui.add_space(theme::SPACE_XS);
        }
    }
    intents
}

/// The day window's one well: a whole number of days, with no unit to choose
/// — a day count is the finest resolution a Rule gives, and the store turns
/// it into a cutoff at read time.
fn draw_window(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    draft: &RuleDateDraft,
    intents: &mut Vec<RuleDateIntent>,
) {
    let mut buffer = draft.window_days.clone();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(NUMBER_W, WELL_H), egui::Sense::hover());
    text_field(
        ui,
        cache,
        palette,
        &mut buffer,
        &TextField {
            id: id.with("window"),
            rect,
            hint: "days",
            leading_icon: None,
            clear_label: None,
            // Escape belongs to the editor that hosts the row, never to the
            // well the cursor happens to sit in (the tag editor's rule).
            dismiss_on_escape: false,
        },
    );
    if buffer != draft.window_days {
        intents.push(RuleDateIntent::WindowDays(buffer));
    }
}
