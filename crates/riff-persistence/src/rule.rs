//! The vocabulary a Custom Smart Playlist is written in.
//!
//! A Rule is one [`MatchableField`] tested by one [`Comparison`] against one
//! value, optionally inverted; Rules live in Rule Boxes, and a list holds
//! exactly one Root Box. Everything here is a persistence-boundary type: `std`
//! only, no storage, no SQL, and no widget — what each field means and what it
//! permits is answered once, in [`crate::field_catalog`].
//!
//! `riff-backend` re-exports this surface at `riff_backend::domain`, the same
//! path [`crate::track::SmartPlaylistKind`] reaches the frontend by.

/// One piece of Track information a Rule can test.
///
/// The variant list is the enumeration — there is no other place a field is
/// spelled — and a Rule refers to a field by its [`MatchableField::stored_id`],
/// never by a display name, so a display name can change without touching any
/// listener's saved Rules.
///
/// A field this build does not know is not a variant: it reads back as an
/// Unreadable Rule, which the store keeps as the row's own raw id and never
/// coerces into a known field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchableField {
    /// The `;`-separated genre tag.
    Genre,
    /// The track's own artist tag — not the display fallback it renders as.
    Artist,
    /// The album's credited artist.
    AlbumArtist,
    /// The album title.
    Album,
    /// The track title.
    Title,
    /// The composer tag.
    Composer,
    /// The comment tag.
    Comment,
    /// The release year.
    Year,
    /// How long the track plays.
    Duration,
    /// How many times the track has finished playing.
    PlayCount,
    /// When the track last finished playing.
    LastPlayed,
    /// When the track first entered the Library.
    DateAdded,
    /// Whether the listener starred the track.
    Favorite,
    /// The track's sample rate, as tagged.
    SampleRate,
    /// The track's channel count, as tagged.
    Channels,
    /// Whether the stored path's extension is one of the catalog's literal
    /// lossless formats.
    Lossless,
}

impl MatchableField {
    /// Every Matchable Field, in the order the editor lists them. The one
    /// enumeration of the sixteen.
    pub const ALL: [Self; 16] = [
        Self::Genre,
        Self::Artist,
        Self::AlbumArtist,
        Self::Album,
        Self::Title,
        Self::Composer,
        Self::Comment,
        Self::Year,
        Self::Duration,
        Self::PlayCount,
        Self::LastPlayed,
        Self::DateAdded,
        Self::Favorite,
        Self::SampleRate,
        Self::Channels,
        Self::Lossless,
    ];

    /// The stable id a Rule stores for this field — the value of
    /// `smart_rules.field_id`, and one of the two CHECK lists the Application
    /// Store's schema enforces. Stable means stable: renaming a variant never
    /// moves this string, because saved Rules are keyed on it.
    #[must_use]
    pub const fn stored_id(self) -> &'static str {
        match self {
            Self::Genre => "genre",
            Self::Artist => "artist",
            Self::AlbumArtist => "album_artist",
            Self::Album => "album",
            Self::Title => "title",
            Self::Composer => "composer",
            Self::Comment => "comment",
            Self::Year => "year",
            Self::Duration => "duration",
            Self::PlayCount => "play_count",
            Self::LastPlayed => "last_played",
            Self::DateAdded => "date_added",
            Self::Favorite => "favorite",
            Self::SampleRate => "sample_rate",
            Self::Channels => "channels",
            Self::Lossless => "lossless",
        }
    }

    /// The field a stored id names, or `None` when this build does not know
    /// that id — which is what makes an Unreadable Rule a readable state
    /// rather than a panic or a silent guess.
    #[must_use]
    pub fn from_stored_id(id: &str) -> Option<Self> {
        let fields = Self::ALL;
        let mut index = 0;
        while index < fields.len() {
            if fields[index].stored_id() == id {
                return Some(fields[index]);
            }
            index += 1;
        }
        None
    }
}

