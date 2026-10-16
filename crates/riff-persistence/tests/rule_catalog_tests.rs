// riff-persistence — the Custom Smart Playlist vocabulary's one house (ticket 01).
//
// A Rule is written in Matchable Fields and comparisons. Both are closed
// enumerations whose stored ids are exactly the two `CHECK` lists the
// Application Store's `smart_rules` column enforces, so the day an id is
// added to one side and not the other is the day this suite fails — it names
// the id instead of a Rule quietly reading back as Unreadable.
//
// Every assertion here is against the catalog's own answers, never against a
// copy of the per-field matrix: the field table lives in `field_catalog` and
// nowhere else.

use riff_persistence::rule::Comparison;
use riff_persistence::rule::MatchableField;

/// The sixteen field ids the store's `field_id` CHECK list spells, in the
/// order the editor lists them. Copied from the spec's DRAFT, not derived from
/// the catalog — this is the contract a parallel ticket writes SQL against.
const STORED_FIELD_IDS: [&str; 16] = [
    "genre",
    "artist",
    "album_artist",
    "album",
    "title",
    "composer",
    "comment",
    "year",
    "duration",
    "play_count",
    "last_played",
    "date_added",
    "favorite",
    "sample_rate",
    "channels",
    "lossless",
];

/// The nine stored comparison ids — nine, because `is_not_missing` is
/// deliberately absent and `negated` is what inverts a test.
const STORED_COMPARISON_IDS: [&str; 9] = [
    "is",
    "contains",
    "greater_than",
    "less_than",
    "equal_to",
    "in_last",
    "before",
    "after",
    "is_missing",
];

#[test]
fn the_sixteen_matchable_fields_are_enumerated_once_under_their_stored_ids() {
    let ids: Vec<&str> = MatchableField::ALL.iter().map(|field| field.stored_id()).collect();
    assert_eq!(
        ids,
        STORED_FIELD_IDS.to_vec(),
        "the enumerated Matchable Fields are exactly the sixteen stored ids, in order"
    );

    for field in MatchableField::ALL {
        assert_eq!(
            MatchableField::from_stored_id(field.stored_id()),
            Some(field),
            "every field id reads back as the field it names"
        );
    }
}

#[test]
fn a_field_id_this_build_does_not_know_is_not_a_matchable_field() {
    // An id outside the list is an Unreadable Rule, so it must have no
    // spelling here at all — and `from_stored_id` answering `None` is how the
    // store keeps that row instead of guessing at a field.
    for unknown in ["", "format", "Genre", "genre ", "bitrate", "album artist"] {
        assert_eq!(
            MatchableField::from_stored_id(unknown),
            None,
            "{unknown:?} is not a Matchable Field this build knows"
        );
    }
}

#[test]
fn the_nine_stored_comparisons_are_enumerated_once_under_their_stored_ids() {
    let ids: Vec<&str> = Comparison::ALL.iter().map(|comparison| comparison.stored_id()).collect();
    assert_eq!(
        ids,
        STORED_COMPARISON_IDS.to_vec(),
        "the enumerated comparisons are exactly the nine stored ids, in order"
    );

    for comparison in Comparison::ALL {
        assert_eq!(
            Comparison::from_stored_id(comparison.stored_id()),
            Some(comparison),
            "every comparison id reads back as the comparison it names"
        );
    }
}

#[test]
fn is_not_missing_has_no_stored_spelling_because_negation_already_inverts() {
    // One meaning spelled two ways makes "has this tree changed?" comparisons
    // lie, so the editor's wording normalises to `is_missing` + negation on
    // the way out and the store never sees a tenth id.
    assert_eq!(
        Comparison::from_stored_id("is_not_missing"),
        None,
        "`is_not_missing` is not a stored comparison"
    );
    assert_eq!(Comparison::ALL.len(), 9, "nine stored comparisons, not ten");
}

#[test]
fn an_unknown_comparison_id_is_not_spelled_as_a_known_one() {
    for unknown in ["", "is_not", "greater then", "EQUAL_TO", "missing"] {
        assert_eq!(
            Comparison::from_stored_id(unknown),
            None,
            "{unknown:?} is not a stored comparison"
        );
    }
}

#[test]
fn is_and_equal_to_are_one_equality_stored_under_two_ids() {
    // Which word a row shows is the catalog's question; what the test MEANS is
    // identical for both, and that is the invariant a query compiler switches
    // on. `Semantic` is where the two meet.
    assert_eq!(
        Comparison::Is.semantic(),
        Comparison::EqualTo.semantic(),
        "\"genre is trip-hop\" and \"year is 1975\" are the same test"
    );

    let mut meanings = Vec::new();
    for comparison in Comparison::ALL {
        let semantic = comparison.semantic();
        if !meanings.contains(&semantic) {
            meanings.push(semantic);
        }
    }
    assert_eq!(
        meanings.len(),
        Comparison::ALL.len() - 1,
        "exactly one pair of stored ids shares one meaning — `is` and `equal_to` — and \
         every other id means something on its own"
    );
}
