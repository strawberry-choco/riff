//! The one field-id → `tracks` column table the `Application Store` speaks.
//!
//! A Rule names the **field id** it tests (`"genre"`, `"sample_rate"`, …) and
//! never a column, so every store read that has to reach a tag fact resolves
//! that id here and nowhere else: the compiled Rule tree through
//! [`field_column`], the bounded value-picker lists through [`picker_column`].
//! Adding a testable field means adding a row to [`FIELD_COLUMNS`]; no read site
//! learns a column name, and no second table exists to drift from the first.
//!
//! This table answers **only** "which column holds this field, and in what
//! shape" — [`FieldColumn::fact`] is storage, not meaning. Everything else a
//! read needs to know — that the field exists at all, that its values are
//! pickable rather than typed, that its value is text rather than a number,
//! which comparisons it permits — is the [`field_catalog`]'s question, and is
//! asked there rather than restated here. The catalog cannot know SQL's column
//! names and the store must not know the catalog's answers; the two meet in
//! [`picker_column`] and in the compiler's own catalog calls.
//!
//! The table is also the store's injection boundary. Every `column` is a
//! `&'static str` written in this file, so the SQL an adapter builds from it
//! interpolates **no input at all** — the field id arriving from a Rule row or
//! a picker request is only ever a lookup key, and an id the catalog does not
//! know, or knows but does not resolve to a column here, is rejected before a
//! single SQL string is produced. Every actual value crossing into a query is
//! a bound parameter.

use riff_persistence::field_catalog::{self, ValueType};
use riff_persistence::rule::MatchableField;

/// One [`MatchableField`] and the `tracks` column that holds its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldColumn {
    /// The field this row resolves.
    pub field: MatchableField,
    /// The `tracks` column a Rule testing this field reads. Always a literal
    /// from this file — never derived from input.
    pub column: &'static str,
    /// How the column stores the fact, and so how a test on it must be made
    /// two-valued before any negation is applied.
    pub fact: Fact,
}

/// The storage shapes a Rule leaf can meet: the only things a read has to
/// branch on besides a comparison's meaning, because absence folds differently
/// for each of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fact {
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

/// The fields the `Application Store` can answer a column-level read for.
///
/// All sixteen [`MatchableField`]s are here, because a compiled Rule tree has
/// to reach every testable column and an id this table does not carry is
/// refused before any SQL exists. Every row points at the column a Rule
/// actually tests, not at the `albums` / `artists` entity tables the browse
/// listings read. That distinction is load-bearing: the entity tables are keyed
/// on the *display* value (`SqliteStore`'s scan writes `display_album_artist()`,
/// whose fallback is `Unknown Artist`), while `tracks.artist` /
/// `tracks.album_artist` hold the tag as it was read or `NULL` when it was
/// absent. A picker over the entity table would offer `Unknown Artist`, a value
/// neither column can contain, so the Rule built from it would match nothing.
///
/// The unit of each numeric column is spelled on its row and in its [`Fact`],
/// because a Rule value and a column are not in the same unit for three of the
/// sixteen fields, and a picker that reads the same column has to convert the
/// same way.
///
/// Growing this table does **not** widen the picker lists, because a picker
/// read goes through [`picker_column`], which asks the field catalog whether
/// the field is pickable at all. Without that gate, the day `title` was appended
/// to make a Title comparison work would quietly have turned the Title picker
/// into a scan of every track name in the Library — a field the catalog gives
/// a text box, not a list.
pub const FIELD_COLUMNS: &[FieldColumn] = &[
    FieldColumn {
        field: MatchableField::Genre,
        column: "genre",
        fact: Fact::GenreSegments,
    },
    FieldColumn {
        field: MatchableField::Artist,
        column: "artist",
        fact: Fact::Text,
    },
    FieldColumn {
        field: MatchableField::AlbumArtist,
        column: "album_artist",
        fact: Fact::Text,
    },
    FieldColumn {
        field: MatchableField::Album,
        column: "album",
        fact: Fact::Text,
    },
    FieldColumn {
        field: MatchableField::Title,
        column: "title",
        fact: Fact::Text,
    },
    FieldColumn {
        field: MatchableField::Composer,
        column: "composer",
        fact: Fact::Text,
    },
    FieldColumn {
        field: MatchableField::Comment,
        column: "comment",
        fact: Fact::Text,
    },
    FieldColumn {
        field: MatchableField::Year,
        column: "year",
        fact: Fact::Integer,
    },
    FieldColumn {
        // `duration_nanos` is nanoseconds; a duration Rule holds milliseconds.
        field: MatchableField::Duration,
        column: "duration_nanos",
        fact: Fact::DurationNanos,
    },
    FieldColumn {
        field: MatchableField::PlayCount,
        column: "play_count",
        fact: Fact::Integer,
    },
    FieldColumn {
        // `last_played_nanos` is signed UTC epoch nanoseconds.
        field: MatchableField::LastPlayed,
        column: "last_played_nanos",
        fact: Fact::HistoryNanos,
    },
    FieldColumn {
        // `date_added_nanos` is signed UTC epoch nanoseconds.
        field: MatchableField::DateAdded,
        column: "date_added_nanos",
        fact: Fact::HistoryNanos,
    },
    FieldColumn {
        field: MatchableField::Favorite,
        column: "favorite",
        fact: Fact::Flag,
    },
    FieldColumn {
        field: MatchableField::SampleRate,
        column: "sample_rate",
        fact: Fact::Integer,
    },
    FieldColumn {
        field: MatchableField::Channels,
        column: "channels",
        fact: Fact::Integer,
    },
    FieldColumn {
        // Derived from the path, in any case: `.FLAC` is lossless.
        field: MatchableField::Lossless,
        column: "path",
        fact: Fact::LosslessSuffix,
    },
];

