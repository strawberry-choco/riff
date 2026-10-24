// riff-infra — migration 020: the Custom Smart Playlist rule tables.
//
// The schema contract the rule-tree storage is built on, asserted against a
// real SQLite file in a fresh temp directory per test (no mock store):
// migration 020 records and creates exactly the three typed tables, adds no
// index, refuses catalog ids it did not enumerate, carries the whole rule
// tree down with the playlist it belongs to, and touches nothing that already
// existed — no backfill, no metadata-version move.
//
// Run: cargo test -p riff-infra --test integration smart_playlist_tables

use super::*;
use riff_persistence::track::METADATA_VERSION;

/// The migration's own version and name, as an independent restatement of
/// what the ticket specifies rather than a read of the shipped list.
const NEW_VERSION: i64 = 20;
const NEW_NAME: &str = "020_smart_playlist_rule_tables";

/// Open — and therefore migrate — a store at `db_path`. Every test starts
/// from its own temp directory, so no store ever shares state with another.
fn open(db_path: &std::path::Path) -> SqliteStore {
    let (changes_tx, _changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    SqliteStore::open_and_migrate(db_path, changes_tx)
        .expect("opening the Application Store must succeed")
}

/// A store in a scratch directory, with the directory guard so the file stays
/// alive for the whole test.
fn fresh_store() -> (tempfile::TempDir, std::path::PathBuf, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let store = open(&db_path);
    (dir, db_path, store)
}

fn query_i64s(store: &SqliteStore, sql: &str) -> Vec<i64> {
    store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            rows.collect()
        })
        .expect("querying the store must work")
}

fn query_strings(store: &SqliteStore, sql: &str) -> Vec<String> {
    store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("querying the store must work")
}

/// `PRAGMA table_info` for one table: each column's name, declared type,
/// `NOT NULL` flag and primary-key position, in declaration order.
fn table_info(store: &SqliteStore, table: &str) -> Vec<(String, String, bool, i64)> {
    store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, i64>(5)?,
                ))
            })?;
            rows.collect()
        })
        .expect("reading a table's column info must work")
}

/// The fully-populated `schema_migrations` inventory this build must record:
/// every shipped version, in order and with no gaps. Migration 021 (the
/// system-notifications preference) shipped after this ticket's `020`; the
/// inventory here is deliberately derived, not hand-listed, so a later
/// migration does not have to touch this file's history fixture again —
/// only the count-style assertions below move with the shipped set.
fn expected_versions() -> Vec<i64> {
    (1..=21).collect()
}

#[test]
fn test_migration_020_is_recorded_beside_the_shipped_history() {
    let (_dir, _path, store) = fresh_store();

    let versions = query_i64s(
        &store,
        "SELECT version FROM schema_migrations ORDER BY version",
    );
    assert_eq!(
        versions,
        expected_versions(),
        "migration 020 must be appended to the inline migration list, not spliced into it"
    );

    let name: String = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT name FROM schema_migrations WHERE version = ?1",
                rusqlite::params![NEW_VERSION],
                |row| row.get(0),
            )
        })
        .expect("migration 020 must have a bookkeeping row");
    assert_eq!(
        name, NEW_NAME,
        "the recorded migration name must be the file's own versioned name"
    );
}

