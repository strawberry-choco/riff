//! Compiles one Custom Smart Playlist [`RuleTree`] into **one SQL predicate**
//! over the `tracks` table, and answers it as a Listing Page.
//!
//! Three rules shape everything here, and each exists because the naive
//! spelling is wrong in a way a passing test would not notice:
//!
//! * **No join, ever.** A Rule tests the Track's own tag, never the album or
//!   artist entity's, so every column comes from this module's static
//!   [`FIELD_COLUMNS`] table keyed by the stored field id. An id that is not in
//!   that table is refused before a single character of SQL is emitted.
//!   (`docs/adr/0003-store-query-model.md` fixes the ordering half of the read;
//!   this file fixes the predicate half.)
//! * **Two-valued before any `NOT`.** SQL answers `NOT` on a missing value with
//!   *missing*, not *yes*, so a bare `NOT (lower(title) = 'rock')` **drops** the
//!   untagged Tracks — the opposite of what "is not" promises a listener. Every
//!   leaf is therefore made two-valued first, and the way it is made two-valued
//!   depends on the type: text folds absence into the empty string **at the
//!   column** (`lower(coalesce(title, ''))`), so absent text fails `is` and
//!   `contains` on the positive side and is swept up by `is not` for free;
//!   numbers and dates have no empty value to fold into, so the **test** is
//!   coalesced instead (`coalesce(duration_nanos > ?, 0)`), which is false when
//!   the column is absent. The same fold is then repeated **at the Box** — see
//!   [`Compiler::group`] for why that second collapse is written into the SQL
//!   instead of left to follow from the leaves.
//! * **None of the listener's words are SQL.** Every value, and every `contains`
//!   needle, is a bound parameter; the only text interpolated into a predicate is
//!   this module's own column names and operators. Case-insensitive matching goes
//!   through `lower(column)` against a Rust-lowercased needle (ADR 0020) and
//!   never through `LIKE`, because `configure_connection` sets
//!   `case_sensitive_like = ON` on every connection — so `LIKE` is
//!   case-*sensitive* here, and reaching for it for its wildcard would smuggle a
//!   second, different match rule into a field the catalog describes once.
//!
//! [`RuleTree`]: riff_persistence::store::RuleTree

use riff_persistence::errors::StoreError;
use riff_persistence::field_catalog;
use riff_persistence::rule::{
    CivilDateTime, Combine, Comparison, MatchableField, RuleValue, Semantic,
};
use riff_persistence::store::RuleTree;
use rusqlite::ToSql;
use rusqlite::types::ToSqlOutput;
use std::time::SystemTime;

use super::sqlite::nanos_from_system_time;

/// How many whole seconds the `in_last` window counts in a day. A day is
/// 86,400 seconds, not a calendar day: a window of 7 days is a span of time
/// measured back from the read, so it does not move with the listener's
/// midnight.
const SECONDS_PER_DAY: u64 = 86_400;

/// Nanoseconds in the store's duration unit per rule value unit: a Rule holds
/// whole milliseconds and the column holds whole nanoseconds, and 1 ms = 1e6 ns
/// is exact for every value either can carry.
const NANOS_PER_MILLI: u64 = 1_000_000;

/// The reason every refusal carries, so a listing that cannot be computed is
/// distinguishable from one that matched nothing. `StoreError` has one variant
/// for every fault the store can raise, so this string — and nothing else — is
/// what a caller and a listener can tell two refusals apart by.
const REFUSAL: &str = "cannot match this Custom Smart Playlist";

fn refusal(reason: impl std::fmt::Display) -> StoreError {
    StoreError::InvalidOperation(format!("{REFUSAL}: {reason}"))
}

/// One Matchable Field's storage facts: the bare `tracks` column its Rules
/// test, and which [`Fact`] shape the column has to be read as.
pub(crate) struct FieldColumn {
    /// The stored id this row is keyed on — `MatchableField::stored_id`, the
    /// value of `smart_rules.field_id`. Keyed by the id **string** rather than
    /// by ticket 01's closed enum so that a row this build cannot read stays
    /// visible to the lookup as the hole it is, and so the Settings page's
    /// value pickers can key their distinct reads on this same table.
    pub(crate) field_id: &'static str,
    /// The bare `tracks` column name: what a picker would `SELECT DISTINCT`,
    /// and what a Rule predicate tests — except for [`Fact::LosslessSuffix`],
    /// which is derived from the path rather than tagged.
    pub(crate) column: &'static str,
    /// How the column stores the fact, and so how a test on it is made
    /// two-valued.
    pub(crate) fact: Fact,
}

