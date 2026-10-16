// riff-infra — the Custom Smart Playlist writes: save, rename, delete, Duplicate.
//
// A listener's whole Rule tree is written in one step and reads back exactly as
// they wrote it — same Rules, same Boxes, same nesting, same sibling order — and
// it survives a restart unchanged. Asserted at the store seam against a real
// SQLite file in a fresh temp directory per test: no mock store can hold a
// transaction, a cascade or a `CHECK`, and the store's own constructor is the
// only way in, because a raw connection runs with `PRAGMA foreign_keys` OFF and
// would make every cascade assertion below vacuous.
//
// Two kinds of assertion are made: what the port read gives back, and — where
// the ticket asks for the storage contract itself — how a value is spelled in its
// column and how many rows a delete left behind. Counting rows is how the
// cascade is proved rather than trusted.
//
// The four writes move the **playlist** generation and the Library generation
// never. That pairing is the odd one the brief states as intended: a list's name
// and tree are list identity, while which Tracks match is a question about Tracks.
//
// Run: cargo test -p riff-infra --test integration smart_playlist_writes

use super::*;
use riff_persistence::rule::{
    Combine, Comparison, MatchableField, RootBox, Rule, RuleBox, RuleChild, RuleValue,
};
use riff_persistence::store::{SmartPlaylistTree, UnreadableRule};

/// A store in its own scratch directory, with the guard so the file stays alive
/// for the whole test, and the channel every committed mutation reports on.
fn fresh_store() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    SqliteStore,
    crossbeam_channel::Receiver<StoreChanged>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let store = SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("opening the Application Store must succeed");
    (dir, db_path, store, changes_rx)
}

/// Every [`StoreChanged`] notice queued so far, in order.
fn drained(rx: &crossbeam_channel::Receiver<StoreChanged>) -> Vec<StoreChanged> {
    rx.try_iter().collect()
}

/// The rows the three rule tables hold, as `(lists, boxes, rules)`.
fn row_counts(store: &SqliteStore) -> (usize, usize, usize) {
    store
        .with_connection(|conn| {
            Ok((
                count_of(conn, "SELECT COUNT(*) FROM smart_playlists")?,
                count_of(conn, "SELECT COUNT(*) FROM smart_rule_boxes")?,
                count_of(conn, "SELECT COUNT(*) FROM smart_rules")?,
            ))
        })
        .expect("counting rows must work")
}

fn count_of(conn: &rusqlite::Connection, sql: &str) -> rusqlite::Result<usize> {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
}

/// The stored `value`, `operator_id` and `negated` of the store's single Rule.
///
/// Read raw because these tests are about the storage contract: what a value is
/// spelled as, and which spelling of "is not missing" lands in the column.
fn stored_rule(store: &SqliteStore) -> (Option<String>, String, i64) {
    store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT value, operator_id, negated FROM smart_rules LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
        })
        .expect("reading the stored rule must work")
}

/// The `field_id` of every Rule the store holds.
fn tree_row_fields(store: &SqliteStore) -> Vec<String> {
    store
        .with_connection(|conn| ids_in(conn, "SELECT field_id FROM smart_rules ORDER BY id"))
        .expect("listing rule fields must work")
}

/// Every box id and rule id the store holds, as `(rules, boxes)`.
fn tree_row_ids(store: &SqliteStore) -> (Vec<String>, Vec<String>) {
    store
        .with_connection(|conn| {
            let rules = ids_in(conn, "SELECT id FROM smart_rules ORDER BY id")?;
            let boxes = ids_in(conn, "SELECT id FROM smart_rule_boxes ORDER BY id")?;
            Ok((rules, boxes))
        })
        .expect("listing row ids must work")
}

fn ids_in(conn: &rusqlite::Connection, sql: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Every Rule row the store holds, exactly as stored, ordered by list, box and
/// sibling position.
///
/// This is the comparison for the assertions that must prove **content** — that a
/// refused write left the tree precisely where it was — rather than reading a
/// refusal message, which is only a claim the UI makes about itself.
fn stored_rule_rows(store: &SqliteStore) -> Vec<(String, String, i64, Option<String>, i64)> {
    store
        .with_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT r.field_id, r.operator_id, r.negated, r.value, r.position \
                 FROM smart_rules r JOIN smart_rule_boxes b ON b.id = r.box_id \
                 ORDER BY b.playlist_id, b.id, r.position",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            rows.collect()
        })
        .expect("reading every stored rule row must work")
}

/// The names the `smart_playlists` table holds, sorted, for the Duplicate
/// assertions.
fn smart_names(store: &SqliteStore) -> Vec<String> {
    store
        .with_connection(|conn| ids_in(conn, "SELECT name FROM smart_playlists ORDER BY name"))
        .expect("reading names must work")
}

// --- fixtures ---------------------------------------------------------------

fn rule(field: MatchableField, comparison: Comparison, value: Option<RuleValue>) -> RuleChild {
    RuleChild::Rule(Rule::new(field, comparison, value))
}

fn negated_absence(field: MatchableField) -> RuleChild {
    RuleChild::Rule(Rule::new(field, Comparison::IsMissing, None).negated())
}

fn nested(combine: Combine, children: Vec<RuleChild>) -> RuleChild {
    RuleChild::Box(RuleBox::new(combine).with_children(children))
}

fn year_rule() -> RuleChild {
    rule(
        MatchableField::Year,
        Comparison::EqualTo,
        Some(RuleValue::Number(1975)),
    )
}

/// A Root Box over `children`, combining with [`Combine::All`].
fn tree(children: Vec<RuleChild>) -> SmartPlaylistTree {
    SmartPlaylistTree::new(RootBox::new(
        RuleBox::new(Combine::All).with_children(children),
    ))
}

/// The mixed tree: Rules and Boxes interleaved in the Root Box, a nested box, and
/// every value shape the storage rules name.
///
/// Sibling order is the assertion's subject, so the order here is deliberately
/// not grouped by kind: rule, box, rule, box, rule.
fn authored_tree() -> SmartPlaylistTree {
    tree(vec![
        rule(
            MatchableField::Genre,
            Comparison::Contains,
            Some(RuleValue::Text("rock".to_string())),
        ),
        nested(
            Combine::Any,
            vec![
                year_rule(),
                rule(
                    MatchableField::Duration,
                    Comparison::GreaterThan,
                    Some(RuleValue::DurationMillis(60_000)),
                ),
            ],
        ),
        rule(
            MatchableField::Favorite,
            Comparison::Is,
            Some(RuleValue::YesNo(true)),
        ),
        nested(
            Combine::All,
            vec![rule(
                MatchableField::LastPlayed,
                Comparison::InLast,
                Some(RuleValue::Days(30)),
            )],
        ),
        negated_absence(MatchableField::Artist),
    ])
}