#[test]
fn test_migration_020_creates_exactly_the_three_rule_tables() {
    let (_dir, _path, store) = fresh_store();

    let tables = query_strings(
        &store,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'smart%' ORDER BY name",
    );
    assert_eq!(
        tables,
        vec!["smart_playlists", "smart_rule_boxes", "smart_rules"],
        "the migration creates three tables and nothing else"
    );

    // smart_playlists: opaque id, unique name, epoch-nanosecond created_at.
    // `PRAGMA table_info` reports the declared `notnull` flag, and the draft
    // spells the id as `TEXT PRIMARY KEY` with no `NOT NULL` — SQLite treats
    // a text primary key as implicitly unique but not implicitly non-null, so
    // the honest expectation here is `false`, not the `true` a reader would
    // guess from the column being a primary key.
    assert_eq!(
        table_info(&store, "smart_playlists"),
        vec![
            ("id".to_string(), "TEXT".to_string(), false, 1),
            ("name".to_string(), "TEXT".to_string(), true, 0),
            ("created_at".to_string(), "INTEGER".to_string(), true, 0),
        ],
        "`smart_playlists` is (id, unique name, created_at as epoch nanoseconds)"
    );

    // smart_rule_boxes: belongs to a playlist, points at a parent box that is
    // nullable so exactly one Root Box can be parentless.
    assert_eq!(
        table_info(&store, "smart_rule_boxes"),
        vec![
            ("id".to_string(), "TEXT".to_string(), false, 1),
            ("playlist_id".to_string(), "TEXT".to_string(), true, 0),
            ("parent_id".to_string(), "TEXT".to_string(), false, 0),
            ("combine".to_string(), "TEXT".to_string(), true, 0),
            ("position".to_string(), "INTEGER".to_string(), true, 0),
        ],
        "`smart_rule_boxes` is (id, playlist_id, nullable parent_id, combine, position)"
    );

    // smart_rules: belongs to a box; `value` is the one nullable column — the
    // draft's "NULL only for `is_missing`" is enforced by the save path.
    assert_eq!(
        table_info(&store, "smart_rules"),
        vec![
            ("id".to_string(), "TEXT".to_string(), false, 1),
            ("box_id".to_string(), "TEXT".to_string(), true, 0),
            ("field_id".to_string(), "TEXT".to_string(), true, 0),
            ("operator_id".to_string(), "TEXT".to_string(), true, 0),
            ("negated".to_string(), "INTEGER".to_string(), true, 0),
            ("value".to_string(), "TEXT".to_string(), false, 0),
            ("position".to_string(), "INTEGER".to_string(), true, 0),
        ],
        "`smart_rules` is (id, box_id, field_id, operator_id, negated, one nullable value, position)"
    );
}

#[test]
fn test_migration_020_adds_no_explicit_index() {
    let (_dir, _path, store) = fresh_store();

    // The only indexes on the three tables are the ones SQLite implies from
    // a primary key or a UNIQUE constraint; a `CREATE INDEX` here would be a
    // deferred-ticket decision sneaking in through the back door.
    let explicit = query_strings(
        &store,
        "SELECT name FROM sqlite_master WHERE type = 'index' \
         AND name NOT LIKE 'sqlite_%' \
         AND tbl_name IN ('smart_playlists', 'smart_rule_boxes', 'smart_rules') \
         ORDER BY name",
    );
    assert!(
        explicit.is_empty(),
        "migration 020 must add no index — every Rule predicate is a rowid scan by design, \
         and indexing is its own deferred ticket. Found: {explicit:?}"
    );
}

/// Insert through the store's own connection, surfacing the database's
/// rejection rather than panicking on it — the constraint tests need to see
/// the `Err` the schema produces.
fn try_insert(store: &SqliteStore, sql: &str, params: &[&dyn rusqlite::ToSql]) -> bool {
    store
        .with_connection(|conn| conn.execute(sql, params))
        .is_ok()
}
/// A playlist, its parentless Root Box, one child box under that root, and one
/// Rule in each box: the smallest tree with a genuine second level, so a
/// cascade claim cannot be satisfied by a one-level chain.
fn seed_two_level_tree(store: &SqliteStore, playlist_id: &str) {
    let root = format!("{playlist_id}-root");
    let child = format!("{playlist_id}-child");
    assert!(
        try_insert(
            store,
            "INSERT INTO smart_playlists (id, name, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                playlist_id,
                format!("list {playlist_id}"),
                1_700_000_000_000_000_000_i64
            ],
        ),
        "seeding a playlist must work"
    );
    assert!(
        try_insert(
            store,
            "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, position) \
             VALUES (?1, ?2, NULL, ?3, ?4)",
            rusqlite::params![root, playlist_id, "all", 0_i64],
        ),
        "seeding the parentless Root Box must work"
    );
    assert!(
        try_insert(
            store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                format!("{playlist_id}-rule-root"),
                root,
                "genre",
                "contains",
                false,
                "rock",
                0_i64
            ],
        ),
        "seeding a Rule in the Root Box must work"
    );
    assert!(
        try_insert(
            store,
            "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, position) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![child, playlist_id, root, "any", 1_i64],
        ),
        "seeding a child box must work"
    );
    assert!(
        try_insert(
            store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                format!("{playlist_id}-rule-child"),
                child,
                "year",
                "greater_than",
                true,
                "1975",
                0_i64
            ],
        ),
        "seeding a Rule in the child box must work"
    );
}

