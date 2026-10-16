//! The Matchable Field catalog — the one source of what each field answers.
//!
//! Six questions, answered once per field: its stable id ( [`MatchableField`]
//! itself), the display name riff shows, its value type, the comparisons it
//! permits, whether a value can be absent at all, and which value widget the
//! editor draws. Nothing else in the product may restate any of them — not the
//! store's `CHECK` lists (which are only the *union* of legal ids), not the
//! query compiler, not the editor's row, and not a test.
//!
//! The widget is named as an *idea* ([`ValueWidget`]) and never as a control
//! type, so this crate stays free of any UI dependency: its manifest carries no
//! `[dependencies]` at all.
//!
//! `riff-backend` re-exports this module at `riff_backend::domain`.

use crate::rule::Comparison;
use crate::rule::MatchableField;
use std::path::Path;

/// What kind of value a field holds, as the catalog sees it. The value type is
/// why a field's comparisons look the way they do — text takes a needle,
/// arithmetic takes an order, a history date takes a window or a boundary —
/// and it is what the stored `value` column is read back as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// A tag or title. Absent text and empty text are the same absence, the
    /// library's existing convention.
    Text,
    /// A whole number the listener can compare.
    Number,
    /// A length of time, entered as hours, minutes and seconds and stored as
    /// whole milliseconds.
    Duration,
    /// A moment in the listener's own history, asked as a moving window or a
    /// zone-less boundary.
    Date,
    /// A yes-or-no fact with no middle state.
    YesNo,
}

/// How `is missing` is decided for a field. Two answers because two shapes:
/// text has an empty value to fold absence into, and everything else has to
/// ask whether the column is there at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Absence {
    /// Absence is the empty value, so one test serves both `is` and
    /// `is missing` and a negated test sweeps the untagged tracks up.
    EmptyText,
    /// Absence is a missing column: `IS NULL`, and no value of any kind counts.
    NullColumn,
}

/// The value control the editor draws, named as an idea rather than as a
/// control type. The frontend owns the translation from here to whatever it
/// actually paints; the catalog only says what kind of answer the field wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueWidget {
    /// A whole number typed in.
    Number,
    /// Hours, minutes and seconds, merged into one number.
    Duration,
    /// A calendar day plus a typed time, read against the listener's own clock.
    DateTime,
    /// A window of whole days counted back from now.
    DateWindow,
    /// Yes or no, with no third state.
    YesNo,
    /// Free text.
    Text,
    /// The values this Library actually holds, offered beside the typed input
    /// rather than instead of it.
    ChooseFromLibrary,
}

impl ValueWidget {
    /// Every widget idea, in the order the spec lists them.
    pub const ALL: [Self; 7] = [
        Self::Number,
        Self::Duration,
        Self::DateTime,
        Self::DateWindow,
        Self::YesNo,
        Self::Text,
        Self::ChooseFromLibrary,
    ];

    /// The idea's name, for a test or a message that has to speak it. Never a
    /// control type: naming one here would drag a UI dependency into the
    /// persistence boundary.
    #[must_use]
    pub const fn idea(self) -> &'static str {
        match self {
            Self::Number => "Number",
            Self::Duration => "Duration",
            Self::DateTime => "DateTime",
            Self::DateWindow => "DateWindow",
            Self::YesNo => "Yes/No",
            Self::Text => "Text",
            Self::ChooseFromLibrary => "Choose from the library",
        }
    }
}

/// One field's answers — the catalog's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    field: MatchableField,
    display_name: &'static str,
    value_type: ValueType,
    comparisons: &'static [Comparison],
    pickable: bool,
}

impl FieldSpec {
    /// The field this row answers for.
    #[must_use]
    pub const fn field(&self) -> MatchableField {
        self.field
    }