/// The refusal's reason, or a panic naming what came back instead — which is also
/// how "the store's one error variant, and no new one" is pinned.
fn refusal<T: std::fmt::Debug>(outcome: Result<T, StoreError>, what: &str) -> String {
    match outcome {
        Err(StoreError::InvalidOperation(reason)) => reason,
        other => {
            panic!("{what} must be refused as a `StoreError::InvalidOperation`, got {other:?}")
        }
    }
}

/// The smallest Track a scan batch accepts, for the tests that need a real
/// Library row beside the rule tables.
fn fixture_track(path: &str) -> Track {
    let id = TrackId::from_path(std::path::Path::new(path));
    Track::fresh_scan(
        id,
        PathBuf::from(path),
        TrackMetadata {
            title: Some("One".to_string()),
            album: Some("Album".to_string()),
            album_artist: Some("Artist".to_string()),
            ..TrackMetadata::default()
        },
        None,
        None,
        None,
        None,
    )
}

// --- the save, and the tree that reads back --------------------------------

#[test]
fn a_saved_tree_reads_back_exactly_as_authored() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let authored = authored_tree();
    let id = store
        .save_smart_playlist(None, "Wide Net", &authored)
        .expect("the first save inserts the row and the tree");

    let read = store
        .smart_playlist_tree(&id)
        .expect("reading the tree must work")
        .expect("the list the save just created is readable");

    // The whole tree, not a summary of it: same Rules, same Boxes, same
    // nesting, same interleaved sibling order, Root Box included.
    assert_eq!(read, authored, "the tree reads back as it was written");
    assert!(
        read.unreadable().is_empty(),
        "a tree this build wrote holds nothing it cannot read"
    );
    assert_eq!(read.root().rule_count(), 6, "every Rule is back");
    let children = read.root().children();
    assert_eq!(children.len(), 5, "the Root Box's one interleaved order");
    assert!(
        matches!(children[1], RuleChild::Box(_)),
        "the second sibling is the nested box, not a rule"
    );
    assert!(
        matches!(children[3], RuleChild::Box(_)),
        "the fourth sibling is the second nested box"
    );
    if let RuleChild::Box(inner) = &children[1] {
        assert_eq!(
            inner.combine(),
            Combine::Any,
            "a box's own combine survives the round-trip"
        );
        assert_eq!(inner.children().len(), 2, "and so do its two Rules");
    }
}

#[test]
fn the_first_save_inserts_the_playlist_row_and_nothing_else_creates_one() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    assert_eq!(row_counts(&store), (0, 0, 0), "a fresh store holds none");

    let id = store
        .save_smart_playlist(None, "First List", &authored_tree())
        .expect("the first save creates the list");
    assert_eq!(
        row_counts(&store),
        (1, 3, 6),
        "one list, its Root Box and two nested boxes, six Rules — and the row appears \
         only because a save wrote it: this port offers no other create"
    );
    assert!(!id.0.is_empty(), "the store generates the id it hands back");

    // A second save of the same draft changes nothing about the row set.
    store
        .save_smart_playlist(Some(&id), "First List", &authored_tree())
        .expect("re-saving the same tree is legal");
    assert_eq!(
        row_counts(&store),
        (1, 3, 6),
        "no save can leave a second copy of the tree behind"
    );
}

#[test]
fn a_second_save_persists_the_name_and_replaces_the_tree_wholesale() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Original Name", &authored_tree())
        .expect("the first save");
    let before = playlist_row(&store, &id);

    // A different tree: one rule, one box. Every row of the first tree is gone.
    let changed = tree(vec![nested(
        Combine::Any,
        vec![rule(
            MatchableField::Album,
            Comparison::Contains,
            Some(RuleValue::Text("Astral".to_string())),
        )],
    )]);
    let again = store
        .save_smart_playlist(Some(&id), "  Renamed In One Save  ", &changed)
        .expect("one Save carries the listener's name and their tree together");

    let after = playlist_row(&store, &again);
    assert_eq!(after.0, before.0, "the id survives every save");
    assert_eq!(
        after.1, "Renamed In One Save",
        "the save writes the trimmed name: the editor's name field and its tree are \
         one listener action, so they land in one transaction or neither does"
    );
    assert_eq!(
        after.2, before.2,
        "created_at survives every save, so a re-save is not a new list"
    );
    assert_eq!(
        row_counts(&store),
        (1, 2, 1),
        "one list row and the new tree — the name write did not delete and re-insert \
         the row, and no row of the old tree is left"
    );
    let now = tree_row_ids(&store);
    assert_eq!(now.0.len(), 1, "one Rule row is left, the new tree's own");
    assert_eq!(
        now.1.len(),
        2,
        "and two boxes: the Root Box and the draft's one box"
    );
    assert!(
        !tree_row_fields(&store).contains(&"genre".to_string()),
        "no Rule of the previous tree survives the wholesale replace — the \
         replaced-by-`album` rows are gone, not merely re-numbered"
    );
    let read = store
        .smart_playlist_tree(&again)
        .expect("read works")
        .expect("the list exists");
    assert_eq!(read, changed, "the replaced tree reads back as the draft");
}

/// The `smart_playlists` row as stored: `(id, name, created_at)`.
fn playlist_row(store: &SqliteStore, id: &PlaylistId) -> (String, String, i64) {
    store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT id, name, created_at FROM smart_playlists WHERE id = ?1",
                [&id.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
        })
        .expect("reading the row back must work")
}