fn count_of(store: &SqliteStore, sql: &str) -> i64 {
    let counts = query_i64s(store, sql);
    *counts
        .first()
        .expect("a COUNT(*) query returns exactly one row")
}

#[test]
fn test_migration_020_accepts_every_id_the_rule_catalog_legalises() {
    let (_dir, _path, store) = fresh_store();
    seed_two_level_tree(&store, "p1");

    // The CHECK lists are the union of the catalog's legal ids, so every one
    // of them must be insertable — not just the handful a query path happens
    // to exercise.
    for field_id in [
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
    ] {
        assert!(
            try_insert(
                &store,
                "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
                 VALUES (?1, 'p1-root', ?2, 'is', 0, 'x', 9)",
                rusqlite::params![format!("legal-field-{field_id}"), field_id],
            ),
            "field_id {field_id:?} is in the catalog, so the CHECK must accept it"
        );
    }
    for operator_id in [
        "is",
        "contains",
        "greater_than",
        "less_than",
        "equal_to",
        "in_last",
        "before",
        "after",
        "is_missing",
    ] {
        assert!(
            try_insert(
                &store,
                "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
                 VALUES (?1, 'p1-root', 'title', ?2, 0, 'x', 9)",
                rusqlite::params![format!("legal-op-{operator_id}"), operator_id],
            ),
            "operator_id {operator_id:?} is one of the nine, so the CHECK must accept it"
        );
    }
}

#[test]
fn test_migration_020_refuses_ids_and_flags_outside_its_check_lists() {
    let (_dir, _path, store) = fresh_store();
    seed_two_level_tree(&store, "p1");

    // The database, not a later Rust validator, is what refuses an id the
    // catalog does not legalise.
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES ('bad-field', 'p1-root', 'bitrate', 'is', 0, 'x', 9)",
            rusqlite::params![],
        ),
        "a field_id outside the catalog must be refused by the database"
    );
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES ('bad-op', 'p1-root', 'title', 'is_not', 0, 'x', 9)",
            rusqlite::params![],
        ),
        "an operator_id outside the catalog must be refused by the database"
    );
    // Nine operators, not ten: "is not missing" has no stored spelling,
    // because `negated` already inverts `is_missing`, and a meaning with two
    // spellings makes the editor's "has this changed?" comparison lie.
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES ('not-missing', 'p1-root', 'title', 'is_not_missing', 0, NULL, 9)",
            rusqlite::params![],
        ),
        "`is_not_missing` must not be a legal stored operator"
    );
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, position) \
             VALUES ('p1-both', 'p1', 'p1-root', 'both', 2)",
            rusqlite::params![],
        ),
        "`combine` is `all` or `any` and nothing else"
    );
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES ('bad-negated', 'p1-root', 'title', 'is', 2, 'x', 9)",
            rusqlite::params![],
        ),
        "`negated` is 0 or 1"
    );
    // The default is the un-inverted test, so a rule can be inserted without
    // naming `negated` at all.
    assert!(
        try_insert(
            &store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, value, position) \
             VALUES ('default-negated', 'p1-root', 'title', 'is', 'x', 9)",
            rusqlite::params![],
        ) && count_of(
            &store,
            "SELECT negated FROM smart_rules WHERE id = 'default-negated'"
        ) == 0,
        "`negated` defaults to 0, the un-inverted test"
    );
}