/// The storage shapes a Rule leaf can meet: the only two things the compiler
/// branches on, besides the comparison's [`Semantic`].
pub(crate) enum Fact {
    /// Nullable free text. Absence is the empty string, folded at the column.
    Text,
    /// The `;`-separated genre set: text for every purpose except `is`, which
    /// asks for one whole segment.
    GenreSegments,
    /// A nullable `INTEGER`. Absence is `IS NULL`; the test is coalesced.
    Integer,
    /// A nullable `INTEGER` of **nanoseconds**, tested against a rule value in
    /// whole milliseconds.
    DurationNanos,
    /// A nullable `INTEGER` of signed UTC epoch **nanoseconds**, written by the
    /// same clock that stamps play history.
    HistoryNanos,
    /// A `NOT NULL` 0/1 flag: already two-valued, with no absence to fold —
    /// which is why the catalog never offers `is missing` for one.
    Flag,
    /// Not a tagged column at all: a case-insensitive suffix match on the
    /// stored path against [`field_catalog::LOSSLESS_EXTENSIONS`].
    LosslessSuffix,
}

/// The one field id → column table, and the only place a field id names
/// storage. Static, canonical, and covering every [`MatchableField`] this build
/// reads: an id absent from it is refused.
///
/// The unit of each numeric column is spelled on its row, because a Rule value
/// and a column are not in the same unit for two of the sixteen fields, and a
/// picker that reads the same column has to convert the same way.
pub(crate) const FIELD_COLUMNS: &[(&str, FieldColumn)] = &[
    (
        "genre",
        FieldColumn {
            field_id: "genre",
            column: "genre",
            fact: Fact::GenreSegments,
        },
    ),
    (
        "artist",
        FieldColumn {
            field_id: "artist",
            column: "artist",
            fact: Fact::Text,
        },
    ),
    (
        "album_artist",
        FieldColumn {
            field_id: "album_artist",
            column: "album_artist",
            fact: Fact::Text,
        },
    ),
    (
        "album",
        FieldColumn {
            field_id: "album",
            column: "album",
            fact: Fact::Text,
        },
    ),
    (
        "title",
        FieldColumn {
            field_id: "title",
            column: "title",
            fact: Fact::Text,
        },
    ),
    (
        "composer",
        FieldColumn {
            field_id: "composer",
            column: "composer",
            fact: Fact::Text,
        },
    ),
    (
        "comment",
        FieldColumn {
            field_id: "comment",
            column: "comment",
            fact: Fact::Text,
        },
    ),
    (
        "year",
        FieldColumn {
            field_id: "year",
            column: "year",
            fact: Fact::Integer,
        },
    ),
    (
        // `duration_nanos` is nanoseconds; a duration Rule holds milliseconds.
        "duration",
        FieldColumn {
            field_id: "duration",
            column: "duration_nanos",
            fact: Fact::DurationNanos,
        },
    ),
    (
        "play_count",
        FieldColumn {
            field_id: "play_count",
            column: "play_count",
            fact: Fact::Integer,
        },
    ),
    (
        // `last_played_nanos` is signed UTC epoch nanoseconds.
        "last_played",
        FieldColumn {
            field_id: "last_played",
            column: "last_played_nanos",
            fact: Fact::HistoryNanos,
        },
    ),
    (
        // `date_added_nanos` is signed UTC epoch nanoseconds.
        "date_added",
        FieldColumn {
            field_id: "date_added",
            column: "date_added_nanos",
            fact: Fact::HistoryNanos,
        },
    ),
    (
        "favorite",
        FieldColumn {
            field_id: "favorite",
            column: "favorite",
            fact: Fact::Flag,
        },
    ),
    (
        "sample_rate",
        FieldColumn {
            field_id: "sample_rate",
            column: "sample_rate",
            fact: Fact::Integer,
        },
    ),
    (
        "channels",
        FieldColumn {
            field_id: "channels",
            column: "channels",
            fact: Fact::Integer,
        },
    ),
    (
        // Derived from the path, in any case: `.FLAC` is lossless.
        "lossless",
        FieldColumn {
            field_id: "lossless",
            column: "path",
            fact: Fact::LosslessSuffix,
        },
    ),
];