#[test]
fn a_saved_tree_survives_reopen() {
    let (dir, db_path, mut store, _changes) = fresh_store();
    let authored = authored_tree();
    let id = store
        .save_smart_playlist(None, "Reopen Me", &authored)
        .expect("the first save");
    drop(store);

    let (reopen_tx, _reopen_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let reopened = SqliteStore::open_and_migrate(&db_path, reopen_tx).expect("reopening works");
    let read = reopened
        .smart_playlist_tree(&id)
        .expect("reading after a reopen must work")
        .expect("the list survived the restart");
    assert_eq!(
        read, authored,
        "the whole tree survived a restart unchanged"
    );
    assert_eq!(
        reopened
            .smart_playlist_tree(&PlaylistId("spl-never-existed".to_string()))
            .expect("an unknown id is not an error"),
        None,
        "and a list that was never saved reads as no row"
    );
    drop(dir);
}

// --- the value column -------------------------------------------------------

/// Every value shape the storage rules name, as `(what, the Rule, the TEXT it is
/// stored as)`. Table-driven because the spellings are one contract with nine
/// cases, and a per-case test would restate the harness nine times.
fn spelling_cases() -> Vec<(&'static str, RuleChild, Option<&'static str>)> {
    vec![
        ("year", year_rule(), Some("1975")),
        (
            "duration",
            rule(
                MatchableField::Duration,
                Comparison::LessThan,
                Some(RuleValue::DurationMillis(4_500_000)),
            ),
            Some("4500000"),
        ),
        (
            "in_last",
            rule(
                MatchableField::DateAdded,
                Comparison::InLast,
                Some(RuleValue::Days(90)),
            ),
            Some("90"),
        ),
        (
            "before",
            rule(
                MatchableField::LastPlayed,
                Comparison::Before,
                Some(RuleValue::DateTime(riff_persistence::rule::CivilDateTime {
                    year: 2024,
                    month: 3,
                    day: 7,
                    hour: 8,
                    minute: 9,
                    second: 10,
                })),
            ),
            Some("2024-03-07 08:09:10"),
        ),
        (
            "after",
            rule(
                MatchableField::DateAdded,
                Comparison::After,
                Some(RuleValue::DateTime(riff_persistence::rule::CivilDateTime {
                    year: 1999,
                    month: 12,
                    day: 31,
                    hour: 23,
                    minute: 59,
                    second: 58,
                })),
            ),
            Some("1999-12-31 23:59:58"),
        ),
        (
            "favourite yes",
            rule(
                MatchableField::Favorite,
                Comparison::Is,
                Some(RuleValue::YesNo(true)),
            ),
            Some("1"),
        ),
        (
            "favourite no",
            rule(
                MatchableField::Favorite,
                Comparison::Is,
                Some(RuleValue::YesNo(false)),
            ),
            Some("0"),
        ),
        (
            "text",
            rule(
                MatchableField::Genre,
                Comparison::Is,
                Some(RuleValue::Text("Art Rock;Progressive Rock".to_string())),
            ),
            Some("Art Rock;Progressive Rock"),
        ),
        (
            "sample rate",
            rule(
                MatchableField::SampleRate,
                Comparison::GreaterThan,
                Some(RuleValue::Number(96_000)),
            ),
            Some("96000"),
        ),
    ]
}

#[test]
fn each_value_is_stored_in_the_one_spelling_the_storage_rules_name() {
    for (what, child, expected) in spelling_cases() {
        let (_dir, _path, mut store, _changes) = fresh_store();
        let authored = tree(vec![child.clone()]);
        let id = store
            .save_smart_playlist(None, "Spelling", &authored)
            .unwrap_or_else(|e| panic!("saving a {what} rule must work: {e}"));
        let (value, operator, negated) = stored_rule(&store);
        assert_eq!(
            value.as_deref(),
            expected,
            "{what}: the value column holds {expected:?}"
        );
        assert_eq!(negated, 0, "{what}: nothing here is negated");
        assert!(
            operator != "is_not_missing",
            "{what}: there is no such spelling to store"
        );
        assert_eq!(
            store
                .smart_playlist_tree(&id)
                .expect("read must work")
                .expect("the list exists"),
            authored,
            "{what}: and it reads back as the Rule that was written"
        );
    }
}

#[test]
fn null_is_stored_only_for_is_missing() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let absence = tree(vec![rule(
        MatchableField::Comment,
        Comparison::IsMissing,
        None,
    )]);
    let id = store
        .save_smart_playlist(None, "Untagged", &absence)
        .expect("an absence is a whole Rule");
    let (value, operator, negated) = stored_rule(&store);
    assert_eq!(value, None, "an absence takes no value");
    assert_eq!(operator, "is_missing");
    assert_eq!(negated, 0);

    let nulls = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM smart_rules WHERE value IS NULL AND operator_id <> \
                 'is_missing'",
                [],
                |row| row.get::<_, i64>(0),
            )
        })
        .expect("counting must work");
    assert_eq!(nulls, 0, "NULL is only ever the absence's own spelling");

    assert_eq!(
        store
            .smart_playlist_tree(&id)
            .expect("read must work")
            .expect("the list exists"),
        absence,
        "and the absence round-trips"
    );
}

/// A negated `is missing` — what the editor words as "is not missing" — is stored
/// as `is_missing` with the negation flag, and that is the only spelling.
#[test]
fn is_not_missing_is_stored_as_is_missing_with_the_negation_set() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let authored = tree(vec![negated_absence(MatchableField::Comment)]);
    let id = store
        .save_smart_playlist(None, "Tagged", &authored)
        .expect("a negated absence is a whole Rule");

    let (value, operator, negated) = stored_rule(&store);
    assert_eq!(
        operator, "is_missing",
        "one meaning, one spelling: the stored id is `is_missing`"
    );
    assert_eq!(negated, 1, "and the inversion rides on the negation flag");
    assert_eq!(
        value, None,
        "no `is_not_missing` row exists, and none can be written"
    );

    // The round-trip is the proof the editor needs: what comes back is the same
    // Rule it sent, so "has this tree changed?" cannot be fooled by a spelling.
    let read = store
        .smart_playlist_tree(&id)
        .expect("read must work")
        .expect("the list exists");
    assert_eq!(read, authored, "the negated absence round-trips");
    store
        .save_smart_playlist(Some(&id), "Tagged", &read)
        .expect("re-saving what the read handed back is accepted");
    assert_eq!(
        store
            .smart_playlist_tree(&id)
            .expect("read must work")
            .expect("the list exists"),
        read,
        "and it still reads back identically after the re-save"
    );
}

// --- refusals ---------------------------------------------------------------

#[test]
fn a_tree_with_no_rule_is_refused_and_nothing_is_stored() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let reason = refusal(
        store
            .save_smart_playlist(None, "Nothing", &tree(Vec::new()))
            .map(|_| ()),
        "saving a tree with no Rules",
    );
    assert!(
        reason.contains("a list with no rules is never stored"),
        "the refusal says why: {reason}"
    );
    assert_eq!(
        row_counts(&store),
        (0, 0, 0),
        "a refused save writes no row, not even the list's own"
    );
}