#[test]
fn test_migration_020_leaves_the_value_column_plain_text() {
    let (_dir, _path, store) = fresh_store();
    seed_two_level_tree(&store, "p1");

    // `value` stays plain nullable TEXT, exactly as the draft writes it. The
    // rule "NULL only for `is_missing`" belongs to the save path, so the
    // schema must NOT reject a NULL value on some other operator — a
    // cross-column CHECK here would be an improvement the draft does not
    // carry, and it could not see an operator's meaning anyway.
    assert!(
        try_insert(
            &store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES ('null-value', 'p1-root', 'title', 'is', 0, NULL, 9)",
            rusqlite::params![],
        ),
        "`value` is plain TEXT: the NULL-only-for-`is_missing` rule is the save path's"
    );
    // And it holds the field-catalog's text spellings verbatim — year, whole
    // milliseconds, a `YYYY-MM-DD HH:MM:SS` instant, a favourite `1`.
    for value in ["1975", "183000", "2024-03-01 09:30:00", "1", ""] {
        assert!(
            try_insert(
                &store,
                "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
                 VALUES (?1, 'p1-root', 'year', 'is', 0, ?2, 9)",
                rusqlite::params![format!("text-value-{value}"), value],
            ),
            "value {value:?} is stored verbatim as text"
        );
    }
}

/// `play_count` and `favorite` are `NOT NULL DEFAULT 0` on the `tracks` row,
/// so they have no missing-value spelling to store a rule about: the catalog
/// legalises both as fields, and the column shapes are what make `is_missing`
/// meaningless for them. Asserted here because it is a storage fact, not a
/// UI one.
#[test]
fn test_play_count_and_favorite_have_no_missing_value_spelling() {
    let (_dir, _path, store) = fresh_store();

    let not_null = query_i64s(
        &store,
        "SELECT COUNT(*) FROM pragma_table_info('tracks') \
         WHERE name IN ('play_count', 'favorite') AND \"notnull\" = 1",
    );
    assert_eq!(
        not_null.first(),
        Some(&2),
        "`play_count` and `favorite` are NOT NULL columns, so a rule over them \
         is never a missing-value test"
    );
}

#[test]
fn test_migration_020_enforces_the_cascade_only_foreign_keys() {
    let (_dir, _path, store) = fresh_store();
    seed_two_level_tree(&store, "p1");

    // The foreign keys are live, which is what makes cascade-only deletion the
    // whole ownership story: nothing can hang off a box or playlist that never
    // existed.
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, position) \
             VALUES ('orphan-box', 'no-such-playlist', NULL, 'all', 0)",
            rusqlite::params![],
        ),
        "a box may not point at a playlist that does not exist"
    );
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, position) \
             VALUES ('orphan-parent', 'p1', 'no-such-box', 'all', 0)",
            rusqlite::params![],
        ),
        "a box may not point at a parent box that does not exist"
    );
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES ('orphan-rule', 'no-such-box', 'title', 'is', 0, 'x', 0)",
            rusqlite::params![],
        ),
        "a rule may not point at a box that does not exist"
    );
    assert!(
        !try_insert(
            &store,
            "INSERT INTO smart_playlists (id, name, created_at) VALUES ('p2', 'list p1', 1)",
            rusqlite::params![],
        ),
        "the playlist name is UNIQUE as written"
    );
}

