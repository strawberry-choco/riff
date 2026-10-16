// riff-persistence — the Matchable Field catalog's facts (ticket 01).
//
// The catalog answers six things per field and is the only place that does:
// its stable id, the word riff shows, its value type, the comparisons it
// permits, whether a value can be absent at all, and which value widget the
// editor draws. Per-field legality has to be answerable in Rust with no
// database, because the store's `CHECK` lists are only the union of legal ids.
//
// These tests therefore ask the catalog the spec's questions. Where a rule is
// quantified ("no arithmetic field takes `contains`") it is quantified over
// the catalog's own answers; nowhere does this file copy the field table.

use std::path::Path;

use riff_persistence::field_catalog::Absence;
use riff_persistence::field_catalog::FieldSpec;
use riff_persistence::field_catalog::ValueWidget;
use riff_persistence::field_catalog::ValueType;
use riff_persistence::field_catalog::fields;
use riff_persistence::rule::Comparison;
use riff_persistence::rule::MatchableField;
use riff_persistence::rule::Semantic;

/// The catalog's rows, looked up by field.
fn spec(field: MatchableField) -> &'static FieldSpec {
    riff_persistence::field_catalog::spec(field)
}

/// The one thing the spec lets a field's row answer for equality: the stored
/// word it shows, if it shows one at all.
fn equality_shown(row: &FieldSpec) -> Option<Comparison> {
    row.comparisons()
        .iter()
        .find(|comparison| comparison.semantic() == Semantic::Equality)
        .copied()
}

#[test]
fn every_matchable_field_has_exactly_one_catalog_row() {
    let catalog: &[FieldSpec] = fields();
    assert_eq!(
        catalog.len(),
        MatchableField::ALL.len(),
        "one row per Matchable Field, so no field is answered twice and none is missed"
    );
    for field in MatchableField::ALL {
        assert_eq!(
            catalog.iter().filter(|row| row.field() == field).count(),
            1,
            "{field:?} is answered by exactly one row"
        );
        assert_eq!(
            spec(field).field(),
            field,
            "the catalog is indexed by the field it answers for"
        );
    }
    // The rows are the editor's order, so the enumeration and the catalog
    // cannot drift into disagreeing about which field comes first.
    let row_order: Vec<MatchableField> = catalog.iter().map(FieldSpec::field).collect();
    assert_eq!(
        row_order,
        MatchableField::ALL.to_vec(),
        "the catalog lists its rows in the order the fields are enumerated"
    );
}

#[test]
fn per_field_operator_legality_is_a_catalog_question_with_no_database() {
    // The spec's worked examples of what must NOT be a choice.
    for (field, comparison, why) in [
        (MatchableField::Year, Comparison::Contains, "\"year contains\""),
        (MatchableField::Year, Comparison::Is, "a number shows the arithmetic word"),
        (MatchableField::Duration, Comparison::Contains, "duration is not text"),
        (MatchableField::PlayCount, Comparison::Before, "a count is not a date"),
        (
            MatchableField::LastPlayed,
            Comparison::GreaterThan,
            "a history date is asked as before/after",
        ),
        (
            MatchableField::Lossless,
            Comparison::Contains,
            "a yes/no fact holds no needle",
        ),
        (
            MatchableField::Genre,
            Comparison::LessThan,
            "a tag has no order",
        ),
    ] {
        assert!(
            !spec(field).permits(comparison),
            "{why} is not a legal Rule on {}: the catalog refuses it in Rust, before any \
             `CHECK` list or SQL is involved",
            spec(field).display_name()
        );
    }

    for row in fields() {
        assert!(
            !row.comparisons().is_empty(),
            "{} offers at least one comparison",
            row.display_name()
        );
        let mut listed: Vec<Comparison> = row.comparisons().to_vec();
        let before = listed.len();
        listed.sort_by_key(|comparison| {
            Comparison::ALL
                .iter()
                .position(|known| known == comparison)
                .unwrap()
        });
        listed.dedup();
        assert_eq!(
            listed.len(),
            before,
            "{} lists each comparison once, so the editor cannot offer it twice",
            row.display_name()
        );
    }
}

#[test]
fn the_catalogs_permitted_comparisons_are_exactly_the_nine_stored_ids() {
    // The union of the rows is what the store's `operator_id` CHECK list is
    // allowed to be: a legal id no field offers would be a tenth spelling, and
    // an offered id outside the nine could not be spelled at all.
    let mut union: Vec<Comparison> = Vec::new();
    for row in fields() {
        for comparison in row.comparisons() {
            if !union.contains(comparison) {
                union.push(*comparison);
            }
        }
    }
    union.sort_by_key(|comparison| {
        Comparison::ALL
            .iter()
            .position(|known| known == comparison)
            .unwrap()
    });
    assert_eq!(
        union,
        Comparison::ALL.to_vec(),
        "the rows cover all nine stored ids between them and offer no id outside them"
    );
}