#[test]
fn any_box_with_no_children_is_refused_the_root_box_included() {
    let (_dir, _path, mut store, _changes) = fresh_store();

    // The Root Box itself, in the other combine, holding nothing.
    let empty_root = SmartPlaylistTree::new(RootBox::new(RuleBox::new(Combine::Any)));
    let reason = refusal(
        store
            .save_smart_playlist(None, "Empty Root", &empty_root)
            .map(|_| ()),
        "saving an empty Root Box",
    );
    assert!(
        reason.contains("a list with no rules is never stored"),
        "an empty Root Box is refused, and says so: {reason}"
    );
    assert_eq!(row_counts(&store), (0, 0, 0), "and writes nothing");

    // A box buried inside an otherwise whole tree.
    let seeded = store
        .save_smart_playlist(None, "Seeded", &tree(vec![year_rule()]))
        .expect("a whole tree saves");
    let buried = tree(vec![year_rule(), nested(Combine::All, Vec::new())]);
    let reason = refusal(
        store
            .save_smart_playlist(Some(&seeded), "Seeded", &buried)
            .map(|_| ()),
        "saving a tree that hides an empty box",
    );
    assert!(
        reason.contains("a rule box with nothing in it"),
        "an empty nested box is refused too: {reason}"
    );
    assert_eq!(
        row_counts(&store),
        (1, 1, 1),
        "the stored tree is exactly what the committing save left"
    );
}

#[test]
fn a_half_filled_rule_is_refused_naming_the_field_it_is_about() {
    let (_dir, _path, mut store, _changes) = fresh_store();

    // `year` takes a value; the editor's draft row left it blank.
    let half = tree(vec![rule(MatchableField::Year, Comparison::EqualTo, None)]);
    let reason = refusal(
        store
            .save_smart_playlist(None, "Half Rule", &half)
            .map(|_| ()),
        "saving a draft row with no value",
    );
    assert!(
        reason.contains("the rule on Year is not fully filled in"),
        "the refusal names the offending row: {reason}"
    );
    assert_eq!(row_counts(&store), (0, 0, 0), "and writes nothing");

    // The other half-filled shape: a value where the field draws none.
    let over = tree(vec![rule(
        MatchableField::Artist,
        Comparison::IsMissing,
        Some(RuleValue::Text("no value belongs here".to_string())),
    )]);
    let reason = refusal(
        store.save_smart_playlist(None, "Over", &over).map(|_| ()),
        "saving a value where none belongs",
    );
    assert!(reason.contains("the rule on Artist"), "{reason}");

    // And a comparison the field does not permit at all.
    let illegal = tree(vec![rule(
        MatchableField::PlayCount,
        Comparison::Contains,
        Some(RuleValue::Text("3".to_string())),
    )]);
    let reason = refusal(
        store
            .save_smart_playlist(None, "Illegal", &illegal)
            .map(|_| ()),
        "saving a comparison the field forbids",
    );
    assert!(reason.contains("the rule on Plays"), "{reason}");
}

/// The refusal string tickets 05, 18 and 21 render, pinned exactly: a refusal
/// raised inside a transaction arrives double-prefixed, which is the store's
/// pre-existing shape for every in-transaction refusal and for a `CHECK`
/// violation alike — matched here, not fought.
#[test]
fn a_refusal_surfaces_as_the_one_error_variant_double_prefixed() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let outcome = store.save_smart_playlist(
        None,
        "Half",
        &tree(vec![rule(MatchableField::Year, Comparison::EqualTo, None)]),
    );
    let Err(StoreError::InvalidOperation(reason)) = outcome else {
        panic!("a refusal is one `InvalidOperation`, got {outcome:?}");
    };
    assert_eq!(
        reason,
        "failed to save Custom Smart Playlist: Invalid operation: store query failed: this \
         list cannot be saved: the rule on Year is not fully filled in",
        "the string inside the variant, with the transaction's own double prefix"
    );
    let shown = StoreError::InvalidOperation(reason.clone()).to_string();
    assert_eq!(
        shown,
        format!("Invalid operation: {reason}"),
        "and `Display` adds one prefix in front of it — what a listener reads"
    );
}

#[test]
fn saving_against_a_list_that_was_deleted_under_the_draft_is_refused() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Brief", &tree(vec![year_rule()]))
        .expect("the save");
    store.delete_smart_playlist(&id).expect("the delete");

    let reason = refusal(
        store
            .save_smart_playlist(Some(&id), "Brief", &authored_tree())
            .map(|_| ()),
        "saving a draft whose list no longer exists",
    );
    assert!(
        reason.contains("no longer exists"),
        "a stale draft is refused rather than silently creating a second row: {reason}"
    );
    assert_eq!(row_counts(&store), (0, 0, 0), "and writes nothing");
}

// --- the Unreadable Rule ----------------------------------------------------

/// Put one Rule row in that this build cannot read.
///
/// A `field_id` outside the catalog is the case the ticket names, and this build's
/// `CHECK` list holds exactly the ids this build knows — so no path this build
/// offers can write the row. That is the fixture's point: the row arrives from
/// *outside*, from a newer riff whose schema knows a field this one does not, and
/// a `CHECK` constrains writes, never reads.
/// `PRAGMA ignore_check_constraints` is how a test stands in for that outside
/// writer; nothing in production sets it.
fn seed_unreadable_rule(store: &SqliteStore, playlist_id: &str, field_id: &str) {
    store
        .with_connection(|conn| {
            let box_id: String = conn.query_row(
                "SELECT id FROM smart_rule_boxes WHERE playlist_id = ?1 AND parent_id IS NULL",
                [playlist_id],
                |row| row.get(0),
            )?;
            conn.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
            conn.execute(
                "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, \
                 position) VALUES (?1, ?2, ?3, 'contains', 0, 'ambient', 99)",
                rusqlite::params![format!("{playlist_id}-rule-outside"), box_id, field_id],
            )?;
            conn.execute_batch("PRAGMA ignore_check_constraints = OFF;")?;
            Ok(())
        })
        .expect("seeding a row this build cannot read must work");
}

#[test]
fn an_unreadable_rule_reads_back_beside_the_tree_and_the_rest_of_it_is_intact() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Mixed", &authored_tree())
        .expect("the save");
    seed_unreadable_rule(&store, &id.0, "mood");

    let read = store
        .smart_playlist_tree(&id)
        .expect("reading a tree with a row it cannot read must not fail")
        .expect("the list exists");

    // The row this build cannot read never reaches the typed tree; the rows it can
    // read are all still there, in order.
    assert_eq!(
        read.root(),
        authored_tree().root(),
        "every interpretable Rule and Box is intact and in order"
    );
    assert_eq!(read.unreadable().len(), 1, "one row is reported raw");
    let raw: &UnreadableRule = &read.unreadable()[0];
    assert_eq!(
        raw.field_id(),
        "mood",
        "the raw field id rides beside the tree"
    );
    assert_eq!(
        raw.operator_id(),
        "contains",
        "and so does the raw operator id"
    );
    assert_eq!(
        raw.value(),
        Some("ambient"),
        "and the value stays as stored"
    );
    assert_eq!(
        raw.position(),
        99,
        "its stored sibling position is reported"
    );
    assert!(!raw.is_negated(), "and so is its negation flag");
    assert!(
        raw.box_path().is_empty(),
        "it hangs in the Root Box, which the address says by being empty"
    );

    // A copy cannot re-insert a field id this build does not know — the `CHECK`
    // list holds exactly the ids it enumerates — so Duplicate refuses naming that
    // Rule rather than dying on the constraint with nothing to say.
    let reason = refusal(
        store.duplicate_smart_playlist(&id).map(|_| ()),
        "duplicating a list with an unknown field id",
    );
    assert!(
        reason.contains("a rule testing \"mood\"")
            && reason.contains("delete that rule before duplicating"),
        "the copy's refusal names the Rule too: {reason}"
    );
    assert_eq!(
        row_counts(&store),
        (1, 3, 7),
        "and a refused Duplicate writes nothing"
    );
}