#[test]
fn test_deleting_a_playlist_cascades_boxes_descendant_boxes_and_every_rule() {
    let (_dir, _path, store) = fresh_store();
    seed_two_level_tree(&store, "p1");
    seed_two_level_tree(&store, "p2");

    assert!(
        try_insert(&store, "DELETE FROM smart_playlists WHERE id = 'p1'", &[]),
        "deleting the playlist row must work"
    );

    // The whole tree went with it: the Root Box, the child box beneath it, and
    // the Rule in each of the two boxes.
    assert_eq!(
        count_of(
            &store,
            "SELECT COUNT(*) FROM smart_rule_boxes WHERE playlist_id = 'p1'"
        ),
        0,
        "deleting a playlist removes its boxes"
    );
    assert_eq!(
        count_of(
            &store,
            "SELECT COUNT(*) FROM smart_rules \
             WHERE box_id IN ('p1-root', 'p1-child')",
        ),
        0,
        "deleting a playlist removes every rule, including one nested two levels down"
    );

    // Nothing orphaned, and nothing collateral: the untouched playlist keeps
    // its full five-row tree.
    assert_eq!(
        count_of(
            &store,
            "SELECT COUNT(*) FROM smart_rule_boxes \
             WHERE playlist_id NOT IN (SELECT id FROM smart_playlists)",
        ),
        0,
        "no box may outlive the playlist it belongs to"
    );
    assert_eq!(
        count_of(
            &store,
            "SELECT COUNT(*) FROM smart_rule_boxes \
             WHERE parent_id NOT IN (SELECT id FROM smart_rule_boxes)",
        ),
        0,
        "no box may keep pointing at a parent the cascade removed"
    );
    assert_eq!(
        count_of(
            &store,
            "SELECT COUNT(*) FROM smart_rules \
             WHERE box_id NOT IN (SELECT id FROM smart_rule_boxes)",
        ),
        0,
        "no rule may outlive its box"
    );
    assert_eq!(
        count_of(
            &store,
            "SELECT COUNT(*) FROM smart_rule_boxes WHERE playlist_id = 'p2'"
        ),
        2,
        "the other playlist's Root Box and its child box are untouched"
    );
    assert_eq!(
        count_of(&store, "SELECT COUNT(*) FROM smart_rules"),
        2,
        "only the deleted playlist's two rules are gone"
    );
}

#[test]
fn test_deleting_a_parent_box_cascades_through_the_self_referential_link() {
    let (_dir, _path, store) = fresh_store();
    seed_two_level_tree(&store, "p1");

    assert!(
        try_insert(
            &store,
            "DELETE FROM smart_rule_boxes WHERE id = 'p1-root'",
            &[]
        ),
        "deleting the Root Box must work"
    );

    // The child box went through the box-to-box cascade, and both rules with
    // the boxes that held them.
    assert_eq!(
        count_of(&store, "SELECT COUNT(*) FROM smart_rule_boxes"),
        0,
        "a parent box's cascade reaches its child boxes"
    );
    assert_eq!(
        count_of(&store, "SELECT COUNT(*) FROM smart_rules"),
        0,
        "and the rules under every descendant box"
    );
    // The playlist row itself survives: it owns the tree, the tree does not
    // own it.
    assert_eq!(
        count_of(&store, "SELECT COUNT(*) FROM smart_playlists"),
        1,
        "deleting a box never takes the playlist row with it"
    );
}

#[test]
fn test_reopening_a_store_written_by_this_build_reapplies_nothing() {
    let (_dir, db_path, store) = fresh_store();
    // A sentinel the re-application would overwrite, on this ticket's own row.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE schema_migrations SET applied_at = 424242 WHERE version = 20",
                rusqlite::params![],
            )
        })
        .expect("marking the migration row must work");
    drop(store);

    // A second open of the same file. Migration 020 writes plain `CREATE TABLE`
    // with no `IF NOT EXISTS`, so if the recorded row were skipped over and the
    // SQL re-run, this open would fail outright rather than silently pass.
    let reopened = open(&db_path);

    assert_eq!(
        query_i64s(
            &reopened,
            "SELECT applied_at FROM schema_migrations WHERE version = 20",
        ),
        vec![424_242],
        "reopening must reuse migration 020's recorded row, not reapply it"
    );
    assert_eq!(
        count_of(&reopened, "SELECT COUNT(*) FROM schema_migrations"),
        21,
        "no duplicate bookkeeping row"
    );
    assert_eq!(
        count_of(
            &reopened,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'smart%'",
        ),
        3,
        "and no second copy of any rule table"
    );
}