/// The test one Rule applies to a field.
///
/// Nine stored ids. `is_not_missing` is deliberately absent from this
/// enumeration: [`Rule::negated`] already inverts any test, and one meaning
/// spelled two ways would make "has this tree changed?" comparisons lie. The
/// editor offers that wording and the save path normalises it to
/// [`Comparison::IsMissing`] with the negation set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Comparison {
    /// The stored word text and yes/no fields show for equality.
    Is,
    /// The needle appears inside the field's text.
    Contains,
    /// The field's number or date is above the value.
    GreaterThan,
    /// The field's number or date is below the value.
    LessThan,
    /// The stored word numeric and duration fields show for equality.
    EqualTo,
    /// A history date falls inside a window of whole days counted back from
    /// the read.
    InLast,
    /// A history date is earlier than the stored instant.
    Before,
    /// A history date is later than the stored instant.
    After,
    /// The field carries no value at all.
    IsMissing,
}

impl Comparison {
    /// Every stored comparison, in the order the store's CHECK list enumerates
    /// them. The one enumeration of the nine.
    pub const ALL: [Self; 9] = [
        Self::Is,
        Self::Contains,
        Self::GreaterThan,
        Self::LessThan,
        Self::EqualTo,
        Self::InLast,
        Self::Before,
        Self::After,
        Self::IsMissing,
    ];

    /// The stable id a Rule stores for this comparison — the value of
    /// `smart_rules.operator_id`.
    #[must_use]
    pub const fn stored_id(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::Contains => "contains",
            Self::GreaterThan => "greater_than",
            Self::LessThan => "less_than",
            Self::EqualTo => "equal_to",
            Self::InLast => "in_last",
            Self::Before => "before",
            Self::After => "after",
            Self::IsMissing => "is_missing",
        }
    }

    /// The comparison a stored id names, or `None` when this build does not
    /// know that id.
    #[must_use]
    pub fn from_stored_id(id: &str) -> Option<Self> {
        let comparisons = Self::ALL;
        let mut index = 0;
        while index < comparisons.len() {
            if comparisons[index].stored_id() == id {
                return Some(comparisons[index]);
            }
            index += 1;
        }
        None
    }

    /// What the comparison actually tests, with the two spellings of equality
    /// collapsed into one. A query compiler switches on this rather than on
    /// the stored id, which is the only way `is` and `equal to` can stay
    /// identical semantics under two ids: `"genre is trip-hop"` and
    /// `"year is 1975"` read as the same word to a listener while `"equal to"`
    /// reads as the arithmetic one, and which of the two a row shows is
    /// decided per field by [`crate::field_catalog`].
    #[must_use]
    pub const fn semantic(self) -> Semantic {
        match self {
            Self::Is | Self::EqualTo => Semantic::Equality,
            Self::Contains => Semantic::Containment,
            Self::GreaterThan => Semantic::GreaterThan,
            Self::LessThan => Semantic::LessThan,
            Self::InLast => Semantic::WithinDays,
            Self::Before => Semantic::EarlierThan,
            Self::After => Semantic::LaterThan,
            Self::IsMissing => Semantic::Absence,
        }
    }
}

/// The meaning a stored comparison id carries. Nine ids, eight meanings:
/// [`Comparison::Is`] and [`Comparison::EqualTo`] are one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Semantic {
    /// One test, two stored spellings.
    Equality,
    /// The needle appears inside the field.
    Containment,
    GreaterThan,
    LessThan,
    /// A moving window of whole days counted back from the read.
    WithinDays,
    EarlierThan,
    LaterThan,
    /// The field holds no value; how absence is decided is the catalog's
    /// [`crate::field_catalog::FieldSpec::absence`] answer.
    Absence,
}

/// How a Rule Box reads its children: everything inside it must match, or any
/// one thing may. The two stored words of `smart_rule_boxes.combine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combine {
    All,
    Any,
}

impl Combine {
    /// Both combine words. The enumeration, once.
    pub const ALL: [Self; 2] = [Self::All, Self::Any];

