//! The one field-id → `tracks` column table the `Application Store` speaks.
//!
//! A Rule names the **field id** it tests (`"genre"`, `"sample_rate"`, …) and
//! never a column, so every store read that has to reach a tag fact — the
//! bounded value-picker lists of
//! [`LibraryQueryStore::picker_texts`] /
//! [`LibraryQueryStore::picker_numbers`] today, and the compiled Rule tree of
//! the Custom Smart Playlist read tomorrow — resolves that id here and
//! nowhere else. Adding a testable field means adding a row to
//! [`FIELD_COLUMNS`]; no read site learns a column name.
//!
//! The table is also the store's injection boundary. Every `column` is a
//! `&'static str` written in this file, so the SQL an adapter builds from it
//! interpolates **no input at all** — the field id arriving from a Rule row or
//! a picker request is only ever a lookup key, and an id that is not in this
//! table is rejected here, before any SQL string is produced. Every actual
//! value crossing into a query is a bound parameter.
//!
//! [`LibraryQueryStore::picker_texts`]: riff_persistence::store::LibraryQueryStore::picker_texts
//! [`LibraryQueryStore::picker_numbers`]: riff_persistence::store::LibraryQueryStore::picker_numbers

/// How a catalogued `tracks` column's values are typed at the SQL boundary.
///
/// This is the column's SQL affinity — which absent test and which ordering
/// an adapter must use — not a UI statement: what a field's value control
/// draws is the field catalog's question in `riff-persistence`, and nothing
/// here restates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// A `TEXT` column: absent is `NULL` **or** an all-whitespace value, and
    /// the deterministic order is byte-wise ascending (`SQLite` `BINARY`).
    Text,
    /// An `INTEGER` column: absent is `NULL` alone — a stored `0` is a
    /// present value — and the deterministic order is ascending numerically.
    Number,
}

/// One catalogued field id and the `tracks` column that holds its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldColumn {
    /// The stable field id as a Rule stores it.
    pub field_id: &'static str,
    /// The `tracks` column a Rule testing this field reads. Always a
    /// literal from this file — never derived from input.
    pub column: &'static str,
    /// The column's SQL type family, which decides its absent test and order.
    pub kind: ColumnKind,
}

/// The fields the `Application Store` can answer a value read for.
///
/// The six entries here are exactly the pickable fields of the feature's
/// catalog: the four text fields whose picker lists come from the Library and
/// the two numeric ones. `title`, `composer` and `comment` are text facts a
/// Rule can test but offer **no** picker list for, so they are deliberately
/// absent — a picker value must come from a column the feature has decided
/// the listener can choose from, and widening that set is a catalog change
/// made elsewhere first.
///
/// Every row points at the column a Rule actually tests, not at the `albums`
/// / `artists` entity tables the browse listings read. That distinction is
/// load-bearing: the entity tables are keyed on the *display* value
/// (`SqliteStore`'s scan writes `display_album_artist()`, whose fallback is
/// `Unknown Artist`), while `tracks.artist` / `tracks.album_artist` hold the
/// tag as it was read or `NULL` when it was absent. A picker over the entity
/// table would offer `Unknown Artist`, a value neither column can contain, so
/// the Rule built from it would match nothing.
pub const FIELD_COLUMNS: &[FieldColumn] = &[
    FieldColumn {
        field_id: "genre",
        column: "genre",
        kind: ColumnKind::Text,
    },
    FieldColumn {
        field_id: "artist",
        column: "artist",
        kind: ColumnKind::Text,
    },
    FieldColumn {
        field_id: "album_artist",
        column: "album_artist",
        kind: ColumnKind::Text,
    },
    FieldColumn {
        field_id: "album",
        column: "album",
        kind: ColumnKind::Text,
    },
    FieldColumn {
        field_id: "sample_rate",
        column: "sample_rate",
        kind: ColumnKind::Number,
    },
    FieldColumn {
        field_id: "channels",
        column: "channels",
        kind: ColumnKind::Number,
    },
];

/// The [`FieldColumn`] catalogued under `field_id`, or `None` when the id
/// names no column.
pub fn field_column(field_id: &str) -> Option<&'static FieldColumn> {
    FIELD_COLUMNS.iter().find(|entry| entry.field_id == field_id)
}

/// The [`FieldColumn`] for `field_id`, but only when its kind is `expected`.
///
/// A picker read asks for text values or numeric values, so asking the
/// `"sample_rate"` column for a text list is a caller error even though the
/// id is catalogued. Resolving through this one function keeps both
/// rejections — an unknown id and a shape mismatch — in front of any SQL.
pub fn picker_column(field_id: &str, expected: ColumnKind) -> Option<&'static FieldColumn> {
    field_column(field_id).filter(|entry| entry.kind == expected)
}

/// Whether `field_id` is the `genre` column: the one catalogued text column
/// whose stored value is a `;`-separated set, so its distinct values are its
/// segments rather than its whole strings.
pub fn is_genre_set(field_id: &str) -> bool {
    field_id == "genre"
}
