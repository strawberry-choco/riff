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
use riff_gui_theme::theme::Palette;
use riff_gui_theme::theme::geometry::rule_date::{PART_GAP, PICKER_W, TIME_PART_W};
use riff_gui_theme::theme::geometry::rule_row::{NUMBER_W, WELL_H};

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

    /// The stored value these buffers spell, or `None` while they do not
    /// spell one.
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
            RuleDateKind::DateTime => self
                .parts()
                .filter(civil_is_storable)
                .map(RuleValue::DateTime),
        }
    }

    /// The parts of a `YYYY-MM-DD HH:MM:SS` pair. The shape a stored
    /// [`CivilDateTime`] round-trips through is fixed by this module and not
    /// invented elsewhere: two digits of month or day, four of year, and
    /// whole seconds only — the same text the store's comparison is written
    /// against.
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

/// Whether a parsed pair is a civil instant the stored text shape can mean:
/// a four-digit year and real month, day and time ranges. A part outside
/// them would spell text no stamp orders beside, so it refuses rather than
/// stores — the calendar cannot produce one (`jiff`'s civil years are
/// four-digit), and nothing else writes the day buffer.
fn civil_is_storable(civil: &CivilDateTime) -> bool {
    (0..=9999).contains(&civil.year)
        && (1..=12).contains(&civil.month)
        && (1..=31).contains(&civil.day)
        && civil.hour < 24
        && civil.minute < 60
        && civil.second < 60
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
/// [`crate::ui::rule_row::RuleIntent::Date`] unchanged — these three carry
/// every date edit there is, so the row never grew a second spelling for one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDateIntent {
    /// The new contents of the day buffer.
    Day(String),
    /// The new contents of the time buffer.
    Time(String),
    /// The new contents of the window-of-days buffer.
    WindowDays(String),
}

/// Draw one date control and report what the listener typed into it. `id` is
/// the row's widget identity for this control — the caller derives it, so
/// two rows in one pane cannot collide.
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
        RuleDateKind::DateTime => draw_fixed(ui, cache, palette, id, draft, &mut intents),
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

/// The day the calendar shows while the day buffer is empty: the civil
/// epoch, and deliberately **not** today — a control seeded from the wall
/// clock would open at a different month every day and could not be pinned
/// by any test. Nothing is stored until the listener saves a picked day; an
/// empty buffer spells no value at all.
const EMPTY_DAY: &str = "1970-01-01";

/// The fixed date: a calendar day from `egui_extras`' date picker beside the
/// time as three plain number wells. The pair is one stored zone-less stamp
/// — see [`RuleDateDraft::value`] for why nothing here touches a clock.
///
/// The picker's own calendar type (`jiff::civil::Date`) is deliberately
/// never spelled in this module: `jiff` reaches the crate only through the
/// datepicker feature, and naming it would demand a direct dependency line
/// this design does not take. The seed and the picked-day read go through
/// inference and public methods only.
///
/// The open popup is never a golden: it paints `jiff`'s *today* at paint
/// time, which the harness's settled clock does not pin, so a baseline with
/// it open would redden on the next calendar day. Its reachability is proven
/// by a real click through a composed frame instead
/// (`tests/ui_tests.rs::rule_row_ui_tests`).
fn draw_fixed(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    id: egui::Id,
    draft: &RuleDateDraft,
    intents: &mut Vec<RuleDateIntent>,
) {
    let mut day = draft
        .day
        .parse()
        .unwrap_or_else(|_| EMPTY_DAY.parse().expect("the seed is a civil date"));
    let salt = id.with("picker").value().to_string();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(PICKER_W, WELL_H), egui::Sense::hover());
    let picked = ui.put(
        rect,
        epi::DatePickerButton::new(&mut day)
            .id_salt(&salt)
            // The picker's calendar emoji is not riff's icon set and is not
            // in the shipped font textures; the date itself is the label a
            // listener — and the harness — reads.
            .show_icon(false),
    );
    if picked.changed() {
        intents.push(RuleDateIntent::Day(format!(
            "{:04}-{:02}-{:02}",
            day.year(),
            day.month(),
            day.day()
        )));
    }
    let (mut hour, mut minute, mut second) = split_time_text(&draft.time);
    for (slot, (buffer, hint)) in [(&mut hour, "hh"), (&mut minute, "mm"), (&mut second, "ss")]
        .into_iter()
        .enumerate()
    {
        ui.add_space(PART_GAP);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(TIME_PART_W, WELL_H), egui::Sense::hover());
        text_field(
            ui,
            cache,
            palette,
            buffer,
            &TextField {
                id: id.with(("time", slot)),
                rect,
                hint,
                leading_icon: None,
                clear_label: None,
                dismiss_on_escape: false,
            },
        );
    }
    let merged = merge_time_text(&hour, &minute, &second);
    if merged != draft.time {
        intents.push(RuleDateIntent::Time(merged));
    }
}

/// `HH:MM:SS` split into its three wells. Any other text is shown as the
/// pieces around the first two colons: a stray colon simply never spells a
/// value, and split-then-merge is the identity on every string a listener
/// can have typed, so nothing shown is quietly rewritten while they fix it.
fn split_time_text(text: &str) -> (String, String, String) {
    if text.is_empty() {
        return (String::new(), String::new(), String::new());
    }
    let mut pieces = text.splitn(3, ':').map(str::to_owned);
    (
        pieces.next().unwrap_or_default(),
        pieces.next().unwrap_or_default(),
        pieces.next().unwrap_or_default(),
    )
}

/// The three wells as the one `HH:MM:SS` buffer — empty when all three are,
/// rather than a bare `::`, so a fresh pair reports nothing.
fn merge_time_text(hour: &str, minute: &str, second: &str) -> String {
    if hour.is_empty() && minute.is_empty() && second.is_empty() {
        return String::new();
    }
    format!("{hour}:{minute}:{second}")
}