    /// The word the editor shows. A Rule never refers to this: it refers to
    /// [`MatchableField::stored_id`], so re-spelling a label cannot touch a
    /// listener's saved data.
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        self.display_name
    }

    /// What kind of value the field holds.
    #[must_use]
    pub const fn value_type(&self) -> ValueType {
        self.value_type
    }

    /// The comparisons this field permits, in the order the editor lists them.
    /// Per-field legality is Rust's question, not SQL's — the store's `CHECK`
    /// lists are only the union of these.
    #[must_use]
    pub const fn comparisons(&self) -> &'static [Comparison] {
        self.comparisons
    }

    /// Whether this field takes this comparison at all.
    #[must_use]
    pub fn permits(&self, comparison: Comparison) -> bool {
        self.comparisons.contains(&comparison)
    }

    /// Whether a value of this field can be absent at all.
    ///
    /// Derived from the row's own list rather than carried beside it: `is
    /// missing` is offered exactly where a value can be absent, and two
    /// separate answers here could drift into disagreeing about a field.
    #[must_use]
    pub fn can_be_absent(&self) -> bool {
        self.permits(Comparison::IsMissing)
    }

    /// How `is missing` is decided for this field, or `None` where it cannot be
    /// absent and so is never asked.
    #[must_use]
    pub fn absence(&self) -> Option<Absence> {
        if !self.can_be_absent() {
            return None;
        }
        Some(match self.value_type {
            ValueType::Text => Absence::EmptyText,
            ValueType::Number | ValueType::Duration | ValueType::Date | ValueType::YesNo => {
                Absence::NullColumn
            }
        })
    }

    /// Whether the field's own values in this Library are pickable, beside the
    /// typed input and never instead of it.
    #[must_use]
    pub const fn is_pickable(&self) -> bool {
        self.pickable
    }

    /// The value widgets the editor draws for this field under this
    /// comparison: an input idea, plus [`ValueWidget::ChooseFromLibrary`] where
    /// the row says the field is pickable. Empty where no value is entered at
    /// all — `is missing`, and any comparison the field does not permit, which
    /// the editor must not be able to reach.
    ///
    /// The widget is answered per comparison because one field legitimately has
    /// two: a history date takes a window of days for `in the last` and a day
    /// plus a time for `before`/`after`.
    #[must_use]
    pub fn value_widgets(&self, comparison: Comparison) -> &'static [ValueWidget] {
        if !self.permits(comparison) || comparison.semantic() == crate::rule::Semantic::Absence {
            return &[];
        }
        match self.value_type {
            ValueType::Text => {
                if self.pickable {
                    TEXT_PICKABLE
                } else {
                    TEXT
                }
            }
            ValueType::Number => {
                if self.pickable {
                    NUMBER_PICKABLE
                } else {
                    NUMBER
                }
            }
            ValueType::Duration => DURATION,
            ValueType::YesNo => YES_NO,
            ValueType::Date => {
                if comparison == Comparison::InLast {
                    DATE_WINDOW
                } else {
                    DATE_TIME
                }
            }
        }
    }

    /// The row's own const constructor: a declarative catalog reads as one
    /// table instead of sixteen blocks of field initialisers.
    const fn row(
        field: MatchableField,
        display_name: &'static str,
        value_type: ValueType,
        comparisons: &'static [Comparison],
        pickable: bool,
    ) -> Self {
        Self {
            field,
            display_name,
            value_type,
            comparisons,
            pickable,
        }
    }
}

/// A text field takes equality and a needle, and can be absent.
const TEXT_COMPARISONS: &[Comparison] =
    &[Comparison::Is, Comparison::Contains, Comparison::IsMissing];
/// An arithmetic field takes the three numeric tests, and can be absent.
const NUMBER_COMPARISONS: &[Comparison] = &[
    Comparison::EqualTo,
    Comparison::GreaterThan,
    Comparison::LessThan,
    Comparison::IsMissing,
];
/// A history date takes a window or a boundary — never an exact instant, which
/// no listener could hit — and can be absent.
const DATE_COMPARISONS: &[Comparison] = &[
    Comparison::InLast,
    Comparison::Before,
    Comparison::After,
    Comparison::IsMissing,
];
/// A count the Library always has an answer for: `NOT NULL DEFAULT 0`, so 0 is
/// present rather than missing.
const COUNT_COMPARISONS: &[Comparison] = &[
    Comparison::EqualTo,
    Comparison::GreaterThan,
    Comparison::LessThan,
];
/// A fact with only ever one side to test, and no absence to ask about.
const YES_NO_COMPARISONS: &[Comparison] = &[Comparison::Is];

const TEXT: &[ValueWidget] = &[ValueWidget::Text];
const TEXT_PICKABLE: &[ValueWidget] = &[ValueWidget::Text, ValueWidget::ChooseFromLibrary];
const NUMBER: &[ValueWidget] = &[ValueWidget::Number];
const NUMBER_PICKABLE: &[ValueWidget] = &[ValueWidget::Number, ValueWidget::ChooseFromLibrary];
const DURATION: &[ValueWidget] = &[ValueWidget::Duration];
const DATE_WINDOW: &[ValueWidget] = &[ValueWidget::DateWindow];
const DATE_TIME: &[ValueWidget] = &[ValueWidget::DateTime];
const YES_NO: &[ValueWidget] = &[ValueWidget::YesNo];