/// The column a stored field id names, or `None` when this build has no column
/// for it — the refusal that happens before any SQL exists.
fn column_for(field_id: &str) -> Option<&'static FieldColumn> {
    FIELD_COLUMNS
        .iter()
        .map(|(_, column)| column)
        .find(|column| column.field_id == field_id)
}

/// SQLite's compiled-in ceiling on one statement's bound parameters
/// (`SQLITE_LIMIT_VARIABLE_NUMBER`, the default since SQLite 3.32 — the bundled
/// engine this store uses is well past that). Rules are uncapped, so a wide
/// enough Box really does reach it.
///
/// The engine's own expression-depth ceiling is deliberately **not** mirrored
/// here: it is a property of the parser rather than a stable published number,
/// so a tree deep enough to pass it fails when the statement is prepared and
/// arrives as a store error carrying the engine's reason. Neither ceiling is
/// dodged by chunking or splitting a predicate: half a predicate answers a
/// different question from the one the listener wrote, and answering it
/// wrongly is worse than refusing.
const BIND_PARAMETER_CEILING: usize = 32_766;

/// One bound value: a needle for the text tests, a number for every other. The
/// two stay apart because SQLite compares an `INTEGER` column against a TEXT
/// parameter **as text** — bind a year as `"500"` and it sorts after `"1975"`.
#[derive(Clone)]
pub(crate) enum Bound {
    /// A `TEXT` parameter.
    Text(String),
    /// An `INTEGER` parameter.
    Integer(i64),
}

impl ToSql for Bound {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        match self {
            Bound::Text(text) => text.as_str().to_sql(),
            Bound::Integer(number) => number.to_sql(),
        }
    }
}

/// The compiled predicate: one SQL boolean expression over `tracks` plus the
/// values it binds, in placeholder order.
pub(crate) struct Predicate {
    /// A `WHERE`-ready boolean expression, parenthesised as a whole so a caller
    /// can append `AND` to it without changing what the tree meant.
    pub(crate) sql: String,
    /// The values behind `?1 .. ?n`, in placeholder order.
    pub(crate) params: Vec<Bound>,
}

/// Compile a whole tree — combined with the listing's current search `needle`
/// by `AND`,
/// `None` for no needle — into one predicate over `tracks`.
pub(crate) fn compile(tree: &RuleTree, needle: Option<&str>) -> Result<Predicate, StoreError> {
    let mut compiler = Compiler { params: Vec::new() };
    let mut sql = compiler.group(tree)?;

    if let Some(needle) = needle {
        // The needle is the Library's own literal-substring rule over the
        // derived Rust-lowercased `search_text` column (ADR 0003's search
        // parity), ANDed *inside* the read so a page cuts its window out of the
        // filtered answer rather than out of the whole list. `search_text` is
        // `NOT NULL`, so nothing here needs folding.
        let placeholder = compiler.bind(Bound::Text(needle.to_lowercase()));
        sql = format!("({sql} AND instr(search_text, {placeholder}) > 0)");
    }

    let predicate = Predicate {
        sql,
        params: compiler.params,
    };
    // `+ 2` for the window's `LIMIT` and `OFFSET`, which the same statement
    // binds after these. Checked before anything is prepared, so an over-wide
    // tree is a refusal that names the tree's own size rather than an engine
    // error arriving half-way through a read.
    if predicate.params.len() + 2 > BIND_PARAMETER_CEILING {
        return Err(refusal(format!(
            "this Rule tree binds {} values, more than the {} the store can bind at once",
            predicate.params.len(),
            BIND_PARAMETER_CEILING
        )));
    }
    Ok(predicate)
}

/// The walk. Holds only the flat parameter list, so a placeholder's number is
/// one past its position in it.
struct Compiler {
    params: Vec<Bound>,
}

impl Compiler {
    /// Bind one value and return the placeholder that stands for it.
    fn bind(&mut self, value: Bound) -> String {
        self.params.push(value);
        format!("?{}", self.params.len())
    }