/// The [`FieldColumn`] for a field the catalog knows, or `None` when this
/// store has no column for it yet.
///
/// A query builder resolving a stored Rule's field uses this; a picker read
/// must go through [`picker_column`] instead, so a field the catalog gives a
/// typed control for stays unreachable however many columns this table learns.
#[must_use]
pub fn field_column(field: MatchableField) -> Option<&'static FieldColumn> {
    FIELD_COLUMNS.iter().find(|entry| entry.field == field)
}

/// The [`FieldColumn`] a bounded picker read may use for `field_id`, but only
/// when the catalog's own answers permit the read.
///
/// Four rejections happen here, and all of them before any SQL is built:
///
/// * an id no [`MatchableField`] is — an Unreadable Rule's field, or a caller
///   that spelled a display name instead of a stored id;
/// * a field the catalog does not mark pickable, which is the case that keeps
///   this table's necessary growth from widening what a listener can pick;
/// * a field whose [`ValueType`] is not `expected`, because asking the
///   `sample_rate` column for a text list is a caller error even though the
///   field is real and pickable;
/// * a field the catalog knows and this table does not — impossible while
///   [`FIELD_COLUMNS`] covers all sixteen fields, and the guard is what keeps a
///   future field appended to the enum without a row here a refusal rather
///   than a read of a column nobody chose.
///
/// `field_id` is used only as a lookup key into the catalog and this table,
/// never as SQL text.
#[must_use]
pub fn picker_column(field_id: &str, expected: ValueType) -> Option<&'static FieldColumn> {
    let field = MatchableField::from_stored_id(field_id)?;
    let spec = field_catalog::spec(field);
    if !spec.is_pickable() || spec.value_type() != expected {
        return None;
    }
    field_column(field)
}

/// Whether `field_id` names the genre column: the one catalogued text field
/// whose stored value is a `;`-separated set, so its distinct values are the
/// tag's entries rather than its whole strings.
///
/// Asked of [`Fact::GenreSegments`] rather than restated as a second id
/// comparison, so the row is the only thing that says which column is a set.
#[must_use]
pub fn is_genre_set(field_id: &str) -> bool {
    MatchableField::from_stored_id(field_id)
        .and_then(field_column)
        .is_some_and(|entry| entry.fact == Fact::GenreSegments)
}