/// A rolled-back store — migration 020's row forgotten and its three tables
/// dropped — so the next open genuinely re-applies the pending migration.
/// Later-shipped migrations (021) stay recorded: the point is a pending 020,
/// not a period-accurate store.
fn roll_back_to_019(store: &mut SqliteStore) {
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "DELETE FROM schema_migrations WHERE version = 20;
                 DROP TABLE smart_rules;
                 DROP TABLE smart_rule_boxes;
                 DROP TABLE smart_playlists;",
            )
        })
        .expect("rolling back to the pre-020 schema must work");
    let versions = query_i64s(
        store,
        "SELECT version FROM schema_migrations ORDER BY version",
    );
    assert_eq!(
        versions.last(),
        Some(&21),
        "the rolled-back store forgets only migration 020's row; every other recorded migration stays"
    );
    assert_eq!(
        count_of(
            store,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'smart%'",
        ),
        0,
        "and must hold none of this migration's tables"
    );
}

/// Every row of the tables migration 020 must leave alone, as flat text.
/// Comparing this dump before and across the upgrade is what makes "nothing
/// was touched" an assertion rather than a hope.
fn existing_data_dump(store: &SqliteStore) -> Vec<String> {
    let mut dumps = Vec::new();
    for (label, sql) in [
        (
            "artists",
            "SELECT 'artists|' || name FROM artists ORDER BY name",
        ),
        (
            "albums",
            "SELECT 'albums|' || album_artist || '|' || title || '|' \
             || IFNULL(year, -1) || '|' || IFNULL(genre, '') \
             FROM albums ORDER BY album_artist, title",
        ),
        (
            "tracks",
            "SELECT 'tracks|' || path || '|' || IFNULL(title, '') || '|' \
             || IFNULL(genre, '') || '|' || IFNULL(year, -1) || '|' \
             || IFNULL(duration_nanos, -1) || '|' || IFNULL(sample_rate, -1) || '|' \
             || IFNULL(channels, -1) || '|' || play_count || '|' \
             || IFNULL(last_played_nanos, -1) || '|' || IFNULL(date_added_nanos, -1) || '|' \
             || favorite || '|' || search_text || '|' || album_artist_key || '|' \
             || album_title_key \
             FROM tracks ORDER BY path",
        ),
        (
            "playlists",
            "SELECT 'playlists|' || id || '|' || name || '|' || created_at \
             FROM playlists ORDER BY id",
        ),
        (
            "playlist_entries",
            "SELECT 'playlist_entries|' || playlist_id || '|' || position || '|' || track_id \
             FROM playlist_entries ORDER BY playlist_id, position",
        ),
        (
            "app_settings",
            "SELECT 'app_settings|' || id || '|' || IFNULL(volume, -1) || '|' \
             || advanced_mode || '|' || high_contrast || '|' || replaygain_enabled || '|' \
             || shuffle || '|' || repeat_mode || '|' || smart_lists_collapsed || '|' || close_quits_app || '|' \
             || metadata_version || '|' || replaygain_mode || '|' \
             || replaygain_pass_track || '|' || replaygain_pass_album || '|' || reduce_motion \
             FROM app_settings",
        ),
        (
            "library_paths",
            "SELECT 'library_paths|' || path FROM library_paths ORDER BY path",
        ),
        (
            "watch_states",
            "SELECT 'watch_states|' || path || '|' || state || '|' || IFNULL(warning_message, '') \
             FROM watch_states ORDER BY path",
        ),
        (
            "store_metadata",
            "SELECT 'store_metadata|' || key || '=' || value FROM store_metadata ORDER BY key",
        ),
    ] {
        let rows = query_strings(store, sql);
        assert!(
            !rows.is_empty(),
            "the seeded {label} table must hold rows for the comparison to mean anything"
        );
        dumps.extend(rows);
    }
    dumps
}