#[test]
fn the_two_text_only_comparisons_are_hidden_on_numeric_and_date_fields() {
    for row in fields() {
        if matches!(
            row.value_type(),
            ValueType::Number | ValueType::Duration | ValueType::Date
        ) {
            assert!(
                !row.permits(Comparison::Contains) && !row.permits(Comparison::Is),
                "{} is not a text field, so the text-only comparisons are hidden on it",
                row.display_name()
            );
        }
        if row.permits(Comparison::Contains) {
            assert_eq!(
                row.value_type(),
                ValueType::Text,
                "{} takes a needle, so the catalog calls it text",
                row.display_name()
            );
        }
    }
}

#[test]
fn is_missing_is_offered_only_where_a_value_can_actually_be_absent() {
    // The three fields the spec names as never absent: `play_count` and
    // `favorite` are `NOT NULL DEFAULT 0` so 0 is present, not missing, and an
    // unknown extension answers as not lossless rather than as missing.
    for never_absent in [
        MatchableField::PlayCount,
        MatchableField::Favorite,
        MatchableField::Lossless,
    ] {
        let row = spec(never_absent);
        assert!(
            !row.can_be_absent(),
            "a value of {} can never be absent",
            row.display_name()
        );
        assert!(
            !row.permits(Comparison::IsMissing),
            "so `is missing` is not offered on {}",
            row.display_name()
        );
    }

    for row in fields() {
        assert_eq!(
            row.can_be_absent(),
            row.permits(Comparison::IsMissing),
            "{}: the absence answer and the offered `is missing` are one fact, not two",
            row.display_name()
        );
        if row.can_be_absent() {
            assert!(
                row.absence().is_some(),
                "{} says how its absence is decided",
                row.display_name()
            );
        } else {
            assert_eq!(
                row.absence(),
                None,
                "{} has no absence to decide",
                row.display_name()
            );
        }
    }
}

#[test]
fn absence_is_the_empty_value_on_text_and_a_missing_column_elsewhere() {
    for row in fields() {
        match row.absence() {
            None => assert!(
                !row.can_be_absent(),
                "{} answers no absence",
                row.display_name()
            ),
            Some(Absence::EmptyText) => assert_eq!(
                row.value_type(),
                ValueType::Text,
                "only a text field can be absent as its empty value"
            ),
            Some(Absence::NullColumn) => assert_ne!(
                row.value_type(),
                ValueType::Text,
                "a non-text field is absent as a missing column, never as an empty string"
            ),
        }
    }
}

#[test]
fn each_field_shows_at_most_one_word_for_equality() {
    // `is` and `equal_to` are one meaning under two stored ids; which one a
    // row shows is this catalog's question, so a row must answer it once.
    for row in fields() {
        let spellings: Vec<Comparison> = row
            .comparisons()
            .iter()
            .filter(|comparison| comparison.semantic() == Semantic::Equality)
            .copied()
            .collect();
        assert!(
            spellings.len() <= 1,
            "{} shows {} as its equality word, never two",
            row.display_name(),
            spellings
                .first()
                .map_or("no", |comparison| comparison.stored_id())
        );
    }

    for row in fields() {
        match row.value_type() {
            ValueType::Text | ValueType::YesNo => assert_eq!(
                equality_shown(row),
                Some(Comparison::Is),
                "{} reads as a listener's plain \"is\" ({} only)",
                row.display_name(),
                Comparison::Is.stored_id()
            ),
            ValueType::Number | ValueType::Duration => assert_eq!(
                equality_shown(row),
                Some(Comparison::EqualTo),
                "{} is arithmetic, so the catalog shows the arithmetic word",
                row.display_name()
            ),
            ValueType::Date => assert_eq!(
                equality_shown(row),
                None,
                "{} is asked as a window or a boundary, never as an exact instant",
                row.display_name()
            ),
        }
    }
}