    /// One Box — or one bare leaf, which is what a tree with no Box in it is —
    /// compiled to a two-valued boolean expression.
    ///
    /// The Box-level collapse is written into the SQL on purpose. Every leaf is
    /// already two-valued, so `AND` / `OR` over a Box's arms cannot produce
    /// unknown *today*; but a Box is also the one node that can receive an arm
    /// from a leaf shape this file does not know yet, and an unknown arm of an
    /// `all` Box makes the whole Box unknown, which `WHERE` reads as false — the
    /// Track vanishes. Folding at both nodes is what keeps that failure from
    /// depending on how carefully the leaves were written.
    ///
    /// Negation lives on a Rule alone (`is not missing` is `is missing`
    /// negated, which is why it never became a tenth operator), so a Box is
    /// never wrapped in `NOT` here — and the `NOT` a Rule carries wraps the
    /// whole folded test, never the operator inside it.
    fn group(&mut self, tree: &RuleTree) -> Result<String, StoreError> {
        match tree {
            RuleTree::Rule {
                field_id,
                operator_id,
                negated,
                value,
            } => {
                let test = self.leaf(field_id, operator_id, value.as_ref())?;
                Ok(if *negated {
                    format!("NOT ({test})")
                } else {
                    format!("({test})")
                })
            }
            RuleTree::Box { combine, children } => {
                let joiner = match Combine::from_stored_id(combine) {
                    Some(Combine::All) => " AND ",
                    Some(Combine::Any) => " OR ",
                    // A combine word this build cannot read is the Box's own
                    // share of an Unreadable Rule, and refusing is the only
                    // honest answer: `all` and `any` disagree about everything
                    // else in the tree.
                    None => {
                        return Err(refusal(format!(
                            "a Rule Box combines with \"{combine}\", which this riff cannot read"
                        )));
                    }
                };
                if children.is_empty() {
                    // Never stored, and never guessed: an empty `all` Box is
                    // vacuously true (every Track matches it) and an empty `any`
                    // Box is false, so answering either one silently is a
                    // confident, opposite answer about the listener's whole
                    // Library. The save path refuses an empty Box; a read that
                    // meets one refuses too.
                    return Err(refusal(
                        "a Rule Box holds no Rule, so there is nothing to match",
                    ));
                }
                let mut arms = Vec::with_capacity(children.len());
                for child in children {
                    arms.push(self.group(child)?);
                }
                Ok(format!("coalesce(({}), 0)", arms.join(joiner)))
            }
        }
    }

    /// One leaf's **positive**, two-valued test. The caller wraps the negation,
    /// which is the only way `is not` can stay the inversion of the whole test
    /// rather than a different operator.
    fn leaf(
        &mut self,
        field_id: &str,
        operator_id: &str,
        value: Option<&RuleValue>,
    ) -> Result<String, StoreError> {
        // Every question below is ticket 01's vocabulary and the catalog
        // answering it — this file never restates what a field permits.
        let Some(field) = MatchableField::from_stored_id(field_id) else {
            return Err(refusal(format!(
                "a saved Rule tests \"{field_id}\", a field this riff cannot read"
            )));
        };
        let Some(column) = column_for(field_id) else {
            return Err(refusal(format!(
                "a saved Rule tests \"{field_id}\", which this build has no column for"
            )));
        };
        let Some(comparison) = Comparison::from_stored_id(operator_id) else {
            return Err(refusal(format!(
                "a saved Rule tests \"{field_id}\" with \"{operator_id}\", which this riff cannot read"
            )));
        };
        let row = field_catalog::spec(field);
        if !row.permits(comparison) {
            // Reachable through SQL alone: `smart_rules` CHECKs the field list
            // and the operator list *separately*, so a `genre greater_than` row
            // satisfies the schema and cannot satisfy the catalog. Per-field
            // legality is Rust's question, answered here rather than in DDL.
            return Err(refusal(format!(
                "the Rule tests \"{field_id}\" with \"{operator_id}\", which that field does not permit"
            )));
        }

        // `is missing` is the one test that takes no value. A value sitting on
        // a valueless row is the save path's mistake, not a reason to stop
        // matching: absence is decided by the column alone.
        if row.value_widgets(comparison).is_empty() {
            return Self::absence(column, field_id, operator_id);
        }
        let Some(value) = value else {
            return Err(refusal(format!(
                "the Rule tests \"{field_id}\" with \"{operator_id}\" and holds no value"
            )));
        };
        if value.value_type() != row.value_type() {
            return Err(refusal(format!(
                "the Rule testing \"{field_id}\" holds a value of the wrong kind"
            )));
        }

        self.test(column, comparison.semantic(), value, field_id, operator_id)
    }