#[test]
fn a_rule_in_a_nested_box_reports_the_box_it_hangs_in() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    // Root Box: rule, box, rule — so the nested box is box child 0 of the root.
    let id = store
        .save_smart_playlist(
            None,
            "Nested",
            &tree(vec![
                year_rule(),
                nested(Combine::Any, vec![year_rule()]),
                year_rule(),
            ]),
        )
        .expect("the save");
    // The store's own box id for the draft's nested box, read rather than guessed:
    // ids are opaque and generated here, so a test must not depend on their shape.
    let nested_box_id: String = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT b.id FROM smart_rule_boxes b JOIN smart_rule_boxes p ON p.id = b.parent_id \
                 WHERE b.playlist_id = ?1 AND p.parent_id IS NULL",
                [&id.0],
                |row| row.get(0),
            )
        })
        .expect("finding the nested box must work");
    store
        .with_connection(|conn| {
            conn.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
            conn.execute(
                "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, \
                 position) VALUES ('outside-2', ?1, 'bpm', 'greater_than', 1, '120', 0)",
                [&nested_box_id],
            )?;
            conn.execute_batch("PRAGMA ignore_check_constraints = OFF;")?;
            Ok(())
        })
        .expect("seeding a Rule inside the nested box must work");

    let read = store
        .smart_playlist_tree(&id)
        .expect("read must work")
        .expect("the list exists");
    assert_eq!(read.unreadable().len(), 1);
    assert_eq!(
        read.unreadable()[0].box_path(),
        [0],
        "the address says: the Root Box's first box child"
    );
    assert_eq!(
        read.unreadable()[0].position(),
        0,
        "and where in it it sits"
    );
    assert!(
        read.unreadable()[0].is_negated(),
        "its negation is reported as stored"
    );
    assert_eq!(
        read.root().rule_count(),
        3,
        "the typed tree holds only the three Rules this build can read"
    );
}

#[test]
fn saving_a_draft_that_still_holds_the_unreadable_rule_is_refused_naming_it() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Mixed", &authored_tree())
        .expect("the save");
    seed_unreadable_rule(&store, &id.0, "mood");
    let read = store
        .smart_playlist_tree(&id)
        .expect("read must work")
        .expect("the list exists");

    let reason = refusal(
        store
            .save_smart_playlist(Some(&id), "Mixed", &read)
            .map(|_| ()),
        "re-saving a tree with an Unreadable Rule in it",
    );
    assert!(
        reason.contains("a rule testing \"mood\" with \"contains\"")
            && reason.contains("delete that rule before saving"),
        "the refusal names that Rule by its own stored ids: {reason}"
    );
    assert_eq!(
        row_counts(&store),
        (1, 3, 7),
        "a refused save changes nothing at all, the unreadable row included"
    );

    // The way out is the row's own delete: the draft without it saves, and the
    // wholesale replace takes the unreadable row with the rest of the old tree.
    let cleaned = tree(authored_tree().root().children().to_vec());
    store
        .save_smart_playlist(Some(&id), "Mixed", &cleaned)
        .expect("the same save succeeds once the row is removed from the draft");
    let after = store
        .smart_playlist_tree(&id)
        .expect("read must work")
        .expect("the list exists");
    assert!(
        after.unreadable().is_empty(),
        "the row went with the tree it hung in — never repaired, never silently dropped"
    );
    assert_eq!(after, cleaned);
    assert_eq!(
        row_counts(&store),
        (1, 3, 6),
        "and the store holds no trace of it"
    );
}

/// A row whose ids this build knows but whose value it cannot re-type for the
/// field is Unreadable too — and needs no pragma to exist, because no `CHECK`
/// constrains the `value` column.
#[test]
fn a_value_this_build_cannot_re_read_for_its_field_is_unreadable_not_guessed() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let id = store
        .save_smart_playlist(None, "MisTyped", &authored_tree())
        .expect("the save");
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE smart_rules SET value = 'not a year' WHERE field_id = 'year'",
                [],
            )
            .map(|_| ())
        })
        .expect("mis-typing a stored value must work");

    let read = store
        .smart_playlist_tree(&id)
        .expect("a bad value is not a read failure")
        .expect("the list exists");
    assert_eq!(read.unreadable().len(), 1, "the row is reported raw");
    assert_eq!(read.unreadable()[0].field_id(), "year");
    assert_eq!(
        read.unreadable()[0].value(),
        Some("not a year"),
        "as stored, never as a guess"
    );
    assert_eq!(
        read.root().rule_count(),
        5,
        "and it is simply absent from the tree this build can interpret"
    );
    refusal(
        store
            .save_smart_playlist(Some(&id), "MisTyped", &read)
            .map(|_| ()),
        "saving a mis-typed row",
    );

    // A value is not covered by a `CHECK`, so the copy carries it verbatim: the
    // Duplicate of a mis-typed list is as unreadable as its original, and nothing
    // on the way through is repaired.
    let copy = store
        .duplicate_smart_playlist(&id)
        .expect("a mis-typed value copies raw")
        .expect("the list exists to copy");
    let copied = store
        .smart_playlist_tree(&copy)
        .expect("reading the copy must work")
        .expect("the copy exists");
    assert_eq!(
        copied.unreadable(),
        read.unreadable(),
        "the copy keeps the row exactly as stored: never repaired, never dropped"
    );

    // A value stored where the field draws none is the same shape of fault.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE smart_rules SET value = 'x' WHERE operator_id = 'is_missing'",
                [],
            )
            .map(|_| ())
        })
        .expect("storing a value beside an absence must work");
    let read = store
        .smart_playlist_tree(&id)
        .expect("read must work")
        .expect("the list exists");
    assert_eq!(
        read.unreadable().len(),
        2,
        "and that row is unreadable too, beside the tree"
    );
}