#[test]
fn the_value_widget_follows_the_field_and_the_comparison() {
    // The date fields are the case where one field needs two widgets, which is
    // why the widget is answered per comparison rather than per row.
    for history_date in [MatchableField::LastPlayed, MatchableField::DateAdded] {
        let row = spec(history_date);
        assert_eq!(
            row.value_widgets(Comparison::InLast),
            &[ValueWidget::DateWindow],
            "{} asks for a moving window of whole days",
            row.display_name()
        );
        for boundary in [Comparison::Before, Comparison::After] {
            assert_eq!(
                row.value_widgets(boundary),
                &[ValueWidget::DateTime],
                "{} takes a day plus a typed time against {}",
                row.display_name(),
                boundary.stored_id()
            );
        }
    }

    assert_eq!(
        spec(MatchableField::Duration).value_widgets(Comparison::GreaterThan),
        &[ValueWidget::Duration],
        "duration entry is hours, minutes and seconds merged into one number"
    );
    for yes_no in [MatchableField::Favorite, MatchableField::Lossless] {
        assert_eq!(
            spec(yes_no).value_widgets(Comparison::Is),
            &[ValueWidget::YesNo],
            "{} answers yes or no",
            spec(yes_no).display_name()
        );
    }
    assert_eq!(
        spec(MatchableField::Title).value_widgets(Comparison::Contains),
        &[ValueWidget::Text],
        "a plain text field gets one text input and no library list"
    );
    assert_eq!(
        spec(MatchableField::Genre).value_widgets(Comparison::Is),
        &[ValueWidget::Text, ValueWidget::ChooseFromLibrary],
        "a pickable text field gets its input AND the library's own values"
    );
    assert_eq!(
        spec(MatchableField::SampleRate).value_widgets(Comparison::EqualTo),
        &[ValueWidget::Number, ValueWidget::ChooseFromLibrary],
        "a pickable numeric field gets its number AND the library's own values"
    );
}

#[test]
fn a_comparison_that_takes_no_value_draws_no_widget() {
    // `is_missing` is the only such comparison, and its stored value is NULL —
    // a draft row that drew a widget for it would invite a value the save
    // path then has to throw away.
    for row in fields() {
        for comparison in Comparison::ALL {
            if comparison == Comparison::IsMissing || !row.permits(comparison) {
                assert!(
                    row.value_widgets(comparison).is_empty(),
                    "{} against {} draws no value widget",
                    row.display_name(),
                    comparison.stored_id()
                );
            } else {
                assert!(
                    !row.value_widgets(comparison).is_empty(),
                    "{} against {} draws at least one value widget",
                    row.display_name(),
                    comparison.stored_id()
                );
            }
        }
    }
}

#[test]
fn the_library_picker_is_an_affordance_beside_an_input_never_instead_of_one() {
    for row in fields() {
        for &comparison in row.comparisons() {
            let widgets = row.value_widgets(comparison);
            if widgets.is_empty() {
                // `is missing` draws no value at all, so it draws no picker
                // either — even on a pickable field.
                continue;
            }
            assert_eq!(
                widgets.contains(&ValueWidget::ChooseFromLibrary),
                row.is_pickable(),
                "where a value is drawn, the picker shows exactly when the row says the field \
                 is pickable — once, from the row"
            );
            if widgets.contains(&ValueWidget::ChooseFromLibrary) {
                assert_eq!(
                    widgets.len(),
                    2,
                    "{} keeps a typed input beside its picker, so a listener can always write \
                     a value the Library does not hold",
                    row.display_name()
                );
                assert!(
                    matches!(row.value_type(), ValueType::Text | ValueType::Number),
                    "{} is pickable, so its values are things the Library can list",
                    row.display_name()
                );
            }
        }
    }
}

#[test]
fn the_widgets_are_named_as_ideas_never_as_controls() {
    // The spec's seven widget ideas, and nothing in the catalog's vocabulary
    // that smells like a toolkit control. This crate carries no UI dependency
    // to name one even by accident — its manifest has no `[dependencies]`.
    let ideas: Vec<&str> = ValueWidget::ALL.iter().map(|widget| widget.idea()).collect();
    assert_eq!(
        ideas,
        vec![
            "Number",
            "Duration",
            "DateTime",
            "DateWindow",
            "Yes/No",
            "Text",
            "Choose from the library",
        ],
        "the catalog names its widget as an idea, exactly the seven the spec lists"
    );

    for widget in ValueWidget::ALL {
        let idea = widget.idea().to_lowercase();
        for control in ["combo", "box", "button", "field", "input", "slider", "spin"] {
            assert!(
                !idea.contains(control),
                "{} is an idea, not a control type ({control} appears nowhere in it)",
                widget.idea()
            );
        }
    }
}