    /// How a field's absence is tested, once its [`Fact`] shape is known.
    ///
    /// Text reuses the one-test shape — the empty string *is* the absence, so
    /// `is missing` and `is ""` are the same SQL and a negated `is` sweeps the
    /// untagged Tracks up. Numbers and dates have no empty value to fold into,
    /// so they ask whether the column is there at all.
    fn absence(
        column: &FieldColumn,
        field_id: &str,
        operator_id: &str,
    ) -> Result<String, StoreError> {
        Ok(match column.fact {
            Fact::Text | Fact::GenreSegments => {
                format!("{} = ''", fold_text(column.column))
            }
            Fact::Integer | Fact::DurationNanos | Fact::HistoryNanos => {
                format!("{} IS NULL", column.column)
            }
            // The catalog never offers `is missing` for a flag or for a path
            // suffix, and neither has an absent state to test: a `NOT NULL
            // DEFAULT 0` column always has an answer, and a path always has an
            // extension-or-not. Reaching here means a row that contradicts the
            // catalog, which is a refusal, not a constant to invent.
            Fact::Flag | Fact::LosslessSuffix => {
                return Err(refusal(format!(
                    "the Rule tests \"{field_id}\" with \"{operator_id}\", but that field has no absent value"
                )));
            }
        })
    }

    /// One leaf's positive test, given the column's shape and what the
    /// comparison means. One function per shape: the dispatch below is the
    /// whole vocabulary of what a leaf can be, and a new [`Fact`] row has to
    /// answer here rather than somewhere inside a hundred-line match.
    fn test(
        &mut self,
        column: &FieldColumn,
        semantic: Semantic,
        value: &RuleValue,
        field_id: &str,
        operator_id: &str,
    ) -> Result<String, StoreError> {
        match (&column.fact, semantic) {
            (Fact::Text | Fact::GenreSegments, Semantic::Equality | Semantic::Containment) => {
                self.text_test(column, semantic, value, field_id, operator_id)
            }
            (
                Fact::Integer | Fact::DurationNanos,
                Semantic::Equality | Semantic::GreaterThan | Semantic::LessThan,
            ) => self.numeric_test(column, semantic, value, field_id, operator_id),
            (
                Fact::HistoryNanos,
                Semantic::WithinDays | Semantic::EarlierThan | Semantic::LaterThan,
            ) => self.history_test(column, semantic, value, field_id, operator_id),
            (Fact::Flag | Fact::LosslessSuffix, Semantic::Equality) => {
                self.yes_no_test(column, value, field_id, operator_id)
            }
            _ => Err(refusal(format!(
                "the Rule tests \"{field_id}\" with \"{operator_id}\" in a way this build cannot compile"
            ))),
        }
    }

    /// Text, folded at the column: absent is the empty string, and the fold is
    /// `lower()` — ASCII only, ADR 0020 — against a needle Rust-lowercased in
    /// full, which is the asymmetry the non-Latin case pins.
    fn text_test(
        &mut self,
        column: &FieldColumn,
        semantic: Semantic,
        value: &RuleValue,
        field_id: &str,
        operator_id: &str,
    ) -> Result<String, StoreError> {
        let RuleValue::Text(needle) = value else {
            return Err(refusal(format!(
                "the Rule testing \"{field_id}\" with \"{operator_id}\" needs a needle"
            )));
        };
        let needle = needle.to_lowercase();
        let folded = fold_text(column.column);
        Ok(match (semantic, &column.fact) {
            (Semantic::Containment, _) => {
                // Containment is a literal substring of the folded text —
                // including for genre, where a needle may land inside one
                // segment or across two.
                let placeholder = self.bind(Bound::Text(needle));
                format!("instr({folded}, {placeholder}) > 0")
            }
            (_, Fact::GenreSegments) => {
                // Genre is a *set*, so `is` asks for one whole segment: the
                // lowered, space-stripped column wrapped in the delimiter on
                // both sides, searched for the wrapped needle. Exact for
                // `"Art Rock;Progressive Rock"`, and quietly wrong for a segment
                // padded with anything that is not a space: a tab survives the
                // strip, so a row stored as `"Art Rock;\tProgressive Rock"`
                // holds no `";progressiverock;"` and does not match. The genres
                // view trims each segment and *does* show that tag — the
                // documented disagreement between a view and a Rule, pinned by a
                // test rather than hidden.
                let segment = needle.replace(' ', "");
                let placeholder = self.bind(Bound::Text(format!(";{segment};")));
                let segments = format!("';' || replace({folded}, ' ', '') || ';'");
                format!("instr({segments}, {placeholder}) > 0")
            }
            _ => {
                let placeholder = self.bind(Bound::Text(needle));
                format!("{folded} = {placeholder}")
            }
        })
    }