    /// The value of `smart_rule_boxes.combine`.
    #[must_use]
    pub const fn stored_id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Any => "any",
        }
    }

    /// The combine a stored id names, or `None` when this build does not know
    /// it — which is the box's own share of why an Unreadable Rule stays
    /// readable instead of being guessed at.
    #[must_use]
    pub fn from_stored_id(id: &str) -> Option<Self> {
        let combines = Self::ALL;
        let mut index = 0;
        while index < combines.len() {
            if combines[index].stored_id() == id {
                return Some(combines[index]);
            }
            index += 1;
        }
        None
    }
}

/// A date and time with no zone: the listener's own clock on the machine that
/// reads it is the whole of its meaning, because a stored zone would make one
/// saved Rule answer differently on another machine.
///
/// Carried as parts, not as a `SystemTime` or a formatted string: the value is
/// user input, wall-clock by nature, and this crate names no date library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CivilDateTime {
    /// The calendar year, as written.
    pub year: i32,
    /// The month, 1 through 12.
    pub month: u8,
    /// The day of the month, 1 through 31.
    pub day: u8,
    /// The hour of the day, 0 through 23.
    pub hour: u8,
    /// The minute, 0 through 59.
    pub minute: u8,
    /// The second, 0 through 59.
    pub second: u8,
}

/// The one value a Rule holds, already typed by the field it tests.
///
/// The store keeps a single `value` TEXT column and reads it back through the
/// catalog, so what a variant means is this type's business while how it is
/// spelled is the store's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RuleValue {
    /// A tag, title or needle.
    Text(String),
    /// A whole number: a year, a play count, a sample rate, a channel count.
    Number(i64),
    /// A length of time in whole milliseconds — the merged hours, minutes and
    /// seconds a listener typed.
    DurationMillis(u64),
    /// A window of whole days counted back from the read.
    Days(u32),
    /// A zone-less day plus a typed time.
    DateTime(CivilDateTime),
    /// Yes or no.
    YesNo(bool),
}

impl RuleValue {
    /// The value type this answer belongs to — the catalog's row and the draft
    /// value meet here, so a Rule can be checked for agreement without any
    /// caller restating which fields take which kind of value.
    #[must_use]
    pub const fn value_type(&self) -> crate::field_catalog::ValueType {
        use crate::field_catalog::ValueType;
        match self {
            Self::Text(_) => ValueType::Text,
            Self::Number(_) => ValueType::Number,
            Self::DurationMillis(_) => ValueType::Duration,
            Self::Days(_) | Self::DateTime(_) => ValueType::Date,
            Self::YesNo(_) => ValueType::YesNo,
        }
    }
}

/// One condition on a Track fact: a field, a comparison, whether the answer is
/// inverted, and the value it is tested against.
///
/// A Rule carries no identity. It is compared by shape, so two drafts that
/// spell the same test are the same Rule, and a store-generated id — opaque,
/// never handed to the frontend — cannot go stale inside a listener's edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    field: MatchableField,
    comparison: Comparison,
    negated: bool,
    value: Option<RuleValue>,
}

impl Rule {
    /// The Rule that asserts its test outright. `None` is only correct for a
    /// comparison that takes no value; see [`Rule::is_fully_formed`].
    #[must_use]
    pub fn new(
        field: MatchableField,
        comparison: Comparison,
        value: Option<RuleValue>,
    ) -> Self {
        Self {
            field,
            comparison,
            negated: false,
            value,
        }
    }

    /// The same test with its answer inverted. This is the only spelling of
    /// "is not", and it works on every comparison — which is exactly why
    /// `is_not_missing` never became a tenth stored id.
    #[must_use]
    pub fn negated(mut self) -> Self {
        self.negated = true;
        self
    }

    /// The Matchable Field this Rule tests.
    #[must_use]
    pub const fn field(&self) -> MatchableField {
        self.field
    }

    /// The comparison this Rule applies.
    #[must_use]
    pub const fn comparison(&self) -> Comparison {
        self.comparison
    }

    /// Whether the answer is inverted.
    #[must_use]
    pub const fn is_negated(&self) -> bool {
        self.negated
    }