// --- rename -----------------------------------------------------------------

#[test]
fn a_rename_refuses_a_name_any_sidebar_row_holds_and_accepts_a_free_one() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    store
        .create_playlist("Night Drive", &[])
        .expect("a Playlist row holds the name");
    let id = store
        .save_smart_playlist(None, "My List", &authored_tree())
        .expect("the save");

    let reason = refusal(
        store.rename_smart_playlist(&id, "night drive").map(|_| ()),
        "renaming onto a name a Playlist holds",
    );
    assert!(
        reason.contains("already exists"),
        "the rule is the cross-table, case-folded one: {reason}"
    );
    assert_eq!(
        playlist_row(&store, &id).1,
        "My List",
        "a refused rename changes nothing"
    );

    let other = store
        .save_smart_playlist(None, "Second List", &authored_tree())
        .expect("the second list");
    let reason = refusal(
        store.rename_smart_playlist(&other, "My List").map(|_| ()),
        "renaming onto a name another Custom Smart Playlist holds",
    );
    assert!(reason.contains("already exists"), "{reason}");

    assert!(
        store
            .rename_smart_playlist(&id, "Renamed List")
            .expect("a free name is accepted"),
        "the rename found the row"
    );
    assert_eq!(playlist_row(&store, &id).1, "Renamed List");
    assert!(
        !store
            .rename_smart_playlist(&PlaylistId("spl-nobody".to_string()), "Whatever")
            .expect("renaming a list that is not there is not an error"),
        "an unknown id renames nothing"
    );
}

#[test]
fn re_saving_or_renaming_a_list_under_its_own_name_never_refuses_it_against_itself() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Own Name", &authored_tree())
        .expect("the save");
    let other = store
        .save_smart_playlist(None, "Different", &authored_tree())
        .expect("the second list");

    // The row being rewritten is exempt — ticket 03's rule, used by both paths.
    assert!(
        store
            .rename_smart_playlist(&id, "Own Name")
            .expect("a rename that changes nothing"),
        "a list never conflicts with itself"
    );
    assert!(
        store
            .rename_smart_playlist(&id, "OWN NAME")
            .expect("the same folded name in another case"),
        "and not when the case differs either"
    );
    store
        .save_smart_playlist(Some(&id), "OWN NAME", &tree(vec![year_rule()]))
        .expect("a re-save under the list's own name is never a conflict with itself");
    store
        .save_smart_playlist(Some(&id), " own name ", &authored_tree())
        .expect(
            "nor is one under its own name padded and in lower case — the exemption \
                 compares trimmed and folded, the same way the rule itself does",
        );
    assert_eq!(
        playlist_row(&store, &id).1,
        "own name",
        "a save writes the name trimmed, so the row and the exemption agree"
    );
    let reason = refusal(
        store.rename_smart_playlist(&other, "own name").map(|_| ()),
        "renaming a second list onto the first's name",
    );
    assert!(reason.contains("already exists"), "{reason}");
    let reason = refusal(
        store
            .save_smart_playlist(Some(&other), " OWN NAME ", &authored_tree())
            .map(|_| ()),
        "saving a second list onto the first's name",
    );
    assert!(reason.contains("already exists"), "{reason}");
    assert_eq!(
        playlist_row(&store, &other).1,
        "Different",
        "and that refusal wrote nothing at all"
    );
}

#[test]
fn a_new_list_under_a_name_a_sidebar_row_holds_is_refused_at_the_save() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    store
        .save_smart_playlist(None, "Taken", &authored_tree())
        .expect("the first list");
    let reason = refusal(
        store
            .save_smart_playlist(None, " taken ", &authored_tree())
            .map(|_| ()),
        "creating under a held name in another case and padding",
    );
    assert!(
        reason.contains("named \"taken\" already exists"),
        "the candidate is compared trimmed and folded: {reason}"
    );
    assert_eq!(
        row_counts(&store),
        (1, 3, 6),
        "a refused creation writes no row"
    );
}

/// The refused half of the one-Save guarantee, proved by **row content**.
///
/// Two ways a split save could half-apply, and this test is what pins them shut:
/// a name the check refuses must leave the tree exactly where it was, and a tree
/// it refuses must leave the name exactly where it was. Neither direction is
/// asserted against the refusal's wording — a message only says what the UI
/// believes it did.
#[test]
fn a_save_onto_a_name_another_sidebar_row_holds_writes_neither_the_name_nor_the_tree() {
    let (_dir, _path, mut store, changes_rx) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Mine", &authored_tree())
        .expect("the list being saved");
    let squatter = store
        .save_smart_playlist(None, "Other List", &tree(vec![year_rule()]))
        .expect("another Custom Smart Playlist holds a name");
    store
        .create_playlist("Night Drive", &[])
        .expect("and so does a Playlist");
    drained(&changes_rx);

    let name_before = playlist_row(&store, &id).1;
    let rows_before = stored_rule_rows(&store);
    let ids_before = tree_row_ids(&store);
    let counts_before = row_counts(&store);

    // Both tables, in another case and padded, so this is ticket 03's folded
    // cross-table rule running inside the save and not a string coincidence.
    let draft = tree(vec![negated_absence(MatchableField::Artist)]);
    for taken in ["NIGHT DRIVE", "other LIST"] {
        refusal(
            store
                .save_smart_playlist(Some(&id), taken, &draft)
                .map(|_| ()),
            "saving an existing list onto a name another sidebar row holds",
        );
        assert_eq!(
            stored_rule_rows(&store),
            rows_before,
            "the refused Save named {taken:?} left every Rule row it could have \
             overwritten exactly as stored: field, operator, negation, value, position"
        );
        assert_eq!(
            tree_row_ids(&store),
            ids_before,
            "and not one box or Rule row was deleted and re-inserted"
        );
        assert_eq!(
            playlist_row(&store, &id).1,
            name_before,
            "and the name the same call carried is untouched too"
        );
    }

    assert_eq!(
        row_counts(&store),
        counts_before,
        "(lists, boxes, rules) unchanged — nothing added, nothing dropped"
    );
    assert_eq!(
        store
            .smart_playlist_tree(&id)
            .expect("read must work")
            .expect("the list exists"),
        authored_tree(),
        "and the port read still returns the tree that was there before the refused Save"
    );
    assert_eq!(playlist_row(&store, &squatter).1, "Other List");
    assert!(
        drained(&changes_rx).is_empty(),
        "a refused Save commits nothing, so it bumps nothing and emits nothing"
    );
}