    /// A whole number, or a duration: the same ordering test, and the same fold
    /// of the *test* rather than the column, because a number has no empty value
    /// to fold into.
    fn numeric_test(
        &mut self,
        column: &FieldColumn,
        semantic: Semantic,
        value: &RuleValue,
        field_id: &str,
        operator_id: &str,
    ) -> Result<String, StoreError> {
        let number = match (&column.fact, value) {
            (Fact::Integer, RuleValue::Number(number)) => *number,
            (Fact::DurationNanos, RuleValue::DurationMillis(millis)) => {
                // Exact, not rounded: 1 ms = 1e6 ns, so a threshold written in
                // whole milliseconds lands on a whole multiple of the column's
                // unit and `>` / `<` cannot straddle a Track by a fraction.
                // Saturating, because a hand-edited value cannot fail a read.
                millis
                    .checked_mul(NANOS_PER_MILLI)
                    .and_then(|nanos| i64::try_from(nanos).ok())
                    .unwrap_or(i64::MAX)
            }
            _ => {
                return Err(refusal(format!(
                    "the Rule testing \"{field_id}\" with \"{operator_id}\" needs a number or a duration"
                )));
            }
        };
        Ok(fold_test(ordering_of(
            column,
            semantic,
            self.bind(Bound::Integer(number)),
        )))
    }

    /// A history date: a moving window counted back from the read, or a
    /// zone-less boundary read against this machine's clock.
    fn history_test(
        &mut self,
        column: &FieldColumn,
        semantic: Semantic,
        value: &RuleValue,
        field_id: &str,
        operator_id: &str,
    ) -> Result<String, StoreError> {
        if semantic == Semantic::WithinDays {
            let RuleValue::Days(days) = value else {
                return Err(refusal(format!(
                    "the Rule testing \"{field_id}\" with \"{operator_id}\" needs a day count"
                )));
            };
            // The cutoff is computed **at read time**, from the same epoch clock
            // the history facts were written with: a window counted back from
            // the day the Rule was saved would stop being a window the next
            // morning.
            let cutoff = self.bind(Bound::Integer(Self::days_cutoff(*days)));
            return Ok(fold_test(format!("{} >= {cutoff}", column.column)));
        }
        let RuleValue::DateTime(civil) = value else {
            return Err(refusal(format!(
                "the Rule testing \"{field_id}\" with \"{operator_id}\" needs a day and a time"
            )));
        };
        let operator = if semantic == Semantic::EarlierThan {
            "<"
        } else {
            ">"
        };
        // Zone-less on purpose, and read against the listener's own clock *on
        // the machine doing the reading*: a saved Rule says what the listener
        // typed, not what instant that maps to. Storing a zone would make one
        // saved Rule answer differently on another machine, which is a thing no
        // listener asks for.
        //
        // So the comparison happens in the local domain on both sides: the
        // epoch-nanosecond column is rendered as local civil text and compared
        // with the stored civil text. Fixed width, zero-padded ASCII digits —
        // lexical order *is* chronological order. The reverse (converting the
        // listener's typed day into epoch seconds to bind as a number) would
        // need this crate to own a timezone database, which it does not have and
        // must not gain for a Rule; `strftime(…, \'localtime\')` is the same
        // machine clock the history stamp was written with.
        //
        // Second granularity, both sides: `before` is strictly earlier than the
        // instant typed, and a stamp inside that same second is neither before
        // nor after it.
        let local = format!(
            "strftime('%Y-%m-%d %H:%M:%S', {} / 1000000000, 'unixepoch', 'localtime')",
            column.column
        );
        let placeholder = self.bind(Bound::Text(Self::civil_sql_text(civil)));
        Ok(fold_test(format!("{local} {operator} {placeholder}")))
    }