    /// The value tested against, if this comparison takes one.
    #[must_use]
    pub fn value(&self) -> Option<&RuleValue> {
        self.value.as_ref()
    }

    /// Whether this Rule is a whole, storable test: the field permits the
    /// comparison, and exactly the value the catalog's widget answer calls for
    /// is present — none where none is drawn, and one of the field's own type
    /// everywhere else.
    ///
    /// Both halves are the catalog's answers, asked here rather than spelled
    /// again: a half-filled draft row and a mis-typed value are the two ways a
    /// Rule that cannot be saved reaches the store, and the save path names the
    /// offender by asking this.
    #[must_use]
    pub fn is_fully_formed(&self) -> bool {
        let row = crate::field_catalog::spec(self.field);
        if !row.permits(self.comparison) {
            return false;
        }
        let draws_a_value = !row.value_widgets(self.comparison).is_empty();
        match &self.value {
            Some(value) => draws_a_value && value.value_type() == row.value_type(),
            None => !draws_a_value,
        }
    }
}

/// One child of a Rule Box, in the box's single sibling order: a Rule, or a
/// box of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleChild {
    /// A leaf test.
    Rule(Rule),
    /// A nested box, holding more of both.
    Box(RuleBox),
}

/// A container that says whether everything inside it must match or whether
/// any one thing may, holding Rules and Boxes in one shared sibling order.
///
/// The order is a single sequence because the store numbers siblings together:
/// the editor draws one interleaved list, and a tree that kept rules and boxes
/// in two lists could not say which row sits third.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleBox {
    combine: Combine,
    children: Vec<RuleChild>,
}

impl RuleBox {
    /// An empty box. Storable only once something is inside it — see
    /// [`RuleBox::is_empty`].
    #[must_use]
    pub const fn new(combine: Combine) -> Self {
        Self {
            combine,
            children: Vec::new(),
        }
    }

    /// The box's siblings, in order.
    #[must_use]
    pub fn with_children(mut self, children: Vec<RuleChild>) -> Self {
        self.children = children;
        self
    }

    /// Whether all of the children must match or any one may.
    #[must_use]
    pub const fn combine(&self) -> Combine {
        self.combine
    }

    /// The one shared sibling order, Rules and Boxes together.
    #[must_use]
    pub fn children(&self) -> &[RuleChild] {
        &self.children
    }

    /// How many Rules sit inside this box and every box below it.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| match child {
                RuleChild::Rule(_) => 1,
                RuleChild::Box(boxed) => boxed.rule_count(),
            })
            .sum()
    }

    /// Whether this box holds no Rule at all, at any depth. An empty box is
    /// never stored — a tree, and every box inside it, has to hold at least one
    /// Rule, because nothing about a Rule-less box is worth interpreting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rule_count() == 0
    }
}

/// The one Rule Box in a list that has no parent: every other box and Rule
/// hangs inside it, and the tree is read from here.
///
/// A wrapper rather than a `RuleBox` so "a list has exactly one" is a fact a
/// caller can hold in a field's type instead of a promise in a comment. It
/// adds no rule of its own: an empty Root Box is refused for the same reason
/// any other empty box is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootBox(RuleBox);

impl RootBox {
    /// The tree's top box.
    #[must_use]
    pub const fn new(boxed: RuleBox) -> Self {
        Self(boxed)
    }

    /// The box itself, for a caller that walks children generically.
    #[must_use]
    pub fn inner(&self) -> &RuleBox {
        &self.0
    }

    /// Whether everything inside the root must match or any one thing may.
    #[must_use]
    pub const fn combine(&self) -> Combine {
        self.0.combine()
    }

    /// The root's children, in their one shared sibling order.
    #[must_use]
    pub fn children(&self) -> &[RuleChild] {
        self.0.children()
    }

    /// How many Rules the whole tree holds.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.0.rule_count()
    }

    /// Whether the tree holds no Rule anywhere in it — the one shape a save
    /// always refuses.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<RuleBox> for RootBox {
    fn from(boxed: RuleBox) -> Self {
        Self::new(boxed)
    }
}