#[test]
fn display_names_are_words_the_rest_of_riff_already_uses() {
    // A display name is the listener's word and never what a Rule refers to,
    // so it can be re-spelled without touching saved data. These are the
    // spellings the brief and the existing surfaces pin: the tag editor's
    // "Album Artist", the inspector's "Plays" and "Last played", and the
    // app's long-standing US "Favorite" rather than the glossary's "Favourite".
    assert_eq!(spec(MatchableField::AlbumArtist).display_name(), "Album Artist");
    assert_eq!(spec(MatchableField::Year).display_name(), "Year");
    assert_eq!(spec(MatchableField::PlayCount).display_name(), "Plays");
    assert_eq!(spec(MatchableField::LastPlayed).display_name(), "Last played");
    assert_eq!(spec(MatchableField::Favorite).display_name(), "Favorite");

    let mut names: Vec<&str> = Vec::new();
    for row in fields() {
        assert!(
            !row.display_name().is_empty(),
            "every row carries the word the editor shows"
        );
        assert!(
            !row.display_name().contains('_'),
            "{} is a stored id, not a display name",
            row.display_name()
        );
        assert_ne!(
            row.display_name(),
            row.field().stored_id(),
            "{} is shown as a word, not as the id a Rule stores",
            row.field().stored_id()
        );
        assert!(
            !names.contains(&row.display_name()),
            "two fields never share one label in the editor"
        );
        names.push(row.display_name());
    }
}

#[test]
fn every_scanned_format_is_classified_by_the_two_literal_lists() {
    // Both constants are visible from this crate — `AUDIO_EXTENSIONS` is the
    // Library Scan's own list and the two classification lists are the
    // catalog's — so the assertion lives beside them instead of inverting a
    // dependency arrow or moving a constant out of the house it lives in.
    for scanned in riff_persistence::store::AUDIO_EXTENSIONS {
        let lossless = riff_persistence::field_catalog::LOSSLESS_EXTENSIONS
            .iter()
            .any(|lossless| lossless == scanned);
        let lossy = riff_persistence::field_catalog::LOSSY_EXTENSIONS
            .iter()
            .any(|lossy| lossy == scanned);
        assert!(
            lossless != lossy,
            "{scanned} is a format the Library can contain, so the catalog must classify it as \
             lossless xor lossy — exactly one of the two lists names it"
        );
    }
}

#[test]
fn the_two_lists_are_literals_that_neither_contain_nor_derive_from_each_other() {
    // Two literal lists, spelled from the spec: what the Library can even
    // contain is a scanning preference, and a Rule classifies a stored path
    // regardless of it. `flac`/`wav` are the whole lossless answer, and the
    // ALAC-in-`.m4a` decision is the pinned wrong answer that lives here.
    assert_eq!(
        riff_persistence::field_catalog::LOSSLESS_EXTENSIONS,
        &["flac", "wav"],
        "lossless is its own literal list"
    );
    assert_eq!(
        riff_persistence::field_catalog::LOSSY_EXTENSIONS,
        &["mp3", "m4a", "aac", "opus", "ogg"],
        "lossy is its own literal list, not the complement of the other"
    );

    for lossless in riff_persistence::field_catalog::LOSSLESS_EXTENSIONS {
        assert!(
            !riff_persistence::field_catalog::LOSSY_EXTENSIONS
                .iter()
                .any(|lossy| lossy == lossless),
            "{lossless} is classified once"
        );
        assert!(
            riff_persistence::field_catalog::extension_is_lossless(lossless),
            "{lossless} is in the list the classifier reads"
        );
    }
}

#[test]
fn a_rule_can_test_losslessness_of_a_path_without_reading_a_file() {
    // The catalog's literal lists are the whole decision: a suffix match on
    // the stored path, case-insensitively, with anything it cannot classify
    // answering as not lossless.
    use riff_persistence::field_catalog::path_is_lossless;

    assert!(path_is_lossless(Path::new("/m/a.flac")), "flac is lossless");
    assert!(path_is_lossless(Path::new("/m/a.FLAC")), "case does not change it");
    assert!(path_is_lossless(Path::new("/m/a.Wav")), "nor does mixed case");
    assert!(
        !path_is_lossless(Path::new("/m/a.m4a")),
        "an ALAC file inside .m4a stays lossy: riff does not read the real codec, and \
         this is the pinned wrong answer, not a bug to fix as a side effect"
    );
    assert!(
        !path_is_lossless(Path::new("/m/a.aiff")),
        "an extension riff does not know is not lossless"
    );
    assert!(
        !path_is_lossless(Path::new("/m/track")),
        "a stored path with no extension is not lossless either"
    );
    assert!(
        !path_is_lossless(Path::new("/m/.hidden")),
        "a dotfile's 'extension' is its name, so it is not lossless"
    );
}
