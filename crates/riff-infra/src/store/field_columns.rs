//! The one field-id → `tracks` column table the `Application Store` speaks.
//!
//! A Rule names the **field id** it tests (`"genre"`, `"sample_rate"`, …) and
//! never a column, so every store read that has to reach a tag fact — the
//! bounded value-picker lists resolved by [`picker_column`] today, and the
//! compiled Rule tree of the Custom Smart Playlist read tomorrow — resolves
//! that id here and nowhere else. Adding a testable field means adding a row
//! to [`FIELD_COLUMNS`]; no read site learns a column name.
//!
//! This table answers **only** "which column holds this field". Everything
//! else a picker read needs to know — that the field exists at all, that its
//! values are pickable rather than typed, that its value is text rather than
//! a number — is the [`field_catalog`]'s question, and is asked there rather
//! than restated here. The catalog cannot know SQL's column names and the
//! store must not know the catalog's answers; the two meet in [`picker_column`].
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
}

/// The fields the `Application Store` can answer a column-level read for.
///
/// Every row points at the column a Rule actually tests, not at the `albums`
/// / `artists` entity tables the browse listings read. That distinction is
/// load-bearing: the entity tables are keyed on the *display* value
/// (`SqliteStore`'s scan writes `display_album_artist()`, whose fallback is
/// `Unknown Artist`), while `tracks.artist` / `tracks.album_artist` hold the
/// tag as it was read or `NULL` when it was absent. A picker over the entity
/// table would offer `Unknown Artist`, a value neither column can contain, so
/// the Rule built from it would match nothing.
///
/// The six rows here are today's whole table, and it is built to grow: the
/// compiled Rule tree has to reach every testable column — `year`,
/// `duration_nanos`, `composer`, `comment`, `path`, `title` and the rest — and
/// each of those is appended here rather than spelled at its new read site.
/// Growing it does **not** widen the picker lists, because a picker read goes
/// through [`picker_column`], which asks the field catalog whether the field
/// is pickable at all. Without that gate, the day `title` was appended to
/// make a Title comparison work would quietly have turned the Title picker
/// into a scan of every track name in the Library — a field the catalog gives
/// a text box, not a list.
pub const FIELD_COLUMNS: &[FieldColumn] = &[
    FieldColumn {
        field: MatchableField::Genre,
        column: "genre",
    },
    FieldColumn {
        field: MatchableField::Artist,
        column: "artist",
    },
    FieldColumn {
        field: MatchableField::AlbumArtist,
        column: "album_artist",
    },
    FieldColumn {
        field: MatchableField::Album,
        column: "album",
    },
    FieldColumn {
        field: MatchableField::SampleRate,
        column: "sample_rate",
    },
    FieldColumn {
        field: MatchableField::Channels,
        column: "channels",
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
/// * a field the catalog knows and this table does not — a compiler field
///   with no picker, whose picker would have nothing to read.
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
#[must_use]
pub fn is_genre_set(field_id: &str) -> bool {
    field_id == MatchableField::Genre.stored_id()
}
