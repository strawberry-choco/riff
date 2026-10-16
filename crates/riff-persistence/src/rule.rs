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