#[test]
fn a_save_refused_by_its_tree_writes_no_name_either() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Still Mine", &authored_tree())
        .expect("the save");

    // Both names below are free, so the name check passes and the tree is what is
    // refused. A save that wrote the name first — or in its own transaction —
    // would rename the list here and still report failure.
    refusal(
        store
            .save_smart_playlist(Some(&id), "Perfectly Free", &tree(Vec::new()))
            .map(|_| ()),
        "a save whose tree holds no Rule",
    );
    refusal(
        store
            .save_smart_playlist(
                Some(&id),
                "Also Free",
                &tree(vec![rule(MatchableField::Year, Comparison::EqualTo, None)]),
            )
            .map(|_| ()),
        "a save whose Rule is half-filled",
    );

    assert_eq!(
        playlist_row(&store, &id).1,
        "Still Mine",
        "name and tree land together or not at all, so a tree refusal renames nothing"
    );
    assert_eq!(
        store
            .smart_playlist_tree(&id)
            .expect("read must work")
            .expect("the list exists"),
        authored_tree(),
        "and left the tree alone"
    );
}

// --- delete -----------------------------------------------------------------

#[test]
fn deleting_a_list_removes_every_box_every_descendant_box_and_every_rule() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let kept = store
        .save_smart_playlist(None, "Neighbour", &authored_tree())
        .expect("a second list, whose tree must survive");
    let doomed = store
        .save_smart_playlist(None, "Doomed", &authored_tree())
        .expect("the list to delete");
    assert_eq!(
        row_counts(&store),
        (2, 6, 12),
        "two lists, six boxes, twelve Rules"
    );

    assert!(
        store
            .delete_smart_playlist(&doomed)
            .expect("the delete commits"),
        "the delete found the row"
    );

    // Asserted by counting rows, never by trusting the cascade.
    assert_eq!(
        row_counts(&store),
        (1, 3, 6),
        "one list's rows are all gone"
    );
    assert_eq!(
        store
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM smart_rule_boxes WHERE playlist_id = ?1",
                    [&doomed.0],
                    |row| row.get::<_, i64>(0),
                )
            })
            .expect("counting must work"),
        0,
        "not one box — Root Box or descendant — is left for the deleted list"
    );
    assert_eq!(
        store
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM smart_rules WHERE box_id IN (SELECT id FROM \
                     smart_rule_boxes WHERE playlist_id = ?1)",
                    [&doomed.0],
                    |row| row.get::<_, i64>(0),
                )
            })
            .expect("counting must work"),
        0,
        "and not one Rule"
    );
    assert_eq!(
        store
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM smart_rule_boxes WHERE parent_id IS NOT NULL AND \
                     parent_id NOT IN (SELECT id FROM smart_rule_boxes)",
                    [],
                    |row| row.get::<_, i64>(0),
                )
            })
            .expect("orphan probe must work"),
        0,
        "nothing anywhere is orphaned by the delete"
    );
    assert!(
        store
            .smart_playlist_tree(&doomed)
            .expect("reading a deleted list is not an error")
            .is_none(),
        "and the deleted list reads as no row"
    );
    assert_eq!(
        store
            .smart_playlist_tree(&kept)
            .expect("read must work")
            .expect("the neighbour survives"),
        authored_tree(),
        "the other list's tree is untouched"
    );
    assert!(
        !store
            .delete_smart_playlist(&PlaylistId("spl-nobody".to_string()))
            .expect("deleting a list that is not there is not an error"),
        "an unknown id deletes nothing"
    );
}

#[test]
fn deleting_a_custom_list_touches_no_playlist() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let track = fixture_track("/music/one.mp3");
    store
        .apply_scan_batch(std::slice::from_ref(&track))
        .expect("seeding a track");
    let playlist = store
        .create_playlist("A Playlist", std::slice::from_ref(&track.id))
        .expect("a Playlist row");
    let id = store
        .save_smart_playlist(None, "Custom", &authored_tree())
        .expect("the save");

    store.delete_smart_playlist(&id).expect("the delete");

    let playlists = store.load_playlists().expect("playlists load");
    assert_eq!(
        playlists.len(),
        1,
        "the Playlist row survived the custom list's delete"
    );
    assert_eq!(playlists[0].id, playlist);
    assert_eq!(
        store
            .load_playlist_entries(&playlist)
            .expect("entries load")
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        vec![track.id],
        "and so did its entries"
    );
}

// --- Duplicate --------------------------------------------------------------

#[test]
fn duplicating_copies_the_tree_under_copy_of_x_with_fresh_ids_and_no_shared_rule() {
    let (_dir, _path, mut store, _changes) = fresh_store();
    let original = store
        .save_smart_playlist(None, "Deep Work", &authored_tree())
        .expect("the save");

    let copy = store
        .duplicate_smart_playlist(&original)
        .expect("the Duplicate is one store operation")
        .expect("the list exists to copy");

    // Names: chosen against what exists at that instant.
    assert_eq!(
        smart_names(&store),
        vec!["Copy of Deep Work".to_string(), "Deep Work".to_string(),],
        "the copy is named `Copy of X`"
    );
    assert_ne!(copy, original, "and its id is the store's own making");

    let copied = store
        .smart_playlist_tree(&copy)
        .expect("read must work")
        .expect("the copy exists");
    assert_eq!(
        copied,
        authored_tree(),
        "same Rules, same Boxes, same order"
    );

    assert_eq!(
        row_counts(&store),
        (2, 6, 12),
        "the copy is its own rows, not a reference"
    );
    let shared = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM smart_rules WHERE box_id IN (SELECT id FROM \
                 smart_rule_boxes WHERE playlist_id = ?1) AND box_id IN (SELECT id FROM \
                 smart_rule_boxes WHERE playlist_id = ?2)",
                rusqlite::params![original.0, copy.0],
                |row| row.get::<_, i64>(0),
            )
        })
        .expect("counting must work");
    assert_eq!(shared, 0, "no Rule row belongs to both lists");

    // Duplicating the same list again takes the numbered name, because
    // `Copy of X` is held by now — `Copy of X 2`, which is the case the story
    // names, and it is decided inside the same transaction as the copy.
    let second = store
        .duplicate_smart_playlist(&original)
        .expect("the second Duplicate")
        .expect("the list exists to copy");
    assert_eq!(
        smart_names(&store),
        vec![
            "Copy of Deep Work".to_string(),
            "Copy of Deep Work 2".to_string(),
            "Deep Work".to_string(),
        ],
        "the second copy of `Deep Work` is `Copy of Deep Work 2`"
    );
    assert_eq!(
        store
            .smart_playlist_tree(&second)
            .expect("read must work")
            .expect("the second copy exists"),
        authored_tree()
    );

    // The numbered name is stepped past when a *Playlist* holds it too: the rule
    // is sidebar-wide, so Duplicate settles on a free name rather than refusing.
    store
        .create_playlist("Copy of Deep Work 3", &[])
        .expect("a Playlist squats on the next candidate name");
    let third = store
        .duplicate_smart_playlist(&original)
        .expect("Duplicate still settles on a free name")
        .expect("the list exists to copy");
    assert!(
        smart_names(&store).contains(&"Copy of Deep Work 4".to_string()),
        "the copy's name is chosen against the names that exist at that instant"
    );
    assert_eq!(
        store
            .smart_playlist_tree(&third)
            .expect("read must work")
            .expect("the third copy exists"),
        authored_tree()
    );

    // A copy of a copy is named for the list it was actually made from.
    let of_copy = store
        .duplicate_smart_playlist(&copy)
        .expect("a copy of a copy")
        .expect("the copy exists");
    assert!(
        smart_names(&store).contains(&"Copy of Copy of Deep Work".to_string()),
        "{of_copy:?}"
    );
}

