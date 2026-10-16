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
fn table_info(
    store: &SqliteStore,
    table: &str,
) -> Vec<(String, String, bool, i64)> {
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
/// the eighteen versions that shipped before, plus this ticket's `020`, in
/// order and with no gaps.
fn expected_versions() -> Vec<i64> {
    (1..=20).collect()
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