/// Every Matchable Field's row, in [`MatchableField::ALL`]'s order.
static FIELDS: [FieldSpec; 16] = [
    FieldSpec::row(
        MatchableField::Genre,
        "Genre",
        ValueType::Text,
        TEXT_COMPARISONS,
        true,
    ),
    FieldSpec::row(
        MatchableField::Artist,
        "Artist",
        ValueType::Text,
        TEXT_COMPARISONS,
        true,
    ),
    FieldSpec::row(
        MatchableField::AlbumArtist,
        "Album Artist",
        ValueType::Text,
        TEXT_COMPARISONS,
        true,
    ),
    FieldSpec::row(
        MatchableField::Album,
        "Album",
        ValueType::Text,
        TEXT_COMPARISONS,
        true,
    ),
    FieldSpec::row(
        MatchableField::Title,
        "Title",
        ValueType::Text,
        TEXT_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::Composer,
        "Composer",
        ValueType::Text,
        TEXT_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::Comment,
        "Comment",
        ValueType::Text,
        TEXT_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::Year,
        "Year",
        ValueType::Number,
        NUMBER_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::Duration,
        "Duration",
        ValueType::Duration,
        NUMBER_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::PlayCount,
        "Plays",
        ValueType::Number,
        COUNT_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::LastPlayed,
        "Last played",
        ValueType::Date,
        DATE_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::DateAdded,
        "Date added",
        ValueType::Date,
        DATE_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::Favorite,
        "Favorite",
        ValueType::YesNo,
        YES_NO_COMPARISONS,
        false,
    ),
    FieldSpec::row(
        MatchableField::SampleRate,
        "Sample rate",
        ValueType::Number,
        NUMBER_COMPARISONS,
        true,
    ),
    FieldSpec::row(
        MatchableField::Channels,
        "Channels",
        ValueType::Number,
        NUMBER_COMPARISONS,
        true,
    ),
    FieldSpec::row(
        MatchableField::Lossless,
        "Lossless",
        ValueType::YesNo,
        YES_NO_COMPARISONS,
        false,
    ),
];

/// The row a field with no row answers with: nothing it permits, nothing it
/// displays. A hole in [`FIELDS`] would then fail the catalog's own coverage
/// test loudly rather than quietly misanswering for a neighbour, and the same
/// shape is what an Unreadable Rule is at the editor.
const UNANSWERED: FieldSpec = FieldSpec::row(
    MatchableField::Lossless,
    "unanswered",
    ValueType::Text,
    &[],
    false,
);

/// Every field's row, in the order the editor lists them.
#[must_use]
pub fn fields() -> &'static [FieldSpec] {
    &FIELDS
}

/// The catalog's answer for one field. Total by construction: the rows cover
/// [`MatchableField::ALL`] in the same order, which the catalog's own tests
/// pin, and an unreachable hole answers nothing.
#[must_use]
pub fn spec(field: MatchableField) -> &'static FieldSpec {
    FIELDS
        .iter()
        .find(|row| row.field() == field)
        .unwrap_or(&UNANSWERED)
}

/// The literal extensions a Rule calls lossless.
///
/// A literal list, deliberately not derived from [`crate::store::AUDIO_EXTENSIONS`]
/// nor from [`LOSSY_EXTENSIONS`]: which formats the Library can even contain is
/// a scanning preference, while a Rule classifies a stored path by its
/// extension regardless of it. The two lists meeting in exactly one place is
/// what keeps them honest; the suite asserts every scanned extension lands in
/// one of them and never both.
pub const LOSSLESS_EXTENSIONS: &[&str] = &["flac", "wav"];

/// The literal extensions a Rule calls lossy. Not the complement of
/// [`LOSSLESS_EXTENSIONS`] and not derived from it, for the same reason.
pub const LOSSY_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "opus", "ogg"];

/// Whether an extension — with or without its dot, in any case — is one of the
/// lossless formats. Unknown extensions answer `false`: a file riff cannot
/// classify is not lossless rather than a guess.
#[must_use]
pub fn extension_is_lossless(extension: &str) -> bool {
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    LOSSLESS_EXTENSIONS
        .iter()
        .any(|lossless| lossless.eq_ignore_ascii_case(extension))
}

/// Whether the format a stored path names is lossless: a case-insensitive
/// suffix on the extension, with no file read and no codec inspected. That is
/// the decision's whole cost, and its known wrong answer — an ALAC file inside
/// a `.m4a` container reports as lossy, because reading the real codec out of
/// files is out of scope for this feature.
#[must_use]
pub fn path_is_lossless(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(extension_is_lossless)
}