/// Populate the pre-existing schema through the open connection — the seed is
/// deliberately raw SQL so this suite depends on no store port beyond opening
/// and migrating.
fn seed_existing_data(store: &SqliteStore) {
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "INSERT INTO artists (name) VALUES ('Zeta'), ('Alpha');
                 INSERT INTO albums (album_artist, title, year, genre)
                   VALUES ('Zeta', 'New Wave', 2019, 'electronic'),
                          ('Alpha', 'Early Work', NULL, NULL);
                 INSERT INTO tracks (path, title, artist, album, album_artist, track_number,
                                     genre, year, composer, comment, duration_nanos, sample_rate,
                                     channels, play_count, last_played_nanos, date_added_nanos,
                                     search_text, album_artist_key, album_title_key, favorite)
                   VALUES ('m:/zeta/01.mp3', 'Opening', 'Zeta', 'New Wave', 'Zeta', 1,
                           'electronic', 2019, 'Zeta', 'note', 183000000000, 44100,
                           2, 7, 1700000000000000000, 1690000000000000000,
                           'opening zeta', 'Zeta', 'New Wave', 1),
                          ('m:/alpha/01.wav', 'Study', 'Alpha', 'Early Work', 'Alpha', NULL,
                           NULL, NULL, NULL, NULL, 61500000000, 48000,
                           1, 0, NULL, NULL,
                           'study alpha', 'Alpha', 'Early Work', 0);
                 INSERT INTO playlists (id, name, created_at)
                   VALUES ('pl-1', 'Road Trip', 1690000000000000000),
                          ('pl-2', 'Gym', 1695000000000000000);
                 INSERT INTO playlist_entries (playlist_id, position, track_id)
                   VALUES ('pl-1', 0, 'm:/zeta/01.mp3'),
                          ('pl-1', 1, 'm:/alpha/01.wav'),
                          ('pl-2', 0, 'm:/zeta/01.mp3');
                 INSERT INTO store_metadata (key, value)
                   VALUES ('last_full_scan', 'the seeded scan summary');
                 INSERT INTO library_paths (path) VALUES ('m:/zeta'), ('m:/alpha');
                 INSERT INTO watch_states (path, state, warning_message)
                   VALUES ('m:/zeta', 'enabled', NULL),
                          ('m:/alpha', 'warning', 'volume vanished');
                 UPDATE app_settings
                   SET volume = 0.42, advanced_mode = 1, high_contrast = 1,
                       replaygain_enabled = 1, shuffle = 1, repeat_mode = 2,
                       smart_lists_collapsed = 1, close_quits_app = 1,
                       replaygain_mode = 1, replaygain_pass_track = 1,
                       replaygain_pass_album = 1, reduce_motion = 1;",
            )
        })
        .expect("seeding the pre-existing schema must work");
}

#[test]
fn test_migration_020_upgrades_an_existing_store_without_touching_its_data() {
    let (_dir, db_path, mut store) = fresh_store();
    seed_existing_data(&store);
    store
        .stamp_metadata_version(METADATA_VERSION)
        .expect("stamping the metadata version must work");

    let before = existing_data_dump(&store);
    // A rule table already holding data, so the rollback provably re-creates
    // it rather than leaving a stale copy behind.
    seed_two_level_tree(&store, "p1");
    roll_back_to_019(&mut store);
    drop(store);

    let upgraded = open(&db_path);

    assert_eq!(
        existing_data_dump(&upgraded),
        before,
        "migrating forward must not touch tracks, albums, artists, playlists, \
         playlist entries, settings or store metadata — not one row, not one column value"
    );
    assert_eq!(
        upgraded
            .metadata_version()
            .expect("reading the metadata version must work"),
        METADATA_VERSION,
        "migration 020 must not move the metadata version: nothing here re-reads a file"
    );
    assert_eq!(
        query_i64s(
            &upgraded,
            "SELECT version FROM schema_migrations ORDER BY version",
        ),
        expected_versions(),
        "and the reopened store lands back on the full inventory"
    );
    assert_eq!(
        count_of(
            &upgraded,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'smart%'",
        ),
        3,
        "the three rule tables arrive on the upgrade"
    );
    assert_eq!(
        count_of(&upgraded, "SELECT COUNT(*) FROM smart_playlists"),
        0,
        "and they arrive empty: nothing is backfilled"
    );
}