#[test]
fn duplicating_a_list_that_does_not_exist_writes_and_bumps_nothing() {
    let (_dir, _path, mut store, changes_rx) = fresh_store();
    let generation = store.playlist_generation();
    assert_eq!(
        store
            .duplicate_smart_playlist(&PlaylistId("spl-nobody".to_string()))
            .expect("an unknown id is not an error"),
        None,
        "there is nothing to copy"
    );
    assert_eq!(generation.current(), 0, "and nothing committed to bump");
    assert!(
        drained(&changes_rx).is_empty(),
        "a mutation that fails to commit emits nothing"
    );
    assert_eq!(row_counts(&store), (0, 0, 0));
}

// --- generations and the emit-beside-bump invariant -------------------------

#[test]
fn every_custom_list_write_bumps_the_playlist_generation_once_and_no_library_move() {
    let (_dir, _path, mut store, changes_rx) = fresh_store();
    let playlists = store.playlist_generation();
    let library = store.library_generation();

    let id = store
        .save_smart_playlist(None, "Gen", &authored_tree())
        .expect("save");
    assert_eq!(playlists.current(), 1, "a save bumps");
    store
        .save_smart_playlist(Some(&id), "Gen", &tree(vec![year_rule()]))
        .expect("a re-save");
    assert_eq!(
        playlists.current(),
        2,
        "a re-save bumps once, not once per row it re-inserted"
    );
    assert!(
        store
            .rename_smart_playlist(&id, "Renamed")
            .expect("a rename commits"),
        "the rename found the row"
    );
    assert_eq!(playlists.current(), 3, "a rename bumps");
    let copy = store
        .duplicate_smart_playlist(&id)
        .expect("a duplicate commits")
        .expect("the list exists");
    assert_eq!(playlists.current(), 4, "a Duplicate bumps once");
    assert!(
        store
            .delete_smart_playlist(&copy)
            .expect("a delete commits"),
        "it deleted"
    );
    assert_eq!(playlists.current(), 5, "a delete bumps");
    assert_eq!(
        library.current(),
        0,
        "none of these writes moves the Library generation: which Tracks match is \
         a question about Tracks, not about a list's tree"
    );

    assert_eq!(
        drained(&changes_rx),
        vec![
            StoreChanged::Playlists(1),
            StoreChanged::Playlists(2),
            StoreChanged::Playlists(3),
            StoreChanged::Playlists(4),
            StoreChanged::Playlists(5),
        ],
        "exactly one notice beside each bump, in commit order"
    );
}

#[test]
fn a_refused_write_bumps_nothing_and_emits_nothing() {
    let (_dir, _path, mut store, changes_rx) = fresh_store();
    let playlists = store.playlist_generation();
    let library = store.library_generation();
    let id = store
        .save_smart_playlist(None, "Whole", &authored_tree())
        .expect("the first save commits");
    assert_eq!(
        drained(&changes_rx),
        vec![StoreChanged::Playlists(1)],
        "the committing save emitted one notice"
    );

    // Refused on every shape the save can say no to.
    refusal(
        store
            .save_smart_playlist(Some(&id), "Whole", &tree(Vec::new()))
            .map(|_| ()),
        "an empty tree",
    );
    refusal(
        store
            .save_smart_playlist(
                Some(&id),
                "Whole",
                &tree(vec![rule(MatchableField::Year, Comparison::EqualTo, None)]),
            )
            .map(|_| ()),
        "a half-filled Rule",
    );
    refusal(
        store
            .save_smart_playlist(None, "Whole", &authored_tree())
            .map(|_| ()),
        "a held name",
    );
    seed_unreadable_rule(&store, &id.0, "bpm");
    let read = store
        .smart_playlist_tree(&id)
        .expect("read must work")
        .expect("the list exists");
    refusal(
        store
            .save_smart_playlist(Some(&id), "Whole", &read)
            .map(|_| ()),
        "an Unreadable Rule",
    );
    refusal(
        store
            .rename_smart_playlist(&PlaylistId("spl-other".to_string()), "Whole")
            .map(|_| ()),
        "a rename onto a held name",
    );

    assert_eq!(
        playlists.current(),
        1,
        "no refusal committed, so no refusal bumped"
    );
    assert_eq!(
        library.current(),
        0,
        "and none moved the Library generation"
    );
    assert!(drained(&changes_rx).is_empty(), "and none emitted a notice");
    assert_eq!(
        store
            .smart_playlist_tree(&id)
            .expect("read must work")
            .expect("the list exists")
            .root()
            .rule_count(),
        6,
        "the seeded unreadable row is not part of the interpretable tree"
    );
    assert_eq!(
        row_counts(&store),
        (1, 3, 7),
        "and the unreadable row is still there, unrepaired: nothing committed"
    );
}

#[test]
fn a_rename_of_a_deleted_list_commits_a_no_op_and_bumps_nothing() {
    let (_dir, _path, mut store, changes_rx) = fresh_store();
    let id = store
        .save_smart_playlist(None, "Brief", &authored_tree())
        .expect("the save");
    store.delete_smart_playlist(&id).expect("the delete");
    drained(&changes_rx);

    let playlists = store.playlist_generation();
    assert!(
        !store
            .rename_smart_playlist(&id, "After The Fact")
            .expect("renaming a gone list is not an error"),
        "there is no row to rename"
    );
    assert_eq!(
        playlists.current(),
        2,
        "one save and one delete bumped; the no-op adds neither"
    );
    assert!(
        drained(&changes_rx).is_empty(),
        "and emits nothing beside that no bump"
    );
}