    /// A fact with two sides and no absent state: the favorite flag, and the
    /// lossless suffix read off the stored path.
    fn yes_no_test(
        &mut self,
        column: &FieldColumn,
        value: &RuleValue,
        field_id: &str,
        operator_id: &str,
    ) -> Result<String, StoreError> {
        let RuleValue::YesNo(yes) = value else {
            return Err(refusal(format!(
                "the Rule testing \"{field_id}\" with \"{operator_id}\" needs a yes or a no"
            )));
        };
        if let Fact::Flag = column.fact {
            let placeholder = self.bind(Bound::Integer(i64::from(*yes)));
            return Ok(format!("{} = {placeholder}", column.column));
        }
        // A case-insensitive **suffix** match on the stored path: no file is
        // opened and no codec is inspected, so an ALAC track inside a `.m4a`
        // container reports as lossy. Accepted, and pinned.
        //
        // `substr(x, -n) = ?` rather than `LIKE \'%.ext\'`: this connection runs
        // with `case_sensitive_like = ON`, and `LIKE` would also make the
        // extension\'s own letters pattern characters to reason about.
        // `lower()` folds ASCII only (ADR 0020) and an extension is ASCII, so
        // for `.FLAC` the fold is exact.
        let mut arms = Vec::with_capacity(field_catalog::LOSSLESS_EXTENSIONS.len());
        for extension in field_catalog::LOSSLESS_EXTENSIONS {
            let suffix = format!(".{}", extension.to_lowercase());
            let placeholder = self.bind(Bound::Text(suffix.clone()));
            arms.push(format!(
                "substr(lower({}), -{}) = {placeholder}",
                column.column,
                suffix.chars().count()
            ));
        }
        let lossless = arms.join(" OR ");
        Ok(if *yes {
            format!("({lossless})")
        } else {
            format!("NOT ({lossless})")
        })
    }

    /// A `before` / `after` boundary as the text a local civil stamp is compared
    /// against. Written from typed parts, so it always has the shape SQLite
    /// reads; parts out of range simply never order beside a real stamp, which
    /// is false — never unknown, never a crash.
    fn civil_sql_text(civil: &CivilDateTime) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            civil.year, civil.month, civil.day, civil.hour, civil.minute, civil.second
        )
    }

    /// The instant `days` whole days back from the read, in the store's epoch
    /// nanoseconds. Saturating: a day count large enough to run the clock behind
    /// the epoch has no meaningful cutoff, and `i64::MIN` then answers
    /// "everything that has a history stamp at all", which is what such a window
    /// means.
    fn days_cutoff(days: u32) -> i64 {
        let nanos_back = u64::from(days)
            .saturating_mul(SECONDS_PER_DAY)
            .saturating_mul(1_000_000_000);
        let now = nanos_from_system_time(SystemTime::now());
        now.saturating_sub(i64::try_from(nanos_back).unwrap_or(i64::MAX))
    }
}

/// The text a text Rule compares: lowered (ASCII only — ADR 0020) and with
/// absence folded into the empty string, which is the library's own standing
/// convention for a tag that is absent and a tag that is empty.
fn fold_text(column: &str) -> String {
    format!("lower(coalesce({column}, ''))")
}

/// The one two-valued fold for a test over a nullable numeric column: an absent
/// column makes the test unknown, and unknown has to mean *false* so that a
/// negated test can mean *yes*.
fn fold_test(test: String) -> String {
    format!("coalesce({test}, 0)")
}

/// `<col> <operator> <placeholder>` for equality and the two orderings. The
/// operator is SQL's own — numeric and date comparisons stay SQL-faithful on
/// the positive side, which is what makes an `equal_to` on a year answer the way
/// the column reads.
fn ordering_of(column: &FieldColumn, semantic: Semantic, placeholder: String) -> String {
    let operator = match semantic {
        Semantic::GreaterThan => ">",
        Semantic::LessThan => "<",
        _ => "=",
    };
    format!("{} {operator} {placeholder}", column.column)
}
